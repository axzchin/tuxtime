use super::Store;
use super::outcome::{
    AddOutcome, BulkCompleteOutcome, BulkDeleteOutcome, CompleteOutcome, DeleteOutcome,
    EditOutcome, PriorityOutcome, Reconcile, RenameOutcome, StoreError, TagOutcome, TimerOutcome,
    TimerQuitOutcome,
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
        if !raw.split_whitespace().any(|t| t == term) {
            return EditOutcome::TermNotFound;
        }
        let new_raw = raw
            .split_whitespace()
            .filter(|t| *t != term)
            .collect::<Vec<_>>()
            .join(" ");
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
        // can't shift earlier indices.
        spawns.sort_by_key(|s| std::cmp::Reverse(s.0));
        let spawned = spawns.len();
        for (abs, parsed) in spawns {
            self.tasks.insert(abs + 1, parsed);
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
            if let TimerOutcome::Stopped {
                elapsed_secs,
                total_secs,
                project,
                activity,
                ..
            } = old
            {
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
            return old;
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
        if let TimerOutcome::Stopped {
            abs,
            elapsed_secs,
            total_secs,
            ..
        } = outcome
        {
            self.push_history();
            if let Err(e) = self.persist() {
                return TimerQuitOutcome::Error(e);
            }
            return TimerQuitOutcome::Stopped {
                abs,
                elapsed_secs,
                total_secs,
            };
        }
        if let TimerOutcome::Error(e) = outcome {
            TimerQuitOutcome::Error(e)
        } else {
            TimerQuitOutcome::NoTimer
        }
    }

    fn timer_start_inner(&mut self, abs: usize) -> TimerOutcome {
        let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
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
                    body: todo::body_only(&t.raw),
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
        let elapsed = (chrono::Local::now().naive_local() - ts.started_at)
            .num_seconds()
            .max(0) as u64;
        let abs = ts.task_abs;
        if abs >= self.tasks.len() {
            return TimerOutcome::OutOfRange;
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
        // accumulated time to the stop date, not the task's creation date.
        // The stop date comes from the store's cached `today` (refreshed by
        // the TUI event loop every iteration) rather than a fresh wall-clock
        // read, keeping the store deterministic and the tests reproducible.
        let new_raw = rebuild_token_line(&new_raw, "log:", None, &format!("log:{}", self.today));
        match todo::parse_line(&new_raw) {
            Ok(parsed) => {
                let project = parsed.projects.first().cloned();
                let activity = parsed.contexts.first().cloned();
                let body = todo::body_only(&parsed.raw);
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

    fn timer_stop(&mut self) -> TimerOutcome {
        let outcome = self.timer_stop_inner();
        if matches!(outcome, TimerOutcome::Stopped { .. }) {
            self.push_history();
            if let Err(e) = self.persist() {
                return TimerOutcome::Error(e);
            }
        }
        outcome
    }

    /// Rename a project across all active and archived tasks. Replaces every
    /// occurrence of `+old` with `+new` (exact token match, not substring).
    /// One undo entry for the entire batch. Does NOT check archived-projects
    /// list — the App layer handles that.
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
                let new_raw = self.tasks[i]
                    .raw
                    .split_whitespace()
                    .map(|tok| {
                        if tok == needle {
                            replacement.as_str()
                        } else {
                            tok
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
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
                let new_raw = self.archive.tasks[i]
                    .raw
                    .split_whitespace()
                    .map(|tok| {
                        if tok == needle {
                            replacement.as_str()
                        } else {
                            tok
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
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
}

/// Rebuild a task's raw line by replacing or adding a specific token in the
/// body portion (after priority + creation date). `replace_prefix` is the
/// token prefix to replace (e.g. "start:", "dur:"). If `drop_prefix` is
/// `Some(p)`, tokens starting with `p` are dropped instead of being carried
/// forward (e.g. drop `start:` when stopping a timer). `new_token` is the
/// full replacement token value (e.g. "start:2026-05-07T12:00:00" or
/// "dur:5400").
/// Shared by the timer and manual time-entry paths to rewrite a task's raw
/// line: replace or add a `key:` token (e.g. `start:`, `dur:`, `log:`) while
/// preserving the priority/date prefix and token order. `drop_prefix` removes
/// a token family (e.g. `start:` when stopping a timer).
pub(crate) fn rebuild_token_line(
    raw: &str,
    replace_prefix: &str,
    drop_prefix: Option<&str>,
    new_token: &str,
) -> String {
    let body = crate::todo::body_after_priority(raw);
    let prefix = &raw[..raw.len() - body.len()];
    let mut new_tokens: Vec<String> = Vec::new();
    let mut has_token = false;
    for tok in body.split_whitespace() {
        if let Some(dp) = drop_prefix
            && tok.starts_with(dp)
        {
            continue;
        }
        if tok.starts_with(replace_prefix) {
            new_tokens.push(new_token.to_string());
            has_token = true;
        } else {
            new_tokens.push(tok.to_string());
        }
    }
    if !has_token {
        new_tokens.push(new_token.to_string());
    }
    if prefix.is_empty() {
        new_tokens.join(" ")
    } else {
        format!("{prefix}{}", new_tokens.join(" "))
    }
}

/// Identity of a recurring task for duplicate-spawn detection: the body with
/// the `due:` token removed and whitespace normalized. Two occurrences of the
/// same recurrence share this identity regardless of due date.
fn recurrence_identity(raw: &str) -> String {
    todo::body_after_priority(raw)
        .split_whitespace()
        .filter(|tok| !tok.starts_with("due:"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build the raw line for the next occurrence of a recurring task.
///
/// Inputs are the pre-completion `raw`, the pre-completion `due:` value
/// (strict-mode anchor), the parsed `RecSpec`, and `today`. Strict mode anchors
/// on the previous due date when present and parseable, else today + interval.
/// Date overflow returns `None` so the caller skips spawning.
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

    let body = todo::body_after_priority(raw);

    // Substitute the first `due:` with the new value, drop later `due:` dups.
    let mut out_tokens: Vec<String> = Vec::new();
    let mut due_seen = false;
    for tok in body.split_whitespace() {
        if let Some(rest) = tok.strip_prefix("due:")
            && !rest.is_empty()
        {
            if !due_seen {
                out_tokens.push(format!("due:{next_due_str}"));
                due_seen = true;
            }
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

#[cfg(test)]
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
