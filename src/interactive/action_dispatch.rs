//! Action dispatch: map an [`Action`] to concrete mutations on [`App`].
//!
//! The entry point is [`apply_action`], which fans out to four specialized
//! sub-dispatch functions. [`handle_normal`] ties together key resolution
//! and action dispatch for the Normal/Visual modes.

use crate::action::Action;
use crate::app::{App, CalendarTarget, Mode, View};
use crate::keybinds::KeyBindings;
use crate::theme;
use crate::{clipboard, todo};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::key_resolve::resolve_normal_key;
use super::timesheet_handler::handle_timesheet_keys;

// ---------------------------------------------------------------------------
// Top-level dispatch
// ---------------------------------------------------------------------------

/// Apply one [`Action`] to the app. This is a **single exhaustive match**:
/// every `Action` variant has exactly one arm, and there is deliberately no
/// `_ => {}` catch-all. Adding a variant to `Action` therefore fails to
/// compile until it is routed here — the compiler enforces that no action is
/// silently dropped.
pub fn apply_action(app: &mut App, action: Action) {
    // Archive view guard: redirect or block actions that can't touch the
    // live list, then fall through to the shared exhaustive dispatch below.
    if app.view() == View::Archive && !apply_archive_action(app, action) {
        return;
    }
    match action {
        // ---- lifecycle / cursor / view ----------------------------------
        Action::Quit => {
            app.stop_timer_on_quit();
            app.nav.quit();
        }
        Action::CursorDown => {
            app.nav
                .move_down(app.visible_indices().len().saturating_sub(1));
        }
        Action::CursorUp => app.nav.move_up(),
        Action::CursorTop => app.nav.move_top(),
        Action::CursorBottom => {
            app.nav
                .move_bottom(app.visible_indices().len().saturating_sub(1));
        }
        Action::HalfPageDown => app.nav.move_down_by(10, app.visible_indices().len()),
        Action::HalfPageUp => app.nav.move_up_by(10),
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
            } else if app.nav.is_visual() {
                app.nav.enter_normal();
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
            app.nav.move_top();
            app.recompute_visible();
            app.save_prefs();
        }
        Action::ToggleShowFuture => {
            app.prefs.toggle_show_future();
            app.nav.move_top();
            app.recompute_visible();
            app.save_prefs();
        }
        Action::ChangeWeekStart => app.cycle_week_start(),
        Action::DismissNudge => {
            app.nav.enter_normal();
            app.session.last_timer_activity = std::time::Instant::now();
        }

        // ---- task mutations ---------------------------------------------
        Action::ToggleComplete => {
            if app.nav.is_visual() && !app.selection.is_empty() {
                app.complete_selected();
            } else if let Some(abs) = app.cur_abs() {
                app.toggle_complete(abs);
            }
        }
        Action::Delete => {
            if app.nav.is_visual() && !app.selection.is_empty() {
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
            if app.nav.is_visual()
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
        Action::ToggleVisual => app.nav.toggle_visual(),

        // ---- mode transitions / overlays --------------------------------
        Action::BeginAdd => {
            app.nav.set_mode(Mode::Insert);
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
                app.nav.set_mode(Mode::Insert);
            }
        }
        Action::BeginEditInsert => {
            if let Some(abs) = app.cur_abs()
                && let Some(raw) = app.task_raw(abs)
            {
                app.selection.enter_edit(abs);
                app.draft_set_insert(raw);
                app.nav.set_mode(Mode::Insert);
            }
        }
        Action::Reschedule => {
            if let Some(abs) = app.cur_abs()
                && let Some(raw) = app.task_raw(abs)
            {
                app.selection.enter_edit(abs);
                app.draft_set_insert(raw);
                app.nav.set_mode(Mode::Insert);
                app.open_calendar(CalendarTarget::Due);
            }
        }
        Action::BeginSearch => {
            app.nav.push_mode(Mode::Search);
            app.draft_clear();
            app.clear_search();
        }
        Action::OpenHelp => app.nav.set_mode(Mode::Help),
        Action::OpenSettings => app.nav.set_mode(Mode::Settings),
        Action::OpenCommandPalette => {
            app.command_palette.open();
            app.nav.push_mode(Mode::CommandPalette);
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
        Action::ManualTimeEntry => app.nav.set_mode(Mode::ManualEntryChoice),
        Action::BeginSessionFromCurrent => {
            let Some(abs) = app.cur_abs() else {
                app.flash("no task to start session from");
                return;
            };
            let (done, running, raw, priority) = {
                let t = &app.store.tasks()[abs];
                (
                    t.done,
                    app.is_timer_running_on(abs),
                    t.raw.clone(),
                    t.priority,
                )
            };
            if done {
                app.flash("cannot carry a completed task");
            } else if running {
                app.flash("stop the timer first (t)");
            } else {
                // Upgraded carry-forward: pre-fill the Insert dialog with the
                // carried-over line (body + projects + contexts + billable,
                // priority preserved) so the narrative can be polished. On
                // save the source line is consumed and this becomes today's
                // entry; no timer is started.
                let body = crate::core::carry_forward_body(&raw);
                let draft = match priority {
                    Some(p) => format!("({p}) {body}"),
                    None => body,
                };
                app.session.carry_forward_from = Some(abs);
                app.draft_clear();
                app.draft_set_insert(draft);
                app.session.manual_time_entry = false;
                app.nav.set_mode(Mode::Insert);
                app.selection.exit_edit();
            }
        }
        Action::OpenProjectManager => app.nav.set_mode(Mode::ManageProjects),
        Action::ConfigureIdleNudge => {
            let mins = app.idle_nudge_seconds() / 60;
            app.draft_clear();
            app.draft_set_insert(mins.to_string());
            // Push so the prompt pops back to the mode the palette returned to.
            app.nav.push_mode(Mode::PromptIdleNudge);
        }
        Action::ConfigureLongTimerNudge => {
            let mins = app.long_timer_nudge_seconds() / 60;
            app.draft_clear();
            app.draft_set_insert(mins.to_string());
            // Push so the prompt pops back to the mode the palette returned to.
            app.nav.push_mode(Mode::PromptLongTimerNudge);
        }
        Action::OpenShare => match app.ensure_share_started() {
            Ok(_) => app.nav.set_mode(Mode::Share),
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
                app.nav.set_mode(Mode::PromptSaveFilter);
                app.draft_clear();
            }
        }
        Action::BeginPromptProject => {
            app.nav.set_mode(Mode::PromptProject);
            app.draft_clear();
        }
        Action::BeginPromptContext => {
            app.nav.set_mode(Mode::PromptContext);
            app.draft_clear();
        }
        Action::CopyLine => copy_current_task(app, false),
        Action::CopyBody => copy_current_task(app, true),
        Action::CopyNarratives => app.flash("open the timesheet (V) to copy narratives"),
        Action::OpenNote => app.open_note_for_current(),
        Action::CreateOrOpenNote => app.create_or_open_note_for_current(),
    }
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
    let Some(task) = app.cur_task() else {
        return;
    };
    let payload = if body_only {
        todo::body_only_from_clean(&task.clean_raw)
    } else {
        task.raw.clone()
    };
    match clipboard::copy(&payload) {
        Ok(()) => app.flash(if body_only { "copied (body)" } else { "copied" }),
        Err(e) => app.flash(format!("copy failed: {e}")),
    }
}
