//! Timer lifecycle: toggle, interrupt, stop-on-quit, and manual time entry.
//! Also owns nudge detection (idle and long-running) and billable toggling.
//!
//! All methods are `impl App` because they need `App` context — the store
//! for timer operations, `flash()` for user feedback, and `prefs` for
//! nudge thresholds.

use super::session::DayBoundaryAction;
use super::{App, Mode, format_duration, parse_duration_input};
use crate::core::outcome::{CarryForwardOutcome, EditOutcome, TimerOutcome, TimerQuitOutcome};
use crate::core::rebuild_token_line;
use crate::todo::Task;
use std::time::Instant;

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
        if matches!(self.nav.mode, Mode::IdleNudge | Mode::LongTimerNudge) {
            return false;
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

    /// True when the active timer is running on the task at `abs`.
    pub fn is_timer_running_on(&self, abs: usize) -> bool {
        self.store.is_timer_running_on(abs)
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
                            format_duration(secs)
                        ));
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
                self.session.last_timer_activity = Instant::now();
            }
            TimerOutcome::Stopped {
                elapsed_secs,
                total_secs,
                project,
                activity,
                body,
                ..
            } => {
                let elapsed = format_duration(elapsed_secs);
                let total = format_duration(total_secs);
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                self.flash(format!("■ {proj}{act}{body} — {elapsed} (total {total})"));
                self.session.last_timer_activity = Instant::now();
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
                let elapsed = format_duration(elapsed_secs);
                let total = format_duration(total_secs);
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                let days = chunks
                    .iter()
                    .map(|(d, s)| format!("{d} {}", format_duration(*s)))
                    .collect::<Vec<_>>()
                    .join(" · ");
                self.flash(format!(
                    "■ {proj}{act}{body} — split across midnight: {days} ({elapsed} total {total})"
                ));
                self.session.last_timer_activity = Instant::now();
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
                let from_elapsed = format_duration(from_elapsed_secs);
                let from_total = format_duration(from_total_secs);
                let to_proj = to_project.map(|p| format!("+{p} ")).unwrap_or_default();
                let to_act = to_activity.map(|a| format!("@{a} ")).unwrap_or_default();
                let from_proj = from_project.map(|p| format!("+{p}")).unwrap_or_default();
                let from_act = from_activity.map(|a| format!("@{a}")).unwrap_or_default();
                self.flash(format!("■ {from_proj}{from_act} {from_elapsed} (total {from_total}) · ▶ {to_proj}{to_act} {to_body}"));
                self.session.last_timer_activity = Instant::now();
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
                let elapsed = format_duration(elapsed_secs);
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                self.flash(format!(
                    "interrupted {proj}{act}{body} ({elapsed}) — enter new task"
                ));
                self.session.last_timer_activity = Instant::now();
                // Open a blank Insert dialog for the interruption entry.
                self.session.manual_time_entry = true;
                self.session.auto_start_on_save = true;
                self.draft_clear();
                self.nav.mode = Mode::Insert;
                self.selection.exit_edit();
            }
            TimerOutcome::Aborted(r) => {
                self.session.last_timer_activity = Instant::now();
                self.handle_reconcile_abort(r);
            }
            TimerOutcome::Error(e) => {
                self.session.last_timer_activity = Instant::now();
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
                let added = format_duration(secs);
                let total_str = format_duration(total);
                let body = crate::todo::body_only(&updated);
                self.flash(format!("added {added} — {body} (total {total_str})"));
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
                    format_duration(total_secs)
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
