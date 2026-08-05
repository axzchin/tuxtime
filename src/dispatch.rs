//! Mode dispatch: maps the active [`Mode`] to its key handler.
//!
//! The dispatcher is the single place that knows which handler owns each
//! mode. Adding a mode means:
//! 1. adding it to the [`Mode`] enum,
//! 2. adding one arm to [`ModeDispatcher::dispatch`],
//! 3. (for new UI) a status-bar label in `ui::status`.
//!
//! The event loop in `main::run` delegates every keypress here; this module
//! never touches the store directly, only forwards to the per-mode handlers.

use ratatui::crossterm::event::KeyEvent;
use tuxtime::app::{App, Mode};
use tuxtime::keybinds::KeyBindings;

use super::action_dispatch::handle_normal;
use super::insert::handle_insert;
use super::overlays::{
    handle_command_palette, handle_help, handle_idle_nudge, handle_manual_entry_choice,
    handle_manage_projects, handle_pick, handle_pick_theme, handle_pick_timesheet_date,
    handle_prompt, handle_search, handle_settings, handle_share, handle_welcome,
};

/// Owns the keybindings and routes each keypress to the handler for the
/// active mode. `handle_normal` needs the bindings; the overlay handlers
/// ignore them.
pub(crate) struct ModeDispatcher<'a> {
    keybinds: &'a KeyBindings,
}

impl<'a> ModeDispatcher<'a> {
    pub(crate) fn new(keybinds: &'a KeyBindings) -> Self {
        Self { keybinds }
    }

    /// Route one keypress to the handler for `app.nav.mode()`. Returns
    /// `true` when the key was consumed (i.e. the app should redraw).
    pub(crate) fn dispatch(&self, app: &mut App, key: KeyEvent) {
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
            Mode::PromptProject | Mode::PromptContext | Mode::PromptSaveFilter
            | Mode::PromptAddTime | Mode::PromptIdleNudge | Mode::PromptLongTimerNudge
            | Mode::PromptRenameProject => {
                handle_prompt(app, key);
            }
            Mode::PickTimesheetDate => handle_pick_timesheet_date(app, key),
            Mode::PickProject | Mode::PickContext | Mode::PickSavedFilter => handle_pick(app, key),
            Mode::PickTheme => handle_pick_theme(app, key),
            Mode::CommandPalette => handle_command_palette(app, key),
            Mode::Share => handle_share(app, key),
            Mode::Welcome => handle_welcome(app, key),
            Mode::Normal | Mode::Visual => handle_normal(app, key, self.keybinds),
            Mode::IdleNudge => handle_idle_nudge(app, key),
            Mode::ManualEntryChoice => handle_manual_entry_choice(app, key),
            Mode::ManageProjects => handle_manage_projects(app, key),
        }
    }
}
