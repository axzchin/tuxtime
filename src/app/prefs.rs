use std::io;

use super::types::{Density, Sort};
use crate::config::Config;
use crate::theme::{self, Theme};

#[derive(Debug, Clone)]
pub struct Layout {
    pub left: bool,
    pub right: bool,
    pub line_num: bool,
    pub status_bar: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            left: true,
            right: true,
            line_num: true,
            status_bar: true,
        }
    }
}

/// User-tunable preferences persisted to `Config`. Cycle/toggle methods return
/// the flash message for the caller to display, sidestepping any `&mut prefs`
/// + `&mut flash_state` borrow tangle on `App`.
#[derive(Debug, Clone)]
pub struct Prefs {
    theme_idx: usize,
    pub density: Density,
    pub sort: Sort,
    pub layout: Layout,
    pub show_done: bool,
    pub show_future: bool,
    /// Metadata keys whose `key:value` tokens are hidden from task rows.
    /// Config-only (no in-app toggle); see `Config::hidden_keys`.
    pub hidden_keys: Vec<String>,
    /// Seconds of inactivity before nudging the user to start a timer.
    pub idle_nudge_seconds: u64,
    /// Seconds a timer can run before nudging the user.
    pub long_timer_nudge_seconds: u64,
    /// Ask before starting a timer (or adding time) on a task whose time
    /// belongs to a previous day (carry-forward prompt). Default true.
    pub prompt_on_day_boundary: bool,
    /// Billable rounding increment in decimal hours: `0.1` (6 min, default),
    /// `0.25` (15 min), or `0` for no rounding (exact decimal hours).
    pub rounding_increment: f64,
}

impl Prefs {
    #[must_use]
    pub fn from_config(cfg: Config) -> Self {
        let theme_idx = cfg
            .theme
            .as_deref()
            .and_then(|name| theme::all().iter().position(|t| t.name == name))
            .unwrap_or(0);
        Self {
            theme_idx,
            density: cfg.density.unwrap_or(Density::Comfortable),
            sort: cfg.sort.unwrap_or(Sort::Priority),
            layout: Layout {
                left: cfg.show_left.unwrap_or(true),
                right: cfg.show_right.unwrap_or(true),
                line_num: cfg.show_line_num.unwrap_or(true),
                status_bar: cfg.show_status_bar.unwrap_or(true),
            },
            show_done: cfg.show_done.unwrap_or(false),
            show_future: cfg.show_future.unwrap_or(false),
            hidden_keys: cfg.hidden_keys,
            idle_nudge_seconds: cfg.idle_nudge_seconds.unwrap_or(900),
            long_timer_nudge_seconds: cfg.long_timer_nudge_seconds.unwrap_or(7200),
            prompt_on_day_boundary: cfg.prompt_on_day_boundary.unwrap_or(true),
            rounding_increment: cfg.rounding_increment.unwrap_or(0.1),
        }
    }

    #[must_use]
    pub fn theme(&self) -> &'static Theme {
        let all = theme::all();
        all[self.theme_idx % all.len()]
    }

    #[must_use]
    pub fn theme_idx(&self) -> usize {
        self.theme_idx
    }

    /// Jump directly to a specific theme by index. Used by the screenshot
    /// example to render every theme; production code should call
    /// `cycle_theme` instead so the change persists with a flash message.
    pub fn set_theme_idx(&mut self, idx: usize) {
        self.theme_idx = idx % theme::all().len();
    }

    #[must_use]
    pub fn sort_label(&self) -> &'static str {
        self.sort.as_str()
    }

    pub fn cycle_theme(&mut self) -> String {
        self.theme_idx = (self.theme_idx + 1) % theme::all().len();
        format!("theme: {}", self.theme().name)
    }

    pub fn cycle_density(&mut self) -> String {
        self.density = match self.density {
            Density::Compact => Density::Comfortable,
            Density::Comfortable => Density::Cozy,
            Density::Cozy => Density::Compact,
        };
        format!("density: {}", self.density)
    }

    /// Cycle the billable rounding increment: `0.1h → 0.25h → exact → 0.1h`.
    /// Returns the flash message with the new label. The f64 comparisons are
    /// safe because the value always comes from one of the three literals
    /// below (or the config default), never from computation.
    pub fn cycle_rounding_increment(&mut self) -> String {
        self.rounding_increment = match self.rounding_increment {
            x if x <= 0.0 => 0.1,
            x if (x - 0.1).abs() < f64::EPSILON => 0.25,
            _ => 0.0,
        };
        format!(
            "rounding: {}",
            crate::app::rounding_increment_label(self.rounding_increment)
        )
    }

    pub fn cycle_sort(&mut self) -> String {
        self.sort = match self.sort {
            Sort::Priority => Sort::Due,
            Sort::Due => Sort::File,
            Sort::File => Sort::Priority,
        };
        format!("sort: {}", self.sort)
    }

    pub fn toggle_left(&mut self) {
        self.layout.left = !self.layout.left;
    }

    pub fn toggle_right(&mut self) {
        self.layout.right = !self.layout.right;
    }

    pub fn toggle_line_num(&mut self) {
        self.layout.line_num = !self.layout.line_num;
    }

    pub fn toggle_show_done(&mut self) {
        self.show_done = !self.show_done;
    }

    pub fn toggle_show_future(&mut self) {
        self.show_future = !self.show_future;
    }

    /// Persist to the XDG config path. Returns the IO error so the caller
    /// can flash it (writing to stderr from inside the alt-screen would
    /// corrupt the TUI). Saving is best-effort — callers that don't care
    /// about reporting can `let _ = prefs.save();`.
    ///
    /// Runs as one atomic `Config::update` under an advisory lock: the
    /// on-disk config is re-read just before writing, so non-pref fields
    /// (like `share_token` / `share_port`, owned by the capture server) and
    /// prefs changed concurrently by another instance survive instead of
    /// being clobbered by a whole-file rewrite from a stale read.
    /// `week_start` lives on `App::env`, not on `Prefs`, so callers must pass
    /// it in so it is persisted alongside the other preferences.
    pub fn save(&self, week_start: crate::app::WeekStart) -> io::Result<()> {
        Config::update(|cfg| {
            cfg.theme = Some(self.theme().name.to_string());
            cfg.density = Some(self.density);
            cfg.sort = Some(self.sort);
            cfg.show_left = Some(self.layout.left);
            cfg.show_right = Some(self.layout.right);
            cfg.show_line_num = Some(self.layout.line_num);
            cfg.show_status_bar = Some(self.layout.status_bar);
            cfg.show_done = Some(self.show_done);
            cfg.show_future = Some(self.show_future);
            cfg.hidden_keys = self.hidden_keys.clone();
            cfg.week_start = Some(week_start);
            cfg.idle_nudge_seconds = Some(self.idle_nudge_seconds);
            cfg.long_timer_nudge_seconds = Some(self.long_timer_nudge_seconds);
            cfg.prompt_on_day_boundary = Some(self.prompt_on_day_boundary);
            cfg.rounding_increment = Some(self.rounding_increment);
        })
    }
}
