//! Headless core: the durable task store, its persistence/I-O, and all task
//! mutations. Carries no view, input, or presentation state — operations return
//! structured [`outcome`] values rather than user-facing strings. Both the TUI
//! (`App` wraps a `Store`) and the CLI (`cmd`) drive this type.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::todo::{self, Task};

mod archive;
mod external;
mod history;
mod mutations;

pub mod filter;
pub mod outcome;

#[cfg(test)]
pub(crate) mod test_support;

pub use archive::Archive;
pub use history::History;
pub use outcome::{
    AddOutcome, ArchiveDeleteOutcome, ArchiveOutcome, BulkCompleteOutcome, BulkDeleteOutcome,
    CompleteOutcome, DeleteOutcome, DrainReport, EditOutcome, PriorityOutcome, Reconcile,
    StoreError, TagOutcome, TimerOutcome, TimerQuitOutcome, UnarchiveOutcome, UndoOutcome,
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
    pub(crate) today: String,
    /// State of the currently running timer, if any. `None` means no timer
    /// is active. The `Instant` is a wall-clock reference for live elapsed
    /// display; on-disk state is in the task's `start:` tag.
    pub(crate) active_timer: Option<TimerState>,
}

/// Wall-clock state for the running timer. The on-disk truth is the task's
/// `start:` tag; this `Instant` exists so the UI can show live elapsed
/// seconds without re-reading the file every frame.
#[derive(Debug, Clone)]
pub struct TimerState {
    pub task_abs: usize,
    pub started_at: Instant,
}

impl Store {
    /// Construct a store, loading the archive (`done.txt`) off-thread from the
    /// sibling of `file_path`. Used by the TUI so the first frame doesn't wait
    /// on the archive read.
    pub fn new(file_path: PathBuf, body: String, today: String) -> Self {
        let archive = Archive::spawn(&file_path);
        Self::assemble(file_path, archive, body, today)
    }

    /// Like [`Store::new`] but with an explicit `done.txt` path (e.g. from a
    /// `DONE_FILE` env var that isn't a sibling of the todo file).
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
    pub fn open_sync(file_path: PathBuf, body: String, today: String) -> Self {
        let archive = Archive::load_sync(&file_path);
        Self::assemble(file_path, archive, body, today)
    }

    /// Like [`Store::open_sync`] but with an explicit `done.txt` path.
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
        let active_timer = tasks.iter().enumerate().find_map(|(i, t)| {
            t.start.as_ref().map(|_| TimerState {
                task_abs: i,
                started_at: Instant::now(),
            })
        });
        Self {
            tasks,
            history: History::default(),
            archive,
            file_path,
            last_disk: body,
            today,
            active_timer,
        }
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn archive(&self) -> &Archive {
        &self.archive
    }

    pub fn today(&self) -> &str {
        &self.today
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// Cloned `raw` for the task at `abs`, or `None` if out of range.
    pub fn task_raw(&self, abs: usize) -> Option<String> {
        self.tasks.get(abs).map(|t| t.raw.clone())
    }

    /// True when at least one live task is marked done.
    pub fn has_completed(&self) -> bool {
        self.tasks.iter().any(|t| t.done)
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
    pub fn timer_running(&self) -> bool {
        self.active_timer.is_some()
    }

    /// Elapsed wall-clock seconds for the live timer display. `None` when no
    /// timer is active.
    pub fn timer_elapsed_secs(&self) -> Option<u64> {
        self.active_timer
            .as_ref()
            .map(|ts| ts.started_at.elapsed().as_secs())
    }

    /// Reference to the task the running timer is on, if any.
    pub fn active_timer_task(&self) -> Option<&Task> {
        self.active_timer
            .as_ref()
            .and_then(|ts| self.tasks.get(ts.task_abs))
    }

    /// True when a timer is running on the task at absolute index `abs`.
    pub fn is_timer_running_on(&self, abs: usize) -> bool {
        self.active_timer
            .as_ref()
            .is_some_and(|ts| ts.task_abs == abs)
    }
}
