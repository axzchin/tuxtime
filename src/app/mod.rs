use std::collections::BTreeMap;
use std::time::Instant;

use chrono::Timelike;
use core::fmt;
use std::cell::Cell;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::config::Config;
use crate::core::Store;
use crate::core::outcome::{DrainReport, Reconcile, TimerOutcome, TimerQuitOutcome};
use crate::note;
use crate::serve::{self, ShareInfo};
use crate::theme::{self, Theme};
use crate::todo::Task;

mod autocomplete;
mod bulk;
mod chord;
mod draft;
mod draft_overlay;
mod flash;
mod mutations;
pub mod palette;
mod picker;
mod prefs;
mod saved;
mod selection;
mod types;
mod visibility;

#[cfg(test)]
pub(crate) mod test_support;

pub use crate::core::Archive;
pub use crate::core::History;
pub use crate::core::filter::{ListDueBucket, ordered_unique};
pub use autocomplete::{ActiveToken, AutocompleteTarget, TokenKind, active_token};
pub use chord::Chord;
pub use draft::{DialogInputMode, DraftCursor, DraftState};
pub use draft_overlay::{
    BuilderField, CalendarState, CalendarTarget, DraftOverlay, OverlayKind, PriorityChooserState,
    REC_UNIT_ORDER, RecurrenceBuilderState, SLASH_ENTRIES, SlashEntry, SlashKind, SlashMenuState,
    format_rec_value, recurrence_next_preview, DurationPickerState, DURATION_PRESETS,
};
pub use flash::Flash;
pub use palette::CommandPaletteState;
pub use prefs::{Layout, Prefs};
pub use selection::Selection;
pub use types::{
    AUTOCOMPLETE_CAP, AddOutcome, Density, FLASH_TTL, Filter, LEADER_WINDOW, Mode, SavedFilter,
    Sort, UNDO_LIMIT, View,
};
pub use visibility::GroupKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeekStart {
    Sunday,
    Monday,
}

impl WeekStart {
    pub fn as_str(self) -> &'static str {
        match self {
            WeekStart::Sunday => "sunday",
            WeekStart::Monday => "monday",
        }
    }
}

impl fmt::Display for WeekStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WeekStart {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sunday" => Ok(WeekStart::Sunday),
            "monday" => Ok(WeekStart::Monday),
            _ => Err(()),
        }
    }
}

pub struct App {
    /// The headless durable store: tasks, archive, history, persistence, and
    /// `today`. Mutate via the methods on `App` (which map store outcomes to
    /// flash messages and refresh the visible cache); read via `tasks()`,
    /// `archive()`, `task_raw()`, etc.
    pub(crate) store: Store,
    /// Crate-private: writing here would not invalidate `visible_cache`.
    /// Read via `view()`; mutate via `set_view()`.
    pub(crate) view: View,
    pub mode: Mode,
    pub prefs: Prefs,
    pub cursor: usize,
    /// Per-view saved cursor, indexed by `View::idx()`. `set_view` snapshots
    /// the outgoing view's cursor here and restores the incoming view's, so
    /// each view remembers where the user last was.
    pub(crate) view_cursor: [usize; 2],
    /// Crate-private: same reason as `view` — `visible_cache` would drift.
    /// Read via `filter()`; mutate via `set_search`/`set_project`/etc.
    pub(crate) filter: Filter,
    pub draft: DraftState,
    pub selection: Selection,
    flash_state: Flash,
    pub chord: Chord,
    pub file_path: PathBuf,
    /// Resolved path of the on-disk config file. Set by the binary after
    /// construction so the settings overlay can render a stable, real path
    /// without the renderer having to reach into the environment itself.
    /// `None` in tests/examples that don't care about the value.
    pub config_path: Option<PathBuf>,
    pub should_quit: bool,
    visible_cache: Vec<usize>,
    /// Parallel to `visible_cache`: `visible_groups[i]` is the group key for
    /// the row at `visible_cache[i]`. `GroupKey::None` for List under
    /// `Sort::File`; priority/due bucket keys under other List sorts; date
    /// keys for Archive. Renderers read this to draw section headers.
    visible_groups: Vec<crate::app::visibility::GroupKey>,
    /// Latest known release tag, populated asynchronously by the update
    /// checker. `None` while we haven't heard back (or the check is disabled,
    /// e.g. in tests). The UI compares this against `CARGO_PKG_VERSION` to
    /// decide whether to surface an "update available" hint.
    pub(crate) latest_version: Option<String>,
    /// Receiver for the background update check. Drained each tick; cleared
    /// once a result has been received or the sender hung up.
    update_check: Option<Receiver<Option<String>>>,
    /// User-named saved searches, loaded from config at startup and
    /// upserted via `fs`. Recalled with the `ff` picker.
    pub saved_filters: Vec<SavedFilter>,
    /// The search string that was active when the `ff` picker opened, so
    /// cancelling (`Esc`) restores it instead of leaving the previewed
    /// filter applied. `None` outside `Mode::PickSavedFilter`.
    saved_pick_restore: Option<String>,
    /// Index into `saved_filters` of the row the `ff` picker currently
    /// previews. Tracked explicitly rather than re-derived from
    /// `filter.search` so duplicate queries don't strand j/k. Only
    /// meaningful while `Mode::PickSavedFilter`; re-seeded on each open.
    saved_pick_idx: usize,
    pub command_palette: CommandPaletteState,
    /// Vertical scroll offset (rows from the top of the line list) for each
    /// view, keyed by `View::idx()`. Updated at render time via `Cell` so the
    /// renderer can keep the cursor row visible without taking `&mut self`.
    pub(crate) view_scroll: [Cell<u16>; 2],
    /// Handle to the in-TUI capture server. `None` until the first time
    /// the user presses `s` (or invokes "show capture QR" from the
    /// palette). Once bound, the entry stays for the rest of the
    /// session and the overlay just re-displays the saved QR.
    share: Option<ShareInfo>,
    /// Base directory used by note actions. Relative `note:<path>` tokens are
    /// resolved under this directory, and generated notes are created below it.
    pub(crate) notes_dir: PathBuf,
    /// Path queued for opening in the user's editor after the TUI temporarily
    /// restores the terminal. Set by OpenNote and drained by the run loop.
    pending_editor_path: Option<PathBuf>,
    /// Theme index captured when the theme picker opened, so cancel
    /// can restore it.
    theme_pick_orig: usize,
    pub week_start: WeekStart,
    /// Timestamp of the last timer activity (start or stop). Used for idle nudge detection.
    pub last_timer_activity: Instant,
    /// True when the running timer has exceeded the long-timer nudge threshold.
    pub long_timer_nudge_active: bool,
    /// Timesheet mode: true = weekly view, false = daily (default).
    pub timesheet_weekly: bool,
    /// Timesheet cursor: index into the project+activity group list.
    pub timesheet_cursor: usize,
    /// True when the current Insert session was entered via `M` (manual time
    /// entry). Drives `dur:` value conversion on save.
    pub manual_time_entry: bool,
}

impl App {
    /// Construct an App whose archive is the sibling `done.txt` of `file_path`.
    pub fn new(file_path: PathBuf, body: String, today: String, cfg: Config) -> Self {
        let store = Store::new(file_path.clone(), body, today);
        Self::from_store(store, file_path, cfg)
    }

    /// Like [`App::new`] but with an explicit `done.txt` path (e.g. `DONE_FILE`).
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
        let saved_filters = cfg
            .filters
            .iter()
            .map(|(name, query)| SavedFilter {
                name: name.clone(),
                query: query.clone(),
            })
            .collect();
        let week_start = cfg.week_start.unwrap_or(WeekStart::Sunday);
        let mut app = Self {
            store,
            view: View::List,
            mode: Mode::Normal,
            prefs: Prefs::from_config(cfg),
            cursor: 0,
            view_cursor: [0; 2],
            filter: Filter::default(),
            draft: DraftState::default(),
            selection: Selection::default(),
            flash_state: Flash::default(),
            chord: Chord::default(),
            file_path,
            config_path: None,
            should_quit: false,
            visible_cache: Vec::new(),
            visible_groups: Vec::new(),
            latest_version: None,
            update_check: None,
            saved_filters,
            saved_pick_restore: None,
            saved_pick_idx: 0,
            command_palette: CommandPaletteState::default(),
            view_scroll: [Cell::new(0), Cell::new(0)],
            share: None,
            notes_dir: note_dir,
            pending_editor_path: None,
            theme_pick_orig: 0,
            week_start,
            last_timer_activity: Instant::now(),
            long_timer_nudge_active: false,
            timesheet_weekly: false,
            timesheet_cursor: 0,
            manual_time_entry: false,
        };
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
        self.file_path = file_path;
        self.cursor = 0;
        self.recompute_visible();
    }

    /// Idempotent: bind the capture server on first call, then store
    /// the [`ShareInfo`] so subsequent calls just re-show the overlay.
    ///
    /// First-call behavior: if the user has a persisted token + port in
    /// config, reuse them so phone bookmarks survive across sessions.
    /// Otherwise, generate a fresh token, let the OS pick a port, and
    /// write both back to the config. If the persisted port is taken
    /// (another tuxtime instance on the same machine, say), fall back to
    /// an OS-assigned port and rewrite the config so the next session
    /// starts fresh.
    pub fn ensure_share_started(&mut self) -> Result<&ShareInfo, String> {
        // Two-step to dodge stable's lack of Polonius: do the bind work
        // first (which needs `&mut self`), then the unconditional final
        // borrow returns the now-present `ShareInfo`.
        if self.share.is_none() {
            let info = self.bind_share()?;
            self.share = Some(info);
        }
        Ok(self
            .share
            .as_ref()
            .expect("share is Some after the bind branch"))
    }

    fn bind_share(&mut self) -> Result<ShareInfo, String> {
        let cfg = Config::load();
        let token = match cfg.share_token {
            Some(t) => t,
            None => serve::net::generate_token().map_err(|e| format!("token: {e}"))?,
        };
        let requested_port = cfg.share_port.unwrap_or(0);
        let info = match serve::start(self.file_path.clone(), token.clone(), requested_port) {
            Ok(info) => info,
            Err(_) if requested_port != 0 => {
                // Configured port is taken — fall back to OS-assigned.
                serve::start(self.file_path.clone(), token.clone(), 0)
                    .map_err(|e| format!("bind: {e}"))?
            }
            Err(e) => return Err(format!("bind: {e}")),
        };
        // Persist token + port back to config so phone bookmarks survive.
        // Load fresh first so we don't clobber any prefs the user has
        // toggled since this App was constructed.
        let mut to_save = Config::load();
        to_save.share_token = Some(info.token.clone());
        to_save.share_port = Some(info.port);
        if let Err(e) = to_save.save() {
            self.flash(format!("share config save failed: {e}"));
        }
        Ok(info)
    }

    pub fn share_info(&self) -> Option<&ShareInfo> {
        self.share.as_ref()
    }

    /// Install the receiver from [`update::spawn_check`](crate::update::spawn_check).
    /// `main` calls this; tests leave it unset so the App stays inert and
    /// doesn't spawn network work as a side effect of construction.
    pub fn set_update_check(&mut self, rx: Receiver<Option<String>>) {
        self.update_check = Some(rx);
    }

    /// Drain the update-check receiver. Returns `true` if a new latest
    /// version was just received — callers use this to trigger a redraw so
    /// the status bar can paint the indicator on the next frame.
    pub fn poll_update_check(&mut self) -> bool {
        let Some(rx) = &self.update_check else {
            return false;
        };
        match rx.try_recv() {
            Ok(maybe_tag) => {
                self.latest_version = maybe_tag;
                self.update_check = None;
                self.latest_version.is_some()
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.update_check = None;
                false
            }
        }
    }

    /// Returns the latest known release tag *if* it is strictly newer than
    /// the running binary. The status bar uses this to decide whether to
    /// draw an "update available" hint.
    pub fn update_available(&self) -> Option<&str> {
        let latest = self.latest_version.as_deref()?;
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
    /// mode-driven affordances.
    pub fn effective_mode(&self) -> Mode {
        match self.mode {
            Mode::CommandPalette => self.command_palette.prior().unwrap_or(self.mode),
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
        if let Err(e) = self.prefs.save(self.week_start) {
            self.flash(format!("config save failed: {e}"));
        }
    }

    pub fn cycle_theme(&mut self) {
        let msg = self.prefs.cycle_theme();
        self.flash(msg);
        self.save_prefs();
    }

    /// Enter theme picker mode. Snapshot the current theme index so
    /// cancel can restore it. j/k live-previews; Enter accepts; Esc
    /// reverts.
    pub fn enter_pick_theme(&mut self) {
        self.theme_pick_orig = self.prefs.theme_idx();
        self.mode = Mode::PickTheme;
    }

    /// Step through themes in `forward` (true = next) direction with
    /// wrap-around. The theme is applied immediately for live preview.
    pub fn pick_theme_step(&mut self, forward: bool) {
        let all = theme::all();
        let len = all.len();
        if len <= 1 {
            return;
        }
        let cur = self.prefs.theme_idx();
        let next = if forward {
            (cur + 1) % len
        } else {
            (cur + len - 1) % len
        };
        self.prefs.set_theme_idx(next);
    }

    /// Accept the previewed theme and persist to config.
    pub fn pick_theme_accept(&mut self) {
        self.mode = Mode::Normal;
        self.save_prefs();
        self.flash(format!("theme: {}", self.theme().name));
    }

    /// Cancel the picker and restore the theme that was active when
    /// the picker opened.
    pub fn pick_theme_cancel(&mut self) {
        self.prefs.set_theme_idx(self.theme_pick_orig);
        self.mode = Mode::Normal;
    }

    pub fn cycle_density(&mut self) {
        let msg = self.prefs.cycle_density();
        self.flash(msg);
        self.save_prefs();
    }

    pub fn cycle_week_start(&mut self) {
        let msg = match self.week_start {
            WeekStart::Sunday => {
                self.week_start = WeekStart::Monday;
                "week_start: monday"
            }
            WeekStart::Monday => {
                self.week_start = WeekStart::Sunday;
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
        self.pending_editor_path = Some(path);
    }

    pub fn notes_dir(&self) -> &PathBuf {
        &self.notes_dir
    }

    pub fn take_pending_editor_path(&mut self) -> Option<PathBuf> {
        self.pending_editor_path.take()
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
        match self.view {
            View::Archive => self.store.archive().tasks().get(i),
            _ => self.store.tasks().get(i),
        }
    }

    /// Pump archive state (startup loader + external `done.txt` edits). Returns
    /// true when the visible archive changed, so the caller redraws. Refreshes
    /// the visible cache when the Archive view is active.
    pub fn poll_archive(&mut self) -> bool {
        let changed = self.store.poll_archive();
        if changed && matches!(self.view, View::Archive) {
            self.recompute_visible();
            self.clamp_cursor();
        }
        changed
    }

    /// Index of the task under the cursor *into `self.tasks`*. Returns `None`
    /// in Archive view because the cursor there points into `archive.tasks()`.
    /// Use this — not `cur_abs()` — for any write that mutates `self.tasks`.
    pub fn cur_task_index_in_tasks(&self) -> Option<usize> {
        if matches!(self.view, View::Archive) {
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
        self.view
    }

    /// Switch top-level view. Recomputes the cache so the next frame reflects
    /// the change, snapshots the outgoing view's cursor, and restores the
    /// incoming view's saved cursor (clamped to the new visible length).
    pub fn set_view(&mut self, view: View) {
        if self.view == view {
            return;
        }
        self.view_cursor[self.view.idx()] = self.cursor;
        self.view = view;
        self.recompute_visible();
        self.cursor = self.view_cursor[view.idx()];
        self.clamp_cursor();
    }

    /// Set the search-filter text. Cursor resets and the cache is recomputed.
    /// Typing into the search prompt calls this on every keystroke.
    pub fn set_search(&mut self, search: String) {
        self.filter.search = search;
        self.cursor = 0;
        self.recompute_visible();
    }

    /// Clear just the search component of the filter.
    pub fn clear_search(&mut self) {
        if self.filter.search.is_empty() {
            return;
        }
        self.filter.search.clear();
        self.cursor = 0;
        self.recompute_visible();
    }

    /// Set or clear the active project filter. `None` removes it.
    pub fn set_project_filter(&mut self, project: Option<String>) {
        self.filter.project = project;
        self.cursor = 0;
        self.recompute_visible();
    }

    /// Set or clear the active context filter. `None` removes it.
    pub fn set_context_filter(&mut self, context: Option<String>) {
        self.filter.context = context;
        self.cursor = 0;
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
        self.cursor = 0;
        self.recompute_visible();
    }

    // ---- shared helpers for the mutation wrappers -----------------------

    /// After a successful mutation that returned an absolute task index,
    /// rebuild the visible cache and move the cursor to follow that task.
    pub(crate) fn after_mutation(&mut self, follow_abs: usize) {
        self.recompute_visible();
        self.follow_cursor(follow_abs);
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
            self.recompute_visible();
            self.clamp_cursor();
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
        self.saved_filters = new_cfg
            .filters
            .iter()
            .map(|(name, query)| SavedFilter {
                name: name.clone(),
                query: query.clone(),
            })
            .collect();
        self.week_start = new_cfg.week_start.unwrap_or(WeekStart::Sunday);
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

    // ---- timer helpers ----

    pub fn timer_running(&self) -> bool {
        self.store.timer_running()
    }

    pub fn timer_elapsed_secs(&self) -> Option<u64> {
        self.store.timer_elapsed_secs()
    }

    pub fn active_timer_task(&self) -> Option<&Task> {
        self.store.active_timer_task()
    }

    /// Idle nudge threshold from prefs.
    pub fn idle_nudge_seconds(&self) -> u64 {
        self.prefs.idle_nudge_seconds
    }

    /// Long-timer nudge threshold from prefs.
    pub fn long_timer_nudge_seconds(&self) -> u64 {
        self.prefs.long_timer_nudge_seconds
    }

    /// Check nudge conditions on each tick. Call from the event loop.
    /// - If no timer running and idle > threshold → enter IdleNudge mode.
    /// - If timer running and elapsed > threshold → set long_timer_nudge_active.
    ///   Returns true when the UI should redraw.
    pub fn check_nudges(&mut self) -> bool {
        // Don't nudge while user is in any non-Normal, non-Visual mode
        // (overlays, dialogs, reading help, etc.).
        if !matches!(self.mode, Mode::Normal | Mode::Visual) {
            self.long_timer_nudge_active = false;
            return false;
        }
        let idle_secs = self.last_timer_activity.elapsed().as_secs();
        if !self.timer_running() && idle_secs >= self.prefs.idle_nudge_seconds {
            self.mode = Mode::IdleNudge;
            return true;
        }
        let was_active = self.long_timer_nudge_active;
        if self.timer_running() {
            let elapsed = self.timer_elapsed_secs().unwrap_or(0);
            self.long_timer_nudge_active = elapsed >= self.prefs.long_timer_nudge_seconds;
        } else {
            self.long_timer_nudge_active = false;
        }
        was_active != self.long_timer_nudge_active
    }

    /// True when the active timer is running on the task at `abs`.
    pub fn is_timer_running_on(&self, abs: usize) -> bool {
        self.store.is_timer_running_on(abs)
    }

    /// Convert a `dur:VALUE` token in `text` from flexible user input (minutes,
    /// decimal hours, clock time, am/pm shorthand) to raw seconds suitable for
    /// the on-disk todo.txt format. Used by `add_from_draft` when
    /// `manual_time_entry` is set.
    pub fn convert_dur_in_text(&self, text: &str) -> String {
        let words: Vec<&str> = text.split_whitespace().collect();
        let mut result: Vec<String> = Vec::with_capacity(words.len());
        for word in &words {
            if let Some(rest) = word.strip_prefix("dur:") {
                if !rest.is_empty() {
                    let secs = parse_duration_input(rest);
                    result.push(format!("dur:{secs}"));
                } else {
                    result.push((*word).to_string());
                }
            } else {
                result.push((*word).to_string());
            }
        }
        result.join(" ")
    }

    /// Build the grouped time entries for the timesheet view. Returns a list
    /// of (key: "+project @activity", total_secs, narratives). The groups are
    /// sorted alphabetically by key (BTreeMap). Considers the weekly/daily
    /// toggle from `self.timesheet_weekly`.
    pub fn build_timesheet_groups(&self) -> Vec<(String, u64, Vec<String>)> {
        let today = self.today().to_string();
        let seven_days_ago = chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d")
            .ok()                .and_then(|d| d.checked_sub_days(chrono::Days::new(6)))
            .map(|d| d.format("%Y-%m-%d").to_string())
            .unwrap_or(today.clone());
        let mut groups: BTreeMap<String, (u64, Vec<String>)> = BTreeMap::new();
        for t in self.tasks().iter().filter(|t| t.dur.is_some_and(|d| d > 0)) {
            let Some(ref cd) = t.created_date else { continue; };
            let in_range = if self.timesheet_weekly {
                cd.as_str() >= seven_days_ago.as_str() && cd.as_str() <= today.as_str()
            } else {
                cd.as_str() == today.as_str()
            };
            if !in_range {
                continue;
            }
            let proj = t.projects.first().map(|p| format!("+{p}")).unwrap_or_default();
            let act = t.contexts.first().map(|a| format!("@{a}")).unwrap_or_default();
            let key = if proj.is_empty() && act.is_empty() {
                "(no project/activity)".to_string()
            } else {
                format!("{proj} {act}").trim().to_string()
            };
            let body = crate::todo::body_only(&t.raw);
            let entry = groups.entry(key).or_insert_with(|| (0, Vec::new()));
            entry.0 += t.dur.unwrap_or(0);
            entry.1.push(body);
        }
        groups.into_iter().map(|(k, (d, n))| (k, d, n)).collect()
    }

    /// Toggle timer on the task under the cursor (List view only).
    pub fn toggle_timer(&mut self) {
        let Some(abs) = self.cur_task_index_in_tasks() else {
            self.flash("no task selected");
            return;
        };
        match self.store.timer_toggle(abs) {
            TimerOutcome::Started { project, activity, body, .. } => {
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                self.flash(format!("▶ {proj}{act}— {body}"));
                self.last_timer_activity = Instant::now();
            }
            TimerOutcome::Stopped { elapsed_secs, total_secs, project, activity, body, .. } => {
                let elapsed = format_duration(elapsed_secs);
                let total = format_duration(total_secs);
                let proj = project.map(|p| format!("+{p} ")).unwrap_or_default();
                let act = activity.map(|a| format!("@{a} ")).unwrap_or_default();
                self.flash(format!("■ {proj}{act}{body} — {elapsed} (total {total})"));
                self.last_timer_activity = Instant::now();
            }
            TimerOutcome::Switched { from_elapsed_secs, from_total_secs, from_project, from_activity, to_project, to_activity, to_body, .. } => {
                let from_elapsed = format_duration(from_elapsed_secs);
                let from_total = format_duration(from_total_secs);
                let to_proj = to_project.map(|p| format!("+{p} ")).unwrap_or_default();
                let to_act = to_activity.map(|a| format!("@{a} ")).unwrap_or_default();
                let from_proj = from_project.map(|p| format!("+{p}")).unwrap_or_default();
                let from_act = from_activity.map(|a| format!("@{a}")).unwrap_or_default();
                self.flash(format!("■ {from_proj}{from_act} {from_elapsed} (total {from_total}) · ▶ {to_proj}{to_act} {to_body}"));
                self.last_timer_activity = Instant::now();
            }
            TimerOutcome::OutOfRange => self.flash("no task selected"),
            TimerOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            TimerOutcome::Error(e) => self.flash(format!("timer: {e}")),
        }
        self.recompute_visible();
    }

    /// Stop the running timer (if any) on quit.
    pub fn stop_timer_on_quit(&mut self) {
        match self.store.stop_timer_on_quit() {
            TimerQuitOutcome::Stopped { total_secs, .. } => {
                self.flash(format!("timer stopped ({} total)", format_duration(total_secs)));
            }
            TimerQuitOutcome::NoTimer => {}
            TimerQuitOutcome::Error(e) => {
                self.flash(format!("timer stop failed: {e}"));
            }
        }
    }
}

pub(crate) fn format_duration(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let billable = format_billable(total_secs);
    if hours > 0 {
        format!("{}h {}m ({billable})", hours, minutes)
    } else {
        format!("{}m ({billable})", minutes)
    }
}

/// Format seconds as billable units (0.1h increments, rounded up).
/// 1 minute = 0.1h, 6 minutes = 0.1h, 30 minutes = 0.5h, etc.
pub(crate) fn format_billable(total_secs: u64) -> String {
    // Round up to nearest 0.1 hour (6 minutes / 360 seconds).
    let tenths = total_secs.div_ceil(360);
    let whole = tenths / 10;
    let frac = tenths % 10;
    if whole > 0 || frac > 0 {
        format!("{}.{}h", whole, frac)
    } else {
        "0.0h".to_string()
    }
}

/// Parse a user-supplied duration string into seconds. Accepts:
/// - plain minutes (no suffix): `90` → 5400s (90 min)
/// - explicit minutes: `90m` → 5400s
/// - decimal hours: `1.5` or `1.5h` → 5400s
/// - explicit seconds: `5400s` → 5400s
/// - clock time: `14:30` → duration from that time today to now
/// - am/pm shorthand: `9am`, `2pm`, `9:30am`, `2:30pm` → duration from then to now
pub(crate) fn parse_duration_input(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }

    // Strip unit suffix to determine the base unit.
    let (num_part, explicit_unit) = if let Some(n) = s.strip_suffix('m') {
        (n.trim(), Some('m'))
    } else if let Some(n) = s.strip_suffix('h') {
        (n.trim(), Some('h'))
    } else if let Some(n) = s.strip_suffix('s') {
        (n.trim(), Some('s'))
    } else {
        (s, None)
    };

    // Clock time with am/pm: "9am", "9:30am", "2pm", "2:30pm"
    if let Some(secs) = parse_ampm_time(s) {
        return secs;
    }

    // Clock time: "14:30" or "9:30" (no am/pm)
    if let Some((h_str, m_str)) = num_part.split_once(':') {
            if let (Ok(h), Ok(m)) = (h_str.parse::<u32>(), m_str.parse::<u32>()) {
                let now = chrono::Local::now();
                let target_secs = h * 3600 + m * 60;
                let now_secs = now.hour() * 3600 + now.minute() * 60 + now.second();
                if target_secs <= now_secs {
                    return (now_secs - target_secs) as u64;
                }
                // Target is in the future — assume yesterday.
                return (now_secs + 24 * 3600 - target_secs) as u64;
            }
        return 0;
    }

    match explicit_unit {
        // Explicit seconds: "5400s"
        Some('s') => num_part.parse::<u64>().unwrap_or(0),
        // Explicit hours: "1.5h"
        Some('h') => {
            if let Ok(h) = num_part.parse::<f64>() {
                (h * 3600.0).max(0.0) as u64
            } else {
                0
            }
        }
        // Explicit minutes: "90m"
        Some('m') => num_part.parse::<u64>().map(|m| m * 60).unwrap_or(0),
        // No suffix: infer — decimal point → hours, plain integer → minutes
        None => {
            if num_part.contains('.') {
                if let Ok(h) = num_part.parse::<f64>() {
                    (h * 3600.0).max(0.0) as u64
                } else {
                    0
                }
            } else {
                // Plain integer → minutes (default for lawyers)
                num_part.parse::<u64>().map(|m| m * 60).unwrap_or(0)
            }
        }
        _ => 0,
    }
}

/// Parse am/pm clock shorthand like "9am", "2:30pm", "12p".
/// Returns the duration in seconds from that time (today, or yesterday if
/// the time is in the future) to now.
fn parse_ampm_time(s: &str) -> Option<u64> {
    let lower = s.trim().to_lowercase();
    let (time_part, is_pm) = if let Some(t) = lower.strip_suffix("am") {
        (t.trim(), false)
    } else if let Some(t) = lower.strip_suffix("pm") {
        (t.trim(), true)
    } else if let Some(t) = lower.strip_suffix('a') {
        (t.trim(), false)
    } else if let Some(t) = lower.strip_suffix('p') {
        (t.trim(), true)
    } else {
        return None;
    };

    let (hour, minute): (u32, u32) = if let Some((h, m)) = time_part.split_once(':') {
        (h.parse().ok()?, m.parse().ok()?)
    } else {
        (time_part.parse().ok()?, 0)
    };

    if hour > 12 || minute >= 60 {
        return None;
    }

    let hour_24 = match (hour, is_pm) {
        (12, false) => 0,   // 12am = midnight
        (12, true) => 12,   // 12pm = noon
        (h, true) if h < 12 => h + 12,
        (h, _) => h,
    };

    let now = chrono::Local::now();
    let target_secs = hour_24 * 3600 + minute * 60;
    let now_secs = now.hour() * 3600 + now.minute() * 60 + now.second();

    let diff = if target_secs <= now_secs {
        now_secs - target_secs
    } else {
        // Future — assume yesterday
        now_secs + 24 * 3600 - target_secs
    };
    Some(diff as u64)
}
