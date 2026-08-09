//! Interactive TUI layer: mode dispatch, key resolution, and the per-mode
//! key handlers that drive [`App`]. Lives in the library so the full
//! keyboard-driven UI can be exercised by the crate's test suite (and by
//! integration tests) rather than living behind the `tuxtime` binary's
//! private module tree. The binary is just the terminal front-end — event
//! loop, startup, and subcommand dispatch — on top of this module.
//!
//! [`dispatch`] routes each keypress to the handler for the active
//! [`Mode`](crate::app::Mode); the handlers translate key events into
//! [`Action`](crate::action::Action)s and `App` mutations.

mod action_dispatch;
mod dispatch;
mod insert;
mod key_resolve;
mod overlays;
mod timesheet_handler;

/// The binary's event loop uses these three entry points directly.
pub use action_dispatch::apply_action;
pub use dispatch::dispatch;
pub use key_resolve::resolve_builtin_single_key;

// Test-facing re-exports: the interactive test suite (`tests.rs`) drives the
// handlers directly. Gated so non-test builds don't carry unused imports —
// every one of these is already reachable within this module tree through
// the modules' own `super::` imports.
#[cfg(test)]
pub(crate) use insert::{EditAction, handle_insert, resolve_edit_key};
#[cfg(test)]
pub(crate) use key_resolve::resolve_normal_key;
#[cfg(test)]
pub(crate) use overlays::{
    handle_day_boundary, handle_idle_nudge, handle_long_timer_nudge, handle_manage_projects,
    handle_pick_nudge_task, handle_pick_timesheet_date, handle_prompt, handle_review_nudge,
    handle_search, handle_settings, handle_stale_timer, handle_welcome,
};
#[cfg(test)]
pub(crate) use timesheet_handler::handle_timesheet_keys;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
