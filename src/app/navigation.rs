//! Navigation state: the active view, mode, cursor position, and
//! associated per-view bookkeeping. Extracted from [`App`] to reduce
//! the God Object's field count and group related concerns.

use std::cell::Cell;

use super::types::{Mode, View};

/// All state related to view navigation: which view, in which mode,
/// at which cursor position, with per-view scroll offsets and saved
/// cursors. Handlers mutate these fields directly (`nav.mode = ...`)
/// while [`App`] provides thin facade methods (`view()`, `set_view()`)
/// that bridge to `recompute_visible` and other App-level concerns.
#[derive(Debug)]
pub struct Navigation {
    pub view: View,
    pub mode: Mode,
    pub cursor: usize,
    /// Per-view saved cursor, indexed by [`View::idx()`]. [`App::set_view`]
    /// snapshots the outgoing view's cursor here and restores the incoming
    /// view's, so each view remembers where the user last was.
    pub view_cursor: [usize; 3],
    /// Vertical scroll offset (rows from the top of the line list) for each
    /// view, keyed by [`View::idx()`]. Updated at render time via `Cell` so
    /// the renderer can keep the cursor row visible without taking `&mut self`.
    pub view_scroll: [Cell<u16>; 3],
    pub should_quit: bool,
    /// The mode to return to when Search is dismissed (Enter/Esc). Set by
    /// overlays like `ManageProjects` that enter Search for inline filtering.
    /// `None` means the caller didn't override — `handle_search` falls back
    /// to [`Mode::Normal`].
    pub pre_search_mode: Option<Mode>,
    /// The mode to return to when a nudge prompt (`PromptIdleNudge` /
    /// `PromptLongTimerNudge`) completes via Enter or Esc. Set to
    /// `Some(Mode::Settings)` by `handle_settings` (the primary entry
    /// point); left as `None` when triggered from the command palette,
    /// so `handle_prompt` falls back to `Mode::Normal`.
    pub nudge_prompt_return: Option<Mode>,
}

impl Navigation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            view: View::List,
            mode: Mode::Normal,
            cursor: 0,
            view_cursor: [0; 3],
            view_scroll: [Cell::new(0), Cell::new(0), Cell::new(0)],
            should_quit: false,
            pre_search_mode: None,
            nudge_prompt_return: None,
        }
    }

    /// Read-only accessor for the active view.
    #[must_use]
    pub fn view(&self) -> View {
        self.view
    }

    // ---- Behavioral API (prefer these over raw field access) ----

    /// Current mode.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Is the user in Normal or Visual mode?
    #[must_use]
    pub fn is_normal_or_visual(&self) -> bool {
        matches!(self.mode, Mode::Normal | Mode::Visual)
    }

    /// Is the user in Visual mode?
    #[must_use]
    pub fn is_visual(&self) -> bool {
        self.mode == Mode::Visual
    }

    /// Current cursor position.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Set the mode. Prefer [`enter_normal`], [`enter_visual`], etc. for
    /// common transitions; use this for programmatic mode assignment.
    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    /// Transition to Normal mode.
    pub fn enter_normal(&mut self) {
        self.mode = Mode::Normal;
    }

    /// Transition to Visual mode.
    pub fn enter_visual(&mut self) {
        self.mode = Mode::Visual;
    }

    /// Set the cursor to an absolute position.
    pub fn set_cursor(&mut self, pos: usize) {
        self.cursor = pos;
    }

    /// Move cursor down, clamped to `max` (0-indexed last valid position).
    pub fn move_down(&mut self, max: usize) {
        if max > 0 {
            self.cursor = (self.cursor + 1).min(max);
        }
    }

    /// Move cursor up (saturating at 0).
    pub fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Move cursor to top.
    pub fn move_top(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to `max` (last valid index).
    pub fn move_bottom(&mut self, max: usize) {
        if max > 0 {
            self.cursor = max;
        }
    }

    /// Move cursor down by `n` rows, clamped.
    pub fn move_down_by(&mut self, n: usize, max: usize) {
        self.cursor = (self.cursor + n).min(max.saturating_sub(1));
    }

    /// Move cursor up by `n` rows, saturating at 0.
    pub fn move_up_by(&mut self, n: usize) {
        self.cursor = self.cursor.saturating_sub(n);
    }

    /// Signal that the app should quit on the next loop iteration.
    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Save the current cursor into the per-view cursor array for `view`,
    /// then restore the cursor saved for the new view.
    pub fn switch_view(&mut self, old_view: View, new_view: View) {
        self.view_cursor[old_view.idx()] = self.cursor;
        self.view = new_view;
        self.cursor = self.view_cursor[new_view.idx()];
    }

    /// Toggle between Visual and Normal mode.
    pub fn toggle_visual(&mut self) {
        self.mode = if self.mode == Mode::Visual {
            Mode::Normal
        } else {
            Mode::Visual
        };
    }
}

impl Default for Navigation {
    fn default() -> Self {
        Self::new()
    }
}
