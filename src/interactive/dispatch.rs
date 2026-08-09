//! Mode dispatch: maps the active [`Mode`] to its key handler.
//!
//! `dispatch` is the single place that knows which handler owns each mode.
//! Adding a mode touches four registries that must stay in sync:
//! 1. the [`Mode`] enum,
//! 2. one arm here in [`dispatch`] (the key handler),
//! 3. one arm in `ui::overlays::draw_overlays` (the renderer) — *unless* the
//!    mode is deliberately draw-less and renders inline (see below),
//! 4. a status hint for the mode in `ui::status`.
//!
//! Draw-less modes: `PickProject`/`PickContext`/`PickSavedFilter` preview the
//! filter directly on the main list (no floating overlay, so they're absent
//! from `draw_overlays`), and `Search` renders its input in the status bar via
//! `ui::status::render_command_line`. Everything else floats an overlay.
//!
//! The binary's event loop delegates every keypress here; this module never
//! touches the store directly, only forwards to the per-mode handlers.

use crate::app::{App, Mode};
use crate::keybinds::KeyBindings;
use ratatui::crossterm::event::KeyEvent;

use super::action_dispatch::handle_normal;
use super::insert::handle_insert;
use super::overlays::{
    handle_command_palette, handle_day_boundary, handle_help, handle_idle_nudge,
    handle_long_timer_nudge, handle_manage_projects, handle_manual_entry_choice, handle_pick,
    handle_pick_nudge_task, handle_pick_theme, handle_pick_timesheet_date, handle_prompt,
    handle_search, handle_settings, handle_share, handle_stale_timer, handle_welcome,
};

/// Route one keypress to the handler for `app.nav.mode()`. The caller is
/// responsible for triggering a redraw after the call. `keybinds` is only
/// consulted by the Normal/Visual handler; the overlay handlers ignore it.
pub fn dispatch(app: &mut App, key: KeyEvent, keybinds: &KeyBindings) {
    // Detect external edits before processing the key. On detection the
    // file is reloaded, the keystroke is consumed (re-press to act on
    // the new state), and the per-mutator checks become no-ops downstream.
    if !app.check_external_changes() {
        return;
    }
    match app.nav.mode() {
        Mode::Insert => handle_insert(app, key),
        Mode::Search => handle_search(app, key),
        Mode::Help => handle_help(app, key),
        Mode::Settings => handle_settings(app, key),
        Mode::PromptProject
        | Mode::PromptContext
        | Mode::PromptSaveFilter
        | Mode::PromptAddTime
        | Mode::PromptIdleNudge
        | Mode::PromptLongTimerNudge
        | Mode::PromptRenameProject => {
            handle_prompt(app, key);
        }
        Mode::PromptDayBoundary => handle_day_boundary(app, key),
        Mode::PickTimesheetDate => handle_pick_timesheet_date(app, key),
        Mode::PickProject | Mode::PickContext | Mode::PickSavedFilter => handle_pick(app, key),
        Mode::PickTheme => handle_pick_theme(app, key),
        Mode::CommandPalette => handle_command_palette(app, key),
        Mode::Share => handle_share(app, key),
        Mode::Welcome => handle_welcome(app, key),
        Mode::Normal | Mode::Visual => handle_normal(app, key, keybinds),
        Mode::IdleNudge => handle_idle_nudge(app, key),
        Mode::PickNudgeTask => handle_pick_nudge_task(app, key),
        Mode::LongTimerNudge => handle_long_timer_nudge(app, key),
        Mode::StaleTimer => handle_stale_timer(app, key),
        Mode::ManualEntryChoice => handle_manual_entry_choice(app, key),
        Mode::ManageProjects => handle_manage_projects(app, key),
    }
}
