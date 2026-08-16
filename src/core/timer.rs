//! Durable timer state: start/stop, midnight-split sessions, and the
//! one-line-per-task-day carry-forward flow. Shares the timer's raw-line
//! rewriting helpers (`rebuild_token_line`, `carry_forward_body`).

use super::Store;
use super::outcome::{CarryForwardOutcome, Reconcile, StoreError, TimerOutcome, TimerQuitOutcome};
use crate::todo;

impl Store {
    // ---- timer mutations ----

    /// Toggle the timer on the task at `abs`. If a timer is already running on
    /// another task, stop that one first. If the timer is running on this task,
    /// stop it instead.
    pub fn timer_toggle(&mut self, abs: usize) -> TimerOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return TimerOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return TimerOutcome::OutOfRange;
        }
        if self
            .active_timer
            .as_ref()
            .is_some_and(|ts| ts.task_abs == abs)
        {
            return self.timer_stop();
        }
        if self.active_timer.is_some() {
            let from_abs = self.active_timer.as_ref().map_or(0, |ts| ts.task_abs);
            let old = self.timer_stop_inner();
            self.push_history();
            let (elapsed_secs, total_secs, project, activity, shift) = match &old {
                TimerOutcome::Stopped {
                    elapsed_secs,
                    total_secs,
                    project,
                    activity,
                    ..
                } => (
                    *elapsed_secs,
                    *total_secs,
                    project.clone(),
                    activity.clone(),
                    0,
                ),
                // A split stop inserted new lines after the stopped task,
                // shifting any target that sat above it.
                TimerOutcome::StoppedSplit {
                    elapsed_secs,
                    total_secs,
                    project,
                    activity,
                    chunks,
                    ..
                } => (
                    *elapsed_secs,
                    *total_secs,
                    project.clone(),
                    activity.clone(),
                    chunks.len().saturating_sub(1),
                ),
                _ => return old,
            };
            let abs = if from_abs < abs { abs + shift } else { abs };
            if abs >= self.tasks.len() {
                return TimerOutcome::OutOfRange;
            }
            let start_outcome = self.timer_start_inner(abs);
            let to_project = self.tasks[abs].projects.first().cloned();
            let to_activity = self.tasks[abs].contexts.first().cloned();
            if let Err(e) = self.persist() {
                return TimerOutcome::Error(e);
            }
            if let TimerOutcome::Started { body: to_body, .. } = start_outcome {
                return TimerOutcome::Switched {
                    from_abs,
                    from_elapsed_secs: elapsed_secs,
                    from_total_secs: total_secs,
                    from_project: project,
                    from_activity: activity,
                    to_abs: abs,
                    to_project,
                    to_activity,
                    to_body,
                };
            }
            return start_outcome;
        }
        self.push_history();
        let outcome = self.timer_start_inner(abs);
        if let Err(e) = self.persist() {
            return TimerOutcome::Error(e);
        }
        outcome
    }

    pub fn stop_timer_on_quit(&mut self) -> TimerQuitOutcome {
        if self.active_timer.is_none() {
            return TimerQuitOutcome::NoTimer;
        }
        let outcome = self.timer_stop_inner();
        match outcome {
            // A midnight-crossing session also rewrites the list (one line
            // per day) — that rewrite must be persisted on quit too, or the
            // elapsed time is silently dropped. The quit outcome collapses
            // the split to the same abs/elapsed/total a plain stop carries;
            // the per-day chunks are only for the caller's flash message.
            TimerOutcome::Stopped {
                abs,
                elapsed_secs,
                total_secs,
                ..
            }
            | TimerOutcome::StoppedSplit {
                abs,
                elapsed_secs,
                total_secs,
                ..
            } => {
                self.push_history();
                if let Err(e) = self.persist() {
                    return TimerQuitOutcome::Error(e);
                }
                TimerQuitOutcome::Stopped {
                    abs,
                    elapsed_secs,
                    total_secs,
                }
            }
            TimerOutcome::Error(e) => TimerQuitOutcome::Error(e),
            _ => TimerQuitOutcome::NoTimer,
        }
    }

    fn timer_start_inner(&mut self, abs: usize) -> TimerOutcome {
        let now = crate::now::now_iso();
        let task = &self.tasks[abs];
        let new_raw = rebuild_token_line(&task.raw, "start:", None, &format!("start:{now}"));
        match todo::parse_line(&new_raw) {
            Ok(parsed) => {
                self.tasks[abs] = parsed;
                // Derive the timer from the tag just written, keeping `start:`
                // the single source of truth for the running timer.
                self.resync_timer();
                let t = &self.tasks[abs];
                TimerOutcome::Started {
                    abs,
                    project: t.projects.first().cloned(),
                    activity: t.contexts.first().cloned(),
                    body: todo::body_only_from_clean(&t.clean_raw),
                }
            }
            Err(e) => TimerOutcome::Error(StoreError::Parse(e)),
        }
    }

    fn timer_stop_inner(&mut self) -> TimerOutcome {
        let Some(ts) = self.active_timer.take() else {
            return TimerOutcome::OutOfRange;
        };
        // Elapsed comes from the durable `start:` timestamp, so a restart or
        // system suspend doesn't undercount billable time.
        let now = chrono::Local::now().naive_local();
        let elapsed = (now - ts.started_at).num_seconds().max(0) as u64;
        let abs = ts.task_abs;
        if abs >= self.tasks.len() {
            return TimerOutcome::OutOfRange;
        }
        // A session that crossed midnight is split per calendar day so each
        // day's sheet shows only that day's time (one line per task-day).
        let chunks = session_chunks(ts.started_at, now);
        if chunks.len() > 1 {
            return self.stop_timer_split(abs, elapsed, chunks);
        }
        let task = &self.tasks[abs];
        let existing_dur = task.dur.unwrap_or(0);
        let new_total = existing_dur + elapsed;
        let new_raw = rebuild_token_line(
            &task.raw,
            "dur:",
            Some("start:"),
            &format!("dur:{new_total}"),
        );
        // Stamp the day the work was logged so the timesheet attributes the
        // accumulated time to that day, not the task's creation date. Same-day
        // sessions use the store's cached `today` (refreshed by the TUI event
        // loop each iteration, injectable in tests/CLI) so the store stays
        // deterministic; only the multi-day split path uses actual session
        // dates, because there the calendar-day boundaries are the point.
        let new_raw = rebuild_token_line(&new_raw, "log:", None, &format!("log:{}", self.today));
        match todo::parse_line(&new_raw) {
            Ok(parsed) => {
                let project = parsed.projects.first().cloned();
                let activity = parsed.contexts.first().cloned();
                let body = todo::body_only_from_clean(&parsed.clean_raw);
                self.tasks[abs] = parsed;
                TimerOutcome::Stopped {
                    abs,
                    elapsed_secs: elapsed,
                    total_secs: new_total,
                    project,
                    activity,
                    body,
                }
            }
            Err(e) => TimerOutcome::Error(StoreError::Parse(e)),
        }
    }

    /// Timer stop for a session that crossed midnight: attribute each day's
    /// chunk to its own line (one line per task-day). The original line keeps
    /// the start-day chunk and is consumed (raw completion preserving its
    /// `dur:`/`log:`/`bill:`); a fresh open line is created for every later
    /// day in the session, carrying body/priority/projects/contexts/`bill:`
    /// (not `due:`/`rec:`/`notes:`), with its own `dur:` and `log:`.
    fn stop_timer_split(
        &mut self,
        abs: usize,
        elapsed: u64,
        chunks: Vec<(String, u64)>,
    ) -> TimerOutcome {
        let (start_day, start_secs) = &chunks[0];
        let (carry, priority) = {
            let task = &self.tasks[abs];
            (carry_forward_body(&task.raw), task.priority)
        };
        let total_secs = self.tasks[abs].dur.unwrap_or(0) + start_secs;
        let new_raw = rebuild_token_line(
            &self.tasks[abs].raw,
            "dur:",
            Some("start:"),
            &format!("dur:{total_secs}"),
        );
        let new_raw = rebuild_token_line(&new_raw, "log:", None, &format!("log:{start_day}"));
        let parsed = match todo::parse_line(&new_raw) {
            Ok(p) => p,
            Err(e) => return TimerOutcome::Error(StoreError::Parse(e)),
        };
        let project = parsed.projects.first().cloned();
        let activity = parsed.contexts.first().cloned();
        let body = todo::body_only_from_clean(&parsed.clean_raw);
        self.tasks[abs] = parsed;
        if let Err(e) = self.complete_consumed_line(abs, start_day) {
            return TimerOutcome::Error(StoreError::Parse(e));
        }
        // One new open line per subsequent day, oldest first, inserted right
        // after the consumed line (ascending targets, so earlier inserts
        // can't shift later ones).
        for (i, (date, secs)) in chunks.iter().enumerate().skip(1) {
            let prefix = match priority {
                Some(p) => format!("({p}) {date} "),
                None => format!("{date} "),
            };
            let new_raw = format!("{prefix}{carry} dur:{secs} log:{date}");
            if let Ok(parsed) = todo::parse_line(&new_raw) {
                self.tasks.insert(abs + i, parsed);
            }
        }
        TimerOutcome::StoppedSplit {
            abs,
            elapsed_secs: elapsed,
            total_secs,
            chunks,
            project,
            activity,
            body,
        }
    }

    fn timer_stop(&mut self) -> TimerOutcome {
        let outcome = self.timer_stop_inner();
        // A midnight-crossing stop splits into per-day lines; that rewrite
        // must reach the file too, or the next reconcile/quit reverts it and
        // the elapsed time silently vanishes.
        if matches!(
            outcome,
            TimerOutcome::Stopped { .. } | TimerOutcome::StoppedSplit { .. }
        ) {
            self.push_history();
            if let Err(e) = self.persist() {
                return TimerOutcome::Error(e);
            }
        }
        outcome
    }

    // ---- one line per task-day: carry forward / midnight split ----

    /// Carry a task forward to today's entry, building the new line from the
    /// source itself: body + priority + projects + contexts + `bill:` (no
    /// time tags, `due:`/`rec:`/`notes:` dropped). Completes the source line
    /// (raw write, no recurrence spawn) and inserts the new line at `abs + 1`.
    /// One undo entry for both.
    pub fn carry_forward(&mut self, abs: usize) -> CarryForwardOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return CarryForwardOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return CarryForwardOutcome::OutOfRange;
        }
        let body = carry_forward_body(&self.tasks[abs].raw);
        let line = match self.tasks[abs].priority {
            Some(p) => format!("({p}) {body}"),
            None => body,
        };
        self.carry_forward_to_inner(abs, &line)
    }

    /// Like [`Store::carry_forward`] but the new line is the caller-supplied
    /// `text` (canonical todo.txt body; the creation date is added here). Used
    /// by the upgraded `N` flow, where the user can polish the carried-over
    /// narrative before saving.
    pub fn carry_forward_to(&mut self, abs: usize, text: &str) -> CarryForwardOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return CarryForwardOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return CarryForwardOutcome::OutOfRange;
        }
        self.carry_forward_to_inner(abs, text)
    }

    /// Carry a task forward and immediately start the timer on the new line
    /// (day-boundary prompt "new entry" path). Any timer running on another
    /// task is stopped first, capturing its time (possibly splitting it
    /// across midnight); the target index is adjusted for the shift a split
    /// causes so the correct task is carried.
    pub fn carry_forward_and_start(&mut self, abs: usize) -> CarryForwardOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return CarryForwardOutcome::Aborted(other),
        }
        // Stop any timer running on another task first (single-timer invariant).
        let other = self.active_timer.as_ref().map(|ts| ts.task_abs);
        let mut shift = 0usize;
        if let Some(o) = other
            && o != abs
        {
            if let TimerOutcome::StoppedSplit { chunks, .. } = self.timer_stop_inner() {
                shift = chunks.len().saturating_sub(1);
            }
            self.push_history();
        }
        let abs = if other.is_some_and(|o| o < abs) {
            abs + shift
        } else {
            abs
        };
        if abs >= self.tasks.len() {
            return CarryForwardOutcome::OutOfRange;
        }
        let body = carry_forward_body(&self.tasks[abs].raw);
        let line = match self.tasks[abs].priority {
            Some(p) => format!("({p}) {body}"),
            None => body,
        };
        match self.carry_forward_to_inner(abs, &line) {
            CarryForwardOutcome::Carried { old, new } => {
                let started = self.timer_start_inner(new);
                match started {
                    TimerOutcome::Started {
                        project,
                        activity,
                        body,
                        ..
                    } => {
                        if let Err(e) = self.persist() {
                            return CarryForwardOutcome::Error(e);
                        }
                        CarryForwardOutcome::CarriedStarted {
                            old,
                            new,
                            project,
                            activity,
                            body,
                        }
                    }
                    TimerOutcome::Error(e) => CarryForwardOutcome::Error(e),
                    TimerOutcome::Aborted(r) => CarryForwardOutcome::Aborted(r),
                    // `new` was just inserted, so start can't be out of range;
                    // surface any such surprise honestly rather than panicking.
                    TimerOutcome::OutOfRange => CarryForwardOutcome::OutOfRange,
                    // timer_start_inner only ever returns Started/Error; keep
                    // the remaining arms so the match stays exhaustive.
                    TimerOutcome::Stopped { .. }
                    | TimerOutcome::StoppedSplit { .. }
                    | TimerOutcome::Switched { .. } => {
                        CarryForwardOutcome::Error(StoreError::Parse(todo::ParseError::Empty))
                    }
                }
            }
            other => other,
        }
    }

    /// Shared carry-forward body: finalize the line, capture any running
    /// timer on the source, consume the old line, insert the new one, persist.
    /// No reconcile here — callers that chain mutations (e.g. after stopping
    /// another timer) would otherwise see the file lag the in-memory state.
    fn carry_forward_to_inner(&mut self, abs: usize, text: &str) -> CarryForwardOutcome {
        // If the source is being timed, capture the elapsed time first so the
        // session isn't silently dropped when the line is consumed. (If that
        // capture itself splits across midnight, the split's lines are
        // inserted above the new carry line below — awkward but harmless, and
        // unreachable via the App flows, which guard running timers before
        // carrying.)
        if self
            .active_timer
            .as_ref()
            .is_some_and(|ts| ts.task_abs == abs)
        {
            let _ = self.timer_stop_inner();
        }
        let line = finalize_carry_line(text, &self.today);
        if todo::body_after_priority(&line).trim().is_empty() {
            return CarryForwardOutcome::Error(StoreError::Parse(todo::ParseError::Empty));
        }
        let new_line = match todo::parse_line(&line) {
            Ok(t) => t,
            Err(e) => return CarryForwardOutcome::Error(StoreError::Parse(e)),
        };
        let was_done = self.tasks[abs].done;
        // The consumed line's done date is the day its time was logged, not
        // today — the completed entry describes when the work happened.
        let done_date = self.tasks[abs]
            .log
            .clone()
            .unwrap_or_else(|| self.today.clone());
        self.push_history();
        if !was_done && let Err(e) = self.complete_consumed_line(abs, &done_date) {
            return CarryForwardOutcome::Error(StoreError::Parse(e));
        }
        self.tasks.insert(abs + 1, new_line);
        self.resync_timer();
        match self.persist() {
            Ok(()) => CarryForwardOutcome::Carried {
                old: abs,
                new: abs + 1,
            },
            Err(e) => CarryForwardOutcome::Error(e),
        }
    }

    /// Consume a task line (raw completion) as of `done_date`, preserving its
    /// `dur:`/`log:`/`bill:` and other tags but stripping `start:` and any
    /// priority (todo.txt convention — matches `mark_done`). Never spawns a
    /// recurrence successor; used for the one-line-per-task-day flow.
    fn complete_consumed_line(
        &mut self,
        abs: usize,
        done_date: &str,
    ) -> Result<(), todo::ParseError> {
        let t = &self.tasks[abs];
        let created = t
            .created_date
            .clone()
            .unwrap_or_else(|| done_date.to_string());
        let body = todo::body_after_priority(&t.raw)
            .split_whitespace()
            .filter(|tok| !tok.starts_with("start:"))
            .collect::<Vec<_>>()
            .join(" ");
        let new_raw = format!("x {done_date} {created} {body}");
        let parsed = todo::parse_line(&new_raw)?;
        self.tasks[abs] = parsed;
        Ok(())
    }
}

fn session_chunks(start: chrono::NaiveDateTime, end: chrono::NaiveDateTime) -> Vec<(String, u64)> {
    let mut chunks = Vec::new();
    let mut cursor = start;
    while cursor < end {
        if let Some(day_end) = cursor
            .date()
            .succ_opt()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
        {
            let chunk_end = day_end.min(end);
            let secs = (chunk_end - cursor).num_seconds().max(0) as u64;
            if secs > 0 {
                chunks.push((cursor.date().format("%Y-%m-%d").to_string(), secs));
            }
            cursor = chunk_end;
        } else {
            // Date overflow (9999-12-31): attribute the remainder to the
            // final day rather than silently dropping it.
            chunks.push((
                cursor.date().format("%Y-%m-%d").to_string(),
                (end - cursor).num_seconds().max(0) as u64,
            ));
            break;
        }
    }
    chunks
}

pub(crate) fn carry_forward_body(raw: &str) -> String {
    let cleaned = todo::body_after_quoted_kv(raw);
    todo::body_after_priority(&cleaned)
        .split_whitespace()
        .filter(|tok| {
            !tok.starts_with("due:")
                && !tok.starts_with("rec:")
                && !tok.starts_with("start:")
                && !tok.starts_with("dur:")
                && !tok.starts_with("log:")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn finalize_carry_line(text: &str, today: &str) -> String {
    let text = text.trim();
    let (priority, body) = if todo::starts_with_priority(text) {
        (
            Some(text.as_bytes()[1] as char),
            todo::strip_priority(text).trim(),
        )
    } else {
        (None, text)
    };
    match priority {
        Some(p) => format!("({p}) {today} {body}"),
        None => format!("{today} {body}"),
    }
}

pub(crate) fn rebuild_token_line(
    raw: &str,
    replace_prefix: &str,
    drop_prefix: Option<&str>,
    new_token: &str,
) -> String {
    let mut has_token = false;
    let rebuilt = crate::todo::map_body_tokens(raw, |tok| {
        if let Some(dp) = drop_prefix
            && tok.starts_with(dp)
        {
            return None;
        }
        if tok.starts_with(replace_prefix) {
            has_token = true;
            return Some(new_token.to_string());
        }
        Some(tok.to_string())
    });
    if has_token {
        rebuilt
    } else if rebuilt.is_empty() {
        new_token.to_string()
    } else {
        format!("{rebuilt} {new_token}")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::core::test_support::build_store;

    // ---- timer log-date stamping ----

    #[test]
    fn timer_stop_stamps_log_date_and_removes_start() {
        // build_store fixes today at 2026-05-06.
        let mut store = build_store("Draft motion +Smith @drafting\n");
        assert!(matches!(
            store.timer_toggle(0),
            TimerOutcome::Started { .. }
        ));
        assert!(store.tasks()[0].start.is_some());
        assert!(matches!(
            store.timer_toggle(0),
            TimerOutcome::Stopped { .. }
        ));
        let raw = &store.tasks()[0].raw;
        assert!(raw.contains("dur:"), "stop must add dur:, got: {raw}");
        assert!(
            raw.contains("log:2026-05-06"),
            "stop must stamp the day the time was logged, got: {raw}"
        );
        assert!(
            !raw.contains("start:"),
            "stop must remove start:, got: {raw}"
        );
    }

    #[test]
    fn timer_stop_replaces_existing_log_date_without_duplicating() {
        // A task tracked earlier keeps its dur but the log date must move to
        // the day of the latest stop — one token, never two.
        let mut store = build_store("Draft motion +Smith @drafting dur:3600 log:2026-05-01\n");
        assert!(matches!(
            store.timer_toggle(0),
            TimerOutcome::Started { .. }
        ));
        assert!(matches!(
            store.timer_toggle(0),
            TimerOutcome::Stopped { .. }
        ));
        let raw = &store.tasks()[0].raw;
        assert_eq!(
            raw.matches("log:").count(),
            1,
            "must not duplicate log:, got: {raw}"
        );
        assert!(
            raw.contains("log:2026-05-06"),
            "log must move to the latest stop date, got: {raw}"
        );
        let dur_val = raw
            .split_whitespace()
            .find_map(|t| t.strip_prefix("dur:"))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        assert!(dur_val >= 3600, "dur must accumulate, got: {raw}");
    }

    // ---- one line per task-day: carry forward + midnight split ----

    #[test]
    fn carry_forward_completes_old_line_and_creates_fresh_entry() {
        let mut store = build_store("Draft motion +Smith @drafting dur:7200 log:2026-05-05\n");
        assert!(matches!(
            store.carry_forward(0),
            CarryForwardOutcome::Carried { old: 0, new: 1 }
        ));
        assert_eq!(store.tasks().len(), 2);
        // Old line consumed: done, keeps dur/log, done date = log date.
        let old = &store.tasks()[0];
        assert!(old.done);
        assert_eq!(old.done_date.as_deref(), Some("2026-05-05"));
        assert_eq!(old.dur, Some(7200));
        assert_eq!(old.log.as_deref(), Some("2026-05-05"));
        // New line: fresh date, same body + tags, no time.
        let new = &store.tasks()[1];
        assert!(!new.done);
        assert_eq!(new.created_date.as_deref(), Some("2026-05-06"));
        assert!(new.raw.contains("Draft motion"));
        assert!(new.raw.contains("+Smith"));
        assert!(new.raw.contains("@drafting"));
        assert_eq!(new.dur, None);
        assert_eq!(new.log, None);
        assert_eq!(new.start, None);
    }

    #[test]
    fn carry_forward_preserves_priority_and_billable() {
        let mut store =
            build_store("(A) 2026-05-01 Firm admin +Admin @admin dur:900 log:2026-05-05 bill:n\n");
        assert!(matches!(
            store.carry_forward(0),
            CarryForwardOutcome::Carried { new: 1, .. }
        ));
        let new = &store.tasks()[1].raw;
        assert!(
            new.starts_with("(A) 2026-05-06"),
            "priority + date prefix, got: {new}"
        );
        assert!(new.contains("Firm admin"));
        assert!(new.contains("+Admin"));
        assert!(new.contains("@admin"));
        assert!(new.contains("bill:n"));
    }

    #[test]
    fn carry_forward_drops_due_rec_and_notes() {
        let raw = "2026-05-01 Draft +Smith due:2026-06-01 rec:+1w note:\"long text\"\n";
        let mut store = build_store(raw);
        assert!(matches!(
            store.carry_forward(0),
            CarryForwardOutcome::Carried { .. }
        ));
        let new_raw = &store.tasks()[1].raw;
        assert!(!new_raw.contains("due:"), "due must not carry: {new_raw}");
        assert!(!new_raw.contains("rec:"), "rec must not carry: {new_raw}");
        assert!(
            !new_raw.contains("note:"),
            "notes must not carry: {new_raw}"
        );
        assert!(
            !new_raw.contains("start:"),
            "start must not carry: {new_raw}"
        );
        assert!(!new_raw.contains("dur:"), "dur must not carry: {new_raw}");
        assert!(!new_raw.contains("log:"), "log must not carry: {new_raw}");
        // Consuming the rec-bearing line must not spawn a successor.
        assert_eq!(store.tasks().len(), 2);
    }

    #[test]
    fn carry_forward_to_uses_custom_narrative() {
        let mut store = build_store("Draft motion +Smith dur:7200 log:2026-05-05\n");
        assert!(matches!(
            store.carry_forward_to(0, "Revised motion +Smith"),
            CarryForwardOutcome::Carried { new: 1, .. }
        ));
        assert_eq!(store.tasks()[1].raw, "2026-05-06 Revised motion +Smith");
        assert!(store.tasks()[0].done);
    }

    #[test]
    fn undo_restores_carry_forward() {
        let mut store = build_store("Draft +Smith dur:7200 log:2026-05-05\n");
        store.carry_forward(0);
        assert_eq!(store.tasks().len(), 2);
        store.undo();
        assert_eq!(store.tasks().len(), 1);
        assert!(!store.tasks()[0].done);
        assert_eq!(store.tasks()[0].dur, Some(7200));
    }

    #[test]
    fn carry_forward_captures_running_timer_before_consuming() {
        // A recent real start so the captured session stays on one day (an
        // ancient tag would split across many days — correct, but not what
        // this test is about).
        let start = (chrono::Local::now() - chrono::Duration::seconds(65))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let mut store = build_store(&format!("Draft +Smith start:{start}\n"));
        assert!(store.timer_running());
        assert!(matches!(
            store.carry_forward(0),
            CarryForwardOutcome::Carried { .. }
        ));
        // The source line was consumed and the timer cleared (start: gone).
        assert!(store.tasks()[0].done);
        assert!(!store.timer_running());
        assert_eq!(store.tasks().len(), 2);
    }

    #[test]
    fn carry_forward_and_start_switches_timer_and_carries() {
        // Timer running on task 0 (recent real start so the captured session
        // stays on one day); carry task 1 forward and start the new line.
        let start = (chrono::Local::now() - chrono::Duration::seconds(65))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let mut store = build_store(&format!(
            "First start:{start}\nSecond +Work dur:7200 log:2026-05-05\n"
        ));
        assert_eq!(store.active_timer_abs(), Some(0));
        match store.carry_forward_and_start(1) {
            CarryForwardOutcome::CarriedStarted { old: 1, new, .. } => {
                assert_eq!(new, 2);
                assert_eq!(store.active_timer_abs(), Some(2));
                assert!(store.tasks()[2].start.is_some());
                assert!(store.tasks()[2].raw.contains("Second"));
                assert!(store.tasks()[2].raw.contains("+Work"));
            }
            other => panic!("expected CarriedStarted, got {other:?}"),
        }
        assert_eq!(store.tasks().len(), 3);
        assert!(store.tasks()[1].done);
    }

    #[test]
    fn session_chunks_same_day_single_chunk() {
        let start =
            chrono::NaiveDateTime::parse_from_str("2026-05-05T09:00:00", "%Y-%m-%dT%H:%M:%S")
                .unwrap();
        let end = chrono::NaiveDateTime::parse_from_str("2026-05-05T10:30:00", "%Y-%m-%dT%H:%M:%S")
            .unwrap();
        assert_eq!(
            session_chunks(start, end),
            vec![("2026-05-05".to_string(), 5400)]
        );
    }

    #[test]
    fn session_chunks_split_across_midnight() {
        let start =
            chrono::NaiveDateTime::parse_from_str("2026-05-05T23:30:00", "%Y-%m-%dT%H:%M:%S")
                .unwrap();
        let end = chrono::NaiveDateTime::parse_from_str("2026-05-06T00:30:00", "%Y-%m-%dT%H:%M:%S")
            .unwrap();
        assert_eq!(
            session_chunks(start, end),
            vec![
                ("2026-05-05".to_string(), 1800),
                ("2026-05-06".to_string(), 1800)
            ]
        );
    }

    #[test]
    fn session_chunks_empty_when_start_after_end() {
        let start =
            chrono::NaiveDateTime::parse_from_str("2026-05-06T10:00:00", "%Y-%m-%dT%H:%M:%S")
                .unwrap();
        let end = chrono::NaiveDateTime::parse_from_str("2026-05-06T09:00:00", "%Y-%m-%dT%H:%M:%S")
            .unwrap();
        assert!(session_chunks(start, end).is_empty());
    }

    #[test]
    fn stop_timer_split_creates_one_line_per_day_and_consumes_original() {
        let mut store = build_store(
            "(A) 2026-05-05 Draft motion +Smith @drafting dur:0 start:2026-05-05T23:30:00\n",
        );
        let chunks = vec![
            ("2026-05-05".to_string(), 1800),
            ("2026-05-06".to_string(), 1800),
        ];
        let out = store.stop_timer_split(0, 3600, chunks);
        assert!(matches!(
            out,
            TimerOutcome::StoppedSplit {
                elapsed_secs: 3600,
                total_secs: 1800,
                ..
            }
        ));
        assert_eq!(store.tasks().len(), 2);
        let old = &store.tasks()[0];
        assert!(old.done);
        assert_eq!(old.done_date.as_deref(), Some("2026-05-05"));
        assert_eq!(old.dur, Some(1800));
        assert_eq!(old.log.as_deref(), Some("2026-05-05"));
        let new = &store.tasks()[1];
        assert!(!new.done);
        assert!(
            new.raw.starts_with("(A) 2026-05-06"),
            "priority carried: {}",
            new.raw
        );
        assert!(new.raw.contains("Draft motion"));
        assert!(new.raw.contains("+Smith"));
        assert!(new.raw.contains("@drafting"));
        assert!(new.raw.contains("dur:1800"));
        assert!(new.raw.contains("log:2026-05-06"));
    }

    // ---- split sessions persist on stop and on quit ----

    /// A midnight-crossing stop splits into per-day lines; the split must be
    /// written to disk, not left in memory only — otherwise the next
    /// reconcile reverts it and the elapsed time silently vanishes.
    #[test]
    fn timer_stop_persists_split() {
        // A start two days back always crosses at least one midnight,
        // whatever the real date the test happens to run on.
        let start = (chrono::Local::now() - chrono::Duration::days(2))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let mut store = build_store(&format!("Draft motion +Smith start:{start}\n"));
        assert!(matches!(
            store.timer_toggle(0),
            TimerOutcome::StoppedSplit { .. }
        ));
        assert!(
            matches!(store.reconcile(), Reconcile::Unchanged),
            "the split must reach the file: disk must equal memory"
        );
        assert!(store.tasks().len() >= 2, "one open line per session day");
        assert!(store.tasks()[0].done, "start-day line consumed");
        assert!(
            store.tasks().iter().all(|t| t.start.is_none()),
            "start: must be stripped from every split line"
        );
    }

    /// Quitting with a midnight-crossing timer must capture and persist the
    /// split the same way a plain stop does — not silently drop the session.
    #[test]
    fn stop_timer_on_quit_persists_split() {
        let start = (chrono::Local::now() - chrono::Duration::days(2))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let mut store = build_store(&format!("Draft motion +Smith start:{start}\n"));
        let out = store.stop_timer_on_quit();
        assert!(
            matches!(out, TimerQuitOutcome::Stopped { .. }),
            "quit must not drop a midnight-crossing session, got {out:?}"
        );
        assert!(
            matches!(store.reconcile(), Reconcile::Unchanged),
            "the split must reach the file on quit"
        );
        assert!(store.tasks().len() >= 2);
        assert!(store.tasks()[0].done);
        assert!(
            store.tasks().iter().all(|t| t.start.is_none()),
            "start: must be stripped from every split line"
        );
    }

    #[test]
    fn stop_timer_split_three_days_creates_three_lines() {
        let mut store = build_store("Research +Matter @research dur:0 start:2026-05-04T23:00:00\n");
        let chunks = vec![
            ("2026-05-04".to_string(), 3600),
            ("2026-05-05".to_string(), 86400),
            ("2026-05-06".to_string(), 1800),
        ];
        let _ = store.stop_timer_split(0, 91800, chunks);
        assert_eq!(store.tasks().len(), 3);
        assert!(store.tasks()[0].done);
        assert_eq!(store.tasks()[0].dur, Some(3600));
        assert_eq!(store.tasks()[1].dur, Some(86400));
        assert_eq!(store.tasks()[1].log.as_deref(), Some("2026-05-05"));
        assert_eq!(store.tasks()[2].dur, Some(1800));
        assert_eq!(store.tasks()[2].log.as_deref(), Some("2026-05-06"));
    }

    // ---- timer re-attachment (start: is the source of truth) ----

    #[test]
    fn timer_attaches_to_shifted_index_after_delete() {
        let mut store = build_store("task one\nDraft start:2026-05-06T10:00:00\n");
        assert_eq!(store.active_timer_abs(), Some(1));
        store.delete(0);
        // The timer task moved from index 1 to 0; the timer must follow it,
        // not keep pointing at index 1 (which is now a different task).
        assert!(store.timer_running(), "timer must survive the delete");
        assert_eq!(store.active_timer_abs(), Some(0));
        assert!(store.tasks()[0].start.is_some());
    }

    #[test]
    fn delete_of_timer_task_clears_timer() {
        let mut store = build_store("Draft start:2026-05-06T10:00:00\n");
        assert!(store.timer_running());
        store.delete(0);
        assert!(
            !store.timer_running(),
            "deleting the running task must clear the timer"
        );
    }

    #[test]
    fn undo_restores_timer_to_correct_task() {
        let mut store = build_store("task one\nDraft start:2026-05-06T10:00:00\n");
        store.delete(0); // timer task shifts to index 0
        assert_eq!(store.active_timer_abs(), Some(0));
        store.undo();
        assert_eq!(store.tasks().len(), 2);
        assert!(store.timer_running(), "timer must survive undo");
        assert_eq!(
            store.active_timer_abs(),
            Some(1),
            "undo must re-attach the timer to the restored task's index"
        );
        assert!(store.tasks()[1].start.is_some());
    }

    #[test]
    fn edit_preserves_running_timer() {
        let mut store = build_store("Draft start:2026-05-06T10:00:00\n");
        assert!(store.timer_running());
        store.edit_line(0, "Draft edited +work start:2026-05-06T10:00:00");
        assert!(
            store.timer_running(),
            "editing the body must not stop a running timer"
        );
        assert_eq!(store.active_timer_abs(), Some(0));
    }

    #[test]
    fn edit_removing_start_clears_timer() {
        let mut store = build_store("Draft start:2026-05-06T10:00:00\n");
        assert!(store.timer_running());
        store.edit_line(0, "Draft edited (start tag removed)");
        assert!(
            !store.timer_running(),
            "removing start: via edit must clear the timer"
        );
    }

    #[test]
    fn timer_elapsed_derived_from_start_token_on_restart() {
        // A task whose timer was started 65s ago (e.g. by a previous app
        // session, then restarted) must resume billing from the durable
        // `start:` tag rather than from load time — `Instant` resets on
        // restart and freezes across suspend, the tag does not.
        let start = (chrono::Local::now() - chrono::Duration::seconds(65))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string();
        let store = build_store(&format!("Draft start:{start}\n"));
        assert!(store.timer_running(), "restart must resume the timer");
        let elapsed = store.timer_elapsed_secs().unwrap_or(0);
        assert!(
            elapsed >= 60,
            "elapsed must count from the durable start tag, got {elapsed}"
        );
    }
}
