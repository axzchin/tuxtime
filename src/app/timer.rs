//! Timer lifecycle: toggle, interrupt, stop-on-quit, and manual time entry.
//! Also owns nudge detection (idle and long-running) and billable toggling.
//!
//! All methods are `impl App` because they need `App` context — the store
//! for timer operations, `flash()` for user feedback, and `prefs` for
//! nudge thresholds.

use std::time::Instant;
use crate::core::outcome::{TimerOutcome, TimerQuitOutcome};
use crate::todo::Task;
use super::{App, Mode, parse_duration_input, format_duration};

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
    /// - If timer running and elapsed > threshold → set `long_timer_nudge_active`.
    ///   Returns true when the UI should redraw.
    pub fn check_nudges(&mut self) -> bool {
        // If the nudge is already showing, don't re-trigger.
        if matches!(self.nav.mode, Mode::IdleNudge) {
            return false;
        }
        let idle_secs = self.session.last_timer_activity.elapsed().as_secs();
        // Idle nudge: fire regardless of mode, but first dismiss the overlay
        // so the popup is visible. Save the pre-nudge view for Dismiss.
        if !self.timer_running() && idle_secs >= self.prefs.idle_nudge_seconds {
            self.session.pre_nudge_view = Some(self.nav.view);
            self.exit_overlay_to_normal();
            self.nav.mode = Mode::IdleNudge;
            return true;
        }
        let was_active = self.session.long_timer_nudge_active;
        if self.timer_running() {
            let elapsed = self.timer_elapsed_secs().unwrap_or(0);
            let now_active = elapsed >= self.prefs.long_timer_nudge_seconds;
            self.session.long_timer_nudge_active = now_active;
        } else {
            self.session.long_timer_nudge_active = false;
        }
        was_active != self.session.long_timer_nudge_active
    }

    /// Dismiss any overlay/dialog mode back to Normal, discarding transient
    /// state (draft, selection). Only used for the idle nudge, which needs a
    /// clean slate for the popup. The long-timer nudge just sets a flag and
    /// lets the status bar show the indicator without destroying user work.
    fn exit_overlay_to_normal(&mut self) {
        self.draft_clear();
        self.selection.exit_edit();
        self.selection.clear();
        self.session.manual_time_entry = false;
        self.nav.mode = Mode::Normal;
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

    /// Toggle timer on the task under the cursor (List view only).
    pub fn toggle_timer(&mut self) {
        let Some(abs) = self.cur_task_index_in_tasks() else {
            self.flash("no task selected");
            return;
        };
        self.toggle_timer_at(abs);
    }

    /// Toggle timer on the task at `abs` without requiring cursor context.
    /// Used by `interrupt_timer` and `add_from_draft` auto-start.
    pub fn toggle_timer_at(&mut self, abs: usize) {
        match self.store.timer_toggle(abs) {
            TimerOutcome::Started { project, activity, body, .. } => {
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                self.flash(format!("▶ {proj}{act}— {body}"));
                self.session.last_timer_activity = Instant::now();
            }
            TimerOutcome::Stopped { elapsed_secs, total_secs, project, activity, body, .. } => {
                let elapsed = format_duration(elapsed_secs);
                let total = format_duration(total_secs);
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                self.flash(format!("■ {proj}{act}{body} — {elapsed} (total {total})"));
                self.session.last_timer_activity = Instant::now();
            }
            TimerOutcome::Switched { from_elapsed_secs, from_total_secs, from_project, from_activity, to_project, to_activity, to_body, .. } => {
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
            TimerOutcome::Stopped { elapsed_secs, project, activity, body, .. } => {
                let elapsed = format_duration(elapsed_secs);
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                self.flash(format!("interrupted {proj}{act}{body} ({elapsed}) — enter new task"));
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
    /// duration string (minutes, hours, or clock time). Flashes the result.
    pub fn add_time_to_current_from_input(&mut self, input: &str) {
        let secs = parse_duration_input(input);
        if secs == 0 {
            self.flash(format!("invalid duration: {input}"));
            return;
        }
        let Some(abs) = self.cur_task_index_in_tasks() else {
            self.flash("no task selected");
            return;
        };
        let current = self.store.tasks()[abs].dur.unwrap_or(0);
        let total = current + secs;
        // Replace/add the dur: token on the raw line using plain string ops
        // (the rest of the codebase avoids regex for line mutation).
        let raw = self.store.task_raw(abs).unwrap_or_default();
        let updated = if let Some(pos) = raw.find("dur:") {
            let val_start = pos + "dur:".len();
            let val_end = raw[val_start..]
                .find(|c: char| c.is_ascii_whitespace())
                .map_or(raw.len(), |n| val_start + n);
            format!("{}dur:{total}{}", &raw[..pos], &raw[val_end..])
        } else {
            format!("{raw} dur:{total}")
        };
        use crate::core::EditOutcome;
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
        use crate::core::EditOutcome;
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
                self.flash(format!("timer stopped ({} total)", format_duration(total_secs)));
            }
            TimerQuitOutcome::NoTimer => {}
            TimerQuitOutcome::Error(e) => {
                self.flash(format!("timer stop failed: {e}"));
            }
        }
    }

}
