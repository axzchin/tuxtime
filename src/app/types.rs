use std::fmt;
use std::ops::{Index, IndexMut};
use std::str::FromStr;
use std::time::Duration;

pub const LEADER_WINDOW: Duration = Duration::from_millis(600);
pub const FLASH_TTL: Duration = Duration::from_millis(1500);
pub const UNDO_LIMIT: usize = 50;
pub const AUTOCOMPLETE_CAP: usize = 8;

/// Outcome of `add_from_draft`. The Enter handler in `main.rs` uses this to
/// decide whether to exit Insert mode: `Parsed` means the NL pre-pass
/// rewrote the buffer but did not save, so the user should stay in Insert
/// to review/edit before pressing Enter a second time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    Saved,
    Parsed,
    Empty,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Search,
    Visual,
    Help,
    Settings,
    PromptProject,    // text input → add project on current task
    PromptContext,    // text input → add/remove context on current task
    PickProject,      // j/k cycles through projects to filter by
    PickContext,      // j/k cycles through contexts to filter by
    PickSavedFilter,  // j/k cycles through saved searches to apply
    PromptSaveFilter, // text input → name the current search and save it
    /// Prompt to add time (in minutes/hours) to the current task's dur.
    PromptAddTime,
    /// Prompt to set the idle nudge threshold (minutes).
    PromptIdleNudge,
    /// Prompt to set the long-timer nudge threshold (minutes).
    PromptLongTimerNudge,
    /// Day-boundary prompt — starting a timer (or adding time) on a task
    /// whose accumulated time belongs to a previous day. `[c]ontinue same
    /// entry / [n]ew entry for today / [esc] cancel`.
    PromptDayBoundary,
    /// Calendar picker → jump the timesheet to a selected date.
    PickTimesheetDate,
    CommandPalette,
    /// QR + URL overlay for the in-TUI capture server. Any key
    /// dismisses; press `s` again to re-open without rebinding (the
    /// server stays running once started).
    Share,
    /// Theme picker dialog — j/k to preview themes, Enter to accept,
    /// Esc to revert.
    PickTheme,
    /// First-run welcome prompt, shown when `tuxtime` is launched with no
    /// target and no `./todo.txt` exists. `c` creates `./todo.txt`, `s`
    /// opens the bundled sample, `q`/`Esc` quits without creating anything.
    Welcome,
    /// Idle nudge popup — shown when no timer has been running for the configured duration.
    IdleNudge,
    /// Long-timer nudge popup — shown when a timer has been running past the
    /// configured threshold (from Normal mode only, so it never destroys
    /// in-progress composition). `[S]top timer / [D]ismiss`.
    LongTimerNudge,
    /// Stale-timer startup prompt — a timer was left running when the app
    /// last closed (or was killed) and has since exceeded the long-timer
    /// threshold. `[k]eep counting / [s]top & log / [d]iscard gap`, so a
    /// zombie session (e.g. closed terminal overnight) never silently bills
    /// the away time. Shown once at launch, before any long-timer nudge.
    StaleTimer,
    /// Manual entry choice popup — [C]urrent task description or [N]ew blank entry.
    ManualEntryChoice,
    /// Project management view (`<P>`) — archive/unarchive/rename projects.
    ManageProjects,
    /// Prompt to rename a project (triggered from the project management view).
    PromptRenameProject,
}

/// Top-level views. The explicit discriminants are the canonical slot
/// indices used by [`ViewMap`]; keep them contiguous and in sync with
/// [`View::COUNT`] (enforced by a test).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum View {
    List = 0,
    Archive = 1,
    Timesheet = 2,
}

impl View {
    /// Number of view variants. Must equal the highest discriminant + 1.
    pub const COUNT: usize = 3;
}

/// A fixed-size map from [`View`] to a value. Replaces the old pattern of
/// parallel `[T; 3]` arrays keyed by a hand-rolled index function, where the
/// index mapping could silently desync from the enum. Indexing by `View`
/// directly makes the mapping explicit and greppable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewMap<T>([T; View::COUNT]);

impl<T> ViewMap<T> {
    #[must_use]
    pub const fn new(values: [T; View::COUNT]) -> Self {
        Self(values)
    }
}

impl<T: Default> Default for ViewMap<T> {
    fn default() -> Self {
        Self(std::array::from_fn(|_| T::default()))
    }
}

impl<T> Index<View> for ViewMap<T> {
    type Output = T;

    fn index(&self, view: View) -> &T {
        &self.0[view as usize]
    }
}

impl<T> IndexMut<View> for ViewMap<T> {
    fn index_mut(&mut self, view: View) -> &mut T {
        &mut self.0[view as usize]
    }
}

#[cfg(test)]
mod view_map_tests {
    use super::*;

    #[test]
    fn view_map_indexes_by_view() {
        let mut m = ViewMap::new([10usize, 20, 30]);
        assert_eq!(m[View::List], 10);
        assert_eq!(m[View::Archive], 20);
        assert_eq!(m[View::Timesheet], 30);
        m[View::Timesheet] = 99;
        assert_eq!(m[View::Timesheet], 99);
    }

    #[test]
    fn view_discriminants_match_map_slots() {
        // The ViewMap slots are the enum discriminants. If a variant is ever
        // reordered or a new one added, COUNT and this mapping must move
        // together.
        assert_eq!(View::List as usize, 0);
        assert_eq!(View::Archive as usize, 1);
        assert_eq!(View::Timesheet as usize, 2);
        assert_eq!(View::COUNT, 3);
        assert!((View::List as usize) < View::COUNT);
        assert!((View::Archive as usize) < View::COUNT);
        assert!((View::Timesheet as usize) < View::COUNT);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Priority,
    Due,
    File,
}

impl Sort {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Sort::Priority => "priority",
            Sort::Due => "due",
            Sort::File => "file",
        }
    }
}

impl fmt::Display for Sort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Sort {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "priority" => Ok(Sort::Priority),
            "due" => Ok(Sort::Due),
            "file" => Ok(Sort::File),
            _ => Err(()),
        }
    }
}

/// Timesheet sort order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimesheetSort {
    /// Group by project+activity (default).
    ProjectActivity,
    /// Group by date, then project+activity within each date.
    Date,
    /// Sort by total duration descending.
    Duration,
}

impl TimesheetSort {
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            TimesheetSort::ProjectActivity => TimesheetSort::Date,
            TimesheetSort::Date => TimesheetSort::Duration,
            TimesheetSort::Duration => TimesheetSort::ProjectActivity,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            TimesheetSort::ProjectActivity => "by project",
            TimesheetSort::Date => "by date",
            TimesheetSort::Duration => "by duration",
        }
    }
}

/// Reference to a task, either active or archived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimesheetTaskRef {
    /// Index into `Store::tasks` (active).
    Active(usize),
    /// Index into `Store::archive().tasks()` (done.txt).
    Archived(usize),
}

/// A single group entry in the timesheet view.
#[derive(Debug, Clone)]
pub struct TimesheetEntry {
    /// ISO date string ("2026-05-07"). The day the time was actually tracked
    /// (`log:` tag) when the line has one, else the task's creation date.
    /// Used for date headers and daily/weekly bucketing.
    pub date: String,
    /// Group key: "+project @activity".
    pub key: String,
    /// Total accumulated seconds for this group.
    pub total_secs: u64,
    /// Narrative body texts (`body_only`).
    pub narratives: Vec<String>,
    /// Task references for each narrative, parallel to `narratives`.
    /// Used by Enter-to-edit and b-toggle (only `Active` refs can be edited).
    pub task_indices: Vec<TimesheetTaskRef>,
    /// Whether this group's entries are billable (true) or non-billable (false).
    pub billable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Density {
    Compact,
    Comfortable,
    Cozy,
}

impl Density {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Density::Compact => "compact",
            Density::Comfortable => "comfortable",
            Density::Cozy => "cozy",
        }
    }
}

impl fmt::Display for Density {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Density {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "compact" => Ok(Density::Compact),
            "comfortable" => Ok(Density::Comfortable),
            "cozy" => Ok(Density::Cozy),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub project: Option<String>,
    pub context: Option<String>,
    pub search: String,
}

impl Filter {
    /// True when at least one of project / context / search is non-empty.
    #[must_use]
    pub fn has_any(&self) -> bool {
        self.project.is_some() || self.context.is_some() || !self.search.is_empty()
    }

    /// Drop every filter component back to its empty state.
    pub fn clear(&mut self) {
        self.project = None;
        self.context = None;
        self.search.clear();
    }
}

/// A user-named saved search. `query` is a `/`-search needle (case-insensitive
/// subsequence match on the task body), recalled via the `ff` picker and
/// persisted as a `filter.<name> = <query>` line in the config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedFilter {
    pub name: String,
    pub query: String,
}

/// Sort order for the project management view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectSort {
    /// Alphabetical by project name.
    Name,
    /// Archived last (unarchived first, then archived).
    StatusArchivedLast,
    /// Archived first (archived first, then unarchived).
    StatusArchivedFirst,
}

impl ProjectSort {
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            ProjectSort::Name => ProjectSort::StatusArchivedLast,
            ProjectSort::StatusArchivedLast => ProjectSort::StatusArchivedFirst,
            ProjectSort::StatusArchivedFirst => ProjectSort::Name,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            ProjectSort::Name => "by name",
            ProjectSort::StatusArchivedLast => "by status",
            ProjectSort::StatusArchivedFirst => "by status (archived first)",
        }
    }
}
