//! Theme picker state: cursor and original selection so cancel can restore,
//! plus the App methods that drive the picker (enter/step/accept/cancel).
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

use super::App;
use crate::app::Mode;
use crate::theme;

impl App {
    /// Enter theme picker mode. Snapshot the current theme index so
    /// cancel can restore it. j/k live-previews; Enter accepts; Esc
    /// reverts. Uses `theme_pick_cursor` (on App) so that config reload
    /// cannot reset the picker position.
    pub fn enter_pick_theme(&mut self) {
        self.theme_picker.orig = self.prefs.theme_idx();
        self.theme_picker.cursor = self.theme_picker.orig;
        self.nav.set_mode(Mode::PickTheme);
    }

    /// Step through themes in `forward` (true = next) direction with
    /// wrap-around. Uses `theme_pick_cursor` so config reload cannot
    /// strand the picker. The preview is applied to `prefs` for live
    /// theme switching; if config reload resets prefs the cursor survives.
    pub fn pick_theme_step(&mut self, forward: bool) {
        let all = theme::all();
        let len = all.len();
        if len <= 1 {
            return;
        }
        let cur = self.theme_picker.cursor;
        let next = if forward {
            (cur + 1) % len
        } else {
            (cur + len - 1) % len
        };
        self.theme_picker.cursor = next;
        self.prefs.set_theme_idx(next); // live preview
    }

    /// Accept the previewed theme and persist to config.
    pub fn pick_theme_accept(&mut self) {
        self.prefs.set_theme_idx(self.theme_picker.cursor);
        self.nav.enter_normal();
        self.save_prefs();
        self.flash(format!("theme: {}", self.theme().name));
    }

    /// Cancel the picker and restore the theme that was active when
    /// the picker opened.
    pub fn pick_theme_cancel(&mut self) {
        self.prefs.set_theme_idx(self.theme_picker.orig);
        self.nav.enter_normal();
    }
}
