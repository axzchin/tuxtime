//! Theme picker state: cursor and original selection so cancel can restore.

#[derive(Debug)]
pub struct ThemePicker {
    /// Theme index captured when the theme picker opened, so cancel
    /// can restore it.
    pub orig: usize,
    /// Cursor position within the theme picker. Decoupled from
    /// `prefs.theme_idx` so that config reload (which replaces `prefs`)
    /// doesn't reset the picker position between j/k presses.
    pub cursor: usize,
}

impl ThemePicker {
    #[must_use]
    pub fn new(theme_idx: usize) -> Self {
        Self {
            orig: theme_idx,
            cursor: theme_idx,
        }
    }
}
