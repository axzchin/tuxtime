//! Project lifecycle: archive/unarchive, rename across all tasks,
//! enumeration (active + archived), and sort modes for the project
//! management view (`Z`).
//!
//! Archived projects are persisted to `~/.config/tuxtime/archived-projects.txt`
//! and hidden from the picker and autocomplete.

use super::{App, ProjectSort};
use std::path::PathBuf;

impl App {
    fn archived_projects_path() -> Option<PathBuf> {
        let base = crate::xdg::config_home()?;
        Some(base.join("tuxtime").join("archived-projects.txt"))
    }

    /// Load the archived projects list from disk. Returns an empty list when
    /// the file doesn't exist (first run).
    pub(crate) fn load_archived_projects() -> Vec<String> {
        let Some(path) = Self::archived_projects_path() else {
            return Vec::new();
        };
        match std::fs::read_to_string(&path) {
            Ok(s) => s
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Persist the in-memory archived projects list to disk atomically.
    pub fn save_archived_projects(&mut self) {
        let Some(path) = Self::archived_projects_path() else {
            return;
        };
        let body = self.project_manager.archived_projects.join("\n");
        if body.is_empty() {
            // Remove the file if the list is empty.
            let _ = std::fs::remove_file(&path);
            return;
        }
        let _ = crate::todo::write_atomic(&path, &format!("{body}\n"));
    }

    /// Toggle whether a project is archived. Archived projects are hidden
    /// from the `fp` picker and autocomplete popup.
    pub fn toggle_archive_project(&mut self, name: &str) {
        if let Some(pos) = self
            .project_manager
            .archived_projects
            .iter()
            .position(|p| p == name)
        {
            self.project_manager.archived_projects.remove(pos);
            self.flash(format!("unarchived +{name}"));
        } else {
            self.project_manager
                .archived_projects
                .push(name.to_string());
            self.flash(format!("archived +{name}"));
        }
        self.save_archived_projects();
    }

    /// Rename a project across all tasks. Also updates the archived projects
    /// list if the renamed project was archived.
    pub fn rename_project(&mut self, old: &str, new: &str) {
        use crate::core::outcome::RenameOutcome;
        match self.store.rename_project(old, new) {
            RenameOutcome::Renamed {
                old,
                new,
                active_count,
                archived_count,
            } => {
                // Update the archived-projects list if the renamed project was archived.
                if let Some(pos) = self
                    .project_manager
                    .archived_projects
                    .iter()
                    .position(|p| p == &old)
                {
                    self.project_manager.archived_projects[pos] = new.clone();
                    self.save_archived_projects();
                }
                // Also rebuild autocomplete cache since project names changed.
                self.rebuild_archive_autocomplete_cache();
                let total = active_count + archived_count;
                self.flash(format!("+{old} → +{new} ({total} tasks)"));
                self.recompute_visible();
            }
            RenameOutcome::NoTasks => {
                self.flash(format!("no tasks with project +{old}"));
            }
            RenameOutcome::InvalidName => {
                self.flash(format!("invalid project name: {new}"));
            }
            RenameOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            RenameOutcome::Error(e) => self.flash(format!("rename failed: {e}")),
        }
    }

    /// All unique project names from active + archived tasks, sorted according
    /// to `self.project_manager.project_sort`. Used by the `ManageProjects` view.
    pub fn all_projects(&self) -> Vec<String> {
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for t in self.store.tasks() {
            for p in &t.projects {
                seen.insert(p.clone());
            }
        }
        for t in self.store.archive().tasks() {
            for p in &t.projects {
                seen.insert(p.clone());
            }
        }
        let mut projects: Vec<String> = seen.into_iter().collect();
        match self.project_manager.project_sort {
            ProjectSort::Name => {
                // Already alphabetically sorted by BTreeSet.
            }
            ProjectSort::StatusArchivedLast => {
                projects.sort_by(|a, b| {
                    let a_arch = self.is_project_archived(a);
                    let b_arch = self.is_project_archived(b);
                    a_arch.cmp(&b_arch).then_with(|| a.cmp(b))
                });
            }
            ProjectSort::StatusArchivedFirst => {
                projects.sort_by(|a, b| {
                    let a_arch = self.is_project_archived(a);
                    let b_arch = self.is_project_archived(b);
                    b_arch.cmp(&a_arch).then_with(|| a.cmp(b))
                });
            }
        }
        projects
    }

    /// All projects filtered by the current search needle (case-insensitive
    /// substring match on project name). When the needle is empty, returns all
    /// projects. Used by both the renderer and key handler for `ManageProjects`.
    pub fn filtered_projects(&self) -> Vec<String> {
        let all = self.all_projects();
        let needle = self.filter().search.to_lowercase();
        if needle.is_empty() {
            all
        } else {
            all.into_iter()
                .filter(|n| n.to_lowercase().contains(&needle))
                .collect()
        }
    }

    /// Cycle the project management view sort mode and flash the new label.
    pub fn cycle_project_sort(&mut self) {
        self.project_manager.project_sort = self.project_manager.project_sort.next();
        self.nav.cursor = 0;
        self.flash(format!(
            "sort: {}",
            self.project_manager.project_sort.label()
        ));
    }

    /// Whether a project is archived.
    pub fn is_project_archived(&self, name: &str) -> bool {
        self.project_manager
            .archived_projects
            .iter()
            .any(|p| p == name)
    }
}
