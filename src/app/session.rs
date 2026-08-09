//! Session state: timer activity tracking, nudge thresholds, and
//! transient insert-mode flags. Extracted from [`App`] to group
//! timer-related concerns.

use std::time::Instant;

use super::types::View;

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

/// Transient state for the nudge task picker (`Mode::PickNudgeTask`): the
/// list of open tasks offered and the highlight position.
#[derive(Debug, Clone)]
pub struct NudgePickerState {
    /// Absolute indices of the open (not done) tasks offered for selection.
    pub abs_list: Vec<usize>,
    pub cursor: usize,
    pub action: NudgePickAction,
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
    /// True when the current Insert session was entered via `M` (manual time
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
    /// Transient state for the nudge task picker (`Mode::PickNudgeTask`).
    /// `None` when the picker isn't open.
    pub nudge_picker: Option<NudgePickerState>,
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
            nudge_picker: None,
        }
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}
