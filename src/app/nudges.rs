//! Nudge detection and recovery: the idle nudge, long-timer nudge, end-of-day
//! review nudge, and the stale-timer startup prompt. Owns the idle/long-timer
//! thresholds, the launch-time idle backdate, and the clock that gates every
//! nudge, plus the stale-timer discard/keep recovery. The task picker these
//! nudges can open (`S`/`m` from the idle nudge) lives in
//! [`super::nudge_picker`]; the timer mutations the nudges trigger live in
//! [`super::timer_actions`].

use super::{App, IdleReason, Mode, Nudge, Screen, parse_clock};
use crate::core::outcome::EditOutcome;
use std::time::Instant;

use chrono::Timelike;

/// True when a `start:` timestamp's calendar day is before `today` — the
/// timer was left running overnight. Extracted for direct testing: the
/// wall-clock elapsed comparison is hard to isolate from the date check in
/// an integration test.
fn timer_start_crossed_day(start: Option<&str>, today: &str) -> bool {
    start.is_some_and(|s| s.get(..10).is_some_and(|d| d < today))
}

impl App {
    /// Idle nudge threshold from prefs.
    pub fn idle_nudge_seconds(&self) -> u64 {
        self.prefs.idle_nudge_seconds
    }

    /// Set the idle nudge threshold in minutes. Persists to config.
    pub fn set_idle_nudge_minutes(&mut self, minutes: u64) {
        self.prefs.idle_nudge_seconds = minutes * 60;
        self.save_prefs();
    }

    /// Effective idle-nudge threshold, shrinking with each dismissal: every
    /// time the idle nudge is dismissed without any time being captured, the
    /// wait until the next reminder is halved (floored at one minute) so an
    /// untracked stretch can't be snoozed indefinitely with a flat threshold.
    /// Any real timer activity ([`note_timer_activity`](Self::note_timer_activity))
    /// resets the escalation back to the full configured threshold.
    pub fn effective_idle_nudge_seconds(&self) -> u64 {
        let mut threshold = self.prefs.idle_nudge_seconds;
        for _ in 0..self.session.idle_nudge_dismissals {
            threshold = (threshold / 2).max(60);
        }
        threshold
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
            Mode::Nudge(Nudge::Idle)
                | Mode::Nudge(Nudge::LongTimer)
                | Mode::Nudge(Nudge::StaleTimer)
                | Mode::Nudge(Nudge::Review)
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
        if self.nav.mode == Mode::Screen(Screen::Normal)
            && !self.timer_running()
            && idle_secs >= self.effective_idle_nudge_seconds()
        {
            self.session.pre_nudge_view = Some(self.nav.view);
            self.exit_overlay_to_normal();
            self.nav.mode = Mode::Nudge(Nudge::Idle);
            fired = true;
        }
        // End-of-day review: once per day, once the configured `review_time`
        // has passed, ask the user to reconcile the day. Only when something
        // has been tracked today (the "tracked nothing at all" failure is
        // already covered by the launch-time idle backdate + idle nudge) and
        // only from Normal mode, so it never clobbers the other nudges or
        // in-progress composition.
        if self.nav.mode == Mode::Screen(Screen::Normal)
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
                self.nav.mode = Mode::Nudge(Nudge::Review);
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
            if now_active && !was_active && self.nav.mode == Mode::Screen(Screen::Normal) {
                self.session.pre_nudge_view = Some(self.nav.view);
                self.nav.mode = Mode::Nudge(Nudge::LongTimer);
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
        // A real capture happened, so the reminder did its job — reset the
        // dismissal escalation back to the full configured threshold.
        self.session.idle_nudge_dismissals = 0;
    }

    /// Dismiss any overlay/dialog mode back to Normal, discarding transient
    /// state (draft, selection). Only used for the nudges, which need a
    /// clean slate for their popup.
    fn exit_overlay_to_normal(&mut self) {
        self.draft_clear();
        self.selection.exit_edit();
        self.selection.clear();
        self.session.manual_time_entry = false;
        self.nav.mode = Mode::Screen(Screen::Normal);
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

    /// Dismiss any nudge (or the stale-timer popup) back to Normal: restore
    /// the pre-nudge view and reset the idle-nudge clock so the popup can't
    /// re-fire on the next tick. The shared exit for the popup dismiss keys
    /// (`D`/`Esc` on the idle, long-timer and stale-timer prompts), `keep`
    /// on the stale-timer prompt, and the view-switch exit from the nudge
    /// task selection.
    ///
    /// The clock is reset directly rather than via
    /// [`note_timer_activity`](Self::note_timer_activity), so a pure
    /// dismissal never rewrites the idle reason — the user is still
    /// untracked, and the nudge will nag again once the clock lapses (only
    /// the instant re-fire is suppressed). Callers that captured time first
    /// (stopped a timer, discarded a gap) have already refreshed the clock;
    /// the duplicate write here is harmless.
    pub(crate) fn dismiss_nudge(&mut self) {
        self.nav.enter_normal();
        if let Some(v) = self.session.pre_nudge_view.take() {
            self.set_view(v);
        }
        self.session.last_timer_activity = std::time::Instant::now();
    }

    /// Dismiss the idle nudge without capturing time. Escalates the reminder
    /// by incrementing the dismissal count (which halves the next wait — see
    /// [`effective_idle_nudge_seconds`](Self::effective_idle_nudge_seconds))
    /// before the shared [`dismiss_nudge`](Self::dismiss_nudge) reset. A real
    /// capture resets the count, so only a string of pure dismissals (no time
    /// logged between them) keeps tightening the cadence.
    pub(crate) fn dismiss_idle_nudge(&mut self) {
        self.session.idle_nudge_dismissals = self.session.idle_nudge_dismissals.saturating_add(1);
        self.dismiss_nudge();
    }

    /// `[k]eep counting` on the stale-timer prompt: dismiss the popup and
    /// leave the timer running (the user is asserting the time is real).
    pub fn keep_stale_timer(&mut self) {
        // Timer is running, so the idle nudge can't fire; dismissing still
        // resets the activity clock so a later stop keeps the cadence sane.
        //
        // Keeping also acknowledges the timer is long-running: set the
        // status-bar ⏰ flag (when it actually is long) so the long-timer
        // popup doesn't instantly re-fire over the same timer the moment
        // this prompt closes — the flag semantics match pressing D on the
        // long-timer nudge itself. A stale timer that merely crossed a day
        // (still under the threshold) leaves the flag off, so the indicator
        // and popup stay accurate.
        let over_threshold = self
            .timer_elapsed_secs()
            .is_some_and(|e| e >= self.prefs.long_timer_nudge_seconds);
        self.session.long_timer_nudge_active = over_threshold;
        self.dismiss_nudge();
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
        let updated = crate::todo::map_body_tokens(&raw, |tok| {
            if is_timer_tag(tok) {
                None
            } else {
                Some(tok.to_string())
            }
        });
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
        app.nav.mode = Mode::Nudge(Nudge::StaleTimer);

        app.keep_stale_timer();

        assert!(app.timer_running(), "keep must leave the timer running");
        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
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
        assert_eq!(
            app.nav.mode,
            Mode::Nudge(Nudge::Idle),
            "first tick must nudge"
        );
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
        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
    }

    /// A running timer must also suppress the backdate (the timer is the
    /// activity — the user is mid-session, not idle).
    #[test]
    fn check_nudges_does_not_backdate_when_timer_running() {
        let mut app = build_app(&format!("Draft +Smith start:{}\n", stale_start(60)));
        assert!(app.timer_running());

        assert!(!app.check_nudges());

        assert_eq!(app.session.idle_reason, IdleReason::TimerStopped);
        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
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

    // ---- idle-nudge escalation (halving dismissals) ----

    /// The effective idle threshold halves with each dismissal, floored at
    /// one minute — a flat snooze would let an untracked stretch continue
    /// indefinitely, but escalation keeps tightening the cadence.
    #[test]
    fn effective_idle_threshold_halves_per_dismissal() {
        let mut app = build_app("Task\n");
        app.prefs.idle_nudge_seconds = 900;
        assert_eq!(app.effective_idle_nudge_seconds(), 900);
        app.session.idle_nudge_dismissals = 1;
        assert_eq!(app.effective_idle_nudge_seconds(), 450);
        app.session.idle_nudge_dismissals = 2;
        assert_eq!(app.effective_idle_nudge_seconds(), 225);
        // Floored at one minute, never zero.
        app.session.idle_nudge_dismissals = 10;
        assert_eq!(app.effective_idle_nudge_seconds(), 60);
    }

    /// Dismissing the idle nudge increments the escalation and resets the
    /// clock, so the next reminder fires after half the configured wait
    /// instead of the full threshold.
    #[test]
    fn dismiss_idle_nudge_escalates_next_fire() {
        let mut app = build_app("2026-05-06 Work +X @dev dur:600 log:2026-05-06\n");
        app.prefs.idle_nudge_seconds = 900;
        app.session.idle_backdated = true; // skip the launch backdate
        app.session.last_timer_activity =
            std::time::Instant::now() - std::time::Duration::from_secs(901);
        app.nav.mode = Mode::Nudge(Nudge::Idle);
        app.session.pre_nudge_view = Some(View::List);

        app.dismiss_idle_nudge();

        assert_eq!(app.session.idle_nudge_dismissals, 1);
        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
        assert_eq!(app.view(), View::List, "pre-nudge view restored");
        // Clock was reset, so no instant re-fire.
        assert!(!app.check_nudges());
        // But after only half the base threshold the escalated nudge fires.
        app.session.last_timer_activity =
            std::time::Instant::now() - std::time::Duration::from_secs(451);
        assert!(app.check_nudges());
        assert_eq!(app.nav.mode, Mode::Nudge(Nudge::Idle));
    }

    /// Any real capture (timer start/stop, manual add) resets the escalation
    /// back to the full configured threshold.
    #[test]
    fn note_timer_activity_resets_dismissal_escalation() {
        let mut app = build_app("Task\n");
        app.session.idle_nudge_dismissals = 3;
        assert!(app.effective_idle_nudge_seconds() < app.prefs.idle_nudge_seconds);

        app.note_timer_activity();

        assert_eq!(app.session.idle_nudge_dismissals, 0);
        assert_eq!(
            app.effective_idle_nudge_seconds(),
            app.prefs.idle_nudge_seconds
        );
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
        assert_eq!(app.nav.mode, Mode::Nudge(Nudge::Review));

        // Already shown today: must not re-fire.
        app.nav.mode = Mode::Screen(Screen::Normal);
        assert!(!app.check_nudges(), "review must not re-fire the same day");
        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
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
        assert_eq!(app.nav.mode, Mode::Nudge(Nudge::Idle));
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
        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
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

        assert_eq!(app.nav.mode, Mode::Nudge(Nudge::LongTimer));
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
        assert_eq!(app.nav.mode, Mode::Nudge(Nudge::LongTimer));

        crate::interactive::handle_long_timer_nudge(&mut app, key('S'));

        assert!(!app.timer_running(), "S stops the timer");
        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
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
        assert_eq!(app.nav.mode, Mode::Nudge(Nudge::LongTimer));

        crate::interactive::handle_long_timer_nudge(&mut app, key('D'));

        assert!(app.timer_running(), "D dismisses without stopping");
        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
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

        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
        assert!(!app.session.long_timer_nudge_active);
        assert!(app.session.pre_nudge_view.is_none());
        assert!(app.timer_running());
    }
}
