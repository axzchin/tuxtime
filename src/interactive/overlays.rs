//! Overlay/modal key handlers for the interactive layer. Each function
//! handles one transient mode (welcome, search, help, settings, pickers,
//! command palette, prompts, project manager, idle nudge, etc.).
//!
//! Public entry points:
//! - [`handle_autocomplete_keys`] — shared by Insert, Search, Prompt modes
//! - All other handlers are `pub(crate)` for use by [`handle_key`].

use crate::app::{App, DayBoundaryAction, Mode, Nudge, Picker, Prompt, Screen, View};
use crate::cli;
use crate::keybinds::KeyBindings;
use crate::todo;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::action_dispatch::{apply_action, handle_normal};
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
            app.nav.push_mode(Mode::Prompt(Prompt::IdleNudge));
        }
        KeyCode::Char('l') => {
            let mins = app.long_timer_nudge_seconds() / 60;
            app.draft_clear();
            app.draft_set_insert(mins.to_string());
            // Open the prompt over Settings so Esc/Enter returns to Settings.
            app.nav.push_mode(Mode::Prompt(Prompt::LongTimerNudge));
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
        KeyCode::Char('I') => {
            app.prefs.toggle_duration_inline();
            app.save_prefs();
        }
        KeyCode::Char('O') => {
            app.prefs.toggle_log_inline();
            app.save_prefs();
        }
        KeyCode::Char('B') => app.cycle_badge_theme(),
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
    // `T` mirrors `j` inside the picker — a second, home-row-friendly way to
    // step the live preview — matching the `j/k/T` legend in its footer.
    if key.code == KeyCode::Char('T') {
        app.pick_theme_step(true);
        return;
    }
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

/// After an add-time attempt, return to the idle-nudge popup when the
/// capture failed. The save functions clear `from_nudge` the moment time
/// actually lands, so the flag still being set here means nothing was
/// recorded (invalid duration, write error); `mode == Mode::Screen(Screen::Normal)`
/// distinguishes a completed failure from a flow that deferred to the
/// day-boundary prompt, which is still open (mode != Normal) and will
/// resolve — or fail — on its own.
fn resolve_nudge_add_outcome(app: &mut App) {
    if app.session.from_nudge && app.nav.mode() == Mode::Screen(Screen::Normal) {
        app.session.from_nudge = false;
        app.nav.set_mode(Mode::Nudge(Nudge::Idle));
    }
}

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
                Mode::Prompt(Prompt::IdleNudge) | Mode::Prompt(Prompt::LongTimerNudge)
            ) {
                // Entered with push_mode (Settings or the mode the command
                // palette returned to), so pop restores the caller. This can
                // also fire while the nudge selection is open (palette →
                // ConfigureIdleNudge pushes over PickNudgeTask) — the pop
                // lands back on the selection, which is exactly right, so
                // this branch must stay ahead of the nudge_picker check.
                app.nav.pop_mode()
            } else if app.nav.mode() == Mode::Prompt(Prompt::RenameProject) {
                Mode::Screen(Screen::ManageProjects)
            } else if app.nav.mode() == Mode::Prompt(Prompt::AddTime) && app.session.from_nudge {
                // Esc from the nudge's add-time recovery flow returns to the
                // nudge popup — the reminder survives a cancelled attempt.
                app.session.from_nudge = false;
                Mode::Nudge(Nudge::Idle)
            } else if app.session.nudge_picker.is_some() {
                // A quick-tag / save-filter prompt opened from the nudge
                // selection (+, c, fs) returns to the selection — the user
                // is still choosing a task, not abandoning the choice.
                Mode::Picker(Picker::NudgeTask)
            } else {
                Mode::Screen(Screen::Normal)
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
                Mode::Prompt(Prompt::IdleNudge) | Mode::Prompt(Prompt::LongTimerNudge)
            );
            let is_rename = prev_mode == Mode::Prompt(Prompt::RenameProject);
            let return_mode = if is_nudge {
                // Entered with push_mode (Settings or the mode the command
                // palette returned to), so pop restores the caller.
                app.nav.pop_mode()
            } else if is_rename {
                Mode::Screen(Screen::ManageProjects)
            } else if app.session.nudge_picker.is_some() {
                // A quick-tag / save-filter prompt opened from the nudge
                // selection (+, c, fs) returns to the selection — the user
                // is still choosing a task, not abandoning the choice.
                Mode::Picker(Picker::NudgeTask)
            } else {
                Mode::Screen(Screen::Normal)
            };
            app.nav.set_mode(return_mode);
            match prev_mode {
                Mode::Prompt(Prompt::Project) => app.add_project_to_current(&value),
                Mode::Prompt(Prompt::Context) => app.toggle_context_on_current(&value),
                Mode::Prompt(Prompt::SaveFilter) => app.save_current_filter_as(&value),
                Mode::Prompt(Prompt::AddTime) => {
                    app.add_time_to_current_from_input(&value);
                    // A failed add (invalid duration, write error) from the
                    // nudge's recovery flow must not drop the reminder.
                    resolve_nudge_add_outcome(app);
                }
                Mode::Prompt(Prompt::IdleNudge) => {
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
                Mode::Prompt(Prompt::LongTimerNudge) => {
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
                Mode::Prompt(Prompt::RenameProject) => {
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
                app.nav.set_mode(Mode::Prompt(Prompt::RenameProject));
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
            app.nav.push_mode(Mode::Screen(Screen::Search));
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Idle nudge (popup when no timer and idle too long)
// ---------------------------------------------------------------------------

/// Idle-nudge popup. The recovery actions (`s` start timer, `m` add time)
/// route through the task picker so they never hit a random task the cursor
/// happens to be on; `n` opens a blank new entry; `d`/`Esc` dismisses.
pub(crate) fn handle_idle_nudge(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('s' | 'S') => app.enter_nudge_picker(crate::app::NudgePickAction::StartTimer),
        KeyCode::Char('m' | 'M') => app.enter_nudge_picker(crate::app::NudgePickAction::AddTime),
        KeyCode::Char('n' | 'N') => {
            // Remember the insert came from the nudge so an Esc-cancel
            // returns to the popup (the reminder survives an aborted
            // recovery) while a save exits to Normal.
            app.session.from_nudge = true;
            app.set_view(View::List);
            app.draft_clear();
            app.session.manual_time_entry = false;
            app.nav.set_mode(Mode::Screen(Screen::Insert));
            app.selection.exit_edit();
            app.session.last_timer_activity = std::time::Instant::now();
        }
        KeyCode::Char('d' | 'D') | KeyCode::Esc => {
            // Dismissing the nudge ends any recovery flow for good, and
            // escalates the reminder (each dismissal halves the next wait)
            // so the popup can't be snoozed indefinitely.
            app.session.from_nudge = false;
            app.dismiss_idle_nudge();
        }
        _ => {}
    }
}

/// End-of-day review nudge: `v` opens the timesheet (today), `m` opens the
/// manual-entry choice, `s`/`Esc` skips — the review won't re-fire today.
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
            app.nav.set_mode(Mode::Nudge(Nudge::ManualEntryChoice));
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

/// Nudge task picker: runs on the *real list view*, so every list key keeps
/// working — navigation (`j`/`k`, arrows, `gg`), search (`/`), filters
/// (`fp`/`fc`/`ff`), sidebars, even `t` to start a timer directly. `Enter`
/// commits the highlighted task (start timer or add time per the action),
/// `Esc` returns to the idle nudge popup.
pub(crate) fn handle_pick_nudge_task(app: &mut App, key: KeyEvent, keybinds: &KeyBindings) {
    // A previous action can leave the selection open in another view — e.g.
    // the command palette's OpenTimesheet/ToggleArchiveView pops back into
    // PickNudgeTask after switching the view. The selection only makes sense
    // on the list, so end it before processing the key, then let the key act
    // in Normal mode.
    if app.nav.view != View::List {
        app.nudge_picker_exit_to_view();
        handle_normal(app, key, keybinds);
        return;
    }
    match key.code {
        KeyCode::Enter => app.nudge_picker_accept(),
        KeyCode::Esc => app.nudge_picker_cancel(),
        _ => {
            handle_normal(app, key, keybinds);
            maybe_finish_nudge_selection(app);
            maybe_abandon_nudge_selection(app);
        }
    }
}

/// A delegated key that left the selection into a mode pushed *over* it
/// (search, command palette) is still part of the selection — it pops back.
/// The filter pickers (project / context / saved-filter) replace the mode
/// outright but are *also* part of the selection: they preview their filter
/// directly on the list and return to it on accept or cancel, so they must
/// never count as an abandonment — dropping back to Normal would let the
/// idle nudge re-fire on the next tick (still idle, no timer started) and
/// yank the user out of a task they were about to pick. But a key that
/// replaced the mode outright (`n`, `e`, `,`, `?`, `P`, …) abandons the
/// selection: restore the pre-selection filter and drop the stale picker
/// state so the cleared filter never leaks into Normal.
fn maybe_abandon_nudge_selection(app: &mut App) {
    // The selection is already over (e.g. `t` started a timer and finished
    // it, restoring a pre-nudge non-List view) — nothing to abandon, and the
    // view check below must not reset the nudge clock on that path.
    if app.session.nudge_picker.is_none() {
        return;
    }
    if matches!(
        app.nav.mode(),
        // The filter pickers preview their filter directly on the list and
        // return to the selection on accept or cancel.
        Mode::Picker(Picker::Project) | Mode::Picker(Picker::Context) | Mode::Picker(Picker::SavedFilter)
        // The quick-tag prompts (+, c) and the save-filter prompt (fs) are
        // selection-compatible detours: they mutate the highlighted task or
        // persist the mid-selection search, then return to the selection.
        | Mode::Prompt(Prompt::Project)
        | Mode::Prompt(Prompt::Context)
        | Mode::Prompt(Prompt::SaveFilter)
    ) {
        return;
    }
    // A delegated key that switched to another view (`V` timesheet, `a`
    // archive) ends the selection cleanly — the selection only makes sense
    // on the list, and reviewing another view is a deliberate dismissal. The
    // nudge clock resets so it doesn't re-fire over the view just opened.
    if app.nav.view != View::List {
        app.nudge_picker_exit_to_view();
        return;
    }
    if app.nav.mode() != Mode::Picker(Picker::NudgeTask)
        && app.nav.peek_under() != Some(Mode::Picker(Picker::NudgeTask))
    {
        app.nudge_picker_abandon();
    }
}

/// A timer started while the list-based selection is open (via `t` — or the
/// day-boundary prompt reached from `t`) completes the recovery on its own:
/// exit to Normal, restoring the pre-selection filter and pre-nudge view.
fn maybe_finish_nudge_selection(app: &mut App) {
    if app.nav.mode() == Mode::Picker(Picker::NudgeTask) && app.timer_running() {
        app.nudge_picker_finish();
    }
}

/// Stale-timer startup prompt: a timer was running when the app last closed
/// (or was killed) and has since exceeded the long-timer threshold. `s` stops
/// and logs everything, `d` discards the unrecorded gap (no time credited),
/// `k`/`Esc` keep it counting — the safe default that never destroys time
/// without an explicit choice.
pub(crate) fn handle_stale_timer(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('s' | 'S') => {
            app.stop_running_timer();
            app.dismiss_nudge();
        }
        KeyCode::Char('d' | 'D') => {
            app.discard_stale_timer();
            app.dismiss_nudge();
        }
        KeyCode::Char('k' | 'K') | KeyCode::Esc => {
            app.keep_stale_timer();
        }
        _ => {}
    }
}

/// Long-timer nudge popup: the running timer has exceeded the configured
/// threshold. `s` stops it (capturing the elapsed time), `d`/`Esc` dismisses
/// and keeps the timer running. Both return to the pre-nudge view.
pub(crate) fn handle_long_timer_nudge(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('s' | 'S') => {
            app.stop_running_timer();
            app.dismiss_nudge();
        }
        KeyCode::Char('d' | 'D') | KeyCode::Esc => {
            app.dismiss_nudge();
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
            // Manual time entry: treat a typed `dur:` as flexible input
            // (minutes/hours/clock time) on save, so `dur:30` logs 30 minutes
            // — not the 30 *seconds* the raw on-disk token would mean.
            app.session.manual_time_entry = true;
            app.nav.set_mode(Mode::Screen(Screen::Insert));
            app.selection.exit_edit();
        }
        KeyCode::Char('a' | 'A') => {
            if let Some(t) = app.cur_task() {
                let body = todo::body_only_from_clean(&t.clean_raw);
                app.nav.set_mode(Mode::Prompt(Prompt::AddTime));
                app.draft_clear();
                app.flash(format!("add time to: {body}"));
            } else {
                app.nav.enter_normal();
                app.flash("no task selected — navigate and press m a");
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
                        // A failed resolution of a nudge-born add keeps the
                        // reminder alive, exactly like the prompt.
                        resolve_nudge_add_outcome(app);
                    }
                }
            }
            // `t` inside the nudge selection that deferred to this prompt
            // now has its timer running — the selection's job is done.
            maybe_finish_nudge_selection(app);
        }
        // Enter defaults to the safe, most common choice: a fresh entry for
        // today. Starting a timer (or adding time) on a carried-over task is
        // the everyday case, and "one line per task-day" is the model — so
        // the plain confirm key should do exactly that without demanding the
        // user learn the continue-vs-new distinction first.
        KeyCode::Enter | KeyCode::Char('n' | 'N') => {
            // New entry for today: carry forward (consuming the old line),
            // then start the timer / add the time on the fresh line.
            let pending = app.session.pending_day_boundary.take();
            app.nav.pop_mode();
            if let Some((abs, action)) = pending {
                match action {
                    DayBoundaryAction::StartTimer => app.day_boundary_new_entry(abs),
                    DayBoundaryAction::AddTime { input } => {
                        app.day_boundary_new_entry_add_time(abs, &input);
                        // A failed resolution of a nudge-born add keeps the
                        // reminder alive, exactly like the prompt.
                        resolve_nudge_add_outcome(app);
                    }
                }
            }
            // `t` inside the nudge selection that deferred to this prompt
            // now has its timer running — the selection's job is done.
            maybe_finish_nudge_selection(app);
        }
        KeyCode::Esc => {
            app.session.pending_day_boundary.take();
            app.nav.pop_mode();
            // Esc from a day-boundary prompt reached during a nudge recovery
            // returns to the popup — and never leaks the flag into Normal.
            if app.session.from_nudge {
                app.session.from_nudge = false;
                app.nav.set_mode(Mode::Nudge(Nudge::Idle));
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
