use std::fmt;
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
    /// Manual entry choice popup — [C]urrent task description or [N]ew blank entry.
    ManualEntryChoice,
    /// Project management view (`<P>`) — archive/unarchive/rename projects.
    ManageProjects,
    /// Prompt to rename a project (triggered from the project management view).
    PromptRenameProject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Archive,
    Timesheet,
}

impl View {
    /// Stable slot index for keying per-view state arrays. Don't reorder the
    /// `View` variants without updating this together.
    #[must_use]
    pub fn idx(self) -> usize {
        match self {
            View::List => 0,
            View::Archive => 1,
            View::Timesheet => 2,
        }
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
    /// ISO date string ("2026-05-07"). Used for date headers in weekly view.
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
