//! Timesheet view: groups tasks with duration by project+activity over a date
//! range (daily or weekly), renders them sorted by project, date, or duration,
//! and provides a calendar picker for jumping between dates.
//!
//! # Architecture
//!
//! [`TimesheetState`] owns the transient navigation state (anchor date, cursor,
//! sort, calendar focus, groups cache, copy flash). It exposes pure methods
//! that don't need [`App`] context. Methods needing `App` context (`today()`,
//! `flash()`, `tasks()`, `filter`, `week_start`) stay on `App` and access
//! `self.timesheet` directly.
//!
//! Thin delegating wrappers on `App` (e.g. `timesheet_shift_days`) exist
//! only where the borrow checker would otherwise reject a call like
//! `app.timesheet.shift_days(app.today())`. When the wrapper would just be a
//! one-line pass-through, callers use `app.timesheet.x()` directly instead.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::Instant;

use chrono::{Datelike, NaiveDate, Weekday};

use super::{
    App, FLASH_TTL, Mode, Screen, TimesheetEntry, TimesheetSort, TimesheetTaskRef, WeekStart,
    billable_units,
};

// ---------------------------------------------------------------------------
// TimesheetState — pure navigation state
// ---------------------------------------------------------------------------

/// All mutable timesheet-related state, extracted from [`App`] to keep the
/// field count manageable. Fields are public so `App` and renderers can
/// mutate them directly; methods that need `App` context stay on `App`.
#[derive(Debug)]
pub struct TimesheetState {
    /// Weekly (true) or daily (false, default) view.
    pub weekly: bool,
    /// Cursor: index into the project+activity narrative list.
    pub cursor: usize,
    /// Sort mode: by project, date, or duration.
    pub sort: TimesheetSort,
    /// Show only non-billable (`DNB`) entries in the timesheet.
    pub dnb_only: bool,
    /// Anchor date (ISO "YYYY-MM-DD"). Defaults to today.
    pub date: String,
    /// Focus date for the calendar picker (`PickTimesheetDate` mode).
    pub calendar_focus: NaiveDate,
    /// Typed input buffer for the calendar picker.
    pub date_input: String,
    /// Vim-style copy flash: `(group_idx, set_time)`.
    pub copy_flash: Option<(usize, Instant)>,
    /// Cached result of [`App::build_timesheet_groups`]. `RefCell` lets
    /// the getter (which takes `&self`) return a clone without recomputing.
    pub groups_cache: RefCell<Option<Vec<TimesheetEntry>>>,
}

impl TimesheetState {
    #[must_use]
    pub fn new(today: &str) -> Self {
        Self {
            weekly: false,
            cursor: 0,
            sort: TimesheetSort::ProjectActivity,
            dnb_only: false,
            date: today.to_string(),
            calendar_focus: NaiveDate::parse_from_str(today, "%Y-%m-%d")
                .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2026, 1, 1).expect("static")),
            date_input: String::new(),
            copy_flash: None,
            groups_cache: RefCell::new(None),
        }
    }

    /// Invalidate the groups cache. Call after any task/archive/filter change
    /// that affects the timesheet view.
    pub fn invalidate_cache(&mut self) {
        self.groups_cache.borrow_mut().take();
    }

    /// Parse `date` to a [`NaiveDate`], falling back to `today`.
    pub fn date_naive(&self, today: &str) -> NaiveDate {
        NaiveDate::parse_from_str(&self.date, "%Y-%m-%d")
            .unwrap_or_else(|_| NaiveDate::parse_from_str(today, "%Y-%m-%d").unwrap_or_default())
    }

    /// Human-readable display form, e.g. "Mon 2026-08-03".
    pub fn date_display(&self, today: &str) -> String {
        self.date_naive(today).format("%a %Y-%m-%d").to_string()
    }

    /// Shift the anchor by `delta` days (negative = backward).
    pub fn shift_days(&mut self, today: &str, delta: i64) {
        if let Some(d) = self
            .date_naive(today)
            .checked_add_signed(chrono::TimeDelta::days(delta))
        {
            self.date = d.format("%Y-%m-%d").to_string();
            self.cursor = 0;
            self.invalidate_cache();
        }
    }

    /// Reset the anchor to `today`.
    pub fn goto_today(&mut self, today: &str) {
        self.date = today.to_string();
        self.cursor = 0;
        self.invalidate_cache();
    }

    /// Handle a typing key while the calendar picker is open.
    pub fn date_type(&mut self, code: ratatui::crossterm::event::KeyCode) {
        match code {
            ratatui::crossterm::event::KeyCode::Char(c) if c.is_ascii_digit() || c == '-' => {
                if self.date_input.len() < 10 {
                    self.date_input.push(c);
                    self.sync_input();
                }
            }
            ratatui::crossterm::event::KeyCode::Backspace => {
                self.date_input.pop();
                self.sync_input();
            }
            _ => {}
        }
    }

    fn sync_input(&mut self) {
        if let Ok(d) = NaiveDate::parse_from_str(&self.date_input, "%Y-%m-%d") {
            self.calendar_focus = d;
        }
    }

    /// Move the calendar focus by dx columns + dy rows (week rows).
    pub fn calendar_move(&mut self, dx: i32, dy: i32) {
        let total_days = dx + dy * 7;
        let next = if total_days >= 0 {
            self.calendar_focus
                .checked_add_days(chrono::Days::new(total_days as u64))
        } else {
            self.calendar_focus
                .checked_sub_days(chrono::Days::new(u64::from(total_days.unsigned_abs())))
        };
        if let Some(d) = next {
            self.calendar_focus = d;
        }
    }

    /// Set calendar focus to `today + days`.
    pub fn calendar_set_relative(&mut self, today: &str, days: i64) {
        let parsed = NaiveDate::parse_from_str(today, "%Y-%m-%d").unwrap_or(self.calendar_focus);
        if let Some(d) = parsed.checked_add_signed(chrono::TimeDelta::days(days)) {
            self.calendar_focus = d;
        }
    }

    /// Clear `copy_flash` when its TTL has expired.
    pub fn clear_stale_copy_flash(&mut self) {
        if self
            .copy_flash
            .is_some_and(|(_, t)| t.elapsed() >= FLASH_TTL)
        {
            self.copy_flash = None;
        }
    }
}

// ---------------------------------------------------------------------------
// App — timesheet methods (needs App context)
// ---------------------------------------------------------------------------

impl App {
    /// Build the grouped time entries for the timesheet view. Returns entries
    /// sorted according to `self.timesheet.sort`. Groups by (date,
    /// project+activity, billable) so weekly view properly separates days.
    /// Uses `self.timesheet.date` as the anchor (defaults to today). For
    /// weekly view, computes the week containing that date according to
    /// `self.env.week_start`. Result is cached in `self.timesheet.groups_cache`.
    pub fn build_timesheet_groups(&self) -> Vec<TimesheetEntry> {
        if let Some(ref cached) = *self.timesheet.groups_cache.borrow() {
            return cached.clone();
        }
        let anchor = &self.timesheet.date;
        // Compute the date range [range_start, range_end] based on view mode.
        let (range_start, range_end) = if self.timesheet.weekly {
            let anchor_date = NaiveDate::parse_from_str(anchor, "%Y-%m-%d").unwrap_or_else(|_| {
                NaiveDate::parse_from_str(self.today(), "%Y-%m-%d").unwrap_or_default()
            });
            let target_weekday = match self.env.week_start {
                WeekStart::Monday => Weekday::Mon,
                WeekStart::Sunday => Weekday::Sun,
            };
            let days_from_target = (7 + anchor_date.weekday().num_days_from_monday()
                - target_weekday.num_days_from_monday())
                % 7;
            let week_start_date = anchor_date
                .checked_sub_days(chrono::Days::new(u64::from(days_from_target)))
                .unwrap_or(anchor_date);
            let week_end_date = week_start_date
                .checked_add_days(chrono::Days::new(6))
                .unwrap_or(week_start_date);
            (
                week_start_date.format("%Y-%m-%d").to_string(),
                week_end_date.format("%Y-%m-%d").to_string(),
            )
        } else {
            (anchor.clone(), anchor.clone())
        };
        type TimesheetGroup = (u64, Vec<String>, Vec<TimesheetTaskRef>);
        let search = self.filter.search.to_lowercase();
        let dnb_only = self.timesheet.dnb_only;
        // Hoisted filter values: the closure below borrows `self` at call
        // sites (via `self.tasks()`), so capture the filter components as
        // owned locals to keep the closure borrowing `self` immutably.
        let project_filter = self.filter.project.clone();
        let context_filter = self.filter.context.clone();
        // Key by (date, project+activity, billable) so weekly view keeps
        // each day's entries in separate groups.
        let mut groups: BTreeMap<(String, String, bool), TimesheetGroup> = BTreeMap::new();

        let mut process = |tasks: &[crate::todo::Task], make_ref: fn(usize) -> TimesheetTaskRef| {
            for (idx, t) in tasks
                .iter()
                .enumerate()
                .filter(|(_, t)| t.dur.is_some_and(|d| d > 0))
            {
                let Some(cd) = t.created_date.as_deref() else {
                    continue;
                };
                // Attribute time to the day it was actually tracked (`log:`)
                // when the line carries a valid one, so work done on a later
                // day than the task's creation shows up on the right day.
                // Lines written before the log tag existed, entered by hand,
                // or carrying an unparseable value fall back to the creation
                // date — an invalid log must never make billable time vanish.
                let work_date = match t.log.as_deref() {
                    Some(l) if is_log_date(l) => l,
                    _ => cd,
                };
                let in_range = work_date >= range_start.as_str() && work_date <= range_end.as_str();
                if !in_range {
                    continue;
                }
                // Honor the active project/context filters, matching the
                // list view's exact-match semantics (`passes_user_filter`)
                // so filtering the timesheet feels like filtering the list.
                if let Some(p) = &project_filter
                    && !t.projects.iter().any(|x| x == p)
                {
                    continue;
                }
                if let Some(c) = &context_filter
                    && !t.contexts.iter().any(|x| x == c)
                {
                    continue;
                }
                let body = crate::todo::body_only_from_clean(&t.clean_raw);
                if !search.is_empty() && !body.to_lowercase().contains(&search) {
                    continue;
                }
                let proj = t
                    .projects
                    .first()
                    .map(|p| format!("+{p}"))
                    .unwrap_or_default();
                let act = t
                    .contexts
                    .first()
                    .map(|a| format!("@{a}"))
                    .unwrap_or_default();
                let key = if proj.is_empty() && act.is_empty() {
                    "(no project/activity)".to_string()
                } else {
                    format!("{proj} {act}").trim().to_string()
                };
                let billable = t.bill.as_deref() != Some("n");
                if dnb_only && billable {
                    continue;
                }
                let entry = groups
                    .entry((work_date.to_string(), key.clone(), billable))
                    .or_insert_with(|| (0, Vec::new(), Vec::new()));
                entry.0 += t.dur.unwrap_or(0);
                entry.1.push(body);
                entry.2.push(make_ref(idx));
            }
        };

        process(self.tasks(), TimesheetTaskRef::Active);
        process(self.store.archive().tasks(), TimesheetTaskRef::Archived);

        let mut entries: Vec<TimesheetEntry> = groups
            .into_iter()
            .map(
                |((date, key, billable), (total_secs, narratives, task_indices))| TimesheetEntry {
                    date,
                    key,
                    total_secs,
                    narratives,
                    task_indices,
                    billable,
                },
            )
            .collect();
        match self.timesheet.sort {
            TimesheetSort::ProjectActivity => {
                entries.sort_by(|a, b| a.key.cmp(&b.key).then_with(|| a.date.cmp(&b.date)));
            }
            TimesheetSort::Date => {
                entries.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.key.cmp(&b.key)));
            }
            TimesheetSort::Duration => {
                entries.sort_by(|a, b| b.total_secs.cmp(&a.total_secs));
            }
        }
        *self.timesheet.groups_cache.borrow_mut() = Some(entries.clone());
        entries
    }
}

impl App {
    /// Per-project time totals for the timesheet's current period, as
    /// `(project, billable_secs, non_billable_secs)`, sorted by billable
    /// hours descending then name. Drives the left sidebar in timesheet
    /// view: a lawyer's billing snapshot per matter. Respects the active
    /// filters, so narrowing the timesheet also narrows the sidebar.
    #[must_use]
    pub fn timesheet_project_totals(&self) -> Vec<(String, u64, u64)> {
        let groups = self.build_timesheet_groups();
        let mut map: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for g in &groups {
            // Group key is "+Smith @drafting" or "(no project/activity)";
            // the project is the first whitespace token starting with '+'
            // (a context-only group has no project to attribute time to).
            let proj = g
                .key
                .split_whitespace()
                .find(|t| t.starts_with('+'))
                .map_or_else(
                    || "(no project)".to_string(),
                    |t| t.trim_start_matches('+').to_string(),
                );
            let entry = map.entry(proj).or_insert((0, 0));
            if g.billable {
                entry.0 += g.total_secs;
            } else {
                entry.1 += g.total_secs;
            }
        }
        let mut totals: Vec<(String, u64, u64)> = map
            .into_iter()
            .map(|(name, (billable, non_billable))| (name, billable, non_billable))
            .collect();
        totals.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        totals
    }

    /// Workday coverage for the timesheet's anchor day: how much of the
    /// configured workday (`workday_start` → `workday_end`) the tracked
    /// seconds account for. Returns `(span_secs, unaccounted_secs,
    /// in_progress)` — `in_progress` is true when the anchor is today and the
    /// clock hasn't reached `workday_end` yet, so the renderer can say
    /// "day in progress" instead of implying the day is done. `None` when
    /// the workday bounds aren't configured (or are malformed/inverted).
    /// `now_min`/`now_date` are passed in so tests stay deterministic.
    #[must_use]
    pub fn workday_coverage(
        &self,
        anchor_date: &str,
        tracked_secs: u64,
        now_min: u32,
        now_date: &str,
    ) -> Option<(u64, u64, bool)> {
        let start = crate::app::parse_clock(self.prefs.workday_start.as_deref()?)?;
        let end = crate::app::parse_clock(self.prefs.workday_end.as_deref()?)?;
        let start_min = start.0 * 60 + start.1;
        let end_min = end.0 * 60 + end.1;
        if end_min <= start_min {
            return None;
        }
        let span_secs = u64::from(end_min - start_min) * 60;
        let unaccounted = span_secs.saturating_sub(tracked_secs);
        let in_progress = anchor_date == now_date && now_min < end_min;
        Some((span_secs, unaccounted, in_progress))
    }

    /// Totals for the timesheet's current period: `(total_secs,
    /// billable_secs, non_billable_secs)`. Pinned at the top of the detail
    /// sidebar so the numbers stay visible while the center scrolls.
    #[must_use]
    pub fn timesheet_period_totals(&self) -> (u64, u64, u64) {
        timesheet_totals(
            &self.build_timesheet_groups(),
            self.prefs.rounding_increment,
        )
        .secs()
    }
}

/// Aggregated totals for a list of timesheet groups: tracked seconds and
/// billable units, each split billable vs non-billable. Pure over an
/// already-built group list so the renderer can pass its local `groups`
/// without re-cloning the cache (unlike the [`App::timesheet_period_totals`]
/// accessors, which rebuild groups from the cache).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TimesheetTotals {
    pub(crate) total_secs: u64,
    pub(crate) billable_secs: u64,
    pub(crate) non_billable_secs: u64,
    pub(crate) total_units: u64,
    pub(crate) billable_units: u64,
    pub(crate) non_billable_units: u64,
}

impl TimesheetTotals {
    /// Seconds triple: `(total, billable, non_billable)`.
    fn secs(self) -> (u64, u64, u64) {
        (self.total_secs, self.billable_secs, self.non_billable_secs)
    }
}

/// Sum a group list's seconds and billable units, split billable vs
/// non-billable. Billable units round per group (1 min × 5 matters = 0.5h at
/// the 0.1 increment), matching the footer the renderer prints.
#[must_use]
pub(crate) fn timesheet_totals(groups: &[TimesheetEntry], increment: f64) -> TimesheetTotals {
    let mut t = TimesheetTotals::default();
    for g in groups {
        let units = billable_units(g.total_secs, increment);
        t.total_secs += g.total_secs;
        t.total_units += units;
        if g.billable {
            t.billable_secs += g.total_secs;
            t.billable_units += units;
        } else {
            t.non_billable_secs += g.total_secs;
            t.non_billable_units += units;
        }
    }
    t
}

/// True when `s` is a parseable `YYYY-MM-DD` date. Guards the timesheet
/// against hand-typed `log:` garbage, which would otherwise put a
/// non-comparable "date" on every entry and hide it from all ranges.
/// Delegates to the shared validator so the fallback semantics can't drift
/// from the day-boundary prompt's effective-log-date check.
fn is_log_date(s: &str) -> bool {
    crate::todo::is_iso_date(s)
}

impl App {
    // ---- date navigation (thin wrappers: borrow checker needs App.today()) ----

    /// Shift the timesheet anchor by `delta` days (negative = backward).
    pub fn timesheet_shift_days(&mut self, delta: i64) {
        let today = self.today().to_string();
        self.timesheet.shift_days(&today, delta);
    }

    /// Reset the timesheet anchor to today.
    pub fn timesheet_goto_today(&mut self) {
        let today = self.today().to_string();
        self.timesheet.goto_today(&today);
    }

    /// Human-readable display form of `timesheet.date`, e.g. "Mon 2026-08-03".
    pub fn timesheet_date_display(&self) -> String {
        self.timesheet.date_display(self.today())
    }

    // ---- narrative helpers ----

    /// Total number of narratives across all timesheet groups. Used to bound
    /// the narrative-level cursor.
    pub fn timesheet_narrative_count(&self) -> usize {
        self.build_timesheet_groups()
            .iter()
            .map(|g| g.task_indices.len())
            .sum()
    }

    /// Resolve a narrative-level cursor position to the underlying group and
    /// task. Returns `(group_idx, narrative_idx_in_group, task_ref)`.
    pub fn timesheet_narrative_at(
        &self,
        cursor: usize,
    ) -> Option<(usize, usize, TimesheetTaskRef)> {
        let groups = self.build_timesheet_groups();
        let mut offset = 0usize;
        for (gi, g) in groups.iter().enumerate() {
            let count = g.task_indices.len();
            if cursor < offset + count {
                let ni = cursor - offset;
                return Some((gi, ni, g.task_indices[ni]));
            }
            offset += count;
        }
        None
    }

    // ---- calendar picker (PickTimesheetDate mode) ----

    /// Set calendar focus to `today + days`.
    /// Needs `App.today()` — thin wrapper around [`TimesheetState::calendar_set_relative`].
    pub fn timesheet_calendar_set_relative(&mut self, days: i64) {
        let today = self.today().to_string();
        self.timesheet.calendar_set_relative(&today, days);
    }

    /// Add or subtract months from the calendar focus.
    pub fn timesheet_calendar_add_months(&mut self, delta: i32) {
        let mut y = self.timesheet.calendar_focus.year();
        let mut m = self.timesheet.calendar_focus.month() as i32 + delta;
        while m > 12 {
            m -= 12;
            y += 1;
        }
        while m < 1 {
            m += 12;
            y -= 1;
        }
        let day = self
            .timesheet
            .calendar_focus
            .day()
            .min(crate::ui::calendar_utils::days_in_month(y, m as u32));
        if let Some(d) = NaiveDate::from_ymd_opt(y, m as u32, day) {
            self.timesheet.calendar_focus = d;
        }
    }

    /// Accept the calendar focus: set `timesheet.date` and return to Normal.
    /// If the user typed a date, prefer it; otherwise use the calendar grid
    /// focus. Invalid typed dates flash an error and stay in the picker.
    pub fn timesheet_calendar_accept(&mut self) {
        if self.timesheet.date_input.is_empty() {
            self.timesheet.date = self.timesheet.calendar_focus.format("%Y-%m-%d").to_string();
        } else if let Ok(d) = NaiveDate::parse_from_str(&self.timesheet.date_input, "%Y-%m-%d") {
            self.timesheet.date = d.format("%Y-%m-%d").to_string();
        } else {
            self.flash(format!("invalid date: {}", self.timesheet.date_input));
            self.timesheet.date_input.clear();
            return;
        }
        self.timesheet.date_input.clear();
        self.timesheet.cursor = 0;
        self.timesheet.invalidate_cache();
        self.nav.mode = Mode::Screen(Screen::Normal);
        let display = self.timesheet_date_display();
        self.flash(format!("jumped to {display}"));
    }

    /// Cancel the calendar: return to Normal without changing the date.
    pub fn timesheet_calendar_cancel(&mut self) {
        self.timesheet.date_input.clear();
        self.nav.mode = Mode::Screen(Screen::Normal);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::app::test_support::build_app_with_config;
    use crate::config::Config;

    fn app_with_workday(start: &str, end: &str) -> App {
        let cfg = Config {
            workday_start: Some(start.into()),
            workday_end: Some(end.into()),
            ..Config::default()
        };
        build_app_with_config("2026-05-06 Work +X @dev dur:3600 log:2026-05-06\n", cfg)
    }

    #[test]
    fn workday_coverage_computes_unaccounted() {
        let app = app_with_workday("09:00", "18:00");
        // 9h span, 1h tracked → 8h unaccounted; past anchor → not in progress.
        let (span, unaccounted, in_progress) = app
            .workday_coverage("2026-05-05", 3600, 17 * 60, "2026-05-06")
            .unwrap();
        assert_eq!(span, 9 * 3600);
        assert_eq!(unaccounted, 8 * 3600);
        assert!(!in_progress, "past days are never 'in progress'");
    }

    #[test]
    fn workday_coverage_in_progress_when_today_before_end() {
        let app = app_with_workday("09:00", "18:00");
        let (_, _, in_progress) = app
            .workday_coverage("2026-05-06", 3600, 10 * 60, "2026-05-06")
            .unwrap();
        assert!(in_progress);
    }

    #[test]
    fn workday_coverage_in_progress_false_after_end() {
        let app = app_with_workday("09:00", "18:00");
        let (_, _, in_progress) = app
            .workday_coverage("2026-05-06", 3600, 19 * 60, "2026-05-06")
            .unwrap();
        assert!(!in_progress);
    }

    #[test]
    fn workday_coverage_clamps_at_zero() {
        let app = app_with_workday("09:00", "18:00");
        let (_, unaccounted, _) = app
            .workday_coverage("2026-05-06", 12 * 3600, 19 * 60, "2026-05-06")
            .unwrap();
        assert_eq!(unaccounted, 0);
    }

    #[test]
    fn workday_coverage_none_without_bounds() {
        let app = build_app_with_config("2026-05-06 Work +X\n", Config::default());
        assert_eq!(app.workday_coverage("2026-05-06", 0, 0, "2026-05-06"), None);
    }

    #[test]
    fn workday_coverage_none_when_end_before_start() {
        let app = app_with_workday("18:00", "09:00");
        assert_eq!(app.workday_coverage("2026-05-06", 0, 0, "2026-05-06"), None);
    }
}
