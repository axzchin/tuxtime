//! Timer lifecycle: toggle, interrupt, stop-on-quit, and the day-boundary
//! prompt (continue vs. new entry) that guards starting a timer — or adding
//! time — on a task whose accumulated time belongs to a previous day. The
//! headless timer mutations live in [`crate::core`]; this module maps their
//! outcomes to flash messages and view refreshes. Nudge detection and the
//! manual-time entry flows live in [`super::nudges`] and
//! [`super::manual_entry`] respectively.

use super::session::DayBoundaryAction;
use super::{App, Mode, Prompt, Screen, format_duration, parse_duration_input};
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

    /// True when the active timer is running on the task at `abs`.
    pub fn is_timer_running_on(&self, abs: usize) -> bool {
        self.store.is_timer_running_on(abs)
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
            self.nav.push_mode(Mode::Prompt(Prompt::DayBoundary));
            return;
        }
        self.toggle_timer_at(abs);
    }

    /// The `Enter` key's timer action. With the `enter_timer_toggle` pref
    /// on, this is a full toggle exactly like `t` (start *and* stop). By
    /// default it is start-only: pressing it again on the same task is a
    /// no-op rather than a stop, so the save-then-Enter flow out of the edit
    /// dialog can't accidentally stop the timer on a double-press. A timer
    /// running on a *different* task is still switched over (single-timer
    /// invariant), and the day-boundary prompt fires the same as `t`.
    pub fn start_timer(&mut self) {
        if self.prefs.enter_timer_toggle {
            return self.toggle_timer();
        }
        let Some(abs) = self.cur_task_index_in_tasks() else {
            self.flash("no task selected");
            return;
        };
        if self.store.is_timer_running_on(abs) {
            return;
        }
        if self.should_prompt_day_boundary(abs) {
            self.session.pending_day_boundary = Some((abs, DayBoundaryAction::StartTimer));
            self.nav.push_mode(Mode::Prompt(Prompt::DayBoundary));
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
    /// forward, then add `input` as `dur:` on the fresh line. Subtracting is
    /// meaningless on a brand-new line (it starts at zero), so negative
    /// input is rejected here — the correction flow targets existing time.
    pub fn day_boundary_new_entry_add_time(&mut self, abs: usize, input: &str) {
        if input.trim().starts_with('-') {
            self.flash("cannot subtract from a new entry");
            return;
        }
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
                        // A real capture completes any nudge recovery flow —
                        // the reminder's job is done.
                        self.session.from_nudge = false;
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
                // Open a blank Insert dialog for the interruption entry,
                // with the same `+` prefill as every other new-task dialog.
                self.session.manual_time_entry = true;
                self.session.auto_start_on_save = true;
                self.draft_clear();
                if self.prefs.prefill_plus_new {
                    self.draft_set_insert("+".to_string());
                }
                self.nav.mode = Mode::Screen(Screen::Insert);
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
    use crate::app::test_support::build_app;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }

    // ---- quick interrupt `+` prefill ----

    /// The quick-interrupt (`T`) blank dialog gets the same `+` prefill as
    /// every other new-task dialog.
    #[test]
    fn interrupt_timer_prefills_plus() {
        let mut app = build_app("Draft +Smith\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        app.toggle_timer();
        assert!(app.timer_running());

        app.interrupt_timer();

        assert_eq!(app.nav.mode, Mode::Screen(Screen::Insert));
        assert_eq!(app.draft.text(), "+");
        assert!(app.session.auto_start_on_save);
        assert!(app.session.manual_time_entry);
    }

    /// With the prefill toggled off, the interrupt dialog stays blank.
    #[test]
    fn interrupt_timer_blank_when_prefill_disabled() {
        let mut app = build_app("Draft +Smith\n");
        app.prefs.prefill_plus_new = false;
        app.nav.cursor = 0;
        app.recompute_visible();
        app.toggle_timer();
        assert!(app.timer_running());

        app.interrupt_timer();

        assert_eq!(app.nav.mode, Mode::Screen(Screen::Insert));
        assert_eq!(app.draft.text(), "");
    }

    // ---- day-boundary prompt (one line per task-day) ----

    #[test]
    fn toggle_timer_prompts_on_previous_day_task() {
        let mut app = build_app("Draft +Smith dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.toggle_timer();

        assert_eq!(app.nav.mode(), Mode::Prompt(Prompt::DayBoundary));
        assert!(!app.timer_running(), "no timer until the prompt resolves");
        // Esc cancels: back to Normal, task untouched.
        crate::interactive::handle_day_boundary(&mut app, esc());
        assert_eq!(app.nav.mode(), Mode::Screen(Screen::Normal));
        assert_eq!(app.tasks().len(), 1);
        assert!(!app.tasks()[0].done);
        assert!(app.session.pending_day_boundary.is_none());
    }

    // ---- Enter: start-only timer action ----

    #[test]
    fn start_timer_starts_when_idle() {
        let mut app = build_app("Draft +Smith\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.start_timer();

        assert!(app.timer_running());
        assert_eq!(app.store.active_timer_abs(), Some(0));
        assert_eq!(app.nav.mode(), Mode::Screen(Screen::Normal));
    }

    #[test]
    fn start_timer_noop_when_already_running_on_task() {
        let mut app = build_app("Draft +Smith\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        app.toggle_timer();
        assert!(app.timer_running());

        // A second Enter must not stop the timer (unlike `t`).
        app.start_timer();

        assert!(app.timer_running());
        assert_eq!(app.store.active_timer_abs(), Some(0));
    }

    #[test]
    fn start_timer_noop_skips_day_boundary_prompt_when_running() {
        // Previous-day time would normally prompt — but the timer is already
        // running on this task, so Enter is a no-op, not a prompt.
        let mut app = build_app("Old +W dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        // Resolve the initial day-boundary prompt via "continue" so the
        // timer actually starts (a plain toggle would leave the prompt up).
        app.toggle_timer();
        assert_eq!(app.nav.mode(), Mode::Prompt(Prompt::DayBoundary));
        crate::interactive::handle_day_boundary(&mut app, key('c'));
        assert!(app.timer_running());

        app.start_timer();

        assert!(app.timer_running());
        assert_eq!(app.nav.mode(), Mode::Screen(Screen::Normal));
        assert!(app.session.pending_day_boundary.is_none());
    }

    #[test]
    fn start_timer_switches_from_another_task() {
        let mut app = build_app("First +A\nSecond +B\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        app.toggle_timer();
        assert_eq!(app.store.active_timer_abs(), Some(0));

        app.nav.cursor = 1;
        app.recompute_visible();
        app.start_timer();

        // Single-timer invariant: the other task's timer is captured and the
        // timer starts on the selected task.
        assert!(app.timer_running());
        assert_eq!(app.store.active_timer_abs(), Some(1));
        assert!(app.tasks()[0].start.is_none());
    }

    /// With the `enter_timer_toggle` pref on, Enter behaves exactly like `t`:
    /// a second press stops the timer instead of being a no-op.
    #[test]
    fn start_timer_toggles_when_pref_enabled() {
        let mut app = build_app("Draft +Smith\n");
        app.prefs.enter_timer_toggle = true;
        app.nav.cursor = 0;
        app.recompute_visible();

        app.start_timer();
        assert!(app.timer_running());
        assert_eq!(app.store.active_timer_abs(), Some(0));

        // Second press stops, matching `t`.
        app.start_timer();
        assert!(!app.timer_running(), "pref on: Enter must stop like t");
        assert!(app.tasks()[0].dur.is_some(), "elapsed time must be captured");
    }

    #[test]
    fn start_timer_prompts_on_previous_day_task() {
        let mut app = build_app("Draft +Smith dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.start_timer();

        assert_eq!(app.nav.mode(), Mode::Prompt(Prompt::DayBoundary));
        assert!(!app.timer_running(), "no timer until the prompt resolves");
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
        assert_eq!(app.nav.mode(), Mode::Prompt(Prompt::DayBoundary));

        crate::interactive::handle_day_boundary(&mut app, key('c'));

        assert!(app.timer_running());
        assert_eq!(app.tasks().len(), 1, "no carry on continue");
        assert!(!app.tasks()[0].done);
        assert_eq!(app.nav.mode(), Mode::Screen(Screen::Normal));
    }

    #[test]
    fn day_boundary_new_entry_carries_and_starts_timer() {
        let mut app = build_app("(A) Draft +Smith @drafting dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        app.toggle_timer();
        assert_eq!(app.nav.mode(), Mode::Prompt(Prompt::DayBoundary));

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
        assert_eq!(app.nav.mode(), Mode::Screen(Screen::Normal));
    }

    /// Enter is the default resolution: a fresh entry for today (carry + start
    /// the timer), so starting a carried-over task is one key — no need to
    /// learn the continue-vs-new distinction first.
    #[test]
    fn day_boundary_enter_defaults_to_new_entry() {
        let mut app = build_app("(A) Draft +Smith @drafting dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        app.toggle_timer();
        assert_eq!(app.nav.mode(), Mode::Prompt(Prompt::DayBoundary));

        crate::interactive::handle_day_boundary(
            &mut app,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        );

        assert_eq!(app.tasks().len(), 2, "Enter must carry forward");
        assert!(app.tasks()[0].done, "old line consumed");
        assert!(app.tasks()[1].raw.contains("Draft"), "narrative carried");
        assert!(app.timer_running(), "timer started on the new line");
        assert_eq!(app.nav.mode(), Mode::Screen(Screen::Normal));
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
        assert_eq!(app.nav.mode(), Mode::Prompt(Prompt::DayBoundary));

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
}
