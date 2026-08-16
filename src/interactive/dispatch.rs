//! Mode dispatch: maps the active [`Mode`] to its key handler.
//!
//! `dispatch` is the single place that knows which handler owns each mode.
//! Adding a mode still touches the [`Mode`] sub-enums plus one arm here (the
//! key handler), one arm in `ui::overlays::draw_overlays` (the renderer), and
//! a status hint in `ui::status` — but the nested [`Screen`]/[`Prompt`]/
//! [`Picker`]/[`Nudge`] categories keep each registry grouped by category, so
//! a whole family can be routed (or rendered) in one arm instead of listing
//! every sibling variant.
//!
//! Draw-less modes: `Picker::Project`/`Picker::Context`/`Picker::SavedFilter`
//! preview the filter directly on the main list (no floating overlay, so
//! they're absent from `draw_overlays`), and `Screen::Search` renders its
//! input in the status bar via `ui::status::render_command_line`. Everything
//! else floats an overlay.
//!
//! The binary's event loop delegates every keypress here; this module never
//! touches the store directly, only forwards to the per-mode handlers.

use crate::app::{App, Mode, Nudge, Picker, Prompt, Screen};
use crate::keybinds::KeyBindings;
use ratatui::crossterm::event::KeyEvent;

use super::action_dispatch::handle_normal;
use super::insert::handle_insert;
use super::overlays::{
    handle_command_palette, handle_day_boundary, handle_help, handle_idle_nudge,
    handle_long_timer_nudge, handle_manage_projects, handle_manual_entry_choice, handle_pick,
    handle_pick_nudge_task, handle_pick_theme, handle_pick_timesheet_date, handle_prompt,
    handle_review_nudge, handle_search, handle_settings, handle_share, handle_stale_timer,
    handle_welcome,
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
        Mode::Screen(screen) => match screen {
            Screen::Normal | Screen::Visual => handle_normal(app, key, keybinds),
            Screen::Insert => handle_insert(app, key),
            Screen::Search => handle_search(app, key),
            Screen::Help => handle_help(app, key),
            Screen::Settings => handle_settings(app, key),
            Screen::CommandPalette => handle_command_palette(app, key),
            Screen::Share => handle_share(app, key),
            Screen::Welcome => handle_welcome(app, key),
            Screen::ManageProjects => handle_manage_projects(app, key),
        },
        Mode::Prompt(prompt) => match prompt {
            Prompt::DayBoundary => handle_day_boundary(app, key),
            Prompt::Project
            | Prompt::Context
            | Prompt::SaveFilter
            | Prompt::AddTime
            | Prompt::IdleNudge
            | Prompt::LongTimerNudge
            | Prompt::RenameProject => handle_prompt(app, key),
        },
        Mode::Picker(picker) => match picker {
            Picker::Project | Picker::Context | Picker::SavedFilter => handle_pick(app, key),
            Picker::Theme => handle_pick_theme(app, key),
            Picker::TimesheetDate => handle_pick_timesheet_date(app, key),
            Picker::NudgeTask => handle_pick_nudge_task(app, key, keybinds),
        },
        Mode::Nudge(nudge) => match nudge {
            Nudge::Idle => handle_idle_nudge(app, key),
            Nudge::LongTimer => handle_long_timer_nudge(app, key),
            Nudge::StaleTimer => handle_stale_timer(app, key),
            Nudge::Review => handle_review_nudge(app, key),
            Nudge::ManualEntryChoice => handle_manual_entry_choice(app, key),
        },
    }
}
