//! Session state: timer activity tracking, nudge thresholds, and
//! transient insert-mode flags. Extracted from [`App`] to group
//! timer-related concerns.

use std::time::Instant;

use super::types::{Filter, View};

/// What the nudge task picker commits on Enter. The picker exists so a nudge
/// never blindly starts a timer (or adds time) to whatever task happens to be
/// under the cursor — the user consciously chooses the task first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NudgePickAction {
    /// Start the timer on the chosen task.
    StartTimer,
    /// Open the add-time prompt targeting the chosen task.
    AddTime,
}

/// Transient state for the nudge task picker (`Mode::Picker(Picker::NudgeTask)`): the
/// action to commit on Enter plus the search/filter state in place before
/// the picker opened. The picker runs on the *real list view* — full
/// navigation, search and filters — so it needs no task list of its own:
/// the visible list IS the offer, and the highlighted task is the choice.
#[derive(Debug, Clone)]
pub struct NudgePickerState {
    /// What Enter commits: start the timer, or open the add-time prompt.
    pub action: NudgePickAction,
    /// The search text + project/context filters in place before the picker
    /// opened. Restored when the selection ends so the picker never leaves
    /// the user's list state changed.
    pub prev_filter: Filter,
    /// The list cursor position before the picker opened, restored on exit
    /// so a nudge recovery never drops the user's place in the list.
    pub prev_cursor: usize,
}

/// Why the idle nudge is (or would be) showing. Drives the popup message so
/// it speaks to the actual failure mode instead of always saying the same
/// thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleReason {
    /// A timer was stopped and no new one started within the threshold — the
    /// classic "forgot to start the next timer" case.
    TimerStopped,
    /// Nothing has been tracked today at all (e.g. the app just relaunched
    /// after a long gap spent in other apps).
    UntrackedDay,
}

/// What the day-boundary prompt's "new entry" choice should do after the
/// fresh line is created (one line per task-day, tuxtime-spec.md §3.5).
#[derive(Debug, Clone)]
pub enum DayBoundaryAction {
    /// Pressing `t`: start the timer on the new line.
    StartTimer,
    /// `M A` add-time: add this duration input to the new line.
    AddTime { input: String },
}

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
    /// True when the current Insert session was entered via `m` (manual time
    /// entry). Drives `dur:` value conversion on save.
    pub manual_time_entry: bool,
    /// Day-boundary prompt state: the task being started + what "new entry"
    /// should do once the fresh line exists. `None` when no prompt is showing.
    pub pending_day_boundary: Option<(usize, DayBoundaryAction)>,
    /// Source task index for the upgraded `N` carry-forward insert. Cleared
    /// on save or cancel.
    pub carry_forward_from: Option<usize>,
    /// True once the launch-time idle backdate has been applied (nothing
    /// tracked today → treat the idle clock as already past the threshold so
    /// the first tick nudges). One-shot per session.
    pub idle_backdated: bool,
    /// Why the idle nudge is (or would be) showing; drives the popup text.
    pub idle_reason: IdleReason,
    /// How many times the idle nudge has been dismissed without any time
    /// being captured since. Each dismissal halves the wait until the next
    /// reminder (floored at one minute); any real capture resets it to 0.
    pub idle_nudge_dismissals: u32,
    /// Transient state for the nudge task picker (`Mode::Picker(Picker::NudgeTask)`).
    /// `None` when the picker isn't open.
    pub nudge_picker: Option<NudgePickerState>,
    /// The day the end-of-day review nudge was last shown (`YYYY-MM-DD`),
    /// so it fires at most once per day.
    pub review_nudge_date: Option<String>,
    /// True while a recovery flow opened from the idle nudge is active: the
    /// `N` new-entry insert, or the add-time prompt reached via the `m`
    /// task picker. Any exit to Normal with this flag still set means the
    /// recovery was abandoned or failed (Esc-cancel, invalid duration, write
    /// error) — the UI returns to the nudge popup instead of dropping to
    /// Normal, so the reminder survives every failed attempt. The flag is
    /// cleared exactly when a capture actually lands (insert saved, time
    /// added) or when the nudge is deliberately dismissed.
    pub from_nudge: bool,
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
            pending_day_boundary: None,
            carry_forward_from: None,
            idle_backdated: false,
            idle_reason: IdleReason::TimerStopped,
            idle_nudge_dismissals: 0,
            nudge_picker: None,
            review_nudge_date: None,
            from_nudge: false,
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
