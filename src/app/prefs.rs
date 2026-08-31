use std::io;

use super::types::{Density, Sort};
use crate::config::Config;
use crate::theme::{self, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BadgeTheme {
    /// Use semantic colors: accent for duration and amber/due for DNB.
    #[default]
    Semantic,
    /// Use subdued theme-neutral colors for a quieter row layout.
    Monochrome,
    /// Bold colored text with no background chip at all.
    Plain,
}

impl BadgeTheme {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Monochrome => "monochrome",
            Self::Plain => "plain",
        }
    }

    #[must_use]
    pub fn from_config(value: Option<&str>) -> Self {
        match value {
            Some("monochrome") => Self::Monochrome,
            Some("plain") => Self::Plain,
            _ => Self::Semantic,
        }
    }
}

impl std::fmt::Display for BadgeTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

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
    /// Whether positive `dur:` values render as compact badges in task rows.
    pub show_duration_inline: bool,
    /// Prefill the new-task dialog with a leading `+` so the user can type
    /// the project name immediately. Default true.
    pub prefill_plus_new: bool,
    /// Where the edit dialog parks the cursor: `false` (default) at the end
    /// of the narrative so typing appends, `true` at the narrative's first
    /// word. Toggle in Settings with `e`.
    pub edit_cursor_narrative_start: bool,
    /// After completing a task with no time logged, open the add-time prompt
    /// so the finished work doesn't silently vanish from the timesheet
    /// (a completed task with no `dur:` never appears there). Default true;
    /// turn off for a plain, no-prompt completion flow.
    pub prompt_complete_no_time: bool,
    /// `Enter` in Normal mode: `false` (default) starts the timer on the
    /// selected task without ever stopping one (a second press is a no-op);
    /// `true` makes it a full toggle exactly like `t`. Toggle in Settings
    /// with `t`.
    pub enter_timer_toggle: bool,
    /// Whether the `log:YYYY-MM-DD` bookkeeping token is rendered inline.
    pub show_log_inline: bool,
    /// Palette used by duration and non-billable metadata chips.
    pub badge_theme: BadgeTheme,
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
    /// End-of-day review prompt time as `HH:MM` (e.g. `17:00`). `None`
    /// disables the wrap-up nudge.
    pub review_time: Option<String>,
    /// Workday start (`HH:MM`) — with `workday_end`, drives the unaccounted-
    /// time coverage line in the daily timesheet.
    pub workday_start: Option<String>,
    /// Workday end (`HH:MM`).
    pub workday_end: Option<String>,
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
            show_duration_inline: cfg.show_duration_inline.unwrap_or(true),
            prefill_plus_new: cfg.prefill_plus_new.unwrap_or(true),
            edit_cursor_narrative_start: cfg.edit_cursor_narrative_start.unwrap_or(false),
            prompt_complete_no_time: cfg.prompt_complete_no_time.unwrap_or(true),
            enter_timer_toggle: cfg.enter_timer_toggle.unwrap_or(false),
            show_log_inline: cfg.show_log_inline.unwrap_or(false),
            badge_theme: BadgeTheme::from_config(cfg.badge_theme.as_deref()),
            idle_nudge_seconds: cfg.idle_nudge_seconds.unwrap_or(900),
            long_timer_nudge_seconds: cfg.long_timer_nudge_seconds.unwrap_or(7200),
            prompt_on_day_boundary: cfg.prompt_on_day_boundary.unwrap_or(true),
            rounding_increment: cfg.rounding_increment.unwrap_or(0.1),
            review_time: cfg.review_time.clone(),
            workday_start: cfg.workday_start.clone(),
            workday_end: cfg.workday_end.clone(),
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

    /// Cycle through the badge palettes: colored chips → subdued chips →
    /// plain text → colored chips.
    pub fn cycle_badge_theme(&mut self) -> String {
        self.badge_theme = match self.badge_theme {
            BadgeTheme::Semantic => BadgeTheme::Monochrome,
            BadgeTheme::Monochrome => BadgeTheme::Plain,
            BadgeTheme::Plain => BadgeTheme::Semantic,
        };
        format!("badge theme: {}", self.badge_theme)
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

    /// Toggle the compact duration badge in list and archive rows.
    pub fn toggle_duration_inline(&mut self) {
        self.show_duration_inline = !self.show_duration_inline;
    }

    /// Toggle the new-task `+` prefill.
    pub fn toggle_prefill_plus_new(&mut self) {
        self.prefill_plus_new = !self.prefill_plus_new;
    }

    /// Toggle where the edit dialog parks its cursor (end vs start of the
    /// narrative). Returns the flash label for the caller to show.
    pub fn toggle_edit_cursor_narrative_start(&mut self) -> &'static str {
        self.edit_cursor_narrative_start = !self.edit_cursor_narrative_start;
        if self.edit_cursor_narrative_start {
            "edit cursor at narrative start"
        } else {
            "edit cursor at narrative end"
        }
    }

    /// Toggle the complete-without-time add-time prompt.
    pub fn toggle_prompt_complete_no_time(&mut self) {
        self.prompt_complete_no_time = !self.prompt_complete_no_time;
    }

    /// Toggle whether `Enter` toggles the timer (like `t`) or only starts it
    /// (never stops). Returns the flash label for the caller to show.
    pub fn toggle_enter_timer_toggle(&mut self) -> &'static str {
        self.enter_timer_toggle = !self.enter_timer_toggle;
        if self.enter_timer_toggle {
            "enter toggles timer"
        } else {
            "enter starts timer only"
        }
    }

    /// Toggle the `log:YYYY-MM-DD` bookkeeping token in list and archive rows.
    pub fn toggle_log_inline(&mut self) {
        self.show_log_inline = !self.show_log_inline;
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
            cfg.show_duration_inline = Some(self.show_duration_inline);
            cfg.prefill_plus_new = Some(self.prefill_plus_new);
            cfg.edit_cursor_narrative_start = Some(self.edit_cursor_narrative_start);
            cfg.prompt_complete_no_time = Some(self.prompt_complete_no_time);
            cfg.enter_timer_toggle = Some(self.enter_timer_toggle);
            cfg.show_log_inline = Some(self.show_log_inline);
            cfg.badge_theme = Some(self.badge_theme.as_str().to_string());
            cfg.week_start = Some(week_start);
            cfg.idle_nudge_seconds = Some(self.idle_nudge_seconds);
            cfg.long_timer_nudge_seconds = Some(self.long_timer_nudge_seconds);
            cfg.prompt_on_day_boundary = Some(self.prompt_on_day_boundary);
            cfg.rounding_increment = Some(self.rounding_increment);
            cfg.review_time = self.review_time.clone();
            cfg.workday_start = self.workday_start.clone();
            cfg.workday_end = self.workday_end.clone();
        })
    }
}
