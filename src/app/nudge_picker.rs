//! Nudge task picker: the `S`/`M` selection opened from the idle nudge. Runs
//! on the real list view (full navigation/search/filter while choosing) and
//! forces the user to deliberately highlight a task before a timer is started
//! or time is added — the popup must never grab the row under the cursor. The
//! idle/long-timer/review/stale-timer detection lives in [`super::nudges`].

use super::session::DayBoundaryAction;
use super::{App, Filter, Mode, Nudge, NudgePickAction, NudgePickerState, Picker, Prompt, View};

impl App {
    /// Open the nudge task picker for the given action. Runs on the *real
    /// list view* so the user keeps full navigation, search and filtering
    /// while choosing — no separate dialog. The active search/filter is
    /// cleared so every open task is visible (the same "nothing hidden"
    /// guarantee the old standalone dialog offered) and saved so the
    /// selection can restore it on exit. The cursor is seeded on the first
    /// open task. The whole point stands: a nudge must never start a timer
    /// (or add time) to a random task just because it happens to be under
    /// the cursor — the highlighted task is the deliberate choice.
    pub fn enter_nudge_picker(&mut self, action: NudgePickAction) {
        if !self.store.tasks().iter().any(|t| !t.done) {
            self.flash("no open tasks — press n to create one");
            self.nav.mode = Mode::Nudge(Nudge::Idle);
            return;
        }
        let state = NudgePickerState {
            action,
            prev_filter: self.filter.clone(),
            prev_cursor: self.nav.cursor,
        };
        // Force the list view and reveal every open task.
        self.set_view(View::List);
        self.set_project_filter(None);
        self.set_context_filter(None);
        self.clear_search();
        // Seed the cursor on the first open task so Enter is always safe.
        self.recompute_visible();
        if let Some(pos) = self
            .visible_indices()
            .iter()
            .position(|&a| !self.store.tasks()[a].done)
        {
            self.nav.cursor = pos;
        }
        self.session.nudge_picker = Some(state);
        self.nav.mode = Mode::Picker(Picker::NudgeTask);
    }

    /// Commit the picker: start the timer (or open add-time) on the task
    /// highlighted in the list, restoring the pre-selection search/filter.
    /// The user consciously navigated to that row — the visible highlight IS
    /// the choice the picker exists to force.
    pub fn nudge_picker_accept(&mut self) {
        let Some(picker) = self.session.nudge_picker.take() else {
            return;
        };
        // The commit always resolves against the LIST highlight. A delegated
        // key can wander into another view (V → timesheet) while the
        // selection is still open; come back first so the timer never starts
        // on a non-list cursor.
        if self.nav.view != View::List {
            self.set_view(View::List);
        }
        let Some(abs) = self.cur_abs() else {
            self.nav.mode = Mode::Nudge(Nudge::Idle);
            return;
        };
        // A completed task must never receive time — stay in selection mode
        // (filter still cleared) so the user can pick an open one.
        if self.store.tasks()[abs].done {
            self.session.nudge_picker = Some(picker);
            self.flash("that task is done — pick an open one");
            return;
        }
        match picker.action {
            NudgePickAction::StartTimer => {
                // Starting a timer is itself a recovery — never inherit a
                // stale flag from a previous flow.
                self.session.from_nudge = false;
                if self.should_prompt_day_boundary(abs) {
                    // The chosen task carries time from a previous day. Ask
                    // first (continue the same entry vs a fresh one for
                    // today) exactly like `t` in the list — never silently
                    // move the entry onto today's sheet. The selection stays
                    // open beneath the prompt so a resolve (c/n) starts the
                    // timer and finishes the selection, while Esc drops back
                    // into the selection to pick a different task.
                    self.session.nudge_picker = Some(picker);
                    self.session.pending_day_boundary = Some((abs, DayBoundaryAction::StartTimer));
                    self.nav.push_mode(Mode::Prompt(Prompt::DayBoundary));
                    return;
                }
                self.toggle_timer_at(abs);
                self.restore_filter(picker.prev_filter, picker.prev_cursor);
                self.nav.enter_normal();
                if let Some(v) = self.session.pre_nudge_view.take() {
                    self.set_view(v);
                }
            }
            NudgePickAction::AddTime => {
                // Remember the prompt came from the nudge so an Esc-cancel
                // returns to the popup (the reminder survives an aborted
                // recovery) while a successful add exits to Normal.
                self.session.from_nudge = true;
                self.restore_filter(picker.prev_filter, picker.prev_cursor);
                // If the restored filter would hide the chosen task, keep
                // the selection's unfiltered view so the add-time prompt
                // targets the task the user actually picked.
                if !self.visible_indices().contains(&abs) {
                    self.filter.search.clear();
                    self.filter.project = None;
                    self.filter.context = None;
                    self.recompute_visible();
                }
                // Point the list cursor at the chosen task so the add-time
                // prompt targets it, then open the prompt.
                if let Some(pos) = self.visible_indices().iter().position(|&a| a == abs) {
                    self.nav.cursor = pos;
                }
                let body = self
                    .store
                    .tasks()
                    .get(abs)
                    .map(|t| crate::todo::body_only_from_clean(&t.clean_raw))
                    .unwrap_or_default();
                self.draft_clear();
                self.flash(format!("add time to: {body}"));
                self.nav.mode = Mode::Prompt(Prompt::AddTime);
            }
        }
    }

    /// Esc from the picker returns to the idle-nudge popup so the user can
    /// pick a different action (or dismiss), restoring the pre-selection
    /// search/filter.
    pub fn nudge_picker_cancel(&mut self) {
        if let Some(p) = self.session.nudge_picker.take() {
            self.restore_filter(p.prev_filter, p.prev_cursor);
        }
        self.nav.mode = Mode::Nudge(Nudge::Idle);
    }

    /// End the picker without the Enter key — used when a timer is started
    /// mid-selection (`t`), which completes the recovery on its own. Restores
    /// the pre-selection search/filter and the pre-nudge view.
    pub fn nudge_picker_finish(&mut self) {
        if let Some(p) = self.session.nudge_picker.take() {
            self.restore_filter(p.prev_filter, p.prev_cursor);
        }
        self.nav.enter_normal();
        if let Some(v) = self.session.pre_nudge_view.take() {
            self.set_view(v);
        }
    }

    /// A delegated key left the selection for a non-overlay mode (`n`, `e`,
    /// `,`, `?`, `P`, …) — the selection is over. Restore the pre-selection
    /// search/filter and drop the stale state WITHOUT touching the mode the
    /// user is now in.
    pub fn nudge_picker_abandon(&mut self) {
        if let Some(p) = self.session.nudge_picker.take() {
            self.restore_filter(p.prev_filter, p.prev_cursor);
        }
    }

    /// A view switch ended the selection (`V` timesheet, `a` archive — a
    /// deliberate dismissal, like pressing D on the popup): restore the
    /// pre-selection filter, drop the picker, exit to Normal, forget the
    /// pre-nudge view (the user chose where to go), and reset the idle-nudge
    /// clock so the reminder doesn't re-fire over the view just opened. The
    /// user is still untracked, so the nudge will nag again once the clock
    /// lapses — only the instant re-fire is suppressed.
    pub fn nudge_picker_exit_to_view(&mut self) {
        self.nudge_picker_abandon();
        // The user chose where to go (V/a already switched the view), so a
        // stale pre-nudge view must not override it — clear it before the
        // dismiss, whose restore then sees None and leaves the view alone.
        self.session.pre_nudge_view = None;
        self.dismiss_nudge();
    }

    /// Put the user's search/project/context filter state back the way it
    /// was before the picker opened and restore the cursor to its pre-nudge
    /// position (clamped to the rebuilt list). One visible-cache rebuild.
    fn restore_filter(&mut self, prev: Filter, prev_cursor: usize) {
        self.filter.search = prev.search;
        self.filter.project = prev.project;
        self.filter.context = prev.context;
        self.recompute_visible();
        self.nav
            .set_cursor(prev_cursor.min(self.visible_indices().len().saturating_sub(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Screen;
    use crate::app::test_support::build_app;
    use crate::keybinds::KeyBindings;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn esc() -> KeyEvent {
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
    }

    /// The picker runs on the real list: the active search/filter is cleared
    /// so every open task is visible (the "nothing hidden" guarantee of the
    /// old standalone dialog), the saved filter is stashed for restore, and
    /// the cursor is seeded on the first open task.
    #[test]
    fn nudge_picker_reveals_all_open_tasks_and_seeds_cursor() {
        let mut app = build_app("First +A\nSecond +B\nx 2026-05-05 Done +C\n");
        // An active filter + stale cursor must not hide anything.
        app.set_project_filter(Some("A".into()));
        app.set_search("zzz".into());
        app.nav.cursor = 1;
        app.recompute_visible();

        app.enter_nudge_picker(NudgePickAction::StartTimer);

        assert_eq!(app.nav.mode, Mode::Picker(Picker::NudgeTask));
        assert_eq!(app.nav.view, View::List);
        assert!(
            app.filter().search.is_empty(),
            "search must be cleared for the selection"
        );
        assert!(
            app.filter().project.is_none(),
            "project filter must be cleared for the selection"
        );
        let seeded = app.cur_abs().expect("cursor must land on a task");
        assert!(
            !app.store.tasks()[seeded].done,
            "cursor must seed on an open task"
        );
        let picker = app
            .session
            .nudge_picker
            .as_ref()
            .expect("picker must be open");
        assert_eq!(picker.action, NudgePickAction::StartTimer);
        assert_eq!(
            picker.prev_filter.search, "zzz",
            "previous search must be saved for restore"
        );
        assert_eq!(
            picker.prev_filter.project.as_deref(),
            Some("A"),
            "previous project filter must be saved for restore"
        );
    }

    /// Enter on the picker starts the timer on the highlighted task — the
    /// row the user navigated to — and returns to the pre-nudge view.
    #[test]
    fn nudge_picker_start_timer_targets_highlighted_task() {
        let mut app = build_app("First +A\nSecond +B\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        app.session.pre_nudge_view = Some(View::Timesheet);

        app.enter_nudge_picker(NudgePickAction::StartTimer);
        // Navigate the real list to the second task and commit.
        app.nav.cursor = 1;
        app.nudge_picker_accept();

        assert!(
            app.is_timer_running_on(1),
            "timer must run on the HIGHLIGHTED task"
        );
        assert!(!app.is_timer_running_on(0));
        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
        assert_eq!(app.nav.view, View::Timesheet, "pre-nudge view restored");
        assert!(app.session.nudge_picker.is_none());
    }

    /// Enter on the add-time picker opens the add-time prompt targeting the
    /// highlighted task, with the confirmation flash naming it.
    #[test]
    fn nudge_picker_add_time_targets_highlighted_task() {
        let mut app = build_app("First +A\nSecond +B\n");
        app.nav.cursor = 1;
        app.recompute_visible();

        app.enter_nudge_picker(NudgePickAction::AddTime);
        app.nav.cursor = 0; // highlight First
        app.nudge_picker_accept();

        assert_eq!(app.nav.mode, Mode::Prompt(Prompt::AddTime));
        assert_eq!(app.nav.view, View::List, "add-time needs the list view");
        assert_eq!(app.nav.cursor, 0, "cursor must point at the chosen task");
        assert!(
            app.flash_active()
                .is_some_and(|m| m.contains("First") && m.contains("add time to")),
            "flash must name the chosen task"
        );
    }

    /// Enter (S) on a task carrying time from a previous day asks about the
    /// day boundary first — exactly like `t` in the list — instead of
    /// silently continuing the old entry onto today's sheet. Resolving with
    /// `c` starts the timer on the same entry and finishes the selection.
    #[test]
    fn nudge_picker_start_timer_prompts_on_previous_day_task() {
        let mut app = build_app("Draft +Smith dur:7200 log:2026-05-05\nSecond +Jones\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        app.enter_nudge_picker(NudgePickAction::StartTimer);

        app.nudge_picker_accept(); // Enter on the previous-day task

        assert_eq!(app.nav.mode, Mode::Prompt(Prompt::DayBoundary));
        assert!(!app.timer_running(), "no timer until the prompt resolves");
        assert!(
            app.session.nudge_picker.is_some(),
            "selection must stay open beneath the prompt"
        );

        // Continue the same entry: timer starts, selection finishes.
        crate::interactive::handle_day_boundary(&mut app, key('c'));
        assert!(app.timer_running(), "timer must start on resolve");
        assert!(app.is_timer_running_on(0));
        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
        assert!(app.session.nudge_picker.is_none());
    }

    /// Resolving the day-boundary prompt with `n` (new entry for today) also
    /// completes the recovery: the carried-forward fresh line carries the
    /// timer and the selection finishes.
    #[test]
    fn nudge_picker_day_boundary_new_entry_completes_selection() {
        let mut app = build_app("Draft +Smith dur:7200 log:2026-05-05\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        app.enter_nudge_picker(NudgePickAction::StartTimer);
        app.nudge_picker_accept();

        assert_eq!(app.nav.mode, Mode::Prompt(Prompt::DayBoundary));
        crate::interactive::handle_day_boundary(&mut app, key('n'));

        assert!(app.timer_running());
        assert_eq!(app.nav.mode, Mode::Screen(Screen::Normal));
        assert!(app.session.nudge_picker.is_none());
    }

    /// Esc from the day-boundary prompt drops back into the selection with
    /// the picker intact, so the user can navigate to a different task
    /// instead of being forced out.
    #[test]
    fn nudge_picker_day_boundary_esc_returns_to_selection() {
        let mut app = build_app("Draft +Smith dur:7200 log:2026-05-05\nSecond +Jones\n");
        app.nav.cursor = 0;
        app.recompute_visible();
        app.enter_nudge_picker(NudgePickAction::StartTimer);
        app.nudge_picker_accept();

        assert_eq!(app.nav.mode, Mode::Prompt(Prompt::DayBoundary));
        crate::interactive::handle_day_boundary(&mut app, esc());

        assert_eq!(app.nav.mode, Mode::Picker(Picker::NudgeTask));
        assert!(!app.timer_running());
        assert!(
            app.session.nudge_picker.is_some(),
            "selection must survive Esc from the prompt"
        );
    }

    /// Finishing the selection (Enter) restores the search/filter that was
    /// active before it opened.
    #[test]
    fn nudge_picker_restores_previous_filter_on_exit() {
        let mut app = build_app("First +A\nSecond +B\n");
        app.set_search("First".into());
        app.set_project_filter(Some("A".into()));
        app.enter_nudge_picker(NudgePickAction::StartTimer);
        assert!(
            app.filter().search.is_empty(),
            "precondition: search cleared during selection"
        );

        app.nudge_picker_accept();

        assert_eq!(
            app.filter().search,
            "First",
            "previous search must come back after the selection"
        );
        assert_eq!(
            app.filter().project.as_deref(),
            Some("A"),
            "previous project filter must come back"
        );
    }

    /// Esc from the picker returns to the idle-nudge popup AND restores the
    /// pre-selection filter — a cancelled choice must not change the user's
    /// list state.
    #[test]
    fn nudge_picker_cancel_restores_filter() {
        let mut app = build_app("First +A\nSecond +B\n");
        app.set_project_filter(Some("B".into()));
        app.enter_nudge_picker(NudgePickAction::StartTimer);
        assert!(app.filter().project.is_none());

        app.nudge_picker_cancel();

        assert_eq!(app.nav.mode, Mode::Nudge(Nudge::Idle));
        assert!(app.session.nudge_picker.is_none());
        assert_eq!(
            app.filter().project.as_deref(),
            Some("B"),
            "filter must be restored on Esc"
        );
    }

    /// Exiting the selection restores the cursor to its pre-nudge position
    /// (clamped), so a nudge recovery never drops the user's place in the
    /// list.
    #[test]
    fn nudge_picker_exit_restores_cursor_position() {
        let mut app = build_app("First\nSecond\nThird\n");
        app.nav.cursor = 2;
        app.recompute_visible();
        app.enter_nudge_picker(NudgePickAction::StartTimer);
        // Navigate somewhere else during the selection.
        app.nav.cursor = 0;

        app.nudge_picker_cancel();

        assert_eq!(app.nav.cursor, 2, "pre-nudge cursor must come back");
    }

    /// A completed task under the cursor must never receive time: Enter
    /// refuses, stays in selection mode, and the reminder survives.
    #[test]
    fn nudge_picker_accept_refuses_done_task() {
        let mut app = build_app("First\nx 2026-05-05 Done\n");
        app.prefs.show_done = true; // done tasks appear in the list
        app.enter_nudge_picker(NudgePickAction::StartTimer);
        app.nav.cursor = 1; // highlight the done task

        app.nudge_picker_accept();

        assert!(!app.timer_running(), "no timer on a done task");
        assert_eq!(
            app.nav.mode,
            Mode::Picker(Picker::NudgeTask),
            "must stay in selection mode to pick an open task"
        );
        assert!(
            app.session.nudge_picker.is_some(),
            "selection state must survive the refusal"
        );
        assert!(
            app.flash_active().is_some_and(|m| m.contains("done")),
            "flash must explain the refusal"
        );
    }

    /// No open tasks: the picker can't open; stay on the idle nudge.
    #[test]
    fn nudge_picker_empty_stays_on_idle_nudge() {
        let mut app = build_app("");
        app.enter_nudge_picker(NudgePickAction::StartTimer);
        assert_eq!(app.nav.mode, Mode::Nudge(Nudge::Idle));
    }

    /// Abandoning the selection (a delegated key replaced the mode, e.g.
    /// `n`) restores the pre-selection filter and drops the stale picker
    /// state without changing the mode the user is now in.
    #[test]
    fn nudge_picker_abandon_restores_filter_keeps_mode() {
        let mut app = build_app("First\n");
        app.set_project_filter(Some("A".into()));
        app.enter_nudge_picker(NudgePickAction::StartTimer);
        assert!(app.filter().project.is_none());
        // The user presses `n` (new task): the Insert dialog opens.
        crate::interactive::dispatch(
            &mut app,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
            &KeyBindings::default(),
        );

        assert_eq!(
            app.nav.mode,
            Mode::Screen(Screen::Insert),
            "Insert stays open"
        );
        assert!(
            app.session.nudge_picker.is_none(),
            "stale picker state must be dropped"
        );
        assert_eq!(
            app.filter().project.as_deref(),
            Some("A"),
            "pre-selection filter must be restored on abandon"
        );
    }
}
