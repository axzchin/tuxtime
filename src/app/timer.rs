//! Timer lifecycle: toggle, interrupt, stop-on-quit, and manual time entry.
//! Also owns nudge detection (idle and long-running) and billable toggling.
//!
//! All methods are `impl App` because they need `App` context — the store
//! for timer operations, `flash()` for user feedback, and `prefs` for
//! nudge thresholds.

use super::session::DayBoundaryAction;
use super::{
    App, IdleReason, Mode, NudgePickAction, NudgePickerState, View, format_duration, parse_clock,
    parse_duration_input,
};
use crate::core::outcome::{CarryForwardOutcome, EditOutcome, TimerOutcome, TimerQuitOutcome};
use crate::core::rebuild_token_line;
use crate::todo::Task;
use std::time::Instant;

use chrono::Timelike;

/// The day a task's accumulated time belongs to: its `log:` value when valid,
/// else its creation date. `None` when neither is a usable calendar date.
fn effective_log_date(t: &Task) -> Option<&str> {
    t.log
        .as_deref()
        .filter(|s| crate::todo::is_iso_date(s))
        .or_else(|| {
            t.created_date
                .as_deref()
                .filter(|s| crate::todo::is_iso_date(s))
        })
}

/// True when a `start:` timestamp's calendar day is before `today` — the
/// timer was left running overnight. Extracted for direct testing: the
/// wall-clock elapsed comparison is hard to isolate from the date check in
/// an integration test.
fn timer_start_crossed_day(start: Option<&str>, today: &str) -> bool {
    start.is_some_and(|s| s.get(..10).is_some_and(|d| d < today))
}

impl App {
    // ---- timer helpers ----

    pub fn timer_running(&self) -> bool {
        self.store.timer_running()
    }

    pub fn timer_elapsed_secs(&self) -> Option<u64> {
        self.store.timer_elapsed_secs()
    }

    pub fn active_timer_task(&self) -> Option<&Task> {
        self.store.active_timer_task()
    }

    /// Idle nudge threshold from prefs.
    pub fn idle_nudge_seconds(&self) -> u64 {
        self.prefs.idle_nudge_seconds
    }

    /// Set the idle nudge threshold in minutes. Persists to config.
    pub fn set_idle_nudge_minutes(&mut self, minutes: u64) {
        self.prefs.idle_nudge_seconds = minutes * 60;
        self.save_prefs();
    }

    /// Long-timer nudge threshold from prefs.
    pub fn long_timer_nudge_seconds(&self) -> u64 {
        self.prefs.long_timer_nudge_seconds
    }

    /// Set the long-timer nudge threshold in minutes. Persists to config.
    pub fn set_long_timer_nudge_minutes(&mut self, minutes: u64) {
        self.prefs.long_timer_nudge_seconds = minutes * 60;
        self.save_prefs();
    }

    /// Check nudge conditions on each tick. Call from the event loop.
    /// - If no timer running and idle > threshold → enter `IdleNudge` mode.
    /// - If timer running and elapsed > threshold → enter `LongTimerNudge`
    ///   mode (popup) from Normal, and always keep the status-bar `⏰`
    ///   indicator (`long_timer_nudge_active`) so it's visible in every mode.
    ///   Returns true when the UI should redraw.
    pub fn check_nudges(&mut self) -> bool {
        // If a nudge popup is already showing, don't re-trigger.
        if matches!(
            self.nav.mode,
            Mode::IdleNudge | Mode::LongTimerNudge | Mode::StaleTimer | Mode::ReviewNudge
        ) {
            return false;
        }
        // Launch-time backdate: if nothing has been tracked today and no
        // timer is running, the user may have just reopened the app after a
        // long gap spent elsewhere (email, court, drafting in another app).
        // Treat the idle clock as already past the threshold so the very
        // first tick nudges — the app must not hand out a fresh 15-minute
        // grace period right after the longest, most billable gaps. Applied
        // once per session; any later timer activity resets the clock and
        // the reason.
        if !self.session.idle_backdated {
            self.session.idle_backdated = true;
            if !self.timer_running() && !self.store.has_time_logged_today() {
                let past = self.prefs.idle_nudge_seconds.saturating_add(1);
                self.session.last_timer_activity =
                    Instant::now() - std::time::Duration::from_secs(past);
                self.session.idle_reason = IdleReason::UntrackedDay;
            }
        }
        let idle_secs = self.session.last_timer_activity.elapsed().as_secs();
        // Idle nudge: a full-screen popup that clears the draft/selection and
        // forces the mode back to Normal. Firing it over Insert, Search, the
        // palette, a prompt, or Settings would silently discard in-progress
        // composition — so it only fires from Normal mode, where there is no
        // transient state to lose. Save the pre-nudge view for Dismiss.
        let mut fired = false;
        if self.nav.mode == Mode::Normal
            && !self.timer_running()
            && idle_secs >= self.prefs.idle_nudge_seconds
        {
            self.session.pre_nudge_view = Some(self.nav.view);
            self.exit_overlay_to_normal();
            self.nav.mode = Mode::IdleNudge;
            fired = true;
        }
        // End-of-day review: once per day, once the configured `review_time`
        // has passed, ask the user to reconcile the day. Only when something
        // has been tracked today (the "tracked nothing at all" failure is
        // already covered by the launch-time idle backdate + idle nudge) and
        // only from Normal mode, so it never clobbers the other nudges or
        // in-progress composition.
        if self.nav.mode == Mode::Normal
            && let Some(rt) = &self.prefs.review_time
            && let Some((rh, rm)) = parse_clock(rt)
            && self.has_time_tracked_today()
            && self.session.review_nudge_date.as_deref() != Some(self.today())
        {
            let now = chrono::Local::now();
            let now_min = now.hour() * 60 + now.minute();
            if now_min >= rh * 60 + rm {
                self.session.review_nudge_date = Some(self.today().to_string());
                self.session.pre_nudge_view = Some(self.nav.view);
                self.exit_overlay_to_normal();
                self.nav.mode = Mode::ReviewNudge;
                fired = true;
            }
        }
        // Long-timer nudge: popup from Normal mode (same state-safety rule as
        // the idle nudge — it must not destroy in-progress composition), plus
        // the status-bar ⏰ flag everywhere so a timer that runs long is
        // visible even mid-Insert or over the palette.
        let was_active = self.session.long_timer_nudge_active;
        if self.timer_running() {
            let elapsed = self.timer_elapsed_secs().unwrap_or(0);
            let now_active = elapsed >= self.prefs.long_timer_nudge_seconds;
            self.session.long_timer_nudge_active = now_active;
            if now_active && !was_active && self.nav.mode == Mode::Normal {
                self.session.pre_nudge_view = Some(self.nav.view);
                self.nav.mode = Mode::LongTimerNudge;
                fired = true;
            }
        } else {
            self.session.long_timer_nudge_active = false;
        }
        fired || (was_active != self.session.long_timer_nudge_active)
    }

    /// Record a timer activity (start / stop / manual add): resets the
    /// idle-nudge clock and marks the idle reason back to the ordinary
    /// timer-stopped case (a real capture happened, so "nothing tracked
    /// today" no longer applies).
    pub(crate) fn note_timer_activity(&mut self) {
        self.session.last_timer_activity = Instant::now();
        self.session.idle_reason = IdleReason::TimerStopped;
    }

    /// Dismiss any overlay/dialog mode back to Normal, discarding transient
    /// state (draft, selection). Only used for the nudges, which need a
    /// clean slate for their popup.
    fn exit_overlay_to_normal(&mut self) {
        self.draft_clear();
        self.selection.exit_edit();
        self.selection.clear();
        self.session.manual_time_entry = false;
        self.nav.mode = Mode::Normal;
    }

    /// Stop the running timer — used by the long-timer nudge's `S` key, so
    /// the popup can capture the elapsed time without the cursor context the
    /// list-view toggle needs.
    pub(crate) fn stop_running_timer(&mut self) {
        let Some(abs) = self.store.active_timer_abs() else {
            return;
        };
        self.toggle_timer_at(abs);
    }

    /// True when the restored timer looks like a *zombie*: the app was closed
    /// or killed while it was running and either the elapsed time has blown
    /// past the long-timer threshold, or the start stamp lives on a previous
    /// calendar day (an overnight stay, however short). Either way the session
    /// would silently bill away time, so startup should ask the user how to
    /// handle it instead of just keeping it running.
    pub fn stale_timer_at_startup(&self) -> bool {
        if !self.timer_running() {
            return false;
        }
        let over_threshold = self
            .timer_elapsed_secs()
            .is_some_and(|e| e >= self.prefs.long_timer_nudge_seconds);
        let crossed_day = timer_start_crossed_day(
            self.active_timer_task().and_then(|t| t.start.as_deref()),
            self.store.today(),
        );
        over_threshold || crossed_day
    }

    /// `[k]eep counting` on the stale-timer prompt: dismiss the popup and
    /// leave the timer running (the user is asserting the time is real).
    pub fn keep_stale_timer(&mut self) {
        self.nav.enter_normal();
        if let Some(v) = self.session.pre_nudge_view.take() {
            self.set_view(v);
        }
        // Timer is running, so the idle nudge can't fire; reset the activity
        // clock anyway so a later stop keeps the nudge cadence sane.
        self.note_timer_activity();
    }

    // ---- nudge task picker (S / M from the idle nudge) ----

    /// Open the nudge task picker for the given action. Lists all open tasks
    /// (unfiltered — the user is choosing deliberately, filters would hide
    /// the very task they mean), seeding the cursor on the currently selected
    /// task when it's among them. The whole point: a nudge must never start a
    /// timer (or add time) to a random task just because it happens to be
    /// under the cursor.
    pub fn enter_nudge_picker(&mut self, action: NudgePickAction) {
        let abs_list: Vec<usize> = self
            .store
            .tasks()
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.done)
            .map(|(i, _)| i)
            .collect();
        if abs_list.is_empty() {
            self.flash("no open tasks — press n to create one");
            self.nav.mode = Mode::IdleNudge;
            return;
        }
        let cursor = self
            .cur_abs()
            .and_then(|a| abs_list.iter().position(|x| *x == a))
            .unwrap_or(0);
        self.session.nudge_picker = Some(NudgePickerState {
            abs_list,
            cursor,
            action,
        });
        self.nav.mode = Mode::PickNudgeTask;
    }

    /// Move the picker highlight (clamped to the list).
    pub fn nudge_picker_step(&mut self, forward: bool) {
        if let Some(p) = self.session.nudge_picker.as_mut() {
            p.cursor = if forward {
                (p.cursor + 1).min(p.abs_list.len().saturating_sub(1))
            } else {
                p.cursor.saturating_sub(1)
            };
        }
    }

    /// Commit the picker: start the timer (or open add-time) on the chosen
    /// task, then leave the nudge flow back to Normal (restoring the
    /// pre-nudge view).
    pub fn nudge_picker_accept(&mut self) {
        let Some(picker) = self.session.nudge_picker.take() else {
            return;
        };
        let Some(&abs) = picker.abs_list.get(picker.cursor) else {
            self.nav.mode = Mode::IdleNudge;
            return;
        };
        match picker.action {
            NudgePickAction::StartTimer => {
                self.toggle_timer_at(abs);
                self.nav.enter_normal();
                if let Some(v) = self.session.pre_nudge_view.take() {
                    self.set_view(v);
                }
            }
            NudgePickAction::AddTime => {
                // Point the list cursor at the chosen task so the add-time
                // prompt targets it, then open the prompt.
                self.set_view(View::List);
                if let Some(pos) = self.visible_indices().iter().position(|&a| a == abs) {
                    self.nav.cursor = pos;
                }
                self.recompute_visible();
                let body = self
                    .store
                    .tasks()
                    .get(abs)
                    .map(|t| crate::todo::body_only_from_clean(&t.clean_raw))
                    .unwrap_or_default();
                self.draft_clear();
                self.flash(format!("add time to: {body}"));
                self.nav.mode = Mode::PromptAddTime;
            }
        }
    }

    /// Esc from the picker returns to the idle-nudge popup so the user can
    /// pick a different action (or dismiss).
    pub fn nudge_picker_cancel(&mut self) {
        self.session.nudge_picker = None;
        self.nav.mode = Mode::IdleNudge;
    }

    /// `[d]iscard gap` on the stale-timer prompt: stop the timer WITHOUT
    /// crediting the elapsed time — the away period was not billable work.
    /// Strips the `start:` tag and leaves `dur:` untouched. Both the token
    /// the parser treats as the timer tag (the first `start:` token — the
    /// same one `resync_timer` keys off) and any timestamp-shaped `start:`
    /// token are removed, so the timer cannot survive on a leftover tag.
    /// Narrative `start:` tokens the parser ignores (a second `start:` word
    /// later in the body) stay put.
    pub fn discard_stale_timer(&mut self) {
        let Some(abs) = self.store.active_timer_abs() else {
            return;
        };
        let raw = self.store.task_raw(abs).unwrap_or_default();
        // Strip exactly the tokens todo.rs recognizes as the timer tag: any
        // `start:` token with a non-empty (non-quoted) value — the same rule
        // `find_kv` uses, which is what `resync_timer` keys off on reload.
        // That covers the ISO timestamp, hand-typed `start:noon` (resync falls
        // back to now), and duplicates alike, so the timer cannot survive on
        // a leftover tag. A bare narrative `start:` (empty value) is NOT a
        // tag per the parser and survives — the pre-hardening code stripped
        // any token merely beginning with `start:`, destroying prose like
        // "meeting start: sharp".
        let is_timer_tag = |tok: &str| -> bool {
            tok.strip_prefix("start:")
                .is_some_and(|v| !v.is_empty() && !v.starts_with('"'))
        };
        let body = crate::todo::body_after_priority(&raw);
        let prefix = &raw[..raw.len() - body.len()];
        let cleaned = body
            .split_whitespace()
            .filter(|tok| !is_timer_tag(tok))
            .collect::<Vec<_>>()
            .join(" ");
        let updated = if prefix.is_empty() {
            cleaned
        } else {
            format!("{prefix}{cleaned}")
        };
        match self.store.edit_line(abs, &updated) {
            EditOutcome::Saved { abs } => {
                self.flash("discarded unrecorded gap — no time added");
                self.note_timer_activity();
                self.after_mutation(abs);
            }
            EditOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            EditOutcome::Error(e) => self.flash(format!("edit failed: {e}")),
            EditOutcome::Empty | EditOutcome::OutOfRange | EditOutcome::TermNotFound => {}
        }
    }

    /// True when the active timer is running on the task at `abs`.
    pub fn is_timer_running_on(&self, abs: usize) -> bool {
        self.store.is_timer_running_on(abs)
    }

    /// True when any entry (active or archived) carries time logged for
    /// today — the gate for the end-of-day review nudge.
    pub fn has_time_tracked_today(&self) -> bool {
        self.store.has_time_logged_today()
    }

    /// Total seconds of time logged today (active + archived), for the
    /// end-of-day review popup's message.
    pub fn today_tracked_secs(&self) -> u64 {
        let today = self.today();
        let f = |t: &crate::todo::Task| -> u64 {
            if t.dur.unwrap_or(0) == 0 {
                return 0;
            }
            let work = t
                .log
                .as_deref()
                .filter(|s| crate::todo::is_iso_date(s))
                .or(t.created_date.as_deref());
            if work == Some(today) {
                t.dur.unwrap_or(0)
            } else {
                0
            }
        };
        self.tasks().iter().map(f).sum::<u64>()
            + self.store.archive().tasks().iter().map(f).sum::<u64>()
    }

    /// Convert a `dur:VALUE` token in `text` from flexible user input (minutes,
    /// decimal hours, clock time, am/pm shorthand) to raw seconds suitable for
    /// the on-disk todo.txt format. Used by `add_from_draft` when
    /// `manual_time_entry` is set.
    pub fn convert_dur_in_text(&self, text: &str) -> String {
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut result: Vec<String> = Vec::with_capacity(words.len());
        for word in &words {
            if let Some(rest) = word.strip_prefix("dur:") {
                if rest.is_empty() {
                    result.push((*word).to_string());
                } else {
                    let secs = parse_duration_input(rest);
                    result.push(format!("dur:{secs}"));
                }
            } else {
                result.push((*word).to_string());
            }
        }
        result.join(" ")
    }

    /// Toggle timer on the task under the cursor (List view only). When the
    /// task already carries time from a previous day (and the day-boundary
    /// prompt is enabled), asks first whether to continue the same entry or
    /// start a fresh entry for today, instead of silently moving the entry.
    pub fn toggle_timer(&mut self) {
        let Some(abs) = self.cur_task_index_in_tasks() else {
            self.flash("no task selected");
            return;
        };
        if self.should_prompt_day_boundary(abs) {
            self.session.pending_day_boundary = Some((abs, DayBoundaryAction::StartTimer));
            self.nav.push_mode(Mode::PromptDayBoundary);
            return;
        }
        self.toggle_timer_at(abs);
    }

    /// True when starting a timer on `abs` should first ask about the day
    /// boundary: the task has accumulated time whose effective log date is
    /// before today, so a plain continue would move that time onto today's
    /// sheet. Never fires when the timer is already running on the task
    /// (that would be a stop, not a start).
    pub fn should_prompt_day_boundary(&self, abs: usize) -> bool {
        if !self.prefs.prompt_on_day_boundary {
            return false;
        }
        if self.store.is_timer_running_on(abs) {
            return false;
        }
        let Some(t) = self.store.tasks().get(abs) else {
            return false;
        };
        if t.dur.unwrap_or(0) == 0 {
            return false;
        }
        let Some(eff) = effective_log_date(t) else {
            return false;
        };
        eff < self.store.today()
    }

    /// Day-boundary prompt "new entry" choice (timer path): carry the task
    /// forward to a fresh line for today and start the timer on it. Any timer
    /// running on another task is stopped first (its time captured).
    pub fn day_boundary_new_entry(&mut self, abs: usize) {
        // The match evaluates to the follow target: `Some(new)` on success,
        // `None` on every other path. `after_follow` then refreshes exactly
        // once — following the new line on success, plain recompute otherwise
        // — so the success path doesn't recompute twice.
        let follow = match self.store.carry_forward_and_start(abs) {
            CarryForwardOutcome::CarriedStarted {
                new,
                project,
                activity,
                body,
                ..
            } => {
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                self.flash(format!("▶ {proj}{act}{body} — new entry for today"));
                self.session.last_timer_activity = Instant::now();
                Some(new)
            }
            CarryForwardOutcome::Carried { .. } | CarryForwardOutcome::OutOfRange => {
                self.flash("could not start new entry");
                None
            }
            CarryForwardOutcome::Aborted(r) => {
                self.handle_reconcile_abort(r);
                None
            }
            CarryForwardOutcome::Error(e) => {
                self.flash(format!("carry failed: {e}"));
                None
            }
        };
        self.after_follow(follow);
    }

    /// Day-boundary prompt "new entry" choice (add-time path): carry the task
    /// forward, then add `input` as `dur:` on the fresh line.
    pub fn day_boundary_new_entry_add_time(&mut self, abs: usize, input: &str) {
        let secs = parse_duration_input(input);
        if secs == 0 {
            self.flash(format!("invalid duration: {input}"));
            return;
        }
        // As in `day_boundary_new_entry`: the match yields the follow target
        // and the tail refreshes exactly once.
        let follow = match self.store.carry_forward(abs) {
            CarryForwardOutcome::Carried { new, .. } => {
                let raw = self.store.task_raw(new).unwrap_or_default();
                let updated = rebuild_token_line(&raw, "dur:", None, &format!("dur:{secs}"));
                let updated =
                    rebuild_token_line(&updated, "log:", None, &format!("log:{}", self.today()));
                match self.store.edit_line(new, &updated) {
                    EditOutcome::Saved { .. } => {
                        self.flash(format!(
                            "added {} — new entry for today",
                            format_duration(secs, self.prefs.rounding_increment)
                        ));
                        // Adding time is a timer activity: reset the idle-nudge
                        // clock so the popup doesn't re-fire right after.
                        self.note_timer_activity();
                        Some(new)
                    }
                    EditOutcome::Aborted(r) => {
                        self.handle_reconcile_abort(r);
                        None
                    }
                    EditOutcome::Error(e) => {
                        self.flash(format!("edit failed: {e}"));
                        None
                    }
                    EditOutcome::Empty | EditOutcome::OutOfRange | EditOutcome::TermNotFound => {
                        None
                    }
                }
            }
            CarryForwardOutcome::Aborted(r) => {
                self.handle_reconcile_abort(r);
                None
            }
            CarryForwardOutcome::OutOfRange => {
                self.flash("task vanished");
                None
            }
            CarryForwardOutcome::Error(e) => {
                self.flash(format!("carry failed: {e}"));
                None
            }
            // `carry_forward` never returns CarriedStarted; the arm exists so
            // the match stays exhaustive if the enum grows a new returner.
            #[allow(unreachable_patterns)]
            CarryForwardOutcome::CarriedStarted { .. } => {
                self.flash("carry failed");
                None
            }
        };
        self.after_follow(follow);
    }

    /// Toggle timer on the task at `abs` without requiring cursor context.
    /// Used by `interrupt_timer` and `add_from_draft` auto-start.
    pub fn toggle_timer_at(&mut self, abs: usize) {
        match self.store.timer_toggle(abs) {
            TimerOutcome::Started {
                project,
                activity,
                body,
                ..
            } => {
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                self.flash(format!("▶ {proj}{act}— {body}"));
                self.note_timer_activity();
            }
            TimerOutcome::Stopped {
                elapsed_secs,
                total_secs,
                project,
                activity,
                body,
                ..
            } => {
                let inc = self.prefs.rounding_increment;
                let elapsed = format_duration(elapsed_secs, inc);
                let total = format_duration(total_secs, inc);
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                self.flash(format!("■ {proj}{act}{body} — {elapsed} (total {total})"));
                self.note_timer_activity();
            }
            // A midnight-crossing stop was split into one line per day; the
            // flash reports the per-day breakdown so the user sees where the
            // time landed (and that a new entry was created).
            TimerOutcome::StoppedSplit {
                elapsed_secs,
                total_secs,
                chunks,
                project,
                activity,
                body,
                ..
            } => {
                let inc = self.prefs.rounding_increment;
                let elapsed = format_duration(elapsed_secs, inc);
                let total = format_duration(total_secs, inc);
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                let days = chunks
                    .iter()
                    .map(|(d, s)| format!("{d} {}", format_duration(*s, inc)))
                    .collect::<Vec<_>>()
                    .join(" · ");
                self.flash(format!(
                    "■ {proj}{act}{body} — split across midnight: {days} ({elapsed} total {total})"
                ));
                self.note_timer_activity();
            }
            TimerOutcome::Switched {
                from_elapsed_secs,
                from_total_secs,
                from_project,
                from_activity,
                to_project,
                to_activity,
                to_body,
                ..
            } => {
                let from_elapsed =
                    format_duration(from_elapsed_secs, self.prefs.rounding_increment);
                let from_total = format_duration(from_total_secs, self.prefs.rounding_increment);
                let to_proj = to_project.map(|p| format!("+{p} ")).unwrap_or_default();
                let to_act = to_activity.map(|a| format!("@{a} ")).unwrap_or_default();
                let from_proj = from_project.map(|p| format!("+{p}")).unwrap_or_default();
                let from_act = from_activity.map(|a| format!("@{a}")).unwrap_or_default();
                self.flash(format!("■ {from_proj}{from_act} {from_elapsed} (total {from_total}) · ▶ {to_proj}{to_act} {to_body}"));
                self.note_timer_activity();
            }
            TimerOutcome::OutOfRange => self.flash("no task selected"),
            TimerOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            TimerOutcome::Error(e) => self.flash(format!("timer: {e}")),
        }
        self.recompute_visible();
    }

    /// Quick interruption: stop the running timer and open a blank Insert
    /// dialog so the user can log the interruption (phone call, colleague
    /// drop-in) without losing the current timer's accumulated time.
    pub fn interrupt_timer(&mut self) {
        if !self.timer_running() {
            self.flash("no timer to interrupt");
            return;
        }
        let Some(active_abs) = self.store.active_timer_abs() else {
            return;
        };
        match self.store.timer_toggle(active_abs) {
            TimerOutcome::Stopped {
                elapsed_secs,
                project,
                activity,
                body,
                ..
            } => {
                let elapsed = format_duration(elapsed_secs, self.prefs.rounding_increment);
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                self.flash(format!(
                    "interrupted {proj}{act}{body} ({elapsed}) — enter new task"
                ));
                self.note_timer_activity();
                // Open a blank Insert dialog for the interruption entry.
                self.session.manual_time_entry = true;
                self.session.auto_start_on_save = true;
                self.draft_clear();
                self.nav.mode = Mode::Insert;
                self.selection.exit_edit();
            }
            TimerOutcome::Aborted(r) => {
                self.note_timer_activity();
                self.handle_reconcile_abort(r);
            }
            TimerOutcome::Error(e) => {
                self.note_timer_activity();
                self.flash(format!("timer: {e}"));
            }
            _ => {}
        }
        self.recompute_visible();
    }

    /// Add time to the current task's `dur:` field from a user-supplied
    /// duration string (minutes, hours, or clock time). When the task's time
    /// belongs to a previous day, prompts to carry forward first so the
    /// existing entry isn't silently moved onto today's sheet.
    pub fn add_time_to_current_from_input(&mut self, input: &str) {
        let Some(abs) = self.cur_task_index_in_tasks() else {
            self.flash("no task selected");
            return;
        };
        if self.should_prompt_day_boundary(abs) {
            self.session.pending_day_boundary = Some((
                abs,
                DayBoundaryAction::AddTime {
                    input: input.to_string(),
                },
            ));
            self.nav.push_mode(Mode::PromptDayBoundary);
            return;
        }
        self.add_time_to_current_at(abs, input);
    }

    /// Add `input` (minutes, hours, or clock time) as `dur:` to the task at
    /// `abs`, stamping today's `log:`. Shared by the direct add-time path and
    /// the day-boundary prompt's "continue same entry" choice.
    pub fn add_time_to_current_at(&mut self, abs: usize, input: &str) {
        let secs = parse_duration_input(input);
        if secs == 0 {
            self.flash(format!("invalid duration: {input}"));
            return;
        }
        let current = self.store.tasks()[abs].dur.unwrap_or(0);
        let total = current + secs;
        // Replace/add the `dur:` token and stamp the `log:` date through the
        // store's shared token-rewrite helper, so all raw-line surgery stays
        // in one place. The log date makes manual additions show up on
        // today's timesheet even when the task was created earlier.
        let raw = self.store.task_raw(abs).unwrap_or_default();
        let updated = rebuild_token_line(&raw, "dur:", None, &format!("dur:{total}"));
        let updated = rebuild_token_line(&updated, "log:", None, &format!("log:{}", self.today()));
        match self.store.edit_line(abs, &updated) {
            EditOutcome::Saved { abs } => {
                let added = format_duration(secs, self.prefs.rounding_increment);
                let total_str = format_duration(total, self.prefs.rounding_increment);
                let body = crate::todo::body_only(&updated);
                self.flash(format!("added {added} — {body} (total {total_str})"));
                // Adding time is a timer activity: reset the idle-nudge clock.
                self.note_timer_activity();
                self.after_mutation(abs);
            }
            EditOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            EditOutcome::Error(e) => self.flash(format!("edit failed: {e}")),
            EditOutcome::Empty | EditOutcome::OutOfRange | EditOutcome::TermNotFound => {}
        }
    }

    /// Toggle the `bill:n` tag on the current task. Adds `bill:n` (marking
    /// non-billable) or removes it (marking billable). Flashes the new status.
    pub fn toggle_billable(&mut self) {
        let Some(abs) = self.cur_task_index_in_tasks() else {
            self.flash("no task selected");
            return;
        };
        self.toggle_billable_at(abs);
    }

    /// Toggle `bill:n` on the task at `abs` (absolute index into tasks).
    /// Used from Timesheet view where the cursor tracks groups, not tasks.
    pub fn toggle_billable_at(&mut self, abs: usize) {
        let raw = self.store.task_raw(abs).unwrap_or_default();
        let (updated, became_nonbillable) = if raw.contains(" bill:n") || raw.ends_with(" bill:n") {
            // Remove `bill:n` — strip the token and collapse whitespace.
            let cleaned = raw
                .split_whitespace()
                .filter(|tok| *tok != "bill:n")
                .collect::<Vec<_>>()
                .join(" ");
            (cleaned, false)
        } else {
            (format!("{raw} bill:n"), true)
        };
        match self.store.edit_line(abs, &updated) {
            EditOutcome::Saved { abs } => {
                if became_nonbillable {
                    self.flash("marked as non-billable");
                } else {
                    self.flash("marked as billable");
                }
                self.after_mutation(abs);
            }
            EditOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            EditOutcome::Error(e) => self.flash(format!("edit failed: {e}")),
            EditOutcome::Empty | EditOutcome::OutOfRange | EditOutcome::TermNotFound => {}
        }
    }

    /// Stop the running timer (if any) on quit.
    pub fn stop_timer_on_quit(&mut self) {
        match self.store.stop_timer_on_quit() {
            TimerQuitOutcome::Stopped { total_secs, .. } => {
                self.flash(format!(
                    "timer stopped ({} total)",
                    format_duration(total_secs, self.prefs.rounding_increment)
                ));
            }
            TimerQuitOutcome::NoTimer => {}
            TimerQuitOutcome::Error(e) => {
                self.flash(format!("timer stop failed: {e}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::View;
    use crate::app::test_support::build_app;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }

    // ---- day-boundary prompt (one line per task-day) ----

    #[test]
    fn toggle_timer_prompts_on_previous_day_task() {
        let mut app = build_app("Draft +Smith dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.toggle_timer();

        assert_eq!(app.nav.mode(), Mode::PromptDayBoundary);
        assert!(!app.timer_running(), "no timer until the prompt resolves");
        // Esc cancels: back to Normal, task untouched.
        crate::interactive::handle_day_boundary(&mut app, esc());
        assert_eq!(app.nav.mode(), Mode::Normal);
        assert_eq!(app.tasks().len(), 1);
        assert!(!app.tasks()[0].done);
        assert!(app.session.pending_day_boundary.is_none());
    }

    #[test]
    fn toggle_timer_prompts_not_for_same_day_or_no_time() {
        let app = build_app("Today +W dur:1800 log:2026-05-06\nNo time yet\nFresh dur:0\n");
        assert!(
            !app.should_prompt_day_boundary(0),
            "log == today: no prompt"
        );
        assert!(!app.should_prompt_day_boundary(1), "no dur: no prompt");
        assert!(!app.should_prompt_day_boundary(2), "dur:0: no prompt");
        assert!(
            !app.should_prompt_day_boundary(99),
            "out of range: no prompt"
        );
        // Previous-day time but the feature disabled: no prompt either.
        let mut app2 = build_app("Old +W dur:7200 log:2026-05-05\n");
        app2.prefs.prompt_on_day_boundary = false;
        assert!(!app2.should_prompt_day_boundary(0));
    }

    #[test]
    fn day_boundary_continue_starts_timer_without_carrying() {
        let mut app = build_app("Draft +Smith dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        app.toggle_timer();
        assert_eq!(app.nav.mode(), Mode::PromptDayBoundary);

        crate::interactive::handle_day_boundary(&mut app, key('c'));

        assert!(app.timer_running());
        assert_eq!(app.tasks().len(), 1, "no carry on continue");
        assert!(!app.tasks()[0].done);
        assert_eq!(app.nav.mode(), Mode::Normal);
    }

    #[test]
    fn day_boundary_new_entry_carries_and_starts_timer() {
        let mut app = build_app("(A) Draft +Smith @drafting dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        app.toggle_timer();
        assert_eq!(app.nav.mode(), Mode::PromptDayBoundary);

        crate::interactive::handle_day_boundary(&mut app, key('n'));

        assert_eq!(app.tasks().len(), 2);
        assert!(app.tasks()[0].done, "old line consumed");
        assert_eq!(app.tasks()[0].dur, Some(7200), "old line keeps its time");
        let new_raw = &app.tasks()[1].raw;
        assert!(
            new_raw.starts_with("(A) 2026-05-06"),
            "priority + fresh date: {new_raw}"
        );
        assert!(new_raw.contains("Draft"), "narrative carried: {new_raw}");
        assert!(new_raw.contains("+Smith"));
        assert!(new_raw.contains("@drafting"));
        assert!(app.timer_running(), "timer started on the new line");
        assert_eq!(app.store.active_timer_abs(), Some(1));
        assert_eq!(app.nav.mode(), Mode::Normal);
    }

    #[test]
    fn add_time_prompts_on_previous_day_and_new_entry_carries_time() {
        let mut app = build_app("Draft +Smith dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("30");
        assert_eq!(app.nav.mode(), Mode::PromptDayBoundary);

        crate::interactive::handle_day_boundary(&mut app, key('n'));

        assert_eq!(app.tasks().len(), 2);
        assert!(app.tasks()[0].done);
        let new_raw = &app.tasks()[1].raw;
        assert!(
            new_raw.contains("dur:1800"),
            "30m added to new entry: {new_raw}"
        );
        assert!(
            new_raw.contains("log:2026-05-06"),
            "log stamped today: {new_raw}"
        );
    }

    #[test]
    fn add_time_continue_adds_to_same_entry() {
        let mut app = build_app("Draft +Smith dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("30");
        assert_eq!(app.nav.mode(), Mode::PromptDayBoundary);

        crate::interactive::handle_day_boundary(&mut app, key('c'));

        assert_eq!(app.tasks().len(), 1);
        assert!(app.tasks()[0].raw.contains("dur:9000"));
        assert!(app.tasks()[0].raw.contains("log:2026-05-06"));
    }

    #[test]
    fn day_boundary_prompt_over_running_switch_captures_other_timer() {
        // Timer running on task 0 (today); user presses t on task 1 whose time
        // is from a previous day → prompt → 'n' carries task 1 and starts the
        // timer on the new line, while task 0's elapsed is captured.
        let start = (chrono::Local::now() - chrono::Duration::seconds(65))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let mut app = build_app(&format!(
            "First +A start:{start}\nSecond +B dur:7200 log:2026-05-05\n"
        ));
        app.nav.cursor = 1;
        app.recompute_visible();

        app.toggle_timer();
        assert_eq!(app.nav.mode(), Mode::PromptDayBoundary);

        crate::interactive::handle_day_boundary(&mut app, key('n'));

        assert_eq!(app.tasks().len(), 3);
        assert!(app.tasks()[1].done, "carried task consumed");
        assert!(
            app.tasks()[0].dur.unwrap_or(0) >= 60,
            "other timer's elapsed captured: {:?}",
            app.tasks()[0].dur
        );
        assert!(app.tasks()[0].start.is_none(), "other timer stopped");
        assert_eq!(app.store.active_timer_abs(), Some(2));
        assert!(app.tasks()[2].raw.contains("Second"));
        assert!(app.tasks()[2].start.is_some());
    }

    #[test]
    fn carry_forward_save_completes_old_and_creates_today_entry() {
        let mut app = build_app("(A) Draft motion +Smith @drafting dur:7200 log:2026-05-05\n");
        // The upgraded N flow pre-fills the draft with the carried line,
        // including the priority; here we simulate a polished save.
        app.session.carry_forward_from = Some(0);
        app.draft_set("(A) Draft motion revised +Smith @drafting".into());

        let outcome = app.add_from_draft();

        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert_eq!(app.tasks().len(), 2);
        assert!(app.tasks()[0].done);
        assert_eq!(
            app.tasks()[1].raw,
            "(A) 2026-05-06 Draft motion revised +Smith @drafting"
        );
        assert_eq!(app.session.carry_forward_from, None, "cleared on save");
    }

    // ---- stale-timer startup prompt ----

    fn stale_start(secs: i64) -> String {
        (chrono::Local::now() - chrono::Duration::seconds(secs))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string()
    }

    /// A restored timer whose elapsed time has blown past the long-timer
    /// threshold (the app was closed/killed while it ran) must be flagged as
    /// stale at startup.
    #[test]
    fn stale_timer_detected_when_elapsed_past_threshold() {
        let app = build_app(&format!("Draft +Smith start:{}\n", stale_start(7300)));
        assert!(app.timer_running());
        assert!(
            app.stale_timer_at_startup(),
            "2h+ elapsed at launch must be treated as a zombie session"
        );
    }

    #[test]
    fn stale_timer_not_detected_under_threshold() {
        let app = build_app(&format!("Draft +Smith start:{}\n", stale_start(60)));
        assert!(!app.stale_timer_at_startup());
    }

    #[test]
    fn stale_timer_not_detected_when_no_timer() {
        let app = build_app("Draft +Smith\n");
        assert!(!app.stale_timer_at_startup());
    }

    /// A timer whose start stamp lives on a previous calendar day is stale
    /// even when the elapsed time is under the long-timer threshold — an
    /// overnight stay would silently bill the whole gap on stop.
    #[test]
    fn stale_timer_detected_when_start_crossed_day_under_threshold() {
        // The store's `today` is 2026-05-06; a start stamped the day before
        // must be stale regardless of the 2h elapsed threshold.
        let app = build_app("Draft +Smith start:2026-05-05T23:50:00\n");
        assert!(app.timer_running());
        // Force the elapsed-based check off so only the day-crossing branch
        // can make it stale.
        assert!(
            timer_start_crossed_day(
                app.active_timer_task().and_then(|t| t.start.as_deref()),
                "2026-05-06"
            ),
            "precondition: start dated the previous day"
        );
        assert!(app.stale_timer_at_startup());
    }

    /// The day-crossing predicate itself: only a start dated strictly before
    /// `today` is stale.
    #[test]
    fn timer_start_crossed_day_predicate() {
        assert!(timer_start_crossed_day(
            Some("2026-05-05T23:59:59"),
            "2026-05-06"
        ));
        assert!(!timer_start_crossed_day(
            Some("2026-05-06T00:00:00"),
            "2026-05-06"
        ));
        assert!(!timer_start_crossed_day(None, "2026-05-06"));
        assert!(!timer_start_crossed_day(
            Some("2026-05-06T23:59:59"),
            "2026-05-05"
        ));
    }

    /// `[d]iscard gap` must strip `start:` without crediting the elapsed
    /// time — the away period was not billable work. Pre-existing time tags
    /// (`dur:`, `log:`) must survive untouched.
    #[test]
    fn discard_stale_timer_strips_start_keeps_dur() {
        let raw = format!(
            "Draft +Smith dur:3600 log:2026-05-05 start:{}\n",
            stale_start(7300)
        );
        let mut app = build_app(&raw);
        assert!(app.timer_running());

        app.discard_stale_timer();

        assert!(!app.timer_running(), "discard must stop the timer");
        let raw = app.task_raw(0).unwrap_or_default();
        assert!(!raw.contains("start:"), "start: must be stripped: {raw}");
        assert!(
            raw.contains("dur:3600"),
            "existing dur must be untouched: {raw}"
        );
        assert!(
            raw.contains("log:2026-05-05"),
            "existing log date must survive: {raw}"
        );
        assert!(
            !raw.contains("dur:10900"),
            "the gap must not be credited (3600 + ~7300): {raw}"
        );
    }

    /// Discard strips the parsed `start:` tag but leaves a bare narrative
    /// `start:` (empty value) untouched — the pre-hardening code removed any
    /// token merely *beginning* with `start:`, destroying prose like
    /// "meeting start: sharp".
    #[test]
    fn discard_stale_timer_keeps_narrative_start_token() {
        let raw = format!("Draft start:{} start:\n", stale_start(7300));
        let mut app = build_app(&raw);
        assert!(app.timer_running());

        app.discard_stale_timer();

        assert!(!app.timer_running(), "discard must stop the timer");
        let raw = app.task_raw(0).unwrap_or_default();
        assert!(
            raw.contains("start:"),
            "narrative start: must survive: {raw}"
        );
        assert!(
            !raw.contains("start:2026"),
            "the timestamp tag must be stripped: {raw}"
        );
    }

    /// A hand-typed non-timestamp start (`start:noon`) is still a timer tag
    /// per the parser (resync falls back to now), so discard must strip it
    /// too or the "stopped" timer would keep running from load.
    #[test]
    fn discard_stale_timer_strips_hand_typed_start() {
        let mut app = build_app("Draft start:noon\n");
        assert!(app.timer_running());

        app.discard_stale_timer();

        assert!(!app.timer_running(), "hand-typed tag must not keep a timer");
        assert!(!app.task_raw(0).unwrap_or_default().contains("start:noon"));
    }

    /// `[k]eep counting` must leave the timer running and restore the
    /// pre-nudge view.
    #[test]
    fn keep_stale_timer_leaves_timer_running_and_restores_view() {
        let mut app = build_app(&format!("Draft +Smith start:{}\n", stale_start(7300)));
        app.session.pre_nudge_view = Some(View::Timesheet);
        app.nav.mode = Mode::StaleTimer;

        app.keep_stale_timer();

        assert!(app.timer_running(), "keep must leave the timer running");
        assert_eq!(app.nav.mode, Mode::Normal);
        assert_eq!(app.nav.view, View::Timesheet, "pre-nudge view restored");
        assert!(app.session.pre_nudge_view.is_none());
    }

    // ---- launch-time idle backdate (nothing tracked today) ----

    /// A fresh launch with no time tracked today must backdate the idle clock
    /// so the first Normal-mode tick nudges immediately — no grace period
    /// after hours spent outside the app.
    #[test]
    fn check_nudges_backdates_idle_when_nothing_tracked_today() {
        let mut app = build_app("Buy milk\n");
        assert!(!app.timer_running());
        assert!(
            !app.store.has_time_logged_today(),
            "precondition: nothing tracked today"
        );

        assert!(app.check_nudges());

        assert!(app.session.idle_backdated);
        assert_eq!(
            app.session.idle_reason,
            IdleReason::UntrackedDay,
            "popup message should say nothing tracked today"
        );
        assert_eq!(app.nav.mode, Mode::IdleNudge, "first tick must nudge");
    }

    /// When time has already been logged today, no backdate happens — the
    /// idle clock starts fresh and the nudge must not fire immediately.
    #[test]
    fn check_nudges_does_not_backdate_when_time_tracked_today() {
        let mut app = build_app("2026-05-06 Work +X @dev dur:600 log:2026-05-06\n");
        assert!(app.store.has_time_logged_today());

        assert!(!app.check_nudges());

        assert!(
            app.session.idle_backdated,
            "flag set (one-shot) but no nudge"
        );
        assert_eq!(
            app.session.idle_reason,
            IdleReason::TimerStopped,
            "reason must stay the ordinary timer-stopped case"
        );
        assert_eq!(app.nav.mode, Mode::Normal);
    }

    /// A running timer must also suppress the backdate (the timer is the
    /// activity — the user is mid-session, not idle).
    #[test]
    fn check_nudges_does_not_backdate_when_timer_running() {
        let mut app = build_app(&format!("Draft +Smith start:{}\n", stale_start(60)));
        assert!(app.timer_running());

        assert!(!app.check_nudges());

        assert_eq!(app.session.idle_reason, IdleReason::TimerStopped);
        assert_eq!(app.nav.mode, Mode::Normal);
    }

    /// Any real timer activity flips the reason back to the ordinary case,
    /// so a later idle nudge doesn't keep claiming "nothing tracked today".
    #[test]
    fn note_timer_activity_resets_idle_reason() {
        let mut app = build_app("Task\n");
        app.session.idle_reason = IdleReason::UntrackedDay;
        app.session.last_timer_activity =
            std::time::Instant::now() - std::time::Duration::from_secs(901);

        app.toggle_timer_at(0); // start

        assert_eq!(app.session.idle_reason, IdleReason::TimerStopped);
        assert!(app.session.last_timer_activity.elapsed().as_secs() < 5);
    }

    // ---- end-of-day review nudge ----

    /// Once the configured review time has passed and something is tracked
    /// today, the review fires — once per day.
    #[test]
    fn review_nudge_fires_once_after_review_time() {
        let mut app = build_app("2026-05-06 Work +X @dev dur:600 log:2026-05-06\n");
        app.prefs.review_time = Some("00:00".into()); // always passed
        assert!(app.has_time_tracked_today());

        assert!(app.check_nudges());
        assert_eq!(app.nav.mode, Mode::ReviewNudge);

        // Already shown today: must not re-fire.
        app.nav.mode = Mode::Normal;
        assert!(!app.check_nudges(), "review must not re-fire the same day");
        assert_eq!(app.nav.mode, Mode::Normal);
    }

    /// The review nudge only fires when time has been tracked today — the
    /// "nothing tracked at all" failure is the idle nudge's job.
    #[test]
    fn review_nudge_skipped_when_nothing_tracked_today() {
        let mut app = build_app("Buy milk\n");
        app.prefs.review_time = Some("00:00".into());
        assert!(!app.has_time_tracked_today());

        // The launch-time backdate fires the IDLE nudge instead (and the
        // review block yields to it — mode is no longer Normal by then).
        assert!(app.check_nudges());
        assert_eq!(app.nav.mode, Mode::IdleNudge);
        assert!(
            app.session.review_nudge_date.is_none(),
            "review must not consume its once-per-day slot when it didn't fire"
        );
    }

    /// A malformed review time disables the feature entirely.
    #[test]
    fn review_nudge_not_fired_with_malformed_time() {
        let mut app = build_app("2026-05-06 Work +X @dev dur:600 log:2026-05-06\n");
        app.prefs.review_time = Some("99:99".into());
        assert!(!app.check_nudges());
        assert_eq!(app.nav.mode, Mode::Normal);
    }

    /// today_tracked_secs sums only today's logged time, active + archived.
    #[test]
    fn today_tracked_secs_counts_today_only() {
        let app = build_app(
            "2026-05-06 Today +X @dev dur:600 log:2026-05-06\n\
             2026-05-05 Yesterday +Y @dev dur:900 log:2026-05-05\n",
        );
        assert_eq!(app.today_tracked_secs(), 600);
    }

    // ---- nudge task picker (S / M from the idle nudge) ----

    /// The picker must list open tasks (done excluded) and seed the cursor
    /// on the currently selected task when it's among them.
    #[test]
    fn nudge_picker_lists_open_tasks_and_seeds_cursor() {
        let mut app = build_app("First +A\nSecond +B\nx 2026-05-05 Done +C\n");
        app.nav.cursor = 1;
        app.recompute_visible();

        app.enter_nudge_picker(NudgePickAction::StartTimer);

        assert_eq!(app.nav.mode, Mode::PickNudgeTask);
        let picker = app
            .session
            .nudge_picker
            .as_ref()
            .expect("picker must be open");
        assert_eq!(picker.abs_list, vec![0, 1], "done tasks must be excluded");
        assert_eq!(picker.cursor, 1, "seed on the selected task");
        assert_eq!(picker.action, NudgePickAction::StartTimer);
    }

    /// Enter on the picker starts the timer on the CHOSEN task — not the
    /// cursor's task — and returns to the pre-nudge view.
    #[test]
    fn nudge_picker_start_timer_targets_chosen_task() {
        let mut app = build_app("First +A\nSecond +B\n");
        // Cursor sits on task 0; the picker will choose task 1 instead.
        app.nav.cursor = 0;
        app.recompute_visible();
        app.session.pre_nudge_view = Some(View::Timesheet);

        app.enter_nudge_picker(NudgePickAction::StartTimer);
        app.nudge_picker_step(true); // move to task 1
        app.nudge_picker_accept();

        assert!(
            app.is_timer_running_on(1),
            "timer must run on the CHOSEN task, not the cursor's"
        );
        assert!(!app.is_timer_running_on(0));
        assert_eq!(app.nav.mode, Mode::Normal);
        assert_eq!(app.nav.view, View::Timesheet, "pre-nudge view restored");
        assert!(app.session.nudge_picker.is_none());
    }

    /// Enter on the add-time picker opens the add-time prompt targeting the
    /// chosen task, with the confirmation flash naming it.
    #[test]
    fn nudge_picker_add_time_targets_chosen_task() {
        let mut app = build_app("First +A\nSecond +B\n");
        app.nav.cursor = 1;
        app.recompute_visible();

        app.enter_nudge_picker(NudgePickAction::AddTime);
        app.nudge_picker_step(false); // move up to task 0
        app.nudge_picker_accept();

        assert_eq!(app.nav.mode, Mode::PromptAddTime);
        assert_eq!(app.nav.view, View::List, "add-time needs the list view");
        assert_eq!(app.nav.cursor, 0, "cursor must point at the chosen task");
        assert!(
            app.flash_active()
                .is_some_and(|m| m.contains("First") && m.contains("add time to")),
            "flash must name the chosen task"
        );
    }

    /// Esc from the picker returns to the idle-nudge popup.
    #[test]
    fn nudge_picker_esc_returns_to_idle_nudge() {
        let mut app = build_app("First\n");
        app.enter_nudge_picker(NudgePickAction::StartTimer);

        app.nudge_picker_cancel();

        assert_eq!(app.nav.mode, Mode::IdleNudge);
        assert!(app.session.nudge_picker.is_none());
    }

    /// No open tasks: the picker can't open; stay on the idle nudge.
    #[test]
    fn nudge_picker_empty_stays_on_idle_nudge() {
        let mut app = build_app("");
        app.enter_nudge_picker(NudgePickAction::StartTimer);
        assert_eq!(app.nav.mode, Mode::IdleNudge);
    }

    // ---- manual entries reset the idle-nudge clock ----

    /// `M → A` (add time to current task) is a timer activity: the idle-nudge
    /// clock must reset so the "No timer running!" popup doesn't fire moments
    /// after the user just logged a forgotten block.
    #[test]
    fn add_time_resets_idle_nudge_clock() {
        let mut app = build_app("Draft +Smith\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        // Make the idle clock look stale (past the 15-min default).
        app.session.last_timer_activity =
            std::time::Instant::now() - std::time::Duration::from_secs(901);

        app.add_time_to_current_from_input("30");

        assert!(
            app.session.last_timer_activity.elapsed().as_secs() < 5,
            "adding time must reset the idle-nudge clock"
        );
    }

    /// `M → N` (manual blank entry with a dur:) is also a timer activity and
    /// must reset the idle-nudge clock.
    #[test]
    fn manual_entry_save_resets_idle_nudge_clock() {
        let mut app = build_app("");
        app.session.manual_time_entry = true;
        app.session.last_timer_activity =
            std::time::Instant::now() - std::time::Duration::from_secs(901);
        app.draft_set("Call with Jim +Smith @phone dur:30".into());

        let outcome = app.add_from_draft();

        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert!(
            app.session.last_timer_activity.elapsed().as_secs() < 5,
            "saving a manual entry must reset the idle-nudge clock"
        );
    }

    /// A plain new task (`n`) is NOT a timer activity — it must not reset the
    /// clock, so the nudge can still remind the user they haven't timed work.
    #[test]
    fn plain_add_does_not_reset_idle_nudge_clock() {
        let mut app = build_app("");
        app.session.last_timer_activity =
            std::time::Instant::now() - std::time::Duration::from_secs(901);
        app.draft_set("Buy milk".into());

        let outcome = app.add_from_draft();

        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert!(
            app.session.last_timer_activity.elapsed().as_secs() >= 900,
            "a plain add must not reset the idle-nudge clock"
        );
    }

    // ---- long-timer nudge popup ----

    /// A timer that has run past the long-timer threshold fires the popup
    /// from Normal mode, remembering the pre-nudge view for later restore.
    #[test]
    fn long_timer_nudge_fires_from_normal_mode() {
        // Backdate the start tag so elapsed exceeds the 2h default threshold.
        let start = (chrono::Local::now() - chrono::Duration::seconds(7300))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let mut app = build_app(&format!("Draft +Smith start:{start}\n"));
        assert!(app.timer_running());
        assert!(app.timer_elapsed_secs().unwrap_or(0) > app.long_timer_nudge_seconds());
        app.nav.view = View::Timesheet;

        assert!(app.check_nudges());

        assert_eq!(app.nav.mode, Mode::LongTimerNudge);
        assert!(app.session.long_timer_nudge_active);
        assert_eq!(
            app.session.pre_nudge_view,
            Some(View::Timesheet),
            "view before the nudge is remembered"
        );
    }

    /// `S` on the long-timer nudge stops the timer (capturing the elapsed
    /// time) and restores the pre-nudge view.
    #[test]
    fn long_timer_nudge_stop_restores_view_and_stops_timer() {
        let start = (chrono::Local::now() - chrono::Duration::seconds(7300))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let mut app = build_app(&format!("Draft +Smith start:{start}\n"));
        app.nav.view = View::Archive;
        app.check_nudges();
        assert_eq!(app.nav.mode, Mode::LongTimerNudge);

        crate::interactive::handle_long_timer_nudge(&mut app, key('S'));

        assert!(!app.timer_running(), "S stops the timer");
        assert_eq!(app.nav.mode, Mode::Normal);
        assert_eq!(app.nav.view, View::Archive, "pre-nudge view restored");
        assert!(app.session.pre_nudge_view.is_none(), "consumed on stop");
        assert!(app.tasks()[0].dur.unwrap_or(0) > 7200, "elapsed captured");
    }

    /// `D` dismisses the popup but leaves the timer running; the nudge flag
    /// stays set so the status bar keeps signalling.
    #[test]
    fn long_timer_nudge_dismiss_keeps_timer_running() {
        let start = (chrono::Local::now() - chrono::Duration::seconds(7300))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let mut app = build_app(&format!("Draft +Smith start:{start}\n"));
        app.check_nudges();
        assert_eq!(app.nav.mode, Mode::LongTimerNudge);

        crate::interactive::handle_long_timer_nudge(&mut app, key('D'));

        assert!(app.timer_running(), "D dismisses without stopping");
        assert_eq!(app.nav.mode, Mode::Normal);
        assert!(
            app.session.long_timer_nudge_active,
            "flag stays for status bar"
        );
    }

    /// A timer still under the threshold must not fire the nudge or touch
    /// the view.
    #[test]
    fn long_timer_nudge_does_not_fire_under_threshold() {
        let start = (chrono::Local::now() - chrono::Duration::seconds(60))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let mut app = build_app(&format!("Draft +Smith start:{start}\n"));
        app.nav.view = View::Timesheet;

        assert!(!app.check_nudges());

        assert_eq!(app.nav.mode, Mode::Normal);
        assert!(!app.session.long_timer_nudge_active);
        assert!(app.session.pre_nudge_view.is_none());
        assert!(app.timer_running());
    }
}
