//! Calendar picker overlay for `due:` and `t:` (threshold) dates.

use chrono::{Days, Months, NaiveDate};

use super::App;
use super::{DraftOverlay, find_kv_value, strip_trigger_literal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarTarget {
    Due,
    Threshold,
}

#[derive(Debug, Clone)]
pub struct CalendarState {
    pub target: CalendarTarget,
    pub focused: NaiveDate,
    /// Set when the picker was auto-triggered by typing `due:` / `t:` —
    /// records the byte offset of the leading key letter so accept can strip
    /// the empty literal before writing the chosen value. `None` for slash-
    /// menu opens; accept then uses `apply_kv` directly.
    pub anchor: Option<usize>,
}

impl App {
    pub fn open_calendar(&mut self, target: CalendarTarget) {
        self.open_calendar_anchored(target, None);
    }

    /// Open the calendar with an optional trigger anchor. The anchor is set
    /// only when the user auto-triggered the picker by typing `due:` / `t:`
    /// directly — accept then strips that literal so `apply_kv` doesn't leave
    /// a duplicate empty token behind.
    pub fn open_calendar_anchored(&mut self, target: CalendarTarget, anchor: Option<usize>) {
        let existing = match target {
            CalendarTarget::Due => find_kv_value(self.draft.text(), "due"),
            CalendarTarget::Threshold => find_kv_value(self.draft.text(), "t"),
        };
        let focused = existing
            .and_then(|v| NaiveDate::parse_from_str(&v, "%Y-%m-%d").ok())
            .or_else(|| NaiveDate::parse_from_str(self.store.today(), "%Y-%m-%d").ok())
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(2026, 1, 1).expect("static date"));
        self.draft
            .set_overlay(Some(DraftOverlay::Calendar(CalendarState {
                target,
                focused,
                anchor,
            })));
    }

    pub fn calendar_state(&self) -> Option<&CalendarState> {
        match self.draft.overlay()? {
            DraftOverlay::Calendar(s) => Some(s),
            _ => None,
        }
    }

    pub fn calendar_move(&mut self, dx: i32, dy: i32) {
        let Some(DraftOverlay::Calendar(s)) = self.draft.overlay_mut() else {
            return;
        };
        let total_days = dx + dy * 7;
        let next = if total_days >= 0 {
            s.focused.checked_add_days(Days::new(total_days as u64))
        } else {
            s.focused
                .checked_sub_days(Days::new(u64::from(total_days.unsigned_abs())))
        };
        if let Some(d) = next {
            s.focused = d;
        }
    }

    pub fn calendar_set_relative(&mut self, days: i64) {
        let Some(today) = NaiveDate::parse_from_str(self.store.today(), "%Y-%m-%d").ok() else {
            return;
        };
        let Some(DraftOverlay::Calendar(s)) = self.draft.overlay_mut() else {
            return;
        };
        let next = if days >= 0 {
            today.checked_add_days(Days::new(days as u64))
        } else {
            today.checked_sub_days(Days::new(days.unsigned_abs()))
        };
        if let Some(d) = next {
            s.focused = d;
        }
    }

    pub fn calendar_add_months(&mut self, n: i32) {
        let Some(DraftOverlay::Calendar(s)) = self.draft.overlay_mut() else {
            return;
        };
        let next = if n >= 0 {
            s.focused.checked_add_months(Months::new(n as u32))
        } else {
            s.focused.checked_sub_months(Months::new(n.unsigned_abs()))
        };
        if let Some(d) = next {
            s.focused = d;
        }
    }

    /// Save the focused date into the draft and close the calendar.
    pub fn calendar_accept(&mut self) {
        let Some(DraftOverlay::Calendar(s)) = self.draft.overlay() else {
            return;
        };
        let target = s.target;
        let anchor = s.anchor;
        let date_str = s.focused.format("%Y-%m-%d").to_string();
        self.draft.set_overlay(None);
        let key = match target {
            CalendarTarget::Due => "due",
            CalendarTarget::Threshold => "t",
        };
        if let Some(a) = anchor {
            // Auto-trigger: remove the just-typed `KEY:` literal (and its
            // leading space) so apply_kv finds/replaces the canonical token
            // — either updating an existing one elsewhere or appending fresh.
            // Without this strip, retriggering on a line that already has a
            // `due:DATE` would leave two `due:` tokens.
            strip_trigger_literal(self, key, a);
        }
        self.apply_kv(key, Some(&date_str));
    }

    /// Clear the current value from the draft and close the calendar.
    pub fn calendar_clear(&mut self) {
        let Some(DraftOverlay::Calendar(s)) = self.draft.overlay() else {
            return;
        };
        let target = s.target;
        let anchor = s.anchor;
        self.draft.set_overlay(None);
        let key = match target {
            CalendarTarget::Due => "due",
            CalendarTarget::Threshold => "t",
        };
        if let Some(a) = anchor {
            strip_trigger_literal(self, key, a);
        }
        self.apply_kv(key, None);
    }

    /// Esc on the calendar. Leaves the buffer untouched — matches the `@`/`+`
    /// autocomplete model where Esc dismisses the popup but keeps any literal
    /// the user has typed so they can finish it by hand.
    pub fn calendar_cancel(&mut self) {
        self.draft.set_overlay(None);
    }

    /// Called after each character is typed into the draft while the calendar
    /// is open in auto-trigger mode. Reads the value typed after `KEY:` at the
    /// anchor and, if it parses as a complete `YYYY-MM-DD` date, moves the
    /// calendar's focused cell to that date. Closes the calendar if the user
    /// has backspaced past the colon (anchor's `KEY:` is no longer in buffer).
    pub fn calendar_sync_from_draft(&mut self) {
        let (anchor, target) = {
            let Some(DraftOverlay::Calendar(s)) = self.draft.overlay() else {
                return;
            };
            let Some(anchor) = s.anchor else {
                return;
            };
            (anchor, s.target)
        };
        let key = match target {
            CalendarTarget::Due => "due",
            CalendarTarget::Threshold => "t",
        };
        let key_with_colon = format!("{key}:");
        let value_start = anchor + key_with_colon.len();
        let parsed_date = {
            let text = self.draft.text();
            if text.get(anchor..value_start) != Some(key_with_colon.as_str()) {
                self.draft.set_overlay(None);
                return;
            }
            let value = text[value_start..]
                .split(|c: char| c.is_ascii_whitespace())
                .next()
                .unwrap_or("");
            NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
        };
        if let Some(date) = parsed_date
            && let Some(DraftOverlay::Calendar(s)) = self.draft.overlay_mut()
        {
            s.focused = date;
        }
    }
}
