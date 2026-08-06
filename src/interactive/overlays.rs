//! Overlay/modal key handlers for the interactive layer. Each function
//! handles one transient mode (welcome, search, help, settings, pickers,
//! command palette, prompts, project manager, idle nudge, etc.).
//!
//! Public entry points:
//! - [`handle_autocomplete_keys`] — shared by Insert, Search, Prompt modes
//! - All other handlers are `pub(crate)` for use by [`handle_key`].

use crate::app::{App, Mode, View};
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
        KeyCode::Char('c') => match cli::ensure_file(app.file_path.clone()) {
            Ok(_) => app.nav.enter_normal(),
            Err(e) => app.flash(format!("could not create {}: {e}", app.file_path.display())),
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
    let return_mode = app.nav.pre_search_mode.take().unwrap_or(Mode::Normal);
    match key.code {
        KeyCode::Esc => {
            app.nav.set_mode(return_mode);
            app.draft_clear();
            app.clear_search();
        }
        KeyCode::Enter => {
            app.nav.set_mode(return_mode);
            app.nav.move_top();
        }
        _ => {
            app.nav.pre_search_mode = Some(return_mode);
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
            app.nav.nudge_prompt_return = Some(Mode::Settings);
            app.nav.set_mode(Mode::PromptIdleNudge);
        }
        KeyCode::Char('l') => {
            let mins = app.long_timer_nudge_seconds() / 60;
            app.draft_clear();
            app.draft_set_insert(mins.to_string());
            app.nav.nudge_prompt_return = Some(Mode::Settings);
            app.nav.set_mode(Mode::PromptLongTimerNudge);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Pickers — project, context, saved-filter, theme
// ---------------------------------------------------------------------------

pub(crate) fn handle_pick(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.pick_step(true),
        KeyCode::Char('k') | KeyCode::Up => app.pick_step(false),
        KeyCode::Enter => app.pick_accept(),
        KeyCode::Esc => app.pick_cancel(),
        _ => {}
    }
}

pub(crate) fn handle_pick_theme(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.pick_theme_step(true),
        KeyCode::Char('k') | KeyCode::Up => app.pick_theme_step(false),
        KeyCode::Enter => app.pick_theme_accept(),
        KeyCode::Esc => app.pick_theme_cancel(),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Command palette
// ---------------------------------------------------------------------------

pub(crate) fn handle_command_palette(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            app.nav.set_mode(app.command_palette.take_prior());
            app.draft_clear();
            return;
        }
        KeyCode::Enter => {
            let chosen = app.command_palette.current_action();
            app.nav.set_mode(app.command_palette.take_prior());
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
                app.nav.nudge_prompt_return.take().unwrap_or(Mode::Normal)
            } else if app.nav.mode() == Mode::PromptRenameProject {
                Mode::ManageProjects
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
                app.nav.nudge_prompt_return.take().unwrap_or(Mode::Normal)
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
                Mode::PromptAddTime => app.add_time_to_current_from_input(&value),
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
        KeyCode::Char('/') => {
            app.nav.pre_search_mode = Some(Mode::ManageProjects);
            app.draft_clear();
            app.nav.set_mode(Mode::Search);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Idle nudge (popup when no timer and idle too long)
// ---------------------------------------------------------------------------

pub(crate) fn handle_idle_nudge(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('N' | 'n') => {
            app.set_view(View::List);
            app.draft_clear();
            app.session.manual_time_entry = false;
            app.nav.set_mode(Mode::Insert);
            app.selection.exit_edit();
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
                let body = todo::body_only(&t.raw);
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
