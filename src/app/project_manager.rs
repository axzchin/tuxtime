//! Project management state: archived projects list, rename in progress,
//! and sort mode for the project management view (`Z`).

use super::types::ProjectSort;

#[derive(Debug)]
pub struct ProjectManager {
    /// Projects the user has archived (hidden from picker and autocomplete).
    /// Persisted to `~/.config/tuxtime/archived-projects.txt`. Bare names,
    /// no `+` prefix.
    pub archived_projects: Vec<String>,
    /// The old project name while in [`super::types::Mode::Prompt(Prompt::RenameProject)`].
    /// Set when entering rename mode; consumed by the rename handler on Enter.
    pub rename_project_old: Option<String>,
    /// Sort mode for the project management view (`Z`). Cycled with `s`.
    pub project_sort: ProjectSort,
}

impl ProjectManager {
    #[must_use]
    pub fn new(archived_projects: Vec<String>) -> Self {
        Self {
            archived_projects,
            rename_project_old: None,
            project_sort: ProjectSort::Name,
        }
    }
}
