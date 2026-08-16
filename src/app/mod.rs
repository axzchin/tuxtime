use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use self::saved::SavedFilterState;
use self::visibility::VisibleList;
use crate::config::Config;
use crate::core::Store;
use crate::core::outcome::{DrainReport, Reconcile};
use crate::note;
use crate::serve::ShareInfo;
use crate::theme::Theme;
use crate::todo::Task;

mod actions;
mod archive_cache;
mod autocomplete;
mod billable;
mod bulk;
mod chord;
mod draft;
mod draft_overlay_state;
mod duration;
mod env;
mod flash;
mod manual_entry;
mod navigation;
mod nudge_picker;
mod nudges;
pub mod palette;
mod picker;
mod prefs;
mod project_manager;
mod projects;
mod saved;
mod saved_filter_picker;
mod selection;
mod session;
mod share_state;
mod theme_picker_state;
mod timer_actions;
mod timesheet;
mod types;
mod update_checker;
mod visibility;
mod week_start;

#[cfg(test)]
pub(crate) mod test_support;

pub use crate::core::Archive;
pub use crate::core::History;
pub use crate::core::filter::{ListDueBucket, ordered_unique};
pub use archive_cache::ArchiveCache;
pub use autocomplete::{ActiveToken, AutocompleteTarget, TokenKind, active_token};
pub use chord::Chord;
pub use draft::{DialogInputMode, DraftCursor, DraftState};
pub use draft_overlay_state::{
    BuilderField, CalendarState, CalendarTarget, DURATION_PRESETS, DraftOverlay,
    DurationPickerState, OverlayKind, PriorityChooserState, REC_UNIT_ORDER, RecurrenceBuilderState,
    SLASH_ENTRIES, SlashEntry, SlashKind, SlashMenuState, format_rec_value,
    recurrence_next_preview,
};
pub use duration::{
    billable_units, format_billable, format_billable_units, format_clock, format_compact_duration,
    parse_clock, rounding_increment_label,
};
pub(crate) use duration::{format_duration, parse_duration_input};
pub use env::Env;
pub use flash::Flash;
pub use navigation::Navigation;
pub use palette::CommandPaletteState;
pub use prefs::{Layout, Prefs};
pub use project_manager::ProjectManager;
pub use saved_filter_picker::SavedFilterPicker;
pub use selection::Selection;
pub use session::{DayBoundaryAction, IdleReason, NudgePickAction, NudgePickerState, Session};
pub use share_state::ShareState;
pub use theme_picker_state::ThemePicker;
pub use timesheet::TimesheetState;
pub(crate) use timesheet::timesheet_totals;
pub use types::{
    AUTOCOMPLETE_CAP, AddOutcome, Density, FLASH_TTL, Filter, LEADER_WINDOW, Mode, Nudge, Picker,
    ProjectSort, Prompt, SavedFilter, Screen, Sort, TimesheetEntry, TimesheetSort,
    TimesheetTaskRef, UNDO_LIMIT, View,
};
pub use update_checker::UpdateChecker;
pub use visibility::GroupKey;
pub use week_start::WeekStart;

pub struct App {
    /// The headless durable store: tasks, archive, history, persistence, and
    /// `today`. Mutate via the methods on `App` (which map store outcomes to
    /// flash messages and refresh the visible cache); read via `tasks()`,
    /// `archive()`, `task_raw()`, etc.
    pub(crate) store: Store,
    /// Navigation state bundle: view, mode, cursor, per-view cursors/scroll,
    /// quit flag, and overlay return-path tracking. Handlers mutate fields
    /// through `app.nav`; [`App`] provides thin facade methods (`view()`,
    /// `set_view()`, `effective_mode()`) that bridge to `recompute_visible`
    /// and other store-level concerns.
    pub nav: Navigation,
    pub prefs: Prefs,
    /// Active filter (search text, project, context). Crate-private: writing
    /// here would not invalidate `visible_cache`.
    pub(crate) filter: Filter,
    pub draft: DraftState,
    pub selection: Selection,
    flash_state: Flash,
    pub chord: Chord,
    /// Filesystem + working-week context: the todo file in use, the resolved
    /// config path, and the week-start day. See [`Env`].
    pub env: Env,
    /// Theme picker state: cursor and original selection.
    pub theme_picker: ThemePicker,
    /// Project management state: archive, rename, sort.
    pub project_manager: ProjectManager,
    /// Share/capture server state.
    pub share_state: ShareState,
    /// Archive autocomplete cache (projects and contexts from done.txt).
    pub archive_cache: ArchiveCache,
    /// Update checker: latest release tag and background receiver.
    pub update_checker: UpdateChecker,

    /// Filtered + sorted visible index list and its parallel group keys.
    /// Owns the same-length invariant between the two arrays; see
    /// `VisibleList`.
    list: VisibleList,
    /// User-named saved searches plus the `ff` picker's transient state.
    /// Loaded from config at startup, upserted via `fs`, recalled with the
    /// `ff` picker.
    pub(crate) saved: SavedFilterState,
    pub command_palette: CommandPaletteState,
    /// Timesheet state bundle: anchor date, cursor, sort, calendar picker,
    /// groups cache, copy flash.
    pub timesheet: TimesheetState,
    /// Session state: timer activity, nudge flags, transient insert flags.
    pub session: Session,
}

impl App {
    /// Construct an App whose archive is the sibling `done.txt` of `file_path`.
    #[must_use]
    pub fn new(file_path: PathBuf, body: String, today: String, cfg: Config) -> Self {
        let store = Store::new(file_path.clone(), body, today);
        Self::from_store(store, file_path, cfg)
    }

    /// Like [`App::new`] but with an explicit `done.txt` path (e.g. `DONE_FILE`).
    #[must_use]
    pub fn new_with_done(
        file_path: PathBuf,
        done_path: PathBuf,
        body: String,
        today: String,
        cfg: Config,
    ) -> Self {
        let store = Store::new_with_done(file_path.clone(), done_path, body, today);
        Self::from_store(store, file_path, cfg)
    }

    fn from_store(store: Store, file_path: PathBuf, cfg: Config) -> Self {
        // Read saved filters and week_start before `cfg` is moved into `Prefs::from_config`.
        let note_dir = note::notes_dir_from_config(cfg.notes_dir.as_deref());
        let saved = SavedFilterState::from_config(&cfg.filters);
        let week_start = cfg.week_start.unwrap_or(WeekStart::Sunday);
        let today = store.today().to_string();
        let mut app = Self {
            store,
            nav: Navigation::new(),
            prefs: Prefs::from_config(cfg),
            filter: Filter::default(),
            draft: DraftState::default(),
            selection: Selection::default(),
            flash_state: Flash::default(),
            chord: Chord::default(),
            env: Env::new(file_path, week_start),
            theme_picker: ThemePicker::new(0),
            project_manager: ProjectManager::new(Self::load_archived_projects()),
            share_state: ShareState::new(note_dir),
            archive_cache: ArchiveCache::default(),
            update_checker: UpdateChecker::default(),
            list: VisibleList::default(),
            saved,
            command_palette: CommandPaletteState::default(),
            timesheet: TimesheetState::new(&today),
            session: Session::new(),
        };
        app.archive_cache.rebuild(app.store.archive());
        app.recompute_visible();
        app
    }

    /// Rebind the App to a different on-disk file at runtime, replacing the
    /// store (tasks, archive, history, external-change baseline) with a fresh
    /// one for `file_path`/`done_path` loaded from `body`. Prefs, saved
    /// filters, theme, and config live on `App` and are left intact. Used by
    /// the first-run welcome prompt to swap from the placeholder file to the
    /// chosen one. Resets the cursor and recomputes the visible cache.
    pub fn open_file(&mut self, file_path: PathBuf, done_path: PathBuf, body: String) {
        let today = self.store.today().to_string();
        self.store = Store::new_with_done(file_path.clone(), done_path, body, today);
        self.env.file_path = file_path;
        self.nav.move_top();
        self.recompute_visible();
    }

    /// Install the receiver from [`update::spawn_check`](crate::update::spawn_check).
    /// `main` calls this; tests leave it unset so the App stays inert and
    /// doesn't spawn network work as a side effect of construction.
    pub fn set_update_check(&mut self, rx: Receiver<Option<String>>) {
        self.update_checker.receiver = Some(rx);
    }

    /// Drain the update-check receiver. Returns `true` if a new latest
    /// version was just received — callers use this to trigger a redraw so
    /// the status bar can paint the indicator on the next frame.
    pub fn poll_update_check(&mut self) -> bool {
        let Some(rx) = &self.update_checker.receiver else {
            return false;
        };
        match rx.try_recv() {
            Ok(maybe_tag) => {
                self.update_checker.latest_version = maybe_tag;
                self.update_checker.receiver = None;
                self.update_checker.latest_version.is_some()
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.update_checker.receiver = None;
                false
            }
        }
    }

    /// Returns the latest known release tag *if* it is strictly newer than
    /// the running binary. The status bar uses this to decide whether to
    /// draw an "update available" hint.
    pub fn update_available(&self) -> Option<&str> {
        let latest = self.update_checker.latest_version.as_deref()?;
        if crate::update::is_newer(latest, env!("CARGO_PKG_VERSION")) {
            Some(latest)
        } else {
            None
        }
    }

    pub fn theme(&self) -> &'static Theme {
        self.prefs.theme()
    }

    /// Mode the rest of the UI should react to. While the command palette is
    /// open, the underlying list/sidebars should keep rendering as if the
    /// user were still in the mode they came from — otherwise opening the
    /// palette mid-Visual hides the multi-select checkboxes and similar
    /// mode-driven affordances. The palette remembers its caller via the
    /// navigation mode stack (`push_mode`), so `effective_mode` reads the
    /// mode beneath it.
    pub fn effective_mode(&self) -> Mode {
        match self.nav.mode() {
            Mode::Screen(Screen::CommandPalette) => {
                self.nav.peek_under().unwrap_or(self.nav.mode())
            }
            m => m,
        }
    }

    pub fn sort_label(&self) -> &'static str {
        self.prefs.sort_label()
    }

    /// Persist preferences. On failure, flashes a short error so the user
    /// sees the problem inside the TUI (writing to stderr would smash the
    /// alt-screen).
    pub fn save_prefs(&mut self) {
        if let Err(e) = self.prefs.save(self.env.week_start) {
            self.flash(format!("config save failed: {e}"));
        }
    }

    pub fn cycle_theme(&mut self) {
        let msg = self.prefs.cycle_theme();
        self.flash(msg);
        self.save_prefs();
    }

    pub fn cycle_density(&mut self) {
        let msg = self.prefs.cycle_density();
        self.flash(msg);
        self.save_prefs();
    }

    /// Cycle the billable rounding increment (0.1h → 0.25h → exact) and
    /// persist. See [`Prefs::cycle_rounding_increment`].
    pub fn cycle_rounding_increment(&mut self) {
        let msg = self.prefs.cycle_rounding_increment();
        self.flash(msg);
        self.save_prefs();
    }

    pub fn cycle_week_start(&mut self) {
        let msg = match self.env.week_start {
            WeekStart::Sunday => {
                self.env.week_start = WeekStart::Monday;
                "week_start: monday"
            }
            WeekStart::Monday => {
                self.env.week_start = WeekStart::Sunday;
                "week_start: sunday"
            }
        };
        self.flash(msg);
        self.recompute_visible();
        self.save_prefs();
    }

    pub fn cycle_sort(&mut self) {
        let msg = self.prefs.cycle_sort();
        self.flash(msg);
        self.recompute_visible();
        self.save_prefs();
    }

    /// Read-only view of the parsed task list. Mutations go through
    /// dedicated methods so history/persist/visible-cache stay coherent.
    pub fn tasks(&self) -> &[Task] {
        self.store.tasks()
    }

    /// Read-only view of the archived (`done.txt`) tasks.
    pub fn archive(&self) -> &Archive {
        self.store.archive()
    }

    /// The cached "today" (ISO `YYYY-MM-DD`) the store resolves dates against.
    pub fn today(&self) -> &str {
        self.store.today()
    }

    pub fn queue_editor_path(&mut self, path: PathBuf) {
        self.share_state.pending_editor_path = Some(path);
    }

    pub fn notes_dir(&self) -> &PathBuf {
        &self.share_state.notes_dir
    }

    pub fn share_info(&self) -> Option<&ShareInfo> {
        self.share_state.share.as_ref()
    }

    pub fn take_pending_editor_path(&mut self) -> Option<PathBuf> {
        self.share_state.pending_editor_path.take()
    }

    /// True when at least one task is marked done. Used by the binary to
    /// decide whether `A` archives or just toggles the archive view.
    pub fn has_completed_tasks(&self) -> bool {
        self.store.has_completed()
    }

    /// Cloned `raw` for the task at `abs`, or `None` if out of range.
    /// Returning an owned `String` so the caller can hold it across `&mut self`
    /// calls (the common shape for "load draft from current task").
    pub fn task_raw(&self, abs: usize) -> Option<String> {
        self.store.task_raw(abs)
    }

    /// Task under the cursor, resolved against the active view's source:
    /// `archive.tasks()` in Archive view, `tasks` otherwise.
    pub fn cur_task(&self) -> Option<&Task> {
        let i = self.cur_abs()?;
        match self.nav.view {
            View::Archive => self.store.archive().tasks().get(i),
            _ => self.store.tasks().get(i),
        }
    }

    /// Pump archive state (startup loader + external `done.txt` edits). Returns
    /// true when the visible archive changed, so the caller redraws. Refreshes
    /// the visible cache when the Archive view is active and rebuilds the
    /// autocomplete cache so projects/contexts from archived tasks stay
    /// available.
    pub fn poll_archive(&mut self) -> bool {
        let changed = self.store.poll_archive();
        if changed {
            self.archive_cache.rebuild(self.store.archive());
            if matches!(self.nav.view, View::Archive) {
                self.refresh_view();
            }
        }
        changed
    }

    /// Index of the task under the cursor *into `self.tasks`*. Returns `None`
    /// in Archive view because the cursor there points into `archive.tasks()`.
    /// Use this — not `cur_abs()` — for any write that mutates `self.tasks`.
    pub fn cur_task_index_in_tasks(&self) -> Option<usize> {
        if matches!(self.nav.view, View::Archive) {
            return None;
        }
        self.cur_abs()
    }

    /// Read-only view of the active filter.
    pub fn filter(&self) -> &Filter {
        &self.filter
    }

    /// Active top-level view (List/Archive).
    pub fn view(&self) -> View {
        self.nav.view
    }

    /// Switch top-level view. Recomputes the cache so the next frame reflects
    /// the change, snapshots the outgoing view's cursor, and restores the
    /// incoming view's saved cursor (clamped to the new visible length).
    pub fn set_view(&mut self, view: View) {
        if self.nav.view == view {
            return;
        }
        self.nav.switch_view(self.nav.view, view);
        self.refresh_view();
    }

    /// Set the search-filter text. Cursor resets and the cache is recomputed.
    /// Typing into the search prompt calls this on every keystroke.
    pub fn set_search(&mut self, search: String) {
        self.filter.search = search;
        self.nav.move_top();
        self.recompute_visible();
    }

    /// Clear just the search component of the filter.
    pub fn clear_search(&mut self) {
        if self.filter.search.is_empty() {
            return;
        }
        self.filter.search.clear();
        self.nav.move_top();
        self.recompute_visible();
    }

    /// Set or clear the active project filter. `None` removes it.
    pub fn set_project_filter(&mut self, project: Option<String>) {
        self.filter.project = project;
        self.nav.move_top();
        self.recompute_visible();
    }

    /// Set or clear the active context filter. `None` removes it.
    pub fn set_context_filter(&mut self, context: Option<String>) {
        self.filter.context = context;
        self.nav.move_top();
        self.recompute_visible();
    }

    /// Update the cached "today" string. When it changes, the visible cache
    /// is recomputed so threshold-hidden tasks become visible the moment the
    /// wall clock crosses midnight (without requiring an app restart).
    /// Returns `true` iff the date actually advanced — callers use this to
    /// trigger a redraw.
    pub fn refresh_today(&mut self, now: String) -> bool {
        if self.store.set_today(now) {
            self.recompute_visible();
            true
        } else {
            false
        }
    }

    /// Drop every filter component (project + context + search).
    pub fn clear_filter(&mut self) {
        if !self.filter.has_any() {
            return;
        }
        self.filter.clear();
        self.nav.move_top();
        self.recompute_visible();
    }

    // ---- shared helpers for the mutation wrappers -----------------------

    /// After a successful mutation that returned an absolute task index,
    /// rebuild the visible cache and move the cursor to follow that task.
    pub(crate) fn after_mutation(&mut self, follow_abs: usize) {
        self.recompute_visible();
        self.follow_cursor(follow_abs);
    }

    /// Recompute the visible cache and re-clamp the cursor, without following
    /// a task. Used by mutations whose result has no natural owner index
    /// (delete, archive, undo) and by transitions that rederive the cursor.
    pub(crate) fn refresh_view(&mut self) {
        self.recompute_visible();
        self.clamp_cursor();
    }

    /// Flash a success message and refresh the view with the cursor following
    /// `abs`. Collapses the flash + `after_mutation` tail that nearly every
    /// mutation wrapper repeats.
    pub(crate) fn commit(&mut self, msg: impl Into<String>, abs: usize) {
        self.flash(msg);
        self.after_mutation(abs);
    }

    /// Refresh the view after a mutation whose success may or may not carry a
    /// task to follow: follows `follow` when present, otherwise just
    /// recomputes. Collapses the tail that two-phase flows (e.g. the
    /// day-boundary prompt) repeat.
    pub(crate) fn after_follow(&mut self, follow: Option<usize>) {
        if let Some(abs) = follow {
            self.after_mutation(abs);
        } else {
            self.recompute_visible();
        }
    }

    /// Handle a store reconcile that reloaded the file from disk: reset
    /// transient input state and refresh the view, matching the old
    /// `apply_external_state` behavior.
    pub(crate) fn on_reload(&mut self) {
        self.selection.clear();
        self.selection.exit_edit();
        self.recompute_visible();
        self.clamp_cursor();
        self.flash("file changed on disk — reloaded");
    }

    /// Map a store reconcile result to the matching flash + view refresh for a
    /// mutation that produced no change of its own (used on the abort paths).
    pub(crate) fn handle_reconcile_abort(&mut self, r: Reconcile) {
        match r {
            Reconcile::Reloaded => self.on_reload(),
            Reconcile::ReadError => self.flash("read failed"),
            Reconcile::Unchanged => {}
        }
    }

    /// Surface a [`DrainReport`] from `Store::drain_inbox` as a flash, matching
    /// the wording the inline drain used to emit, and refresh the view when
    /// tasks were merged.
    pub(crate) fn apply_drain(&mut self, report: DrainReport) {
        if report.merged > 0 {
            self.refresh_view();
        }
        if let Some(err) = report.error {
            self.flash(err);
        } else if report.merged > 0 {
            if report.skipped > 0 {
                self.flash(format!(
                    "merged {} from inbox ({} skipped)",
                    report.merged, report.skipped
                ));
            } else {
                self.flash(format!("merged {} from inbox", report.merged));
            }
        } else if report.skipped > 0 {
            self.flash(format!(
                "inbox: {} unparseable, nothing merged",
                report.skipped
            ));
        }
    }

    /// Reload config, updating prefs, saved filters, week start, and nudge thresholds.
    pub fn reload_config(&mut self, new_cfg: Config) {
        self.prefs = Prefs::from_config(new_cfg.clone());
        // Replace the filter list in place so the `ff` picker's transient
        // restore-point/cursor survive a mid-picker reload.
        self.saved.list = new_cfg
            .filters
            .iter()
            .map(|(name, query)| SavedFilter {
                name: name.clone(),
                query: query.clone(),
            })
            .collect();
        self.env.week_start = new_cfg.week_start.unwrap_or(WeekStart::Sunday);
        self.recompute_visible();
    }

    /// Reconcile against disk and drain the inbox. Returns `true` when it is
    /// safe to proceed (disk unchanged); `false` when the file was reloaded or
    /// unreadable. The TUI run loop and `handle_key` call this each tick.
    pub fn check_external_changes(&mut self) -> bool {
        let reconcile = self.store.reconcile();
        if matches!(reconcile, Reconcile::Reloaded) {
            self.on_reload();
        }
        let report = self.store.drain_inbox();
        self.apply_drain(report);
        matches!(reconcile, Reconcile::Unchanged)
    }
}
