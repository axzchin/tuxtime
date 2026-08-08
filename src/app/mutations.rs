//! App-level mutation wrappers. Each resolves the cursor to an absolute task
//! index, delegates to the headless [`Store`](crate::core::Store), then maps the
//! returned outcome to a flash message and refreshes the visible cache/cursor.
//! All task logic (recurrence, persistence, reconciliation) lives in the store.

use super::App;
use super::types::{AddOutcome, View};
use crate::app::WeekStart;
use crate::core::AddOutcome as CoreAdd;
use crate::core::{
    ArchiveDeleteOutcome, ArchiveOneOutcome, ArchiveOutcome, CarryForwardOutcome, CompleteOutcome,
    DeleteOutcome, EditOutcome, PriorityOutcome, TagOutcome, UnarchiveOutcome, UndoOutcome,
};
use crate::nl;
use crate::note;

impl App {
    pub fn toggle_complete(&mut self, abs: usize) {
        match self.store.toggle_complete(abs) {
            CompleteOutcome::Completed { abs } => {
                self.flash("completed");
                self.after_mutation(abs);
            }
            CompleteOutcome::CompletedSpawned { next, .. } => {
                self.flash("completed +next");
                self.after_mutation(next);
            }
            CompleteOutcome::Uncompleted { abs } => {
                self.flash("uncompleted");
                self.after_mutation(abs);
            }
            CompleteOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            CompleteOutcome::OutOfRange => {}
            CompleteOutcome::Error(e) => self.flash(format!("complete failed: {e}")),
        }
    }

    pub fn cycle_priority(&mut self, abs: usize) {
        match self.store.cycle_priority(abs) {
            PriorityOutcome::Changed { abs, .. } => self.after_mutation(abs),
            PriorityOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            PriorityOutcome::OutOfRange => {}
            PriorityOutcome::Error(e) => self.flash(format!("priority failed: {e}")),
        }
    }

    pub fn delete(&mut self, abs: usize) {
        match self.store.delete(abs) {
            DeleteOutcome::Deleted { .. } => {
                self.flash("deleted");
                self.recompute_visible();
                self.clamp_cursor();
            }
            DeleteOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            DeleteOutcome::OutOfRange => {}
            DeleteOutcome::Error(e) => self.flash(format!("write failed: {e}")),
        }
    }

    pub fn add_from_draft(&mut self) -> AddOutcome {
        let text = self.draft.text().trim().to_string();
        if text.is_empty() {
            return AddOutcome::Empty;
        }

        // Carry-forward save (upgraded `N`): the source task is consumed and
        // this line becomes today's entry. Skipped from the NL pre-pass — the
        // carried narrative is already canonical and must not be reinterpreted.
        if let Some(from) = self.session.carry_forward_from.take() {
            return self.save_carry_forward(from, &text);
        }

        // Natural-language pre-pass. If the buffer reads like prose and the
        // parser extracted anything structured, rewrite the draft to canonical
        // todo.txt and bail before saving — the user's *next* Enter saves the
        // now-canonical form through the store.
        if nl::looks_like_natural_language(&text)
            && let Ok(today) = chrono::NaiveDate::parse_from_str(self.store.today(), "%Y-%m-%d")
            && let Some(parsed) = nl::try_parse(&text, today)
        {
            let canonical = nl::format_as_todo_txt(&parsed);
            if canonical != text {
                let body_was_filled = !parsed.body.trim().is_empty();
                self.draft_set(canonical);
                if body_was_filled {
                    self.flash("parsed natural language; press Enter to save");
                } else {
                    self.flash("parsed; please edit the body, then Enter to save");
                }
                return AddOutcome::Parsed;
            }
        }

        // If entered via manual time entry (`M`), convert `dur:` values from
        // flexible user input (minutes, decimal hours, clock time) to raw seconds.
        let text = if self.session.manual_time_entry {
            self.session.manual_time_entry = false;
            self.convert_dur_in_text(&text)
        } else {
            text
        };

        match self.store.add_finalized(&text) {
            CoreAdd::Added { abs } => {
                if self.session.auto_start_on_save {
                    self.session.auto_start_on_save = false;
                    self.flash("added");
                    self.after_mutation(abs);
                    // Start the timer on the newly-created interruption entry.
                    self.toggle_timer_at(abs);
                    AddOutcome::Saved
                } else {
                    self.flash("added");
                    self.after_mutation(abs);
                    AddOutcome::Saved
                }
            }
            CoreAdd::Empty => AddOutcome::Empty,
            CoreAdd::Aborted(r) => {
                self.handle_reconcile_abort(r);
                AddOutcome::Invalid
            }
            CoreAdd::Error(e) => {
                self.flash(format!("invalid: {e}"));
                AddOutcome::Invalid
            }
        }
    }

    /// Save path for the upgraded `N` carry-forward: completes the source line
    /// and inserts the (user-polished) draft as today's entry. No timer is
    /// started — the user presses `t` when ready.
    fn save_carry_forward(&mut self, from: usize, text: &str) -> AddOutcome {
        match self.store.carry_forward_to(from, text) {
            CarryForwardOutcome::Carried { new, .. } => {
                self.flash("new entry — previous day completed");
                self.after_mutation(new);
                AddOutcome::Saved
            }
            CarryForwardOutcome::Aborted(r) => {
                self.handle_reconcile_abort(r);
                AddOutcome::Invalid
            }
            CarryForwardOutcome::OutOfRange => {
                self.flash("task vanished");
                AddOutcome::Invalid
            }
            // `carry_forward_to` never returns CarriedStarted; the arm exists
            // so the match stays exhaustive if the enum grows a new returner.
            #[allow(unreachable_patterns)]
            CarryForwardOutcome::CarriedStarted { .. } => {
                self.flash("carry failed");
                AddOutcome::Invalid
            }
            CarryForwardOutcome::Error(e) => {
                self.flash(format!("carry failed: {e}"));
                AddOutcome::Invalid
            }
        }
    }

    pub fn save_edit(&mut self) {
        let Some(idx) = self.selection.editing() else {
            return;
        };
        let text = self.draft.text().to_string();
        match self.store.edit_line(idx, &text) {
            EditOutcome::Saved { abs } => {
                self.flash("saved");
                self.after_mutation(abs);
            }
            // Empty draft / vanished index: quiet no-op, as before.
            EditOutcome::Empty | EditOutcome::OutOfRange | EditOutcome::TermNotFound => {}
            EditOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            EditOutcome::Error(e) => self.flash(format!("invalid: {e}")),
        }
    }

    pub fn add_project_to_current(&mut self, name: &str) {
        let Some(abs) = self.cur_abs() else {
            return;
        };
        match self.store.add_project(abs, name) {
            TagOutcome::Added { abs, name } => {
                self.flash(format!("+{name}"));
                self.after_mutation(abs);
            }
            TagOutcome::Removed { .. } | TagOutcome::Unchanged | TagOutcome::OutOfRange => {}
            TagOutcome::InvalidName => self.flash("invalid project name"),
            TagOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            TagOutcome::Error(e) => self.flash(format!("invalid: {e}")),
        }
    }

    pub fn toggle_context_on_current(&mut self, name: &str) {
        let Some(abs) = self.cur_abs() else {
            return;
        };
        match self.store.toggle_context(abs, name) {
            TagOutcome::Added { abs, name } => {
                self.flash(format!("@{name}"));
                self.after_mutation(abs);
            }
            TagOutcome::Removed { abs, name } => {
                self.flash(format!("removed @{name}"));
                self.after_mutation(abs);
            }
            TagOutcome::Unchanged | TagOutcome::OutOfRange => {}
            TagOutcome::InvalidName => self.flash("invalid context name"),
            TagOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            TagOutcome::Error(e) => self.flash(format!("invalid: {e}")),
        }
    }

    pub fn open_note_for_current(&mut self) {
        self.open_note_for_current_with_create(false);
    }

    pub fn create_or_open_note_for_current(&mut self) {
        self.open_note_for_current_with_create(true);
    }

    fn open_note_for_current_with_create(&mut self, create: bool) {
        let Some(task) = self.cur_task().cloned() else {
            return;
        };
        let target = note::target_for_task(&task, &self.share_state.notes_dir);

        if !target.existed_in_task {
            if !create {
                self.flash("no note; press O to create");
                return;
            }
            if matches!(self.view(), View::Archive) {
                self.flash("archived task has no note");
                return;
            }
            let Some(abs) = self.cur_task_index_in_tasks() else {
                return;
            };
            match self.store.append_at(abs, &format!("note:{}", target.rel)) {
                EditOutcome::Saved { abs } => self.after_mutation(abs),
                EditOutcome::Aborted(r) => {
                    self.handle_reconcile_abort(r);
                    return;
                }
                EditOutcome::Error(e) => {
                    self.flash(format!("note link failed: {e}"));
                    return;
                }
                EditOutcome::Empty | EditOutcome::OutOfRange | EditOutcome::TermNotFound => return,
            }
        }

        if target.path.exists() {
            self.queue_editor_path(target.path);
            return;
        }
        if !create {
            self.flash("note missing; press O to create");
            return;
        }
        if let Some(parent) = target.path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            self.flash(format!("note mkdir failed: {e}"));
            return;
        }
        if let Err(e) = std::fs::write(&target.path, note::note_template(&task)) {
            self.flash(format!("note create failed: {e}"));
            return;
        }
        self.queue_editor_path(target.path);
    }

    pub fn undo(&mut self) {
        match self.store.undo() {
            UndoOutcome::Undone => {
                self.flash("undo");
                self.recompute_visible();
                self.clamp_cursor();
            }
            UndoOutcome::Nothing => {}
            UndoOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            UndoOutcome::Error(e) => self.flash(format!("write failed: {e}")),
        }
    }

    pub fn archive_completed(&mut self) {
        match self.store.archive_completed() {
            ArchiveOutcome::Archived { count } => {
                self.flash(format!("archived {count}"));
                self.recompute_visible();
                self.clamp_cursor();
                self.rebuild_archive_autocomplete_cache();
            }
            ArchiveOutcome::Nothing => self.flash("nothing to archive"),
            ArchiveOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            ArchiveOutcome::Error(e) => self.flash(format!("archive failed: {e}")),
        }
    }

    /// Archive a single completed task at `abs` in the live list.
    pub fn archive_one(&mut self, abs: usize) {
        match self.store.archive_one(abs) {
            ArchiveOneOutcome::Archived => {
                self.flash("archived");
                self.recompute_visible();
                self.clamp_cursor();
                self.rebuild_archive_autocomplete_cache();
            }
            ArchiveOneOutcome::NotCompleted => {
                self.flash("complete task first (x)");
            }
            ArchiveOneOutcome::OutOfRange => {}
            ArchiveOneOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            ArchiveOneOutcome::Error(e) => self.flash(format!("archive failed: {e}")),
        }
    }

    /// Move an archived task back into the live list. `archive_idx` indexes
    /// `archive().tasks()` (the cursor source in Archive view).
    pub fn unarchive(&mut self, archive_idx: usize) {
        match self.store.unarchive(archive_idx) {
            UnarchiveOutcome::Unarchived => {
                self.flash("unarchived");
                self.recompute_visible();
                self.clamp_cursor();
                self.rebuild_archive_autocomplete_cache();
            }
            UnarchiveOutcome::OutOfRange => {}
            UnarchiveOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            UnarchiveOutcome::DoneReloaded => {
                self.flash("done.txt changed on disk — reloaded");
                self.recompute_visible();
                self.clamp_cursor();
                self.rebuild_archive_autocomplete_cache();
            }
            UnarchiveOutcome::Error(e) => self.flash(format!("unarchive failed: {e}")),
        }
    }

    /// Permanently remove an archived task from `done.txt`.
    pub fn archive_delete(&mut self, archive_idx: usize) {
        match self.store.archive_delete(archive_idx) {
            ArchiveDeleteOutcome::Deleted => {
                self.flash("deleted from archive");
                self.recompute_visible();
                self.clamp_cursor();
                self.rebuild_archive_autocomplete_cache();
            }
            ArchiveDeleteOutcome::OutOfRange => {}
            ArchiveDeleteOutcome::DoneReloaded => {
                self.flash("done.txt changed on disk — reloaded");
                self.recompute_visible();
                self.clamp_cursor();
                self.rebuild_archive_autocomplete_cache();
            }
            ArchiveDeleteOutcome::Error(e) => self.flash(format!("delete failed: {e}")),
        }
    }

    /// Add a finalized (already canonical todo.txt) task line directly.
    pub fn add_finalized(&mut self, text: &str) {
        match self.store.add_finalized(text) {
            CoreAdd::Added { abs } => {
                self.flash("added");
                self.after_mutation(abs);
            }
            CoreAdd::Empty => {}
            CoreAdd::Aborted(r) => self.handle_reconcile_abort(r),
            CoreAdd::Error(e) => self.flash(format!("invalid: {e}")),
        }
    }

    pub fn toggle_week_start_date(&mut self) {
        self.env.week_start = match self.env.week_start {
            WeekStart::Sunday => WeekStart::Monday,
            WeekStart::Monday => WeekStart::Sunday,
        };
        self.save_prefs();
    }
}

#[cfg(test)]
mod tests {
    use crate::app::{
        WeekStart,
        test_support::{build_app, build_app_with_config, test_path},
    };
    use crate::config::Config;

    fn key(c: char) -> ratatui::crossterm::event::KeyEvent {
        ratatui::crossterm::event::KeyEvent::new(
            ratatui::crossterm::event::KeyCode::Char(c),
            ratatui::crossterm::event::KeyModifiers::NONE,
        )
    }

    #[test]
    fn open_file_rebinds_path_body_and_resets_cursor() {
        let mut app = build_app("old one\nold two\nold three\n");
        app.nav.cursor = 2;
        let new_path = test_path();
        let done = new_path.parent().expect("temp parent").join("done.txt");

        app.open_file(new_path.clone(), done, "fresh task\n".into());

        assert_eq!(
            app.env.file_path, new_path,
            "file_path must point at the new file"
        );
        assert_eq!(app.tasks().len(), 1, "tasks must reflect the new body");
        assert_eq!(app.tasks()[0].raw, "fresh task");
        assert_eq!(
            app.visible_indices().len(),
            1,
            "visible cache must be recomputed"
        );
        assert_eq!(app.nav.cursor, 0, "cursor must reset for the new file");
    }

    #[test]
    fn add_project_rejects_whitespace_in_name() {
        let mut app = build_app("a +health\n");
        app.add_project_to_current("two words");
        assert_eq!(app.tasks()[0].projects, vec!["health"]);
        assert_eq!(app.tasks()[0].raw, "a +health");
        assert_eq!(app.flash_active(), Some("invalid project name"));
    }

    #[test]
    fn add_project_accepts_dashes_underscores_unicode() {
        let mut app = build_app("a\n");
        app.add_project_to_current("life-admin_2026");
        assert_eq!(app.tasks()[0].projects, vec!["life-admin_2026"]);
        app.add_project_to_current("café");
        assert_eq!(app.tasks()[0].projects, vec!["life-admin_2026", "café"]);
    }

    #[test]
    fn toggle_complete_flashes_completed_then_spawned() {
        let mut app = build_app("a\n");
        app.toggle_complete(0);
        assert!(app.tasks()[0].done);
        assert_eq!(app.flash_active(), Some("completed"));

        let mut app = build_app("(A) 2026-04-15 Pay rent due:2026-04-15 rec:+1m\n");
        app.toggle_complete(0);
        assert_eq!(app.tasks().len(), 2);
        assert_eq!(app.flash_active(), Some("completed +next"));
    }

    #[test]
    fn toggle_context_rejects_whitespace_in_name() {
        let mut app = build_app("a @home\n");
        app.toggle_context_on_current("two words");
        assert_eq!(app.tasks()[0].contexts, vec!["home"]);
        assert_eq!(app.tasks()[0].raw, "a @home");
        assert_eq!(app.flash_active(), Some("invalid context name"));
    }

    #[test]
    fn create_or_open_note_appends_link_creates_file_and_queues_editor() {
        let dir = test_path().with_extension("notes");
        let cfg = Config {
            notes_dir: Some(dir.to_string_lossy().into_owned()),
            ..Config::default()
        };
        let mut app = build_app_with_config("Write PR summary +work @desk\n", cfg);

        app.create_or_open_note_for_current();

        let raw = &app.tasks()[0].raw;
        assert!(
            raw.contains("note:projects/tuxtime-tasks/write-pr-summary.md"),
            "task should get stable generated note token: {raw}"
        );
        let expected = dir.join("projects/tuxtime-tasks/write-pr-summary.md");
        assert_eq!(app.take_pending_editor_path(), Some(expected.clone()));
        let body = std::fs::read_to_string(expected).expect("created note exists");
        assert!(body.starts_with("# Write PR summary\n"));
        assert!(body.contains("## My notes\n\n"));
    }

    #[test]
    fn open_note_without_existing_token_does_not_create_or_mutate_task() {
        let dir = test_path().with_extension("notes");
        let cfg = Config {
            notes_dir: Some(dir.to_string_lossy().into_owned()),
            ..Config::default()
        };
        let mut app = build_app_with_config("Write PR summary +work @desk\n", cfg);

        app.open_note_for_current();

        assert_eq!(app.tasks()[0].raw, "Write PR summary +work @desk");
        assert_eq!(app.flash_active(), Some("no note; press O to create"));
        assert!(app.take_pending_editor_path().is_none());
        assert!(!dir.exists());
    }

    #[test]
    fn open_note_with_existing_file_queues_editor_without_rewriting_task() {
        let dir = test_path().with_extension("notes");
        let note = dir.join("projects/example.md");
        std::fs::create_dir_all(note.parent().expect("note parent")).expect("create note parent");
        std::fs::write(&note, "# Existing\n").expect("write existing note");
        let cfg = Config {
            notes_dir: Some(dir.to_string_lossy().into_owned()),
            ..Config::default()
        };
        let raw = "Write PR summary +work note:projects/example.md\n";
        let mut app = build_app_with_config(raw, cfg);

        app.open_note_for_current();

        assert_eq!(app.tasks()[0].raw, raw.trim());
        assert_eq!(app.take_pending_editor_path(), Some(note));
    }

    #[test]
    fn add_from_draft_rewrites_nl_prose_into_canonical_draft() {
        let mut app = build_app("");
        app.draft_set(
            "Pay rent monthly on the first of the month, show the todo 3 days before the due date. \
             It's part of project home and context bank"
                .into(),
        );
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Parsed);
        assert_eq!(app.tasks().len(), 0);
        assert_eq!(
            app.draft.text(),
            "Pay rent +home @bank due:2026-06-01 rec:+1m t:-3d"
        );
        assert_eq!(
            app.flash_active(),
            Some("parsed natural language; press Enter to save")
        );
    }

    #[test]
    fn add_from_draft_second_call_saves_canonical_form() {
        let mut app = build_app("");
        app.draft_set("Buy milk tomorrow".into());
        assert_eq!(app.add_from_draft(), crate::app::AddOutcome::Parsed);
        assert_eq!(app.tasks().len(), 0);
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert_eq!(app.tasks().len(), 1);
        assert!(app.tasks()[0].raw.contains("Buy milk"));
        assert_eq!(app.tasks()[0].due.as_deref(), Some("2026-05-07"));
    }

    #[test]
    fn add_from_draft_plain_words_save_on_first_enter() {
        let mut app = build_app("");
        app.draft_set("Buy milk".into());
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert_eq!(app.tasks().len(), 1);
        assert!(app.tasks()[0].raw.ends_with("Buy milk"));
        assert_eq!(app.flash_active(), Some("added"));
    }

    #[test]
    fn test_toggling_week_start() {
        let mut app = build_app("");
        app.toggle_week_start_date();
        assert_eq!(app.env.week_start, WeekStart::Monday);
        app.toggle_week_start_date();
        assert_eq!(app.env.week_start, WeekStart::Sunday);
    }

    // ── add_time_to_current_from_input ───────────────────────────────

    #[test]
    fn add_time_to_task_with_existing_dur() {
        let mut app = build_app("Draft motion +Smith @drafting dur:3600\n");
        app.nav.cursor = 0; // cursor on the task
        app.recompute_visible();

        app.add_time_to_current_from_input("30"); // add 30 minutes (1800 s)

        let raw = &app.tasks()[0].raw;
        assert!(
            raw.contains("dur:5400"),
            "dur should be 3600+1800=5400, got: {raw}"
        );
        assert!(
            !raw.contains("dur:dur"),
            "must not double the dur: prefix, got: {raw}"
        );
        assert!(
            app.flash_active()
                .is_some_and(|m| m.contains("added 30m") && m.contains("total 1h 30m")),
            "flash should include added and total"
        );
    }

    #[test]
    fn add_time_to_task_without_dur() {
        let mut app = build_app("Review PR +work @dev\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("15"); // add 15 minutes (900 s)

        let raw = &app.tasks()[0].raw;
        assert!(raw.contains("dur:900"), "dur should be 900, got: {raw}");
        assert!(
            app.flash_active().is_some_and(|m| m.contains("added 15m")),
            "flash should confirm addition"
        );
    }

    #[test]
    fn add_time_stamps_log_date() {
        // Manual additions must carry today's log date so they show up on
        // today's timesheet even when the task was created earlier.
        let mut app = build_app("Draft motion +Smith @drafting\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("30");

        let raw = &app.tasks()[0].raw;
        assert!(
            raw.contains("log:2026-05-06"),
            "manual add must stamp today's log date, got: {raw}"
        );
    }

    #[test]
    fn add_time_replaces_existing_log_date_without_duplicating() {
        // Adding time to a task whose time belongs to a previous day now asks
        // first; "continue same entry" moves the log to today exactly once.
        let mut app = build_app("Draft motion +Smith @drafting dur:3600 log:2026-05-01\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("30");
        assert_eq!(app.nav.mode(), crate::app::Mode::PromptDayBoundary);
        crate::interactive::handle_day_boundary(&mut app, key('c'));

        let raw = &app.tasks()[0].raw;
        assert_eq!(
            raw.matches("log:").count(),
            1,
            "must not duplicate log:, got: {raw}"
        );
        assert!(
            raw.contains("log:2026-05-06"),
            "log must move to today, got: {raw}"
        );
    }

    #[test]
    fn add_time_with_invalid_input_flashes_error() {
        let mut app = build_app("Draft motion +Smith @drafting dur:3600\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("not-a-number");

        let raw = &app.tasks()[0].raw;
        assert!(
            raw.contains("dur:3600"),
            "dur should be unchanged, got: {raw}"
        );
        assert!(
            app.flash_active()
                .is_some_and(|m| m.contains("invalid duration")),
            "flash should report invalid"
        );
    }

    #[test]
    fn add_time_with_no_task_selected_flashes_error() {
        let mut app = build_app("Draft motion +Smith @drafting dur:3600\n");
        app.nav.cursor = 999; // no task here
        app.recompute_visible();

        app.add_time_to_current_from_input("30");

        assert!(
            app.flash_active()
                .is_some_and(|m| m.contains("no task selected")),
            "flash should say no task selected"
        );
    }

    #[test]
    fn add_time_decimal_hours_parses_correctly() {
        let mut app = build_app("Meeting notes +work dur:0\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.add_time_to_current_from_input("1.5"); // 1.5 hours = 5400 seconds

        assert!(
            app.tasks()[0].raw.contains("dur:5400"),
            "1.5h should produce 5400s"
        );
    }

    #[test]
    fn add_time_clock_format_parses_correctly() {
        // "14:30" means "from 14:30 until now" — the exact result depends on
        // wall-clock time, so we just verify it produces a positive number and
        // no failure.
        let mut app = build_app("Meeting notes +work\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        let before_dur = app.tasks()[0].dur;
        app.add_time_to_current_from_input("14:30");
        let after_dur = app.tasks()[0].dur;
        assert!(
            after_dur.unwrap_or(0) > before_dur.unwrap_or(0),
            "clock-time input should add positive seconds"
        );
    }

    // ── toggle_billable ───────────────────────────────────────────────

    #[test]
    fn toggle_billable_adds_bill_n_tag() {
        let mut app = build_app("Draft motion +Smith @drafting dur:3600\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.toggle_billable();

        assert!(
            app.tasks()[0].raw.contains("bill:n"),
            "should add bill:n tag"
        );
        assert_eq!(app.tasks()[0].bill.as_deref(), Some("n"));
        assert_eq!(app.flash_active(), Some("marked as non-billable"));
    }

    #[test]
    fn toggle_billable_removes_bill_n_tag() {
        let mut app = build_app("Firm admin +Admin @admin dur:900 bill:n\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.toggle_billable();

        assert!(
            !app.tasks()[0].raw.contains("bill:n"),
            "should remove bill:n tag, got: {}",
            app.tasks()[0].raw
        );
        assert_eq!(app.tasks()[0].bill, None);
        assert_eq!(app.flash_active(), Some("marked as billable"));
    }

    #[test]
    fn toggle_billable_with_no_task_selected_flashes_error() {
        let mut app = build_app("Draft motion +Smith @drafting dur:3600\n");
        app.nav.cursor = 999;
        app.recompute_visible();

        app.toggle_billable();

        assert_eq!(app.flash_active(), Some("no task selected"));
    }

    #[test]
    fn toggle_billable_preserves_other_tags() {
        let mut app = build_app("(A) Draft motion +Smith @drafting due:2026-08-15 dur:3600\n");
        app.nav.cursor = 0;
        app.recompute_visible();

        app.toggle_billable();

        let raw = &app.tasks()[0].raw;
        assert!(raw.contains("bill:n"), "should add bill:n");
        assert!(raw.contains("+Smith"), "project preserved");
        assert!(raw.contains("@drafting"), "context preserved");
        assert!(raw.contains("due:2026-08-15"), "due preserved");
        assert!(raw.contains("dur:3600"), "dur preserved");
        assert!(raw.contains("(A)"), "priority preserved");
    }

    // ── save and start timer (Ctrl+Enter) ────────────────────────────

    #[test]
    fn add_from_draft_save_and_start_timer() {
        let mut app = build_app("");
        app.draft_set("Buy milk".into());
        // Save the task first.
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert_eq!(app.tasks().len(), 1);
        assert!(!app.timer_running(), "timer not started yet");

        // Ctrl+Enter: now start the timer on the newly created task.
        app.toggle_timer();
        assert!(
            app.timer_running(),
            "timer should be running after toggle_timer"
        );
        let active = app.active_timer_task().expect("active timer task");
        assert!(
            active.raw.contains("Buy milk"),
            "timer on correct task, got: {}",
            active.raw
        );
    }

    #[test]
    fn add_from_draft_save_and_start_uses_last_added_task() {
        let mut app = build_app("existing task\n");
        assert_eq!(app.tasks().len(), 1);
        app.draft_set("new task".into());
        let outcome = app.add_from_draft();
        assert_eq!(outcome, crate::app::AddOutcome::Saved);
        assert_eq!(app.tasks().len(), 2);

        // Ctrl+Enter starts timer on the newly created task, not the existing one.
        app.toggle_timer();
        assert!(app.timer_running());
        let active = app.active_timer_task().expect("active timer task");
        assert!(
            active.raw.contains("new task"),
            "timer on new task, got: {}",
            active.raw
        );
    }
}
