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
}

impl Default for Navigation {
    fn default() -> Self {
        Self::new()
    }
}
