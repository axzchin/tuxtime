//! Navigation state: the active view, mode, cursor position, and
//! associated per-view bookkeeping. Extracted from [`App`] to reduce
//! the God Object's field count and group related concerns.

use std::cell::Cell;

use super::types::{Mode, Screen, View, ViewMap};

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
    /// Per-view saved cursor, keyed by `View`. [`App::set_view`] snapshots
    /// the outgoing view's cursor here and restores the incoming view's, so
    /// each view remembers where the user last was.
    pub view_cursor: ViewMap<usize>,
    /// Vertical scroll offset (rows from the top of the line list) for each
    /// view, keyed by `View`. Updated at render time via `Cell` so the
    /// renderer can keep the cursor row visible without taking `&mut self`.
    pub view_scroll: ViewMap<Cell<u16>>,
    pub should_quit: bool,
    /// Return-path stack for modal overlays. [`push_mode`](Self::push_mode)
    /// saves the current mode and enters a new one; [`pop_mode`](Self::pop_mode)
    /// restores the most recently saved mode (falling back to [`Mode::Screen(Screen::Normal)`]
    /// on an empty stack). Replaces the hand-rolled `pre_search_mode` /
    /// `nudge_prompt_return` `Option<Mode>` fields — overlays no longer
    /// reimplement "where do I go back to" with bespoke fields.
    mode_stack: Vec<Mode>,
}

impl Navigation {
    #[must_use]
    pub fn new() -> Self {
        Self {
            view: View::List,
            mode: Mode::Screen(Screen::Normal),
            cursor: 0,
            view_cursor: ViewMap::default(),
            view_scroll: ViewMap::default(),
            should_quit: false,
            mode_stack: Vec::new(),
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
        matches!(
            self.mode,
            Mode::Screen(Screen::Normal) | Mode::Screen(Screen::Visual)
        )
    }

    /// Is the user in Visual mode?
    #[must_use]
    pub fn is_visual(&self) -> bool {
        self.mode == Mode::Screen(Screen::Visual)
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

    /// Enter a modal overlay, remembering the current mode so `pop_mode` can
    /// restore it on dismissal. The stack replaces bespoke `Option<Mode>`
    /// return-path fields: any overlay that returns to its caller opens with
    /// `push_mode` and closes with `pop_mode`.
    pub fn push_mode(&mut self, mode: Mode) {
        self.mode_stack.push(self.mode);
        self.mode = mode;
    }

    /// Leave a modal overlay entered with [`push_mode`](Self::push_mode),
    /// restoring the saved mode (or [`Mode::Screen(Screen::Normal)`] on an empty stack).
    pub fn pop_mode(&mut self) -> Mode {
        self.mode = self
            .mode_stack
            .pop()
            .unwrap_or(Mode::Screen(Screen::Normal));
        self.mode
    }

    /// The mode beneath the current overlay, if the overlay was entered with
    /// [`push_mode`](Self::push_mode). The command palette uses this to keep
    /// rendering the underlying UI in the mode it came from (e.g. Visual
    /// while the palette overlays it).
    #[must_use]
    pub fn peek_under(&self) -> Option<Mode> {
        self.mode_stack.last().copied()
    }

    /// Transition to Normal mode.
    pub fn enter_normal(&mut self) {
        self.mode = Mode::Screen(Screen::Normal);
    }

    /// Transition to Visual mode.
    pub fn enter_visual(&mut self) {
        self.mode = Mode::Screen(Screen::Visual);
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

    /// Save the current cursor into the per-view cursor map for `view`,
    /// then restore the cursor saved for the new view.
    pub fn switch_view(&mut self, old_view: View, new_view: View) {
        self.view_cursor[old_view] = self.cursor;
        self.view = new_view;
        self.cursor = self.view_cursor[new_view];
    }

    /// Toggle between Visual and Normal mode.
    pub fn toggle_visual(&mut self) {
        self.mode = if self.mode == Mode::Screen(Screen::Visual) {
            Mode::Screen(Screen::Normal)
        } else {
            Mode::Screen(Screen::Visual)
        };
    }
}

impl Default for Navigation {
    fn default() -> Self {
        Self::new()
    }
}
