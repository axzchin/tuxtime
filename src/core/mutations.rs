use super::Store;
use super::outcome::{
    AddOutcome, BulkCompleteOutcome, BulkDeleteOutcome, CompleteOutcome, DeleteOutcome,
    EditOutcome, MoveOutcome, PriorityOutcome, Reconcile, RenameOutcome, StoreError, TagOutcome,
};
use crate::recurrence::{self, RecSpec};
use crate::todo::{self, TagError};

impl Store {
    pub fn toggle_complete(&mut self, abs: usize) -> CompleteOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return CompleteOutcome::Aborted(other),
        }
        let Some(t) = self.tasks.get(abs) else {
            return CompleteOutcome::OutOfRange;
        };
        let was_done = t.done;
        // Capture rec/due/raw of the pre-completion task — `mark_done` rewrites
        // `raw` (and strips priority), so the next-instance build must read
        // these before the mutation lands.
        let rec_spec = if was_done {
            None
        } else {
            t.rec.as_deref().and_then(recurrence::parse_rec_spec)
        };
        let raw_before = t.raw.clone();
        let due_before = t.due.clone();

        self.push_history();
        let result = if was_done {
            self.tasks[abs].unmark_done()
        } else {
            self.tasks[abs].mark_done(&self.today)
        };
        match result {
            Ok(()) => {
                let spawned = rec_spec.and_then(|spec| {
                    let next_raw = build_next_instance(
                        &raw_before,
                        due_before.as_deref(),
                        &spec,
                        &self.today,
                    )?;
                    // A single occurrence yields at most one live successor.
                    let identity = recurrence_identity(&next_raw);
                    let already_live = self.tasks.iter().enumerate().any(|(i, t)| {
                        i != abs && !t.done && recurrence_identity(&t.raw) == identity
                    });
                    if already_live {
                        return None;
                    }
                    let parsed = todo::parse_line(&next_raw).ok()?;
                    self.tasks.insert(abs + 1, parsed);
                    Some(abs + 1)
                });
                // Spawn insertion shifts indices above `abs` — re-attach the
                // running timer to the task that still carries `start:`.
                self.resync_timer();
                if let Err(e) = self.persist() {
                    return CompleteOutcome::Error(e);
                }
                match (was_done, spawned) {
                    (true, _) => CompleteOutcome::Uncompleted { abs },
                    (false, Some(next)) => CompleteOutcome::CompletedSpawned { abs, next },
                    (false, None) => CompleteOutcome::Completed { abs },
                }
            }
            Err(e) => CompleteOutcome::Error(StoreError::Parse(e)),
        }
    }

    pub fn cycle_priority(&mut self, abs: usize) -> PriorityOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return PriorityOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return PriorityOutcome::OutOfRange;
        }
        self.push_history();
        match self.tasks[abs].cycle_priority() {
            Ok(priority) => match self.persist() {
                Ok(()) => PriorityOutcome::Changed { abs, priority },
                Err(e) => PriorityOutcome::Error(e),
            },
            Err(e) => PriorityOutcome::Error(StoreError::Parse(e)),
        }
    }

    /// Set or clear a task's priority outright (CLI `pri` / `depri`).
    pub fn set_priority_at(&mut self, abs: usize, priority: Option<char>) -> PriorityOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return PriorityOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return PriorityOutcome::OutOfRange;
        }
        self.push_history();
        match self.tasks[abs].set_priority(priority) {
            Ok(()) => match self.persist() {
                Ok(()) => PriorityOutcome::Changed { abs, priority },
                Err(e) => PriorityOutcome::Error(e),
            },
            Err(e) => PriorityOutcome::Error(StoreError::Parse(e)),
        }
    }

    pub fn delete(&mut self, abs: usize) -> DeleteOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return DeleteOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return DeleteOutcome::OutOfRange;
        }
        self.push_history();
        self.tasks.remove(abs);
        // Removal shifts indices above `abs` — keep the timer attached to the
        // task that still carries `start:` (or clear it if that task was the
        // one deleted).
        self.resync_timer();
        match self.persist() {
            Ok(()) => DeleteOutcome::Deleted { abs },
            Err(e) => DeleteOutcome::Error(e),
        }
    }

    /// Reorder tasks by applying a list of index swaps in order. Used by
    /// manual task ordering (`J`/`K`) — the App computes the minimal swap
    /// list that moves the cursor (or the whole visual selection) one step
    /// while respecting sort-group boundaries.
    pub fn move_tasks(&mut self, swaps: &[(usize, usize)]) -> MoveOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return MoveOutcome::Aborted(other),
        }
        if swaps
            .iter()
            .any(|&(abs, target)| abs >= self.tasks.len() || target >= self.tasks.len())
        {
            return MoveOutcome::OutOfRange;
        }
        if swaps.is_empty() || swaps.iter().all(|&(abs, target)| abs == target) {
            return MoveOutcome::Unchanged;
        }
        let previous = self.tasks.clone();
        for &(abs, target) in swaps {
            self.tasks.swap(abs, target);
        }
        // Swapping moves the `start:`-bearing task between indices without
        // changing the length — re-attach the timer to the task that still
        // carries `start:`.
        self.resync_timer();
        match self.persist() {
            Ok(()) => {
                self.history.push(previous);
                MoveOutcome::Moved
            }
            Err(e) => {
                self.tasks = previous;
                self.resync_timer();
                MoveOutcome::Error(e)
            }
        }
    }

    /// Add a task from free text, running the full natural-language pipeline
    /// (`inbox::canonicalize_line`). Used by the CLI `add` command.
    pub fn add_line(&mut self, text: &str) -> AddOutcome {
        self.add_with(text, true)
    }

    /// Add a task from text that is already canonical todo.txt (no NL pass,
    /// just creation-date prefix + validation). Used by the TUI add-prompt's
    /// save path, where the draft was already rewritten to canonical form.
    pub fn add_finalized(&mut self, text: &str) -> AddOutcome {
        self.add_with(text, false)
    }

    fn add_with(&mut self, text: &str, natural_language: bool) -> AddOutcome {
        let text = text.trim();
        if text.is_empty() {
            return AddOutcome::Empty;
        }
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return AddOutcome::Aborted(other),
        }
        let parsed = if natural_language {
            match chrono::NaiveDate::parse_from_str(&self.today, "%Y-%m-%d") {
                Ok(d) => crate::inbox::canonicalize_line(text, d),
                // Defensive fallback (only a test sets a bad today): skip NL.
                Err(_) => crate::inbox::finalize_line(text, &self.today),
            }
        } else {
            crate::inbox::finalize_line(text, &self.today)
        };
        match parsed {
            Ok(task) => {
                self.push_history();
                self.tasks.push(task);
                match self.persist() {
                    Ok(()) => AddOutcome::Added {
                        abs: self.tasks.len() - 1,
                    },
                    Err(e) => AddOutcome::Error(e),
                }
            }
            Err(e) => AddOutcome::Error(StoreError::Parse(e)),
        }
    }

    /// Replace an entire task line (CLI `replace`, TUI edit save).
    pub fn edit_line(&mut self, abs: usize, text: &str) -> EditOutcome {
        let text = text.trim();
        if text.is_empty() {
            return EditOutcome::Empty;
        }
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return EditOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return EditOutcome::OutOfRange;
        }
        self.rewrite_raw(abs, text)
    }

    /// Append text to the end of a task line (CLI `append`).
    pub fn append_at(&mut self, abs: usize, text: &str) -> EditOutcome {
        let text = text.trim();
        if text.is_empty() {
            return EditOutcome::Empty;
        }
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return EditOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return EditOutcome::OutOfRange;
        }
        let new_raw = format!("{} {}", self.tasks[abs].raw.trim_end(), text);
        self.rewrite_raw(abs, &new_raw)
    }

    /// Prepend text to the start of a task's body — after any leading
    /// priority/dates so the line stays well-formed (CLI `prepend`).
    pub fn prepend_at(&mut self, abs: usize, text: &str) -> EditOutcome {
        let text = text.trim();
        if text.is_empty() {
            return EditOutcome::Empty;
        }
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return EditOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return EditOutcome::OutOfRange;
        }
        let raw = &self.tasks[abs].raw;
        let body = todo::body_after_priority(raw);
        let prefix = &raw[..raw.len() - body.len()];
        let new_raw = if body.is_empty() {
            format!("{prefix}{text}")
        } else {
            format!("{prefix}{text} {body}")
        };
        self.rewrite_raw(abs, &new_raw)
    }

    /// Remove a single whitespace-delimited term from a task line (CLI
    /// `del N TERM`). Returns `TermNotFound` when the term isn't present.
    pub fn remove_term_at(&mut self, abs: usize, term: &str) -> EditOutcome {
        let term = term.trim();
        if term.is_empty() {
            return EditOutcome::Empty;
        }
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return EditOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return EditOutcome::OutOfRange;
        }
        let raw = &self.tasks[abs].raw;
        // Only body tokens are removable — a term that happens to equal the
        // priority or a date must not strip the line's metadata prefix.
        if !todo::body_after_priority(raw)
            .split_whitespace()
            .any(|t| t == term)
        {
            return EditOutcome::TermNotFound;
        }
        let new_raw =
            todo::map_body_tokens(raw, |t| if t == term { None } else { Some(t.to_string()) });
        self.rewrite_raw(abs, &new_raw)
    }

    /// Parse `new_raw`, snapshot for undo, replace the task at `abs`, persist.
    /// Caller is responsible for reconcile + bounds checks.
    fn rewrite_raw(&mut self, abs: usize, new_raw: &str) -> EditOutcome {
        match todo::parse_line(new_raw) {
            Ok(task) => {
                self.push_history();
                self.tasks[abs] = task;
                // Edits that drop `start:` from the running task's line must
                // stop the timer (the tag is the source of truth); edits that
                // preserve it keep the timer attached. The edit dialog carries
                // the full raw line, so body-only edits are unaffected.
                self.resync_timer();
                match self.persist() {
                    Ok(()) => EditOutcome::Saved { abs },
                    Err(e) => EditOutcome::Error(e),
                }
            }
            Err(e) => EditOutcome::Error(StoreError::Parse(e)),
        }
    }

    pub fn add_project(&mut self, abs: usize, name: &str) -> TagOutcome {
        let name = name.trim();
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return TagOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return TagOutcome::OutOfRange;
        }
        let mut task = self.tasks[abs].clone();
        match task.add_project(name) {
            Ok(true) => {
                self.push_history();
                self.tasks[abs] = task;
                match self.persist() {
                    Ok(()) => TagOutcome::Added {
                        abs,
                        name: name.to_string(),
                    },
                    Err(e) => TagOutcome::Error(e),
                }
            }
            Ok(false) => TagOutcome::Unchanged,
            Err(TagError::Invalid) => TagOutcome::InvalidName,
            Err(TagError::Parse(e)) => TagOutcome::Error(StoreError::Parse(e)),
        }
    }

    pub fn toggle_context(&mut self, abs: usize, name: &str) -> TagOutcome {
        let name = name.trim();
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return TagOutcome::Aborted(other),
        }
        if abs >= self.tasks.len() {
            return TagOutcome::OutOfRange;
        }
        let has = self.tasks[abs].contexts.iter().any(|c| c == name);
        let mut task = self.tasks[abs].clone();
        let result = if has {
            task.remove_context(name).map(|_| ())
        } else {
            task.add_context(name).map(|_| ())
        };
        match result {
            Ok(()) => {
                self.push_history();
                self.tasks[abs] = task;
                if let Err(e) = self.persist() {
                    return TagOutcome::Error(e);
                }
                if has {
                    TagOutcome::Removed {
                        abs,
                        name: name.to_string(),
                    }
                } else {
                    TagOutcome::Added {
                        abs,
                        name: name.to_string(),
                    }
                }
            }
            Err(TagError::Invalid) => TagOutcome::InvalidName,
            Err(TagError::Parse(e)) => TagOutcome::Error(StoreError::Parse(e)),
        }
    }

    /// Bulk-complete the given task indices, spawning recurring successors.
    /// Indices that are out of range or already done are skipped.
    pub fn complete_many(&mut self, indices: &[usize]) -> BulkCompleteOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return BulkCompleteOutcome::Aborted(other),
        }
        let to_complete: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&i| i < self.tasks.len() && !self.tasks[i].done)
            .collect();
        if to_complete.is_empty() {
            return BulkCompleteOutcome::NothingToComplete;
        }
        self.push_history();
        // Pass 1: complete in place, collecting spawn lines by original index.
        let mut spawns: Vec<(usize, todo::Task)> = Vec::new();
        for abs in to_complete.iter().copied() {
            let t = &self.tasks[abs];
            let raw = t.raw.clone();
            let due = t.due.clone();
            let rec_spec = t.rec.as_deref().and_then(recurrence::parse_rec_spec);
            let created = t.created_date.clone().unwrap_or_else(|| self.today.clone());
            let body = todo::body_after_priority(&raw).to_string();
            let new_raw = format!("x {} {} {}", self.today, created, body);
            if let Ok(parsed) = todo::parse_line(&new_raw) {
                self.tasks[abs] = parsed;
            }
            if let Some(spec) = rec_spec
                && let Some(next_raw) =
                    build_next_instance(&raw, due.as_deref(), &spec, &self.today)
                && let Ok(next) = todo::parse_line(&next_raw)
            {
                spawns.push((abs, next));
            }
        }
        // Pass 2: insert spawns at original_abs+1, descending, so later inserts
        // can't shift earlier indices. Dedupe like `toggle_complete`: a spawn
        // whose identity already has a live carrier is dropped, so
        // bulk-completing several occurrences of the same recurring task
        // yields a single successor rather than one per completed line.
        spawns.sort_by_key(|s| std::cmp::Reverse(s.0));
        let mut live_identities: Vec<String> = self
            .tasks
            .iter()
            .filter(|t| !t.done)
            .map(|t| recurrence_identity(&t.raw))
            .collect();
        let mut spawned = 0usize;
        for (abs, parsed) in spawns {
            let identity = recurrence_identity(&parsed.raw);
            if live_identities.contains(&identity) {
                continue;
            }
            live_identities.push(identity);
            self.tasks.insert(abs + 1, parsed);
            spawned += 1;
        }
        self.resync_timer();
        let completed = to_complete.len();
        match self.persist() {
            Ok(()) => BulkCompleteOutcome::Done { completed, spawned },
            Err(e) => BulkCompleteOutcome::Error(e),
        }
    }

    /// Bulk-delete the given task indices.
    pub fn delete_many(&mut self, indices: &[usize]) -> BulkDeleteOutcome {
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return BulkDeleteOutcome::Aborted(other),
        }
        let mut indices: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|&i| i < self.tasks.len())
            .collect();
        if indices.is_empty() {
            return BulkDeleteOutcome::Nothing;
        }
        indices.sort_by(|a, b| b.cmp(a));
        self.push_history();
        let deleted = indices.len();
        for abs in indices {
            self.tasks.remove(abs);
        }
        self.resync_timer();
        match self.persist() {
            Ok(()) => BulkDeleteOutcome::Done { deleted },
            Err(e) => BulkDeleteOutcome::Error(e),
        }
    }

    // ---- timer mutations ----

    pub fn rename_project(&mut self, old: &str, new: &str) -> RenameOutcome {
        let old = old.trim();
        let new = new.trim();
        if new.is_empty() || !todo::is_valid_tag_name(new) {
            return RenameOutcome::InvalidName;
        }
        if new == old {
            return RenameOutcome::NoTasks;
        }
        match self.reconcile() {
            Reconcile::Unchanged => {}
            other => return RenameOutcome::Aborted(other),
        }

        let needle = format!("+{old}");
        let replacement = format!("+{new}");
        let mut active_count = 0usize;
        let mut archived_count = 0usize;

        // Rename in active tasks.
        for i in 0..self.tasks.len() {
            if self.tasks[i].projects.iter().any(|p| p == old) {
                let new_raw = todo::map_body_tokens(&self.tasks[i].raw, |tok| {
                    if tok == needle.as_str() {
                        Some(replacement.clone())
                    } else {
                        Some(tok.to_string())
                    }
                });
                if let Ok(parsed) = todo::parse_line(&new_raw) {
                    self.tasks[i] = parsed;
                    active_count += 1;
                }
            }
        }

        // Rename in archived tasks.
        let mut archive_modified = false;
        for i in 0..self.archive.tasks.len() {
            if self.archive.tasks[i].projects.iter().any(|p| p == old) {
                let new_raw = todo::map_body_tokens(&self.archive.tasks[i].raw, |tok| {
                    if tok == needle.as_str() {
                        Some(replacement.clone())
                    } else {
                        Some(tok.to_string())
                    }
                });
                if let Ok(parsed) = todo::parse_line(&new_raw) {
                    self.archive.tasks[i] = parsed;
                    archived_count += 1;
                    archive_modified = true;
                }
            }
        }

        if active_count == 0 && archived_count == 0 {
            return RenameOutcome::NoTasks;
        }

        self.push_history();
        if let Err(e) = self.persist() {
            return RenameOutcome::Error(e);
        }
        if archive_modified {
            let archive_body = todo::serialize(&self.archive.tasks);
            if let Err(e) = todo::write_atomic(&self.archive.path, &archive_body) {
                return RenameOutcome::Error(StoreError::ArchiveIo(e));
            }
            self.archive.last_disk = archive_body;
            self.archive.last_meta = super::file_sig(&self.archive.path);
        }
        RenameOutcome::Renamed {
            old: old.to_string(),
            new: new.to_string(),
            active_count,
            archived_count,
        }
    }

    // ---- one line per task-day: carry forward / midnight split ----
}

fn recurrence_identity(raw: &str) -> String {
    todo::body_after_priority(raw)
        .split_whitespace()
        .filter(|tok| !tok.starts_with("due:"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_next_instance(
    raw: &str,
    due: Option<&str>,
    spec: &RecSpec,
    today: &str,
) -> Option<String> {
    use chrono::NaiveDate;
    let today_date = NaiveDate::parse_from_str(today, "%Y-%m-%d").ok()?;
    let anchor = if spec.strict {
        due.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .unwrap_or(today_date)
    } else {
        today_date
    };
    let next_due = recurrence::advance(anchor, spec)?;
    let next_due_str = next_due.format("%Y-%m-%d").to_string();
    // The *old* due, when parseable — used to shift an absolute `t:` by the
    // same amount the due moved (see `shift_absolute_threshold`).
    let old_due_date = due.and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());

    let body = todo::body_after_priority(raw);

    // Substitute the first `due:` with the new value, drop later `due:` dups.
    let mut out_tokens: Vec<String> = Vec::new();
    let mut due_seen = false;
    for tok in todo::split_body_tokens(body) {
        if let Some(rest) = tok.strip_prefix("due:")
            && !rest.is_empty()
        {
            if !due_seen {
                out_tokens.push(format!("due:{next_due_str}"));
                due_seen = true;
            }
            continue;
        }
        // Shift an absolute `t:` along with the `due:` so "N days before due"
        // stays true on the successor; relative `t:` passes through verbatim
        // (it re-anchors against the new due at render time).
        if let Some(rest) = tok.strip_prefix("t:")
            && !rest.is_empty()
        {
            out_tokens.push(
                shift_absolute_threshold(tok, old_due_date, next_due)
                    .unwrap_or_else(|| tok.to_string()),
            );
            continue;
        }
        // The successor starts with no time: drop the live timer and the
        // accumulated session tags, or the next occurrence would double-count
        // the completed session (and a copied `start:` would violate the
        // single-timer invariant). Matches `carry_forward_body`.
        if tok.starts_with("start:") || tok.starts_with("dur:") || tok.starts_with("log:") {
            continue;
        }
        out_tokens.push(tok.to_string());
    }
    if !due_seen {
        out_tokens.push(format!("due:{next_due_str}"));
    }

    let prefix = match todo::parse_line(raw).ok().and_then(|t| t.priority) {
        Some(p) => format!("({p}) {today} "),
        None => format!("{today} "),
    };
    Some(format!("{prefix}{}", out_tokens.join(" ")))
}

/// Shift an absolute `t:` threshold by the same day-delta the `due:` moved, so
/// "N days before due" survives the spawn. Relative thresholds re-anchor
/// against the successor's `due:` on their own, and an absolute threshold with
/// no parseable old `due:` (or a malformed `t:` value) is left untouched —
/// returning `None` so the caller copies the token verbatim.
fn shift_absolute_threshold(
    tok: &str,
    old_due: Option<chrono::NaiveDate>,
    next_due: chrono::NaiveDate,
) -> Option<String> {
    let value = tok.strip_prefix("t:")?;
    let crate::threshold::ThresholdSpec::Absolute(date) = crate::threshold::parse_threshold(value)?
    else {
        return None;
    };
    let old_due = old_due?;
    let delta = next_due.signed_duration_since(old_due).num_days();
    date.checked_add_signed(chrono::TimeDelta::days(delta))
        .map(|d| format!("t:{}", d.format("%Y-%m-%d")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::core::outcome::CompleteOutcome;
    use crate::core::test_support::build_store;

    #[test]
    fn toggle_complete_marks_pending_task_done() {
        let mut store = build_store("a\n");
        assert!(matches!(
            store.toggle_complete(0),
            CompleteOutcome::Completed { .. }
        ));
        assert!(store.tasks()[0].done);
    }

    #[test]
    fn toggle_complete_undoes_done_task() {
        let mut store = build_store("x 2026-05-05 2026-05-01 finish report\n");
        assert!(store.tasks()[0].done);
        assert!(matches!(
            store.toggle_complete(0),
            CompleteOutcome::Uncompleted { .. }
        ));
        assert!(!store.tasks()[0].done);
        assert_eq!(store.tasks()[0].raw, "2026-05-01 finish report");
    }

    #[test]
    fn toggle_complete_spawns_next_for_strict_monthly() {
        let mut store = build_store("(A) 2026-04-15 Pay rent due:2026-04-15 rec:+1m\n");
        assert!(matches!(
            store.toggle_complete(0),
            CompleteOutcome::CompletedSpawned { .. }
        ));
        assert_eq!(store.tasks().len(), 2);
        assert!(store.tasks()[0].done);
        assert!(!store.tasks()[1].done);
        assert_eq!(store.tasks()[1].due.as_deref(), Some("2026-05-15"));
        assert_eq!(store.tasks()[1].rec.as_deref(), Some("+1m"));
        assert_eq!(store.tasks()[1].priority, Some('A'));
    }

    #[test]
    fn toggle_complete_spawns_next_for_normal_weekly_no_due() {
        let mut store = build_store("Water plants rec:1w\n");
        store.set_today("2026-05-09".to_string());
        store.toggle_complete(0);
        assert_eq!(store.tasks().len(), 2);
        assert_eq!(store.tasks()[1].due.as_deref(), Some("2026-05-16"));
        assert_eq!(store.tasks()[1].rec.as_deref(), Some("1w"));
    }

    #[test]
    fn toggle_complete_clamps_month_end() {
        let mut store = build_store("Pay bill due:2026-01-31 rec:+1m\n");
        store.set_today("2026-01-31".to_string());
        store.toggle_complete(0);
        assert_eq!(store.tasks()[1].due.as_deref(), Some("2026-02-28"));
    }

    #[test]
    fn toggle_complete_no_rec_does_not_spawn() {
        let mut store = build_store("a\n");
        store.toggle_complete(0);
        assert_eq!(store.tasks().len(), 1);
    }

    #[test]
    fn toggle_complete_invalid_rec_completes_without_spawn() {
        let mut store = build_store("a rec:bogus\n");
        assert!(matches!(
            store.toggle_complete(0),
            CompleteOutcome::Completed { .. }
        ));
        assert_eq!(store.tasks().len(), 1);
        assert!(store.tasks()[0].done);
    }

    #[test]
    fn toggle_complete_strict_with_bad_due_falls_back_to_today() {
        let mut store = build_store("Stretch due:tomorrow rec:+2d\n");
        store.set_today("2026-05-09".to_string());
        store.toggle_complete(0);
        assert_eq!(store.tasks().len(), 2);
        assert_eq!(store.tasks()[1].due.as_deref(), Some("2026-05-11"));
    }

    #[test]
    fn toggle_complete_undo_rolls_back_completion_and_spawn() {
        let mut store = build_store("Do thing due:2026-05-15 rec:+1w\n");
        store.toggle_complete(0);
        assert_eq!(store.tasks().len(), 2);
        store.undo();
        assert_eq!(store.tasks().len(), 1);
        assert!(!store.tasks()[0].done);
    }

    #[test]
    fn toggle_complete_does_not_respawn_when_live_successor_exists() {
        let mut store = build_store("Water plants due:2026-05-15 rec:1d\n");
        store.set_today("2026-05-15".to_string());
        store.toggle_complete(0);
        assert_eq!(store.tasks().len(), 2);
        store.toggle_complete(0);
        assert_eq!(store.tasks().len(), 2);
        assert!(!store.tasks()[0].done);
        assert!(matches!(
            store.toggle_complete(0),
            CompleteOutcome::Completed { .. }
        ));
        assert_eq!(store.tasks().len(), 2);
    }

    #[test]
    fn toggle_complete_drops_duplicate_due_tokens_in_spawn() {
        let mut store = build_store("Bug due:2026-05-15 due:2026-09-09 rec:+1d\n");
        store.toggle_complete(0);
        let next_raw = &store.tasks()[1].raw;
        assert_eq!(next_raw.matches("due:").count(), 1);
        assert!(next_raw.contains("due:2026-05-16"));
    }

    // ---- recurrence × timer interaction ----

    #[test]
    fn spawn_reshifts_running_timer_to_the_shifted_task() {
        // Task 0 is recurring; task 1 carries a live timer (`start:`).
        let mut store = build_store(
            "Pay rent due:2026-05-06 rec:+1m\nDraft motion start:2026-05-06T10:00:00\n",
        );
        assert_eq!(store.active_timer_abs(), Some(1));
        assert!(store.is_timer_running_on(1));

        assert!(matches!(
            store.toggle_complete(0),
            CompleteOutcome::CompletedSpawned { .. }
        ));
        // The successor is inserted at index 1, pushing the timed task to 2.
        // `resync_timer` must re-attach the timer to the task that still
        // carries `start:`, not blindly keep index 1.
        assert_eq!(store.tasks().len(), 3);
        assert_eq!(store.active_timer_abs(), Some(2));
        assert!(store.is_timer_running_on(2));
        assert!(!store.is_timer_running_on(1), "successor must not run");
        // Index 1 is the recurring successor, not the timed task.
        assert_eq!(store.tasks()[1].rec.as_deref(), Some("+1m"));
        assert_eq!(store.tasks()[1].due.as_deref(), Some("2026-06-06"));
        // The timed task still carries its `start:` tag.
        assert!(store.task_raw(2).unwrap().contains("start:"));
    }

    #[test]
    fn spawn_successor_starts_with_no_time() {
        // A recurring task completed while a timer runs (start:) and with
        // accumulated dur:/log: must yield a successor that starts fresh:
        // no start:, dur:, or log: — otherwise the next occurrence would
        // double-count the completed session's time.
        let mut store = build_store(
            "Water plants due:2026-05-06 rec:1d dur:3600 log:2026-05-05 start:2026-05-06T10:00:00\n",
        );
        store.toggle_complete(0);
        assert_eq!(store.tasks().len(), 2);
        let next_raw = store.task_raw(1).unwrap();
        assert!(
            !next_raw.contains("start:"),
            "successor must not inherit the timer: {next_raw}"
        );
        assert!(
            !next_raw.contains("dur:"),
            "successor must not inherit elapsed time: {next_raw}"
        );
        assert!(
            !next_raw.contains("log:"),
            "successor must not inherit the log date: {next_raw}"
        );
        // The successor keeps the recurrence and a fresh due.
        assert_eq!(store.tasks()[1].rec.as_deref(), Some("1d"));
        assert_eq!(store.tasks()[1].due.as_deref(), Some("2026-05-07"));
        // The completed original keeps its accumulated time.
        assert!(store.tasks()[0].done);
        assert!(store.task_raw(0).unwrap().contains("dur:3600"));
    }

    #[test]
    fn completing_running_recurring_task_keeps_timer_on_the_done_task() {
        let mut store =
            build_store("Water plants due:2026-05-06 rec:1d start:2026-05-06T10:00:00\n");
        assert_eq!(store.active_timer_abs(), Some(0));
        store.toggle_complete(0);
        // `mark_done` preserves `start:`, so the live timer stays attached to
        // the now-completed task (index 0) rather than being stolen by the
        // successor. Completing a task does not stop its timer.
        assert_eq!(store.tasks().len(), 2);
        assert!(store.tasks()[0].done);
        assert_eq!(store.active_timer_abs(), Some(0));
        assert!(store.task_raw(0).unwrap().contains("start:"));
        assert!(!store.is_timer_running_on(1));
    }

    #[test]
    fn non_strict_spawn_anchors_to_completion_date_not_existing_due() {
        // Without the `+` prefix the next due is computed from the completion
        // date (today), not from the stale previous due date.
        let mut store = build_store("Water plants due:2026-03-01 rec:1w\n");
        store.set_today("2026-05-09".to_string());
        store.toggle_complete(0);
        assert_eq!(store.tasks()[1].due.as_deref(), Some("2026-05-16"));
    }

    #[test]
    fn add_line_runs_natural_language() {
        let mut store = build_store("");
        assert!(matches!(
            store.add_line("Buy milk tomorrow"),
            AddOutcome::Added { .. }
        ));
        assert_eq!(store.tasks().len(), 1);
        assert!(store.tasks()[0].raw.contains("Buy milk"));
        // build_store today = 2026-05-06.
        assert_eq!(store.tasks()[0].due.as_deref(), Some("2026-05-07"));
    }

    #[test]
    fn add_finalized_does_not_reinterpret_prose() {
        let mut store = build_store("");
        store.add_finalized("Buy milk tomorrow");
        assert_eq!(store.tasks().len(), 1);
        // No NL pass: "tomorrow" stays literal, no due assigned.
        assert!(store.tasks()[0].due.is_none());
    }

    #[test]
    fn set_priority_and_depri() {
        let mut store = build_store("buy milk\n");
        assert!(matches!(
            store.set_priority_at(0, Some('A')),
            PriorityOutcome::Changed {
                priority: Some('A'),
                ..
            }
        ));
        assert_eq!(store.tasks()[0].priority, Some('A'));
        store.set_priority_at(0, None);
        assert_eq!(store.tasks()[0].priority, None);
    }

    #[test]
    fn move_task_persists_and_undoes() {
        let mut store = build_store("first\nsecond\nthird\n");
        assert!(matches!(
            store.move_tasks(&[(1, 2), (0, 1)]),
            MoveOutcome::Moved
        ));
        assert_eq!(
            store
                .tasks()
                .iter()
                .map(|task| task.raw.as_str())
                .collect::<Vec<_>>(),
            ["third", "first", "second"]
        );
        assert_eq!(
            std::fs::read_to_string(&store.file_path).expect("read todo.txt"),
            "third\nfirst\nsecond\n"
        );
        store.undo();
        assert_eq!(
            store
                .tasks()
                .iter()
                .map(|task| task.raw.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
    }

    #[test]
    fn move_tasks_aborts_after_external_edit() {
        let mut store = build_store("first\nsecond\n");
        std::fs::write(&store.file_path, "external\nfirst\nsecond\n")
            .expect("write external change");

        assert!(matches!(
            store.move_tasks(&[(0, 1)]),
            MoveOutcome::Aborted(Reconcile::Reloaded)
        ));
        assert_eq!(
            store
                .tasks()
                .iter()
                .map(|task| task.raw.as_str())
                .collect::<Vec<_>>(),
            ["external", "first", "second"]
        );
    }

    #[test]
    fn move_tasks_resyncs_running_timer_to_swapped_task() {
        let mut store = build_store("first start:2026-05-06T10:00:00\nsecond\n");
        assert_eq!(store.active_timer_abs(), Some(0));
        assert!(matches!(store.move_tasks(&[(0, 1)]), MoveOutcome::Moved));
        assert_eq!(
            store.active_timer_abs(),
            Some(1),
            "the running timer must follow the start:-bearing task across the swap"
        );
        assert_eq!(store.tasks()[1].raw, "first start:2026-05-06T10:00:00");
    }

    #[test]
    fn append_and_prepend_and_replace() {
        let mut store = build_store("(A) 2026-05-01 do thing\n");
        store.append_at(0, "+work");
        assert!(store.tasks()[0].projects.contains(&"work".to_string()));
        store.prepend_at(0, "URGENT");
        // Prepend lands after the priority + creation date.
        assert!(store.tasks()[0].raw.starts_with("(A) 2026-05-01 URGENT"));
        store.edit_line(0, "completely new");
        assert_eq!(store.tasks()[0].raw, "completely new");
    }

    #[test]
    fn remove_term_removes_token_or_reports_missing() {
        let mut store = build_store("call mom +family @phone\n");
        assert!(matches!(
            store.remove_term_at(0, "+family"),
            EditOutcome::Saved { .. }
        ));
        assert!(!store.tasks()[0].projects.contains(&"family".to_string()));
        assert!(matches!(
            store.remove_term_at(0, "+nope"),
            EditOutcome::TermNotFound
        ));
    }

    /// `del N TERM` removes only body tokens: a term that happens to equal the
    /// priority or a creation date must not strip the line's metadata prefix
    /// (the bug `raw.split_whitespace()` filtering introduced — the prefix is
    /// preserved by `todo::map_body_tokens`).
    #[test]
    fn remove_term_never_strips_priority_or_date_prefix() {
        let mut store = build_store("(A) 2026-05-01 call mom +family\n");
        // The creation-date token is metadata, not a removable body term.
        assert!(matches!(
            store.remove_term_at(0, "2026-05-01"),
            EditOutcome::TermNotFound
        ));
        assert!(store.tasks()[0].raw.starts_with("(A) 2026-05-01"));
        // Same for the priority token.
        assert!(matches!(
            store.remove_term_at(0, "(A)"),
            EditOutcome::TermNotFound
        ));
        assert!(store.tasks()[0].raw.starts_with("(A) 2026-05-01"));
        // A genuine body token is still removable, leaving the prefix intact.
        assert!(matches!(
            store.remove_term_at(0, "+family"),
            EditOutcome::Saved { .. }
        ));
        assert!(store.tasks()[0].raw.starts_with("(A) 2026-05-01"));
        assert!(!store.tasks()[0].raw.contains("+family"));
    }

    #[test]
    fn add_project_clean_and_invalid() {
        let mut store = build_store("a\n");
        assert!(matches!(
            store.add_project(0, "health"),
            TagOutcome::Added { .. }
        ));
        assert_eq!(store.tasks()[0].projects, vec!["health"]);
        assert!(matches!(
            store.add_project(0, "two words"),
            TagOutcome::InvalidName
        ));
        assert!(matches!(
            store.add_project(0, "health"),
            TagOutcome::Unchanged
        ));
    }

    #[test]
    fn complete_many_marks_and_spawns() {
        let mut store = build_store("a\nPay rent due:2026-04-15 rec:+1m\nb\nWater plants rec:1w\n");
        store.set_today("2026-05-09".to_string());
        let out = store.complete_many(&[1, 3]);
        assert!(matches!(
            out,
            BulkCompleteOutcome::Done {
                completed: 2,
                spawned: 2
            }
        ));
        assert_eq!(store.tasks().len(), 6);
        assert!(store.tasks()[1].done);
        assert_eq!(store.tasks()[2].due.as_deref(), Some("2026-05-15"));
        assert_eq!(store.tasks()[3].raw, "b");
        assert!(store.tasks()[4].done);
        assert_eq!(store.tasks()[5].due.as_deref(), Some("2026-05-16"));
    }

    #[test]
    fn complete_many_skips_already_done() {
        let mut store = build_store("a\nx 2026-05-05 2026-05-01 b\nc\n");
        store.complete_many(&[0, 1, 2]);
        assert!(store.tasks()[0].done);
        assert_eq!(store.tasks()[1].done_date.as_deref(), Some("2026-05-05"));
        assert!(store.tasks()[2].done);
    }

    #[test]
    fn complete_many_does_not_double_spawn_same_recurring_identity() {
        // Two live occurrences of the same strict recurring task, both
        // selected at once. Only the latest occurrence leaves a live
        // successor; the superseded one must not spawn a duplicate.
        let mut store = build_store(
            "Water plants due:2026-05-06 rec:+1d\nWater plants due:2026-05-07 rec:+1d\n",
        );
        let out = store.complete_many(&[0, 1]);
        assert!(matches!(
            out,
            BulkCompleteOutcome::Done {
                completed: 2,
                spawned: 1
            }
        ));
        let live: Vec<_> = store.tasks().iter().filter(|t| !t.done).collect();
        assert_eq!(live.len(), 1, "exactly one live successor expected");
        assert_eq!(live[0].due.as_deref(), Some("2026-05-08"));
    }

    #[test]
    fn complete_many_skips_spawn_when_live_successor_already_exists() {
        // Bulk-completing a recurring task whose successor already exists live
        // must not spawn a second one (mirrors `toggle_complete`'s guard).
        let mut store =
            build_store("Water plants due:2026-05-06 rec:1d\nWater plants due:2026-05-07 rec:1d\n");
        let out = store.complete_many(&[0]);
        assert!(matches!(
            out,
            BulkCompleteOutcome::Done {
                completed: 1,
                spawned: 0
            }
        ));
        assert_eq!(
            store.tasks().iter().filter(|t| !t.done).count(),
            1,
            "the pre-existing successor is the only live task"
        );
    }

    #[test]
    fn spawn_preserves_quoted_note_verbatim() {
        // A quoted note with internal spaces (and a `due:`-shaped word inside)
        // must round-trip intact into the successor; a whitespace split would
        // slice it and rewrite/drop the inner tokens.
        let mut store = build_store(
            "Water plants due:2026-05-06 rec:1d note:\"call ops re: due:2026-05-20\"\n",
        );
        store.toggle_complete(0);
        let next = &store.tasks()[1];
        assert_eq!(next.notes, vec!["call ops re: due:2026-05-20".to_string()]);
        assert!(next.raw.contains("note:\"call ops re: due:2026-05-20\""));
        assert_eq!(next.due.as_deref(), Some("2026-05-07"));
    }

    #[test]
    fn spawn_shifts_absolute_threshold_with_due() {
        // "3 days before due" must stay true after the due advances a month.
        let mut store = build_store("Pay rent due:2026-05-15 t:2026-05-12 rec:+1m\n");
        store.toggle_complete(0);
        assert_eq!(store.tasks()[1].due.as_deref(), Some("2026-06-15"));
        assert_eq!(store.tasks()[1].threshold.as_deref(), Some("2026-06-12"));
    }

    #[test]
    fn spawn_shifts_absolute_threshold_across_month_end_clamp() {
        // Jan 31 + 1m clamps to Feb 28; the 3-day-before offset must survive.
        let mut store = build_store("Pay bill due:2026-01-31 t:2026-01-28 rec:+1m\n");
        store.toggle_complete(0);
        assert_eq!(store.tasks()[1].due.as_deref(), Some("2026-02-28"));
        assert_eq!(store.tasks()[1].threshold.as_deref(), Some("2026-02-25"));
    }

    #[test]
    fn spawn_leaves_relative_threshold_untouched() {
        // Relative `t:` re-anchors against the new due at render time.
        let mut store = build_store("Pay rent due:2026-05-15 t:-3d rec:+1m\n");
        store.toggle_complete(0);
        assert_eq!(store.tasks()[1].threshold.as_deref(), Some("-3d"));
    }

    #[test]
    fn spawn_leaves_absolute_threshold_when_no_parseable_due() {
        // No old due to compute a delta from: keep the absolute date verbatim
        // rather than guessing.
        let mut store = build_store("Standup t:2026-05-12 rec:1d\n");
        store.toggle_complete(0);
        assert_eq!(store.tasks()[1].threshold.as_deref(), Some("2026-05-12"));
    }

    #[test]
    fn delete_many_removes_all() {
        let mut store = build_store("a\nb\nc\nd\n");
        assert!(matches!(
            store.delete_many(&[1, 3]),
            BulkDeleteOutcome::Done { deleted: 2 }
        ));
        assert_eq!(store.tasks().len(), 2);
        assert_eq!(store.tasks()[0].raw, "a");
        assert_eq!(store.tasks()[1].raw, "c");
    }

    // ---- timer log-date stamping ----
}
