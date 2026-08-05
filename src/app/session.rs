//! Session state: timer activity tracking, nudge thresholds, and
//! transient insert-mode flags. Extracted from [`App`] to group
//! timer-related concerns.

use std::time::Instant;

use super::types::View;

/// Timer and nudge session state. All fields are `pub` so handlers
/// can mutate them directly through `app.session`, following the same
/// pattern as [`super::Navigation`].
#[derive(Debug)]
pub struct Session {
    /// Timestamp of the last timer activity (start or stop). Used for
    /// idle nudge detection.
    pub last_timer_activity: Instant,
    /// True when the running timer has exceeded the long-timer nudge threshold.
    pub long_timer_nudge_active: bool,
    /// True when the next successful `add_from_draft` save should auto-start
    /// a timer on the new task. Set by `interrupt_timer()`; cleared on use.
    pub auto_start_on_save: bool,
    /// The view the user was in before the idle nudge fired, so Dismiss
    /// can restore it. `None` when no nudge is active.
    pub pre_nudge_view: Option<View>,
    /// True when the current Insert session was entered via `M` (manual time
    /// entry). Drives `dur:` value conversion on save.
    pub manual_time_entry: bool,
}

impl Session {
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_timer_activity: Instant::now(),
            long_timer_nudge_active: false,
            auto_start_on_save: false,
            pre_nudge_view: None,
            manual_time_entry: false,
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
