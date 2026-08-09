//! Headless core: the durable task store, its persistence/I-O, and all task
//! mutations. Carries no view, input, or presentation state — operations return
//! structured [`outcome`] values rather than user-facing strings. Both the TUI
//! (`App` wraps a `Store`) and the CLI (`cmd`) drive this type.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::NaiveDateTime;

use crate::todo::{self, Task};

mod archive;
mod external;
mod history;
mod mutations;
mod timer;

pub mod filter;
pub mod outcome;

#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use timer::{carry_forward_body, rebuild_token_line};

pub use archive::Archive;
pub use history::History;
pub use outcome::{
    AddOutcome, ArchiveDeleteOutcome, ArchiveOneOutcome, ArchiveOutcome, BulkCompleteOutcome,
    BulkDeleteOutcome, CarryForwardOutcome, CompleteOutcome, DeleteOutcome, DrainReport,
    EditOutcome, PriorityOutcome, Reconcile, StoreError, TagOutcome, TimerOutcome,
    TimerQuitOutcome, UnarchiveOutcome, UndoOutcome,
};

/// The durable task store. Owns the live task list, the sibling `done.txt`
/// archive, undo history, and the on-disk reconciliation snapshot.
pub struct Store {
    pub(crate) tasks: Vec<Task>,
    pub(crate) history: History,
    pub(crate) archive: Archive,
    pub(crate) file_path: PathBuf,
    /// Snapshot of the file body the last time we read or wrote it; used by
    /// `reconcile` to detect external edits.
    pub(crate) last_disk: String,
    /// Signature `(mtime, len)` of the todo file the last time we read it or
    /// wrote it. Lets [`Store::reconcile`] skip the full-file read on every
    /// keystroke when the file hasn't changed. `None` means "unknown — read"
    /// (also used to represent a missing file).
    pub(crate) last_meta: Option<(SystemTime, u64)>,
    pub(crate) today: String,
    /// State of the currently running timer, if any. `None` means no timer
    /// is active. Re-derived from the task list (whichever task carries a
    /// `start:` tag) by [`Store::resync_timer`], so it can never point at
    /// the wrong task after a reload, undo, or index-shifting mutation.
    pub(crate) active_timer: Option<TimerState>,
}

/// Wall-clock state for the running timer. The on-disk truth is the task's
/// `start:` tag; `started_at` is that tag parsed back, so elapsed time is
/// computed from the durable timestamp rather than a monotonic clock. That
/// keeps the timer honest across app restarts (which reset `Instant`) and
/// system suspend (which freezes it).
#[derive(Debug, Clone)]
pub struct TimerState {
    pub task_abs: usize,
    pub started_at: NaiveDateTime,
}

/// Cheap file-change signature: `(mtime, size)`. `None` when the file is
/// missing or its metadata can't be read (in which case the caller should
/// read anyway to learn what's going on). Comparing signatures avoids a
/// full-file read on every keystroke/tick; on filesystems with nanosecond
/// mtime resolution an unchanged signature means unchanged content.
pub(crate) fn file_sig(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}

impl Store {
    /// Construct a store, loading the archive (`done.txt`) off-thread from the
    /// sibling of `file_path`. Used by the TUI so the first frame doesn't wait
    /// on the archive read.
    #[must_use]
    pub fn new(file_path: PathBuf, body: String, today: String) -> Self {
        let archive = Archive::spawn(&file_path);
        Self::assemble(file_path, archive, body, today)
    }

    /// Like [`Store::new`] but with an explicit `done.txt` path (e.g. from a
    /// `DONE_FILE` env var that isn't a sibling of the todo file).
    #[must_use]
    pub fn new_with_done(
        file_path: PathBuf,
        done_path: PathBuf,
        body: String,
        today: String,
    ) -> Self {
        let archive = Archive::spawn_at(done_path);
        Self::assemble(file_path, archive, body, today)
    }

    /// Construct a store, loading the sibling archive synchronously (no
    /// background thread). Used by the one-shot CLI.
    #[must_use]
    pub fn open_sync(file_path: PathBuf, body: String, today: String) -> Self {
        let archive = Archive::load_sync(&file_path);
        Self::assemble(file_path, archive, body, today)
    }

    /// Like [`Store::open_sync`] but with an explicit `done.txt` path.
    #[must_use]
    pub fn open_sync_with_done(
        file_path: PathBuf,
        done_path: PathBuf,
        body: String,
        today: String,
    ) -> Self {
        let archive = Archive::load_sync_at(done_path);
        Self::assemble(file_path, archive, body, today)
    }

    fn assemble(file_path: PathBuf, archive: Archive, body: String, today: String) -> Self {
        let tasks = todo::parse_file(&body);
        let mut store = Self {
            tasks,
            history: History::default(),
            archive,
            file_path,
            last_disk: body,
            last_meta: None,
            today,
            active_timer: None,
        };
        store.resync_timer();
        store
    }

    /// Re-derive `active_timer` from the task list. The on-disk `start:` tag
    /// is the single source of truth for a running timer: after a reload,
    /// undo, or any mutation that shifts task indices, the in-memory
    /// `task_abs` may point at the wrong task. This scan re-attaches the
    /// timer to whichever task actually carries `start:` (at most one —
    /// starting a timer stops any other first), or clears it when the tag is
    /// gone. Cheap (a linear scan), so it runs after every index-shifting or
    /// raw-rewriting mutation.
    pub(crate) fn resync_timer(&mut self) {
        self.active_timer = self.tasks.iter().enumerate().find_map(|(i, t)| {
            let token = t.start.as_deref()?;
            let started_at = NaiveDateTime::parse_from_str(token, "%Y-%m-%dT%H:%M:%S")
                // A hand-typed `start:` that isn't a parseable timestamp can't
                // be used for billing; fall back to "now" so the timer still
                // runs and counts from load rather than crashing or stalling.
                .unwrap_or_else(|_| chrono::Local::now().naive_local());
            Some(TimerState {
                task_abs: i,
                started_at,
            })
        });
    }

    #[must_use]
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    #[must_use]
    pub fn archive(&self) -> &Archive {
        &self.archive
    }

    #[must_use]
    pub fn today(&self) -> &str {
        &self.today
    }

    #[must_use]
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Cloned `raw` for the task at `abs`, or `None` if out of range.
    #[must_use]
    pub fn task_raw(&self, abs: usize) -> Option<String> {
        self.tasks.get(abs).map(|t| t.raw.clone())
    }

    /// True when at least one live task is marked done.
    #[must_use]
    pub fn has_completed(&self) -> bool {
        self.tasks.iter().any(|t| t.done)
    }

    /// True when any task (live or archived) carries time logged for the
    /// store's current `today`. This is the signal that time capture has
    /// happened this day — a fresh launch with nothing logged today means the
    /// user may have worked for hours outside the app, so the idle nudge
    /// should fire immediately instead of granting a fresh grace period.
    #[must_use]
    pub fn has_time_logged_today(&self) -> bool {
        let check =
            |t: &Task| t.log.as_deref() == Some(self.today.as_str()) && t.dur.unwrap_or(0) > 0;
        self.tasks.iter().any(check) || self.archive.tasks().iter().any(check)
    }

    /// Update the cached "today". Returns `true` iff the value changed, so the
    /// caller knows to recompute any date-dependent view state.
    pub fn set_today(&mut self, today: String) -> bool {
        if self.today == today {
            return false;
        }
        self.today = today;
        true
    }

    /// True when a timer is currently running.
    #[must_use]
    pub fn timer_running(&self) -> bool {
        self.active_timer.is_some()
    }

    /// Elapsed wall-clock seconds for the live timer display. `None` when no
    /// timer is active. Computed from the durable `start:` timestamp, so it
    /// survives app restarts and counts through system suspend.
    #[must_use]
    pub fn timer_elapsed_secs(&self) -> Option<u64> {
        self.active_timer.as_ref().map(|ts| {
            (chrono::Local::now().naive_local() - ts.started_at)
                .num_seconds()
                .max(0) as u64
        })
    }

    /// Reference to the task the running timer is on, if any.
    #[must_use]
    pub fn active_timer_task(&self) -> Option<&Task> {
        self.active_timer
            .as_ref()
            .and_then(|ts| self.tasks.get(ts.task_abs))
    }

    /// True when a timer is running on the task at absolute index `abs`.
    #[must_use]
    pub fn is_timer_running_on(&self, abs: usize) -> bool {
        self.active_timer
            .as_ref()
            .is_some_and(|ts| ts.task_abs == abs)
    }

    /// Absolute index of the task the active timer is running on, if any.
    #[must_use]
    pub fn active_timer_abs(&self) -> Option<usize> {
        self.active_timer.as_ref().map(|ts| ts.task_abs)
    }
}
