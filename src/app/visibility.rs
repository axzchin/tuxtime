use super::App;
use super::WeekStart;
use super::types::{Filter, Sort, View};
use crate::core::filter::{self, ListDueBucket};

/// One entry per visible row, parallel to `VisibleList::cache`. Renderers
/// detect group transitions by comparing successive entries; under
/// `Sort::File` every row is `None` so the renderer skips headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupKey {
    None,
    ArchiveDate(String),
    /// `Some('A'..='Z')` for a graded priority, `None` for unprioritized.
    ListPriority(Option<char>),
    ListDue(ListDueBucket),
}

/// The filtered + sorted visible index list and its parallel group keys.
/// Owns the invariant that both vectors always have the same length: every
/// write goes through `rebuild_list`/`rebuild_archive`, which produce the
/// groups in the same pass as the indices. This bundle used to be two naked
/// parallel `Vec`s on `App` (the sibling of the per-view cursor map, which
/// lives in `Navigation`).
#[derive(Debug, Default, Clone)]
pub(crate) struct VisibleList {
    cache: Vec<usize>,
    groups: Vec<GroupKey>,
}

impl VisibleList {
    pub(crate) fn indices(&self) -> &[usize] {
        &self.cache
    }

    pub(crate) fn groups(&self) -> &[GroupKey] {
        &self.groups
    }

    /// The absolute task index under `cursor`, or `None` when the list is
    /// empty.
    pub(crate) fn cur(&self, cursor: usize) -> Option<usize> {
        self.cache.get(cursor).copied()
    }

    /// Clamp `cursor` into `[0, len)`; an empty list pins it at 0.
    pub(crate) fn clamp(&mut self, cursor: &mut usize) {
        let len = self.cache.len();
        if len == 0 {
            *cursor = 0;
        } else if *cursor >= len {
            *cursor = len - 1;
        }
    }

    /// Move `cursor` to wherever `abs` lives in the current list. Falls back
    /// to clamping if `abs` was filtered out.
    pub(crate) fn follow(&mut self, abs: usize, cursor: &mut usize) {
        if let Some(pos) = self.cache.iter().position(|&i| i == abs) {
            *cursor = pos;
        } else {
            self.clamp(cursor);
        }
    }

    /// Compute a fresh List-view list under the active filter + sort. Reads
    /// the task slice through `tasks` so callers can pass `store.tasks()`
    /// without borrowing `App` twice.
    pub(crate) fn rebuild_list(
        tasks: &[crate::todo::Task],
        filter: &Filter,
        show_done: bool,
        show_future: bool,
        today: &str,
        sort: Sort,
        week_start: &WeekStart,
    ) -> Self {
        let needle = (!filter.search.is_empty()).then_some(filter.search.as_str());

        let mut idxs: Vec<usize> = (0..tasks.len())
            .filter(|&i| {
                filter::list_predicate(&tasks[i], show_done, show_future, today, filter, needle)
            })
            .collect();

        filter::sort_by_prefs(&mut idxs, tasks, sort);

        let groups: Vec<GroupKey> = match sort {
            Sort::File => vec![GroupKey::None; idxs.len()],
            Sort::Priority => idxs
                .iter()
                .map(|&i| GroupKey::ListPriority(tasks[i].priority))
                .collect(),
            Sort::Due => idxs
                .iter()
                .map(|&i| GroupKey::ListDue(filter::due_bucket(&tasks[i], today, week_start)))
                .collect(),
        };
        Self {
            cache: idxs,
            groups,
        }
    }

    /// Compute a fresh Archive-view list, sorted by done-date descending.
    pub(crate) fn rebuild_archive(archive: &[crate::todo::Task]) -> Self {
        let mut idxs: Vec<usize> = (0..archive.len()).collect();
        idxs.sort_by(|&a, &b| {
            archive[b]
                .done_date
                .as_deref()
                .unwrap_or("")
                .cmp(archive[a].done_date.as_deref().unwrap_or(""))
        });
        let groups: Vec<GroupKey> = idxs
            .iter()
            .map(|&i| {
                let date = archive[i]
                    .done_date
                    .clone()
                    .unwrap_or_else(|| "unknown".into());
                GroupKey::ArchiveDate(date)
            })
            .collect();
        Self {
            cache: idxs,
            groups,
        }
    }
}

impl App {
    /// Indices into the active view's task source after filter + sort, in
    /// display order. The source is `archive().tasks()` in Archive view,
    /// `tasks()` otherwise. Reads the cache populated by `recompute_visible`.
    pub fn visible_indices(&self) -> &[usize] {
        self.list.indices()
    }

    /// Group key per row, parallel to `visible_indices()`.
    pub fn visible_groups(&self) -> &[GroupKey] {
        self.list.groups()
    }

    /// Recompute the cached visible-index list and parallel group keys. Call
    /// after any mutation that affects filter/sort/view/tasks/archive.
    /// Also invalidates the timesheet groups cache so the next timesheet
    /// render forces a fresh computation — no more stale views after
    /// archive/unarchive.
    pub fn recompute_visible(&mut self) {
        match self.nav.view {
            View::List => {
                let tasks = self.store.tasks();
                let today = self.store.today();
                let filter = &self.filter;
                self.list = VisibleList::rebuild_list(
                    tasks,
                    filter,
                    self.prefs.show_done,
                    self.prefs.show_future,
                    today,
                    self.prefs.sort,
                    &self.env.week_start,
                );
            }
            View::Archive => {
                let archive = self.store.archive().tasks();
                self.list = VisibleList::rebuild_archive(archive);
            }
            View::Timesheet => {}
        }
        self.timesheet.invalidate_cache();
    }

    pub fn cur_abs(&self) -> Option<usize> {
        self.list.cur(self.nav.cursor)
    }

    pub fn clamp_cursor(&mut self) {
        self.list.clamp(&mut self.nav.cursor);
    }

    /// Move the cursor to wherever `abs` lives in the current visible list.
    /// Falls back to clamping if `abs` was filtered out.
    pub(super) fn follow_cursor(&mut self, abs: usize) {
        self.list.follow(abs, &mut self.nav.cursor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::build_app;
    use crate::core::filter::ListDueBucket;

    #[test]
    fn search_matches_subsequence() {
        let mut app = build_app("2026-05-01 Call dentist\n2026-05-01 buy milk\n");
        app.filter.search = "cade".into();
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 1);
    }

    #[test]
    fn search_matches_body_not_dates() {
        let mut app = build_app("2026-05-01 buy milk\n2026-04-01 something else\n");
        app.filter.search = "2026".into();
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 0);
    }

    #[test]
    fn visible_cache_updates_after_mutation() {
        let mut app = build_app("a\nb\nc\n");
        assert_eq!(app.visible_indices().len(), 3);
        app.draft_set("d".into());
        app.add_from_draft();
        assert_eq!(app.visible_indices().len(), 4);
    }

    #[test]
    fn list_cursor_survives_archive_roundtrip() {
        let mut app = build_app("a\nb\nc\nd\ne\n");
        app.nav.cursor = 3;
        app.set_view(View::Archive);
        app.set_view(View::List);
        assert_eq!(app.nav.cursor, 3, "cursor lost on List → Archive → List");
    }

    #[test]
    fn archive_indices_point_into_archive_tasks() {
        let mut app = build_app("a\n");
        let path = app.archive().path().to_path_buf();
        app.store.archive = crate::app::Archive::for_test(
            crate::todo::parse_file(
                "x 2026-05-01 2026-04-01 first\nx 2026-05-02 2026-04-02 second\n",
            ),
            String::new(),
            path,
        );
        app.set_view(View::Archive);
        let idxs = app.visible_indices();
        assert_eq!(idxs.len(), 2);
        for &i in idxs {
            assert!(app.archive().tasks().get(i).is_some());
        }
    }

    /// The invariant the bundle exists to own: indices and group keys are
    /// always the same length, for both views.
    #[test]
    fn indices_and_groups_stay_parallel() {
        let mut app = build_app("(A) a\n(B) b\nc\n");
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), app.visible_groups().len());

        app.set_view(View::Archive);
        assert_eq!(app.visible_indices().len(), app.visible_groups().len());

        app.set_view(View::List);
        app.prefs.sort = Sort::Due;
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), app.visible_groups().len());
    }

    #[test]
    fn list_groups_are_none_under_sort_file() {
        let mut app = build_app("(A) a\n(B) b\nc\n");
        app.prefs.sort = Sort::File;
        app.recompute_visible();
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 3);
        for g in groups {
            assert!(matches!(g, GroupKey::None));
        }
    }

    #[test]
    fn list_groups_track_priority_under_sort_priority() {
        let mut app = build_app("(A) a\n(B) b\nc\n(A) a2\n");
        app.prefs.sort = Sort::Priority;
        app.recompute_visible();
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[0], GroupKey::ListPriority(Some('A')));
        assert_eq!(groups[1], GroupKey::ListPriority(Some('A')));
        assert_eq!(groups[2], GroupKey::ListPriority(Some('B')));
        assert_eq!(groups[3], GroupKey::ListPriority(None));
    }

    #[test]
    fn list_groups_bucket_due_dates_under_sort_due() {
        let raw = "a due:2026-05-04\n\
                   b due:2026-05-06\n\
                   c due:2026-05-08\n\
                   d due:2026-05-15\n\
                   e due:2026-05-25\n\
                   f\n";
        let mut app = build_app(raw);
        app.prefs.sort = Sort::Due;
        app.recompute_visible();
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 6);
        assert_eq!(groups[0], GroupKey::ListDue(ListDueBucket::Overdue));
        assert_eq!(groups[1], GroupKey::ListDue(ListDueBucket::Today));
        assert_eq!(groups[2], GroupKey::ListDue(ListDueBucket::ThisWeek));
        assert_eq!(groups[3], GroupKey::ListDue(ListDueBucket::NextWeek));
        assert_eq!(groups[4], GroupKey::ListDue(ListDueBucket::Later));
        assert_eq!(groups[5], GroupKey::ListDue(ListDueBucket::NoDue));
    }

    #[test]
    fn future_absolute_threshold_hidden_by_default() {
        let mut app = build_app("future task t:2030-01-01\nvisible task\n");
        assert_eq!(app.visible_indices().len(), 1);
        assert_eq!(app.tasks()[app.visible_indices()[0]].raw, "visible task");
        app.prefs.show_future = true;
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 2);
    }

    #[test]
    fn relative_threshold_anchors_on_due() {
        let mut app = build_app("Pay rent due:2026-05-15 t:-3d\n");
        assert_eq!(app.visible_indices().len(), 0);
        app.prefs.show_future = true;
        app.recompute_visible();
        assert_eq!(app.visible_indices().len(), 1);
    }

    #[test]
    fn refresh_today_unhides_tasks_when_date_advances() {
        let mut app = build_app("future task t:2026-05-07\nvisible task\n");
        assert_eq!(app.visible_indices().len(), 1);
        let changed = app.refresh_today("2026-05-07".into());
        assert!(changed);
        assert_eq!(app.today(), "2026-05-07");
        assert_eq!(app.visible_indices().len(), 2);
    }

    #[test]
    fn refresh_today_is_noop_when_date_unchanged() {
        let mut app = build_app("a\n");
        let changed = app.refresh_today("2026-05-06".into());
        assert!(!changed);
        assert_eq!(app.today(), "2026-05-06");
    }

    #[test]
    fn archive_visible_groups_are_done_date_desc() {
        let mut app = build_app("a\n");
        let path = app.archive().path().to_path_buf();
        app.store.archive = crate::app::Archive::for_test(
            crate::todo::parse_file(
                "x 2026-04-01 2026-03-01 older\nx 2026-05-02 2026-04-02 newer\n",
            ),
            String::new(),
            path,
        );
        app.set_view(View::Archive);
        let groups = app.visible_groups();
        assert_eq!(groups.len(), 2);
        let first = match &groups[0] {
            GroupKey::ArchiveDate(d) => d.as_str(),
            _ => panic!("expected ArchiveDate"),
        };
        let second = match &groups[1] {
            GroupKey::ArchiveDate(d) => d.as_str(),
            _ => panic!("expected ArchiveDate"),
        };
        assert_eq!(first, "2026-05-02");
        assert_eq!(second, "2026-04-01");
    }
}
