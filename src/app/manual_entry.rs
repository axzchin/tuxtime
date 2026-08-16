//! Manual time entry: adding (or subtracting) time on a task without running
//! a timer. Covers the flexible duration grammar (`30`, `1.5h`, `14:30`,
//! `9am`, leading `-` for corrections) and the day-boundary prompt's
//! add-time path. The timer itself lives in [`super::timer_actions`].

use super::session::DayBoundaryAction;
use super::{App, Mode, Prompt, format_duration, parse_duration_input};
use crate::core::outcome::EditOutcome;
use crate::core::rebuild_token_line;

impl App {
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
            self.nav.push_mode(Mode::Prompt(Prompt::DayBoundary));
            return;
        }
        self.add_time_to_current_at(abs, input);
    }

    /// Add `input` (minutes, hours, or clock time) as `dur:` to the task at
    /// `abs`, stamping today's `log:`. A leading `-` (e.g. `-30`, `-1.5`)
    /// *removes* time instead — the correction path for a timer left running
    /// too long — clamped so a task never goes below zero. Shared by the
    /// direct add-time path and the day-boundary prompt's "continue same
    /// entry" choice.
    pub fn add_time_to_current_at(&mut self, abs: usize, input: &str) {
        let trimmed = input.trim();
        let (delta, removing) = match trimmed.strip_prefix('-') {
            Some(rest) => (parse_duration_input(rest), true),
            None => (parse_duration_input(trimmed), false),
        };
        if delta == 0 {
            self.flash(format!("invalid duration: {input}"));
            return;
        }
        let current = self.store.tasks()[abs].dur.unwrap_or(0);
        let total = if removing {
            current.saturating_sub(delta)
        } else {
            current + delta
        };
        // Replace/add the `dur:` token and stamp the `log:` date through the
        // store's shared token-rewrite helper, so all raw-line surgery stays
        // in one place. The log date makes manual additions show up on
        // today's timesheet even when the task was created earlier.
        let raw = self.store.task_raw(abs).unwrap_or_default();
        let updated = rebuild_token_line(&raw, "dur:", None, &format!("dur:{total}"));
        let updated = rebuild_token_line(&updated, "log:", None, &format!("log:{}", self.today()));
        match self.store.edit_line(abs, &updated) {
            EditOutcome::Saved { abs } => {
                let delta_str = format_duration(delta, self.prefs.rounding_increment);
                let total_str = format_duration(total, self.prefs.rounding_increment);
                let body = crate::todo::body_only(&updated);
                if removing {
                    self.flash(format!("removed {delta_str} — {body} (total {total_str})"));
                } else {
                    self.flash(format!("added {delta_str} — {body} (total {total_str})"));
                }
                // Adding time is a timer activity: reset the idle-nudge clock.
                self.note_timer_activity();
                // A real capture completes any nudge recovery flow (the
                // flag is only ever set on the nudge's own add-time path).
                self.session.from_nudge = false;
                self.after_mutation(abs);
            }
            EditOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            EditOutcome::Error(e) => self.flash(format!("edit failed: {e}")),
            EditOutcome::Empty | EditOutcome::OutOfRange | EditOutcome::TermNotFound => {}
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

    // ---- day-boundary prompt (add-time path) ----

    #[test]
    fn add_time_prompts_on_previous_day_and_new_entry_carries_time() {
        let mut app = build_app("Draft +Smith dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("30");
        assert_eq!(app.nav.mode(), Mode::Prompt(Prompt::DayBoundary));

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
        assert_eq!(app.nav.mode(), Mode::Prompt(Prompt::DayBoundary));

        crate::interactive::handle_day_boundary(&mut app, key('c'));

        assert_eq!(app.tasks().len(), 1);
        assert!(app.tasks()[0].raw.contains("dur:9000"));
        assert!(app.tasks()[0].raw.contains("log:2026-05-06"));
    }

    // ---- subtract time (negative durations in the add-time prompt) ----

    /// `-30` removes 30 minutes from the task's accumulated time instead of
    /// adding; the flash says "removed".
    #[test]
    fn add_time_negative_removes_minutes() {
        let mut app = build_app("Draft +Smith dur:7200\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("-30");

        assert_eq!(app.tasks()[0].dur, Some(5400), "2h − 30m = 1.5h");
        assert!(
            app.flash_active()
                .is_some_and(|m| m.contains("removed 30m") && m.contains("total 1h 30m")),
            "flash must confirm the removal, got: {:?}",
            app.flash_active()
        );
    }

    /// A negative decimal hour removes that many hours; the result clamps at
    /// zero when the subtraction would go below it.
    #[test]
    fn add_time_negative_clamps_at_zero() {
        let mut app = build_app("Draft +Smith dur:600\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("-1h");

        assert_eq!(
            app.tasks()[0].dur,
            Some(0),
            "10m − 1h must clamp at 0, never go negative"
        );
        assert!(
            app.flash_active().is_some_and(|m| m.contains("total 0m")),
            "flash must report the clamped total, got: {:?}",
            app.flash_active()
        );
    }

    /// `-1.5h` removes 1.5 hours using the same flexible duration grammar.
    #[test]
    fn add_time_negative_decimal_hours() {
        let mut app = build_app("Draft +Smith dur:7200\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("-1.5h");

        assert_eq!(app.tasks()[0].dur, Some(1800), "2h − 1.5h = 30m");
    }

    /// A negative duration on the day-boundary "new entry" path is refused:
    /// a fresh line starts at zero, so there is nothing to subtract.
    #[test]
    fn add_time_negative_refused_on_new_entry() {
        let mut app = build_app("Draft +Smith dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("-30");
        assert_eq!(app.nav.mode(), Mode::Prompt(Prompt::DayBoundary));
        crate::interactive::handle_day_boundary(&mut app, key('n'));

        assert_eq!(
            app.tasks().len(),
            1,
            "no carry on a refused negative new-entry add"
        );
        assert!(
            app.flash_active()
                .is_some_and(|m| m.contains("cannot subtract")),
            "flash must explain the refusal, got: {:?}",
            app.flash_active()
        );
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
}
