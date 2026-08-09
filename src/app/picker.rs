use super::App;
use super::types::{Mode, TimesheetTaskRef, View};
use crate::core::filter::unique_values;

impl App {
    /// Enter project-picker mode. Seeds the filter from the cursor task's
    /// first project (falling back to the current filter, then alphabetical
    /// first). Archived projects are excluded from the picker.
    pub fn enter_pick_project(&mut self) {
        let all: Vec<String> = unique_values(self.store.tasks(), |t| &t.projects)
            .into_iter()
            .filter(|p| !self.is_project_archived(p))
            .collect();
        if all.is_empty() {
            self.flash("no projects");
            return;
        }
        let seed = self
            .picker_seed_task()
            .and_then(|t| t.projects.first().cloned())
            .filter(|p| all.contains(p) && !self.is_project_archived(p))
            .or_else(|| self.filter.project.clone())
            .filter(|p| all.contains(p))
            .unwrap_or_else(|| all[0].clone());
        self.filter.project = Some(seed);
        self.nav.cursor = 0;
        self.nav.mode = Mode::PickProject;
        self.recompute_visible();
        self.flash_pick_project();
    }

    pub fn enter_pick_context(&mut self) {
        let all = unique_values(self.store.tasks(), |t| &t.contexts);
        if all.is_empty() {
            self.flash("no contexts");
            return;
        }
        let seed = self
            .picker_seed_task()
            .and_then(|t| t.contexts.first().cloned())
            .filter(|c| all.contains(c))
            .or_else(|| self.filter.context.clone())
            .filter(|c| all.contains(c))
            .unwrap_or_else(|| all[0].clone());
        self.filter.context = Some(seed);
        self.nav.cursor = 0;
        self.nav.mode = Mode::PickContext;
        self.recompute_visible();
        self.flash_pick_context();
    }

    /// The task whose tags seed the filter pickers. In timesheet view this
    /// is the entry under the *timesheet* cursor (so `f`/`F` filter the
    /// matter you're actually looking at), not the stale list cursor; in
    /// every other view it's the list cursor's task.
    fn picker_seed_task(&self) -> Option<&crate::todo::Task> {
        if matches!(self.nav.view, View::Timesheet) {
            return match self.timesheet_narrative_at(self.timesheet.cursor) {
                Some((_, _, TimesheetTaskRef::Active(abs))) => self.store.tasks().get(abs),
                _ => None,
            };
        }
        self.cur_abs().and_then(|i| self.store.tasks().get(i))
    }

    /// Enter saved-filter picker mode. Seeds to the saved filter whose query
    /// equals the active search (so reopening highlights the current one),
    /// else the first. Stashes the pre-picker search so `pick_cancel` can
    /// revert. Inside the picker, j/k cycle through saved searches.
    pub fn enter_pick_saved(&mut self) {
        if self.saved.list.is_empty() {
            self.flash("no saved filters");
            return;
        }
        self.saved.picker.restore = Some(self.filter.search.clone());
        // Highlight the saved filter matching the active search, else the
        // first; track the index so duplicate queries stay distinguishable.
        self.saved.picker.idx = self
            .saved
            .list
            .iter()
            .position(|f| f.query == self.filter.search)
            .unwrap_or(0);
        self.filter.search = self.saved.list[self.saved.picker.idx].query.clone();
        self.nav.cursor = 0;
        self.nav.mode = Mode::PickSavedFilter;
        self.recompute_visible();
        self.flash_pick_saved();
    }

    /// Commit an open picker (Enter): keep the previewed filter, return to
    /// Normal. For the saved-filter picker this also drops the revert
    /// snapshot so a later cancel elsewhere can't resurrect it. A picker
    /// opened from the nudge selection (`f p` / `f c` / `f f` while choosing
    /// a task) returns to the selection — landing in Normal would let the
    /// idle nudge re-fire on the next tick (still idle, no timer) before
    /// the user can pick a task.
    pub fn pick_accept(&mut self) {
        if self.nav.mode == Mode::PickSavedFilter {
            self.saved.picker.restore = None;
        }
        if self.session.nudge_picker.is_some() {
            self.nav.mode = Mode::PickNudgeTask;
        } else {
            self.nav.mode = Mode::Normal;
        }
    }

    /// Cancel an open picker. Clears only the filter that was being picked
    /// (so escaping the context picker doesn't drop a project filter that
    /// the user set independently). The saved-filter picker restores the
    /// search that was active before it opened.
    pub fn pick_cancel(&mut self) {
        match self.nav.mode {
            Mode::PickProject => self.filter.project = None,
            Mode::PickContext => self.filter.context = None,
            Mode::PickSavedFilter => {
                self.filter.search = self.saved.picker.restore.take().unwrap_or_default();
            }
            _ => {}
        }
        self.nav.cursor = 0;
        if self.session.nudge_picker.is_some() {
            self.nav.mode = Mode::PickNudgeTask;
        } else {
            self.nav.mode = Mode::Normal;
        }
        self.recompute_visible();
    }

    /// Step through projects/contexts within picker mode.
    pub fn pick_step(&mut self, forward: bool) {
        match self.nav.mode {
            Mode::PickProject => {
                let all: Vec<String> = unique_values(self.store.tasks(), |t| &t.projects)
                    .into_iter()
                    .filter(|p| !self.is_project_archived(p))
                    .collect();
                if all.is_empty() {
                    return;
                }
                self.filter.project = Some(step(&all, self.filter.project.as_deref(), forward));
                self.nav.cursor = 0;
                self.recompute_visible();
                self.flash_pick_project();
            }
            Mode::PickContext => {
                let all = unique_values(self.store.tasks(), |t| &t.contexts);
                if all.is_empty() {
                    return;
                }
                self.filter.context = Some(step(&all, self.filter.context.as_deref(), forward));
                self.nav.cursor = 0;
                self.recompute_visible();
                self.flash_pick_context();
            }
            Mode::PickSavedFilter => {
                let len = self.saved.list.len();
                if len == 0 {
                    return;
                }
                self.saved.picker.idx = if forward {
                    (self.saved.picker.idx + 1) % len
                } else {
                    (self.saved.picker.idx + len - 1) % len
                };
                self.filter.search = self.saved.list[self.saved.picker.idx].query.clone();
                self.nav.cursor = 0;
                self.recompute_visible();
                self.flash_pick_saved();
            }
            _ => {}
        }
    }

    fn flash_pick_project(&mut self) {
        let all: Vec<String> = unique_values(self.store.tasks(), |t| &t.projects)
            .into_iter()
            .filter(|p| !self.is_project_archived(p))
            .collect();
        if let Some(cur) = self.filter.project.clone() {
            let pos = position_of(&all, &cur);
            self.flash(format!("+{}  ({}/{})", cur, pos + 1, all.len()));
        }
    }

    fn flash_pick_context(&mut self) {
        let all = unique_values(self.store.tasks(), |t| &t.contexts);
        if let Some(cur) = self.filter.context.clone() {
            let pos = position_of(&all, &cur);
            self.flash(format!("@{}  ({}/{})", cur, pos + 1, all.len()));
        }
    }

    fn flash_pick_saved(&mut self) {
        let len = self.saved.list.len();
        if let Some(f) = self.saved.list.get(self.saved.picker.idx) {
            let name = f.name.clone();
            self.flash(format!("{}  ({}/{})", name, self.saved.picker.idx + 1, len));
        }
    }
}

/// Wrap-around step through `all` in the requested direction.
fn step(all: &[String], current: Option<&str>, forward: bool) -> String {
    debug_assert!(!all.is_empty());
    let len = all.len();
    let cur_idx = current.and_then(|c| all.iter().position(|x| x == c));
    let next = match cur_idx {
        None => 0,
        Some(i) if forward => (i + 1) % len,
        Some(i) => (i + len - 1) % len,
    };
    all[next].clone()
}

fn position_of(all: &[String], needle: &str) -> usize {
    all.iter().position(|x| x == needle).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::build_app;

    #[test]
    fn step_forward_wraps() {
        let all = vec![
            "finance".to_string(),
            "health".to_string(),
            "work".to_string(),
        ];
        assert_eq!(step(&all, None, true), "finance".to_string());
        assert_eq!(step(&all, Some("finance"), true), "health".to_string());
        assert_eq!(step(&all, Some("health"), true), "work".to_string());
        assert_eq!(step(&all, Some("work"), true), "finance".to_string());
    }

    #[test]
    fn step_backward_wraps() {
        let all = vec![
            "finance".to_string(),
            "health".to_string(),
            "work".to_string(),
        ];
        assert_eq!(step(&all, Some("finance"), false), "work".to_string());
        assert_eq!(step(&all, Some("work"), false), "health".to_string());
        assert_eq!(step(&all, Some("health"), false), "finance".to_string());
    }

    #[test]
    fn pick_cancel_clears_only_relevant_filter() {
        let mut app = build_app(crate::sample::TODO_RAW);
        // Pretend the user already had a project filter set, then opened
        // the context picker. Cancelling the context picker must keep the
        // project filter intact.
        app.filter.project = Some("work".into());
        app.enter_pick_context();
        assert!(app.filter.context.is_some());
        app.pick_cancel();
        assert_eq!(app.filter.project.as_deref(), Some("work"));
        assert!(app.filter.context.is_none());
        assert_eq!(app.nav.mode, Mode::Normal);
    }

    #[test]
    fn pick_saved_seeds_first_steps_and_cancel_reverts() {
        use crate::app::SavedFilter;
        let mut app = build_app(crate::sample::TODO_RAW);
        app.saved.list = vec![
            SavedFilter {
                name: "a".into(),
                query: "alpha".into(),
            },
            SavedFilter {
                name: "b".into(),
                query: "beta".into(),
            },
        ];
        app.set_search("pre".into()); // an unrelated search active beforehand
        app.enter_pick_saved();
        assert_eq!(app.nav.mode, Mode::PickSavedFilter);
        // No saved query equals "pre", so it seeds to the first filter.
        assert_eq!(app.filter().search, "alpha");
        app.pick_step(true);
        assert_eq!(app.filter().search, "beta");
        app.pick_step(true); // wraps
        assert_eq!(app.filter().search, "alpha");
        app.pick_cancel();
        assert_eq!(app.nav.mode, Mode::Normal);
        assert_eq!(app.filter().search, "pre"); // reverted to pre-picker search
    }

    #[test]
    fn pick_saved_seeds_from_active_query_and_commit_keeps() {
        use crate::app::SavedFilter;
        let mut app = build_app(crate::sample::TODO_RAW);
        app.saved.list = vec![
            SavedFilter {
                name: "a".into(),
                query: "alpha".into(),
            },
            SavedFilter {
                name: "b".into(),
                query: "beta".into(),
            },
        ];
        app.set_search("beta".into());
        app.enter_pick_saved();
        // The active search matches filter "b", so it seeds there.
        assert_eq!(app.filter().search, "beta");
        app.pick_step(true); // wraps to "a"
        assert_eq!(app.filter().search, "alpha");
        app.pick_accept();
        assert_eq!(app.nav.mode, Mode::Normal);
        assert_eq!(app.filter().search, "alpha"); // commit keeps the preview
    }

    #[test]
    fn enter_pick_saved_empty_flashes_and_stays_normal() {
        let mut app = build_app(crate::sample::TODO_RAW);
        app.enter_pick_saved();
        assert_eq!(app.nav.mode, Mode::Normal);
        assert_eq!(app.flash_active(), Some("no saved filters"));
    }

    #[test]
    fn pick_saved_steps_by_index_through_duplicate_queries() {
        // Two filters share a query; selection must track the index, not
        // be re-derived from `filter.search` (which can't tell them apart
        // and would strand j/k on the first match).
        use crate::app::SavedFilter;
        let mut app = build_app(crate::sample::TODO_RAW);
        app.saved.list = vec![
            SavedFilter {
                name: "a".into(),
                query: "dup".into(),
            },
            SavedFilter {
                name: "b".into(),
                query: "dup".into(),
            },
            SavedFilter {
                name: "c".into(),
                query: "other".into(),
            },
        ];
        app.enter_pick_saved();
        assert_eq!(app.flash_active(), Some("a  (1/3)"));
        app.pick_step(true);
        assert_eq!(app.flash_active(), Some("b  (2/3)"));
        assert_eq!(app.filter().search, "dup");
        app.pick_step(true);
        assert_eq!(app.flash_active(), Some("c  (3/3)"));
        assert_eq!(app.filter().search, "other");
        app.pick_step(true); // wraps
        assert_eq!(app.flash_active(), Some("a  (1/3)"));
        assert_eq!(app.filter().search, "dup");
        app.pick_step(false); // back-wrap to "c"
        assert_eq!(app.flash_active(), Some("c  (3/3)"));
    }

    #[test]
    fn picker_seeds_from_cursor_task_and_steps() {
        let mut app = build_app(crate::sample::TODO_RAW);
        // Cursor task's first project is "work". Sidebar order is by count
        // desc, name asc: [work(4), health(3), finance(1), home(1),
        // learning(1), personal(1), travel(1)].
        app.enter_pick_project();
        assert!(matches!(app.nav.mode, Mode::PickProject));
        assert_eq!(app.filter.project.as_deref(), Some("work"));
        // Forward: work → health
        app.pick_step(true);
        assert_eq!(app.filter.project.as_deref(), Some("health"));
        // Forward: health → finance
        app.pick_step(true);
        assert_eq!(app.filter.project.as_deref(), Some("finance"));
        // Backward from finance → health
        app.pick_step(false);
        assert_eq!(app.filter.project.as_deref(), Some("health"));
    }

    /// A filter picker opened from the nudge selection (`f p` while choosing
    /// a task) returns to the selection on commit — landing in Normal would
    /// let the idle nudge re-fire (still idle, no timer) before a task can
    /// be picked.
    #[test]
    fn pick_accept_returns_to_selection_when_nudge_picker_open() {
        let mut app = build_app(crate::sample::TODO_RAW);
        app.enter_nudge_picker(crate::app::NudgePickAction::StartTimer);
        app.enter_pick_project();
        assert_eq!(app.nav.mode, Mode::PickProject);

        app.pick_accept();

        assert_eq!(app.nav.mode, Mode::PickNudgeTask);
        assert!(
            app.session.nudge_picker.is_some(),
            "selection must survive the filter commit"
        );
        assert!(
            app.filter().project.is_some(),
            "committed filter must stay applied for the selection"
        );
    }

    /// Escaping a filter picker opened from the selection also returns to
    /// it, with only the picked filter cleared.
    #[test]
    fn pick_cancel_returns_to_selection_when_nudge_picker_open() {
        let mut app = build_app(crate::sample::TODO_RAW);
        app.enter_nudge_picker(crate::app::NudgePickAction::StartTimer);
        app.enter_pick_project();
        assert_eq!(app.nav.mode, Mode::PickProject);

        app.pick_cancel();

        assert_eq!(app.nav.mode, Mode::PickNudgeTask);
        assert!(app.session.nudge_picker.is_some());
        assert!(
            app.filter().project.is_none(),
            "cancel clears only the picked filter"
        );
    }
}
