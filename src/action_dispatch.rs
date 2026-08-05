//! Action dispatch: map an [`Action`] to concrete mutations on [`App`].
//!
//! The entry point is [`apply_action`], which fans out to four specialized
//! sub-dispatch functions. [`handle_normal`] ties together key resolution
//! and action dispatch for the Normal/Visual modes.

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use tuxtime::action::Action;
use tuxtime::app::{App, CalendarTarget, Mode, View};
use tuxtime::keybinds::KeyBindings;
use tuxtime::theme;
use tuxtime::{clipboard, todo};

use super::key_resolve::resolve_normal_key;
use super::timesheet_handler::handle_timesheet_keys;

// ---------------------------------------------------------------------------
// Top-level dispatch
// ---------------------------------------------------------------------------

pub(crate) fn apply_action(app: &mut App, action: Action) {
    if app.view() == View::Archive && !apply_archive_action(app, action) {
        return;
    }
    apply_cursor_actions(app, action);
    apply_mutation_actions(app, action);
    apply_overlay_actions(app, action);
}

// ---------------------------------------------------------------------------
// Archive view guard
// ---------------------------------------------------------------------------

/// Returns `false` when the action was blocked (read-only flash).
fn apply_archive_action(app: &mut App, action: Action) -> bool {
    match action {
        Action::ToggleComplete => {
            if let Some(idx) = app.cur_abs() {
                app.unarchive(idx);
            }
            return false;
        }
        Action::Delete => {
            if let Some(idx) = app.cur_abs() {
                app.archive_delete(idx);
            }
            return false;
        }
        Action::BeginAdd
        | Action::BeginEdit
        | Action::BeginEditInsert
        | Action::CyclePriority
        | Action::ToggleVisual
        | Action::ToggleSelected
        | Action::BeginSearch
        | Action::BeginPromptProject
        | Action::BeginPromptContext
        | Action::PickProject
        | Action::PickContext
        | Action::PickSavedFilter
        | Action::SaveCurrentFilter
        | Action::CycleSort
        | Action::ToggleShowDone
        | Action::ToggleShowFuture
        | Action::Undo => {
            app.flash("read-only in archive");
            return false;
        }
        _ => {}
    }
    true
}

// ---------------------------------------------------------------------------
// Cursor movement, view switching, pane toggles, quit
// ---------------------------------------------------------------------------

fn apply_cursor_actions(app: &mut App, action: Action) {
    let len = app.visible_indices().len();
    match action {
        Action::Quit => {
            app.stop_timer_on_quit();
            app.nav.should_quit = true;
        }
        Action::CursorDown => {
            if len > 0 {
                app.nav.cursor = (app.nav.cursor + 1).min(len - 1);
            }
        }
        Action::CursorUp => app.nav.cursor = app.nav.cursor.saturating_sub(1),
        Action::CursorTop => app.nav.cursor = 0,
        Action::CursorBottom => {
            if len > 0 {
                app.nav.cursor = len - 1;
            }
        }
        Action::HalfPageDown => app.nav.cursor = (app.nav.cursor + 10).min(len.saturating_sub(1)),
        Action::HalfPageUp => app.nav.cursor = app.nav.cursor.saturating_sub(10),
        Action::GoList => app.set_view(View::List),
        Action::ToggleArchiveView => {
            let next = if app.view() == View::Archive {
                View::List
            } else {
                View::Archive
            };
            app.set_view(next);
        }
        Action::EscapeStack => {
            let has_pc = app.filter().project.is_some() || app.filter().context.is_some();
            let has_search = !app.filter().search.is_empty();
            if has_pc {
                app.set_project_filter(None);
                app.set_context_filter(None);
            } else if has_search {
                app.draft_clear();
                app.clear_search();
            } else if !app.selection.is_empty() {
                app.selection.clear();
            } else if app.nav.mode == Mode::Visual {
                app.nav.mode = Mode::Normal;
            } else if app.view() != View::List {
                app.set_view(View::List);
            }
        }
        Action::OpenTimesheet => {
            let next = if app.view() == View::Timesheet {
                View::List
            } else {
                app.timesheet.date = app.today().to_string();
                app.timesheet.cursor = 0;
                View::Timesheet
            };
            app.set_view(next);
        }
        Action::ToggleLeftPane => {
            app.prefs.toggle_left();
            app.save_prefs();
        }
        Action::ToggleRightPane => {
            app.prefs.toggle_right();
            app.save_prefs();
        }
        Action::CycleTheme => app.cycle_theme(),
        Action::CycleDensity => app.cycle_density(),
        Action::CycleSort => app.cycle_sort(),
        Action::ToggleLineNum => {
            app.prefs.toggle_line_num();
            app.save_prefs();
        }
        Action::ToggleShowDone => {
            app.prefs.toggle_show_done();
            app.nav.cursor = 0;
            app.recompute_visible();
            app.save_prefs();
        }
        Action::ToggleShowFuture => {
            app.prefs.toggle_show_future();
            app.nav.cursor = 0;
            app.recompute_visible();
            app.save_prefs();
        }
        Action::ChangeWeekStart => app.cycle_week_start(),
        Action::DismissNudge => {
            app.nav.mode = Mode::Normal;
            app.session.last_timer_activity = std::time::Instant::now();
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Task mutations: complete, delete, priority, billable, archive, undo
// ---------------------------------------------------------------------------

fn apply_mutation_actions(app: &mut App, action: Action) {
    match action {
        Action::ToggleComplete => {
            if app.nav.mode == Mode::Visual && !app.selection.is_empty() {
                app.complete_selected();
            } else if let Some(abs) = app.cur_abs() {
                app.toggle_complete(abs);
            }
        }
        Action::Delete => {
            if app.nav.mode == Mode::Visual && !app.selection.is_empty() {
                app.delete_selected();
            } else if let Some(abs) = app.cur_abs() {
                app.delete(abs);
            }
        }
        Action::CyclePriority => {
            if let Some(abs) = app.cur_abs() {
                app.cycle_priority(abs);
            }
        }
        Action::Undo => app.undo(),
        Action::ToggleSelected => {
            if app.nav.mode == Mode::Visual
                && let Some(abs) = app.cur_abs()
            {
                app.selection.toggle(abs);
            }
        }
        Action::ArchiveCompleted => {
            if app.view() == View::Archive {
                app.flash("already in archive");
            } else if app.has_completed_tasks() {
                app.archive_completed();
            } else {
                app.flash("no completed tasks to archive");
            }
        }
        Action::ToggleBillable => app.toggle_billable(),
        Action::QuickInterrupt => app.interrupt_timer(),
        Action::ToggleVisual => {
            app.nav.mode = if app.nav.mode == Mode::Visual {
                Mode::Normal
            } else {
                Mode::Visual
            };
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Mode transitions: insert, search, help, settings, palette, picker, timer
// ---------------------------------------------------------------------------

fn apply_overlay_actions(app: &mut App, action: Action) {
    match action {
        Action::BeginAdd => {
            app.nav.mode = Mode::Insert;
            app.draft_clear();
            app.selection.exit_edit();
            app.session.manual_time_entry = false;
        }
        Action::BeginEdit => {
            if let Some(abs) = app.cur_abs()
                && let Some(raw) = app.task_raw(abs)
            {
                app.selection.enter_edit(abs);
                app.draft_set(raw);
                app.nav.mode = Mode::Insert;
            }
        }
        Action::BeginEditInsert => {
            if let Some(abs) = app.cur_abs()
                && let Some(raw) = app.task_raw(abs)
            {
                app.selection.enter_edit(abs);
                app.draft_set_insert(raw);
                app.nav.mode = Mode::Insert;
            }
        }
        Action::Reschedule => {
            if let Some(abs) = app.cur_abs()
                && let Some(raw) = app.task_raw(abs)
            {
                app.selection.enter_edit(abs);
                app.draft_set_insert(raw);
                app.nav.mode = Mode::Insert;
                app.open_calendar(CalendarTarget::Due);
            }
        }
        Action::BeginSearch => {
            app.nav.mode = Mode::Search;
            app.draft_clear();
            app.clear_search();
        }
        Action::OpenHelp => app.nav.mode = Mode::Help,
        Action::OpenSettings => app.nav.mode = Mode::Settings,
        Action::OpenCommandPalette => {
            let prior = app.nav.mode;
            app.command_palette.open(prior);
            app.nav.mode = Mode::CommandPalette;
            app.draft_clear();
        }
        Action::OpenThemePicker => {
            if theme::all().len() <= 1 {
                app.flash("only one theme");
            } else {
                app.enter_pick_theme();
            }
        }
        Action::TimerStartStop => app.toggle_timer(),
        Action::ManualTimeEntry => app.nav.mode = Mode::ManualEntryChoice,
        Action::BeginSessionFromCurrent => {
            if let Some(t) = app.cur_task() {
                let body = todo::body_only(&t.raw);
                app.draft_clear();
                app.draft_set_insert(format!("{body} dur:"));
                app.session.manual_time_entry = true;
                app.nav.mode = Mode::Insert;
                app.selection.exit_edit();
            } else {
                app.flash("no task to start session from");
            }
        }
        Action::OpenProjectManager => app.nav.mode = Mode::ManageProjects,
        Action::ConfigureIdleNudge => {
            let mins = app.idle_nudge_seconds() / 60;
            app.draft_clear();
            app.draft_set_insert(mins.to_string());
            app.nav.mode = Mode::PromptIdleNudge;
        }
        Action::ConfigureLongTimerNudge => {
            let mins = app.long_timer_nudge_seconds() / 60;
            app.draft_clear();
            app.draft_set_insert(mins.to_string());
            app.nav.mode = Mode::PromptLongTimerNudge;
        }
        Action::OpenShare => match app.ensure_share_started() {
            Ok(_) => app.nav.mode = Mode::Share,
            Err(e) => app.flash(format!("share unavailable: {e}")),
        },
        Action::ArmF => app.chord.arm('f'),
        Action::PickProject => app.enter_pick_project(),
        Action::PickContext => app.enter_pick_context(),
        Action::PickSavedFilter => app.enter_pick_saved(),
        Action::SaveCurrentFilter => {
            if app.filter().search.is_empty() {
                app.flash("no active search to save");
            } else {
                app.nav.mode = Mode::PromptSaveFilter;
                app.draft_clear();
            }
        }
        Action::BeginPromptProject => {
            app.nav.mode = Mode::PromptProject;
            app.draft_clear();
        }
        Action::BeginPromptContext => {
            app.nav.mode = Mode::PromptContext;
            app.draft_clear();
        }
        Action::CopyLine => copy_current_task(app, false),
        Action::CopyBody => copy_current_task(app, true),
        Action::CopyNarratives => app.flash("open the timesheet (V) to copy narratives"),
        Action::OpenNote => app.open_note_for_current(),
        Action::CreateOrOpenNote => app.create_or_open_note_for_current(),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Normal/Visual mode key handler
// ---------------------------------------------------------------------------

pub(crate) fn handle_normal(app: &mut App, key: KeyEvent, keybinds: &KeyBindings) {
    if app.view() == View::Timesheet
        && !matches!(
            key.code,
            KeyCode::Esc | KeyCode::Char('V') | KeyCode::Char('Z') | KeyCode::Char('/')
        )
    {
        handle_timesheet_keys(app, key);
        return;
    }
    if let Some(action) = resolve_normal_key(app, key, keybinds) {
        apply_action(app, action);
    }
    app.clamp_cursor();
}

// ---------------------------------------------------------------------------
// Copy helper (shared by CopyLine / CopyBody actions)
// ---------------------------------------------------------------------------

fn copy_current_task(app: &mut App, body_only: bool) {
    let Some(raw) = app.cur_task().map(|t| t.raw.clone()) else {
        return;
    };
    let payload = if body_only {
        todo::body_only(&raw)
    } else {
        raw
    };
    match clipboard::copy(&payload) {
        Ok(()) => app.flash(if body_only { "copied (body)" } else { "copied" }),
        Err(e) => app.flash(format!("copy failed: {e}")),
    }
}
