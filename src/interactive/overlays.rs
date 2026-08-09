//! Overlay/modal key handlers for the interactive layer. Each function
//! handles one transient mode (welcome, search, help, settings, pickers,
//! command palette, prompts, project manager, idle nudge, etc.).
//!
//! Public entry points:
//! - [`handle_autocomplete_keys`] — shared by Insert, Search, Prompt modes
//! - All other handlers are `pub(crate)` for use by [`handle_key`].

use crate::app::{App, DayBoundaryAction, Mode, View};
use crate::cli;
use crate::todo;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::action_dispatch::apply_action;
use super::insert::{DraftEffect, apply_to_draft};

// ---------------------------------------------------------------------------
// Welcome & share — first-run and network overlay
// ---------------------------------------------------------------------------

/// First-run welcome prompt. `c` creates `./todo.txt`; `s` opens the sample;
/// `q`/`Esc` quits without creating anything.
pub(crate) fn handle_welcome(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('c') => match cli::ensure_file(app.env.file_path.clone()) {
            Ok(_) => app.nav.enter_normal(),
            Err(e) => app.flash(format!(
                "could not create {}: {e}",
                app.env.file_path.display()
            )),
        },
        KeyCode::Char('s') => match cli::sample_path() {
            Ok(sample) => {
                let done = cli::done_path(&sample);
                let body = std::fs::read_to_string(&sample).unwrap_or_default();
                app.open_file(sample, done, body);
                app.nav.enter_normal();
            }
            Err(e) => app.flash(format!("could not open sample: {e}")),
        },
        KeyCode::Char('q') | KeyCode::Esc => app.nav.quit(),
        _ => {}
    }
}

/// Share overlay: any key dismisses, returning to Normal. The server
/// keeps running in the background.
pub(crate) fn handle_share(app: &mut App, _key: KeyEvent) {
    app.nav.enter_normal();
}

// ---------------------------------------------------------------------------
// Search, help, settings — transient overlays
// ---------------------------------------------------------------------------

pub(crate) fn handle_search(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.nav.pop_mode();
            app.draft_clear();
            app.clear_search();
        }
        KeyCode::Enter => {
            app.nav.pop_mode();
            app.nav.move_top();
        }
        _ => {
            if apply_to_draft(app, key) == DraftEffect::TextChanged {
                app.set_search(app.draft.text().to_string());
            }
        }
    }
}

pub(crate) fn handle_help(app: &mut App, key: KeyEvent) {
    if matches!(key.code, KeyCode::Esc | KeyCode::Char('?' | 'q')) {
        app.nav.enter_normal();
    }
}

pub(crate) fn handle_settings(app: &mut App, key: KeyEvent) {
    if matches!(key.code, KeyCode::Esc | KeyCode::Char(',' | 'q')) {
        app.nav.enter_normal();
        return;
    }
    match key.code {
        KeyCode::Char('i') => {
            let mins = app.idle_nudge_seconds() / 60;
            app.draft_clear();
            app.draft_set_insert(mins.to_string());
            // Open the prompt over Settings so Esc/Enter returns to Settings.
            app.nav.push_mode(Mode::PromptIdleNudge);
        }
        KeyCode::Char('l') => {
            let mins = app.long_timer_nudge_seconds() / 60;
            app.draft_clear();
            app.draft_set_insert(mins.to_string());
            // Open the prompt over Settings so Esc/Enter returns to Settings.
            app.nav.push_mode(Mode::PromptLongTimerNudge);
        }
        KeyCode::Char('r') => app.cycle_rounding_increment(),
        // The settings screen advertises these keys in its rows; route them
        // to the same handlers as Normal mode so the hints work *inside*
        // settings instead of being dead text. Each persists via save_prefs.
        KeyCode::Char('D') => app.cycle_density(),
        KeyCode::Char('S') => app.cycle_sort(),
        KeyCode::Char('L') => {
            app.prefs.toggle_line_num();
            app.save_prefs();
        }
        KeyCode::Char('H') => {
            app.prefs.toggle_show_done();
            app.save_prefs();
        }
        KeyCode::Char('F') => {
            app.prefs.toggle_show_future();
            app.save_prefs();
        }
        KeyCode::Char('[') => {
            app.prefs.toggle_left();
            app.save_prefs();
        }
        KeyCode::Char(']') => {
            app.prefs.toggle_right();
            app.save_prefs();
        }
        KeyCode::Char('Z') => app.enter_pick_theme(),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Pickers — project, context, saved-filter, theme
// ---------------------------------------------------------------------------

/// Shared `j`/`k`/`Enter`/`Esc` handling for the list-style pickers (project,
/// context, saved-filter, theme). `step(forward)` moves the highlight,
/// `accept` commits, `cancel` reverts.
fn handle_list_picker(
    app: &mut App,
    key: KeyEvent,
    step: impl Fn(&mut App, bool),
    accept: impl Fn(&mut App),
    cancel: impl Fn(&mut App),
) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => step(app, true),
        KeyCode::Char('k') | KeyCode::Up => step(app, false),
        KeyCode::Enter => accept(app),
        KeyCode::Esc => cancel(app),
        _ => {}
    }
}

pub(crate) fn handle_pick(app: &mut App, key: KeyEvent) {
    handle_list_picker(app, key, App::pick_step, App::pick_accept, App::pick_cancel);
}

pub(crate) fn handle_pick_theme(app: &mut App, key: KeyEvent) {
    handle_list_picker(
        app,
        key,
        App::pick_theme_step,
        App::pick_theme_accept,
        App::pick_theme_cancel,
    );
}

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

pub(crate) fn handle_command_palette(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            app.nav.pop_mode();
            app.draft_clear();
            return;
        }
        KeyCode::Enter => {
            let chosen = app.command_palette.current_action();
            app.nav.pop_mode();
            app.draft_clear();
            if let Some(action) = chosen {
                apply_action(app, action);
            }
            return;
        }
        KeyCode::Down => {
            app.command_palette.step(1);
            return;
        }
        KeyCode::Up => {
            app.command_palette.step(-1);
            return;
        }
        KeyCode::Char('n') if ctrl => {
            app.command_palette.step(1);
            return;
        }
        KeyCode::Char('p') if ctrl => {
            app.command_palette.step(-1);
            return;
        }
        _ => {}
    }
    if apply_to_draft(app, key) == DraftEffect::TextChanged {
        app.command_palette.refresh(app.draft.text());
    }
}

// ---------------------------------------------------------------------------
// Autocomplete keys — shared by Insert, Search, Prompt
// ---------------------------------------------------------------------------

pub(crate) fn handle_autocomplete_keys(app: &mut App, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Up => {
            app.autocomplete_step(false);
            true
        }
        KeyCode::Down => {
            app.autocomplete_step(true);
            true
        }
        KeyCode::Char('n') if ctrl => {
            app.autocomplete_step(true);
            true
        }
        KeyCode::Char('p') if ctrl => {
            app.autocomplete_step(false);
            true
        }
        KeyCode::Esc => {
            app.draft.suppress_autocomplete();
            true
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Prompt — generic text-input prompt (project, context, filter, add-time,
// nudge thresholds, rename project)
// ---------------------------------------------------------------------------

pub(crate) fn handle_prompt(app: &mut App, key: KeyEvent) {
    if app.autocomplete_visible() {
        match key.code {
            KeyCode::Tab => {
                app.autocomplete_accept();
                return;
            }
            _ => {
                if handle_autocomplete_keys(app, key) {
                    return;
                }
            }
        }
    }

    match key.code {
        KeyCode::Esc => {
            let return_mode = if matches!(
                app.nav.mode(),
                Mode::PromptIdleNudge | Mode::PromptLongTimerNudge
            ) {
                // Entered with push_mode (Settings or the mode the command
                // palette returned to), so pop restores the caller.
                app.nav.pop_mode()
            } else if app.nav.mode() == Mode::PromptRenameProject {
                Mode::ManageProjects
            } else if app.nav.mode() == Mode::PromptAddTime && app.session.from_nudge {
                // Esc from the nudge's add-time recovery flow returns to the
                // nudge popup — the reminder survives a cancelled attempt.
                app.session.from_nudge = false;
                Mode::IdleNudge
            } else {
                Mode::Normal
            };
            app.nav.set_mode(return_mode);
            app.draft_clear();
        }
        KeyCode::Enter => {
            let prev_mode = app.nav.mode();
            let value = app.draft.text().to_string();
            app.draft_clear();
            let is_nudge = matches!(
                prev_mode,
                Mode::PromptIdleNudge | Mode::PromptLongTimerNudge
            );
            let is_rename = prev_mode == Mode::PromptRenameProject;
            let return_mode = if is_nudge {
                // Entered with push_mode (Settings or the mode the command
                // palette returned to), so pop restores the caller.
                app.nav.pop_mode()
            } else if is_rename {
                Mode::ManageProjects
            } else {
                Mode::Normal
            };
            app.nav.set_mode(return_mode);
            match prev_mode {
                Mode::PromptProject => app.add_project_to_current(&value),
                Mode::PromptContext => app.toggle_context_on_current(&value),
                Mode::PromptSaveFilter => app.save_current_filter_as(&value),
                Mode::PromptAddTime => {
                    app.add_time_to_current_from_input(&value);
                    // A failed add (invalid duration, write error) from the
                    // nudge's recovery flow must not drop the reminder: if
                    // nothing was recorded and the flow didn't defer to the
                    // day-boundary prompt, return to the popup instead of
                    // Normal. A real save cleared the flag inside
                    // `add_time_to_current_at`, so the flag still being set
                    // here means the capture did not land.
                    if app.session.from_nudge && app.nav.mode() == Mode::Normal {
                        app.session.from_nudge = false;
                        app.nav.set_mode(Mode::IdleNudge);
                    }
                }
                Mode::PromptIdleNudge => {
                    if let Ok(mins) = value.parse::<u64>() {
                        if mins > 0 {
                            app.set_idle_nudge_minutes(mins);
                            app.flash(format!("idle nudge: {mins} min"));
                        } else {
                            app.flash("idle nudge must be at least 1 minute");
                        }
                    } else {
                        app.flash(format!("invalid minutes: {value}"));
                    }
                }
                Mode::PromptLongTimerNudge => {
                    if let Ok(mins) = value.parse::<u64>() {
                        if mins > 0 {
                            app.set_long_timer_nudge_minutes(mins);
                            app.flash(format!("long timer nudge: {mins} min"));
                        } else {
                            app.flash("long timer nudge must be at least 1 minute");
                        }
                    } else {
                        app.flash(format!("invalid minutes: {value}"));
                    }
                }
                Mode::PromptRenameProject => {
                    if let Some(old) = app.project_manager.rename_project_old.take() {
                        app.rename_project(&old, &value);
                    }
                }
                _ => {}
            }
        }
        _ => {
            apply_to_draft(app, key);
        }
    }
}

// ---------------------------------------------------------------------------
// Project manager (Z)
// ---------------------------------------------------------------------------

pub(crate) fn handle_manage_projects(app: &mut App, key: KeyEvent) {
    let filtered = app.filtered_projects();
    let flen = filtered.len();
    match key.code {
        KeyCode::Esc | KeyCode::Char('q' | 'P') => {
            app.nav.enter_normal();
            app.clear_search();
        }
        KeyCode::Char('j') | KeyCode::Down => app.nav.move_down(flen.saturating_sub(1)),
        KeyCode::Char('k') | KeyCode::Up => app.nav.move_up(),
        KeyCode::Char('x') => {
            if let Some(name) = filtered.get(app.nav.cursor) {
                app.toggle_archive_project(name);
            }
        }
        KeyCode::Char('r') => {
            if let Some(name) = filtered.get(app.nav.cursor).cloned() {
                app.project_manager.rename_project_old = Some(name.clone());
                app.draft_clear();
                app.draft_set_insert(name);
                app.nav.set_mode(Mode::PromptRenameProject);
            }
        }
        KeyCode::Char('s') => {
            app.cycle_project_sort();
        }
        // Sidebar toggles work here too — view-independent chrome.
        KeyCode::Char('[') => {
            app.prefs.toggle_left();
            app.save_prefs();
        }
        KeyCode::Char(']') => {
            app.prefs.toggle_right();
            app.save_prefs();
        }
        KeyCode::Char('/') => {
            // Push search over ManageProjects so Esc/Enter pops straight back.
            app.draft_clear();
            app.nav.push_mode(Mode::Search);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Idle nudge (popup when no timer and idle too long)
// ---------------------------------------------------------------------------

/// Idle-nudge popup. The recovery actions (`S` start timer, `M` add time)
/// route through the task picker so they never hit a random task the cursor
/// happens to be on; `N` opens a blank new entry; `D`/`Esc` dismisses.
pub(crate) fn handle_idle_nudge(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('s' | 'S') => app.enter_nudge_picker(crate::app::NudgePickAction::StartTimer),
        KeyCode::Char('m' | 'M') => app.enter_nudge_picker(crate::app::NudgePickAction::AddTime),
        KeyCode::Char('N' | 'n') => {
            // Remember the insert came from the nudge so an Esc-cancel
            // returns to the popup (the reminder survives an aborted
            // recovery) while a save exits to Normal.
            app.session.from_nudge = true;
            app.set_view(View::List);
            app.draft_clear();
            app.session.manual_time_entry = false;
            app.nav.set_mode(Mode::Insert);
            app.selection.exit_edit();
            app.session.last_timer_activity = std::time::Instant::now();
        }
        KeyCode::Char('D') | KeyCode::Esc => {
            // Dismissing the nudge ends any recovery flow for good.
            app.session.from_nudge = false;
            app.nav.enter_normal();
            if let Some(v) = app.session.pre_nudge_view.take() {
                app.set_view(v);
            }
            app.session.last_timer_activity = std::time::Instant::now();
        }
        _ => {}
    }
}

/// End-of-day review nudge: `V` opens the timesheet (today), `M` opens the
/// manual-entry choice, `S`/`Esc` skips — the review won't re-fire today.
pub(crate) fn handle_review_nudge(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('v' | 'V') => {
            app.timesheet.date = app.today().to_string();
            app.timesheet.cursor = 0;
            app.nav.enter_normal();
            app.session.pre_nudge_view = None;
            app.set_view(View::Timesheet);
        }
        KeyCode::Char('m' | 'M') => {
            app.nav.enter_normal();
            app.session.pre_nudge_view = None;
            app.nav.set_mode(Mode::ManualEntryChoice);
        }
        KeyCode::Char('s' | 'S') | KeyCode::Esc => {
            app.nav.enter_normal();
            if let Some(v) = app.session.pre_nudge_view.take() {
                app.set_view(v);
            }
        }
        _ => {}
    }
}

/// Nudge task picker: `j`/`k` move the highlight, `Enter` commits the chosen
/// task (start timer or add time per the action), `Esc` returns to the idle
/// nudge popup.
pub(crate) fn handle_pick_nudge_task(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.nudge_picker_step(true),
        KeyCode::Char('k') | KeyCode::Up => app.nudge_picker_step(false),
        KeyCode::Enter => app.nudge_picker_accept(),
        KeyCode::Esc => app.nudge_picker_cancel(),
        _ => {}
    }
}

/// Stale-timer startup prompt: a timer was running when the app last closed
/// (or was killed) and has since exceeded the long-timer threshold. `S` stops
/// and logs everything, `D` discards the unrecorded gap (no time credited),
/// `K`/`Esc` keep it counting — the safe default that never destroys time
/// without an explicit choice.
pub(crate) fn handle_stale_timer(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('s' | 'S') => {
            app.stop_running_timer();
            app.nav.enter_normal();
            if let Some(v) = app.session.pre_nudge_view.take() {
                app.set_view(v);
            }
            app.session.last_timer_activity = std::time::Instant::now();
        }
        KeyCode::Char('d' | 'D') => {
            app.discard_stale_timer();
            app.nav.enter_normal();
            if let Some(v) = app.session.pre_nudge_view.take() {
                app.set_view(v);
            }
        }
        KeyCode::Char('k' | 'K') | KeyCode::Esc => {
            app.keep_stale_timer();
        }
        _ => {}
    }
}

/// Long-timer nudge popup: the running timer has exceeded the configured
/// threshold. `S` stops it (capturing the elapsed time), `D`/`Esc` dismisses
/// and keeps the timer running. Both return to the pre-nudge view.
pub(crate) fn handle_long_timer_nudge(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('S' | 's') => {
            app.stop_running_timer();
            app.nav.enter_normal();
            if let Some(v) = app.session.pre_nudge_view.take() {
                app.set_view(v);
            }
            app.session.last_timer_activity = std::time::Instant::now();
        }
        KeyCode::Char('D') | KeyCode::Esc => {
            app.nav.enter_normal();
            if let Some(v) = app.session.pre_nudge_view.take() {
                app.set_view(v);
            }
            app.session.last_timer_activity = std::time::Instant::now();
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Manual time entry choice (M) — new entry or add to existing
// ---------------------------------------------------------------------------

pub(crate) fn handle_manual_entry_choice(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('n' | 'N') => {
            app.draft_clear();
            app.session.manual_time_entry = false;
            app.nav.set_mode(Mode::Insert);
            app.selection.exit_edit();
        }
        KeyCode::Char('a' | 'A') => {
            if let Some(t) = app.cur_task() {
                let body = todo::body_only_from_clean(&t.clean_raw);
                app.nav.set_mode(Mode::PromptAddTime);
                app.draft_clear();
                app.flash(format!("add time to: {body}"));
            } else {
                app.nav.enter_normal();
                app.flash("no task selected — navigate and press M A");
                app.session.last_timer_activity = std::time::Instant::now();
            }
        }
        KeyCode::Esc => {
            app.nav.enter_normal();
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Day-boundary prompt — starting a timer (or adding time) on a task whose
// time belongs to a previous day. One line per task-day (tuxtime-spec §3.5).
// ---------------------------------------------------------------------------

pub(crate) fn handle_day_boundary(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('c' | 'C') => {
            // Continue the same entry: normal toggle/add — the user accepted
            // that this moves the entry onto today's sheet.
            let pending = app.session.pending_day_boundary.take();
            app.nav.pop_mode();
            if let Some((abs, action)) = pending {
                match action {
                    DayBoundaryAction::StartTimer => app.toggle_timer_at(abs),
                    DayBoundaryAction::AddTime { input } => {
                        app.add_time_to_current_at(abs, &input);
                        // Same guarantee as the prompt: a failed resolution
                        // (invalid duration, write error) of a nudge-born add
                        // keeps the reminder alive.
                        if app.session.from_nudge && app.nav.mode() == Mode::Normal {
                            app.session.from_nudge = false;
                            app.nav.set_mode(Mode::IdleNudge);
                        }
                    }
                }
            }
        }
        KeyCode::Char('n' | 'N') => {
            // New entry for today: carry forward (consuming the old line),
            // then start the timer / add the time on the fresh line.
            let pending = app.session.pending_day_boundary.take();
            app.nav.pop_mode();
            if let Some((abs, action)) = pending {
                match action {
                    DayBoundaryAction::StartTimer => app.day_boundary_new_entry(abs),
                    DayBoundaryAction::AddTime { input } => {
                        app.day_boundary_new_entry_add_time(abs, &input);
                        // Same guarantee as the prompt: a failed resolution
                        // of a nudge-born add keeps the reminder alive.
                        if app.session.from_nudge && app.nav.mode() == Mode::Normal {
                            app.session.from_nudge = false;
                            app.nav.set_mode(Mode::IdleNudge);
                        }
                    }
                }
            }
        }
        KeyCode::Esc => {
            app.session.pending_day_boundary.take();
            app.nav.pop_mode();
            // Esc from a day-boundary prompt reached during a nudge recovery
            // returns to the popup — and never leaks the flag into Normal.
            if app.session.from_nudge {
                app.session.from_nudge = false;
                app.nav.set_mode(Mode::IdleNudge);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Timesheet date picker (g in timesheet view)
// ---------------------------------------------------------------------------

pub(crate) fn handle_pick_timesheet_date(app: &mut App, key: KeyEvent) {
    let is_date_char = matches!(key.code, KeyCode::Char(c) if c.is_ascii_digit() || c == '-');
    if is_date_char || matches!(key.code, KeyCode::Backspace) {
        app.timesheet.date_type(key.code);
        return;
    }
    match key.code {
        KeyCode::Char('h') | KeyCode::Left => app.timesheet.calendar_move(-1, 0),
        KeyCode::Char('l') | KeyCode::Right => app.timesheet.calendar_move(1, 0),
        KeyCode::Char('k') | KeyCode::Up => app.timesheet.calendar_move(0, -1),
        KeyCode::Char('j') | KeyCode::Down => app.timesheet.calendar_move(0, 1),
        KeyCode::Char('t') => app.timesheet_calendar_set_relative(0),
        KeyCode::Char('T') => app.timesheet_calendar_set_relative(1),
        KeyCode::Char('w') => app.timesheet_calendar_set_relative(7),
        KeyCode::Char('m') => app.timesheet_calendar_add_months(1),
        KeyCode::Char('M') => app.timesheet_calendar_add_months(-1),
        KeyCode::Enter => app.timesheet_calendar_accept(),
        KeyCode::Esc => app.timesheet_calendar_cancel(),
        _ => {}
    }
}
