//! Environment bundle: which files the app operates on and the working-week
//! preference. Grouping them keeps the `App` struct (and every call site that
//! needs a path or the week anchor) from naming three unrelated fields.

use std::path::PathBuf;

use super::WeekStart;

/// Filesystem + working-week context for the whole app.
///
/// - `file_path` is the todo.txt in use, mirrored from the `Store`'s own
///   copy. App keeps its own so the UI can display it and the first-run
///   welcome prompt can rebind it without reaching into the store; the two
///   are kept in sync by `App::open_file`.
/// - `config_path` is the resolved on-disk config file. Set by the binary
///   after construction so the settings overlay can render a stable, real
///   path without the renderer reaching into the environment itself. `None`
///   in tests/examples that don't care about the value.
/// - `week_start` is the day the timesheet week (and week-scoped due
///   buckets) begins on.
pub struct Env {
    pub file_path: PathBuf,
    pub config_path: Option<PathBuf>,
    pub week_start: WeekStart,
}

impl Env {
    /// Build the environment for a fresh app: the given todo file, no
    /// resolved config path yet (the binary fills it in), and a week start
    /// derived from the loaded config.
    #[must_use]
    pub fn new(file_path: PathBuf, week_start: WeekStart) -> Self {
        Self {
            file_path,
            config_path: None,
            week_start,
        }
    }
}
