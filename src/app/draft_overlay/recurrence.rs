//! Recurrence builder overlay for `rec:` specs (interval, unit, mode).

use chrono::NaiveDate;

use super::App;
use super::{DraftOverlay, find_kv_value, strip_trigger_literal};
use crate::recurrence::{self, RecSpec, RecUnit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuilderField {
    Interval,
    Unit,
    Mode,
}

/// Single source of truth for the unit pill order. The renderer iterates this
/// same slice so a `rec:3b` (business-day) spec opened in the builder still
/// shows up as a selectable pill instead of being silently coerced to Week on
/// the next adjust.
pub const REC_UNIT_ORDER: &[RecUnit] = &[
    RecUnit::Day,
    RecUnit::BusinessDay,
    RecUnit::Week,
    RecUnit::Month,
    RecUnit::Year,
];

#[derive(Debug, Clone)]
pub struct RecurrenceBuilderState {
    pub interval: u32,
    pub unit: RecUnit,
    /// `true` writes `rec:+Nu` (strict — anchor on previous due), `false`
    /// writes `rec:Nu` (after-complete).
    pub strict: bool,
    pub field: BuilderField,
    /// See `CalendarState::anchor`.
    pub anchor: Option<usize>,
}

impl App {
    pub fn open_recurrence_builder(&mut self) {
        self.open_recurrence_builder_anchored(None);
    }

    pub fn open_recurrence_builder_anchored(&mut self, anchor: Option<usize>) {
        let existing = find_kv_value(self.draft.text(), "rec");
        let parsed = existing.as_deref().and_then(recurrence::parse_rec_spec);
        let mut state = match parsed {
            Some(spec) => RecurrenceBuilderState {
                interval: spec.n.max(1),
                unit: spec.unit,
                strict: spec.strict,
                field: BuilderField::Interval,
                anchor: None,
            },
            None => RecurrenceBuilderState {
                interval: 1,
                unit: RecUnit::Week,
                strict: false,
                field: BuilderField::Interval,
                anchor: None,
            },
        };
        state.anchor = anchor;
        self.draft
            .set_overlay(Some(DraftOverlay::RecurrenceBuilder(state)));
    }

    pub fn recurrence_state(&self) -> Option<&RecurrenceBuilderState> {
        match self.draft.overlay()? {
            DraftOverlay::RecurrenceBuilder(s) => Some(s),
            _ => None,
        }
    }

    pub fn recurrence_focus(&mut self, delta: i32) {
        let Some(DraftOverlay::RecurrenceBuilder(s)) = self.draft.overlay_mut() else {
            return;
        };
        let order = [
            BuilderField::Interval,
            BuilderField::Unit,
            BuilderField::Mode,
        ];
        let cur = order.iter().position(|f| *f == s.field).unwrap_or(0) as i32;
        let next = ((cur + delta).rem_euclid(order.len() as i32)) as usize;
        s.field = order[next];
    }

    /// Adjust the currently-focused field. `+1`/`-1` increments interval or
    /// cycles unit / mode. Interval clamps at 1 (no zero intervals — the
    /// recurrence parser rejects them).
    pub fn recurrence_adjust(&mut self, delta: i32) {
        let Some(DraftOverlay::RecurrenceBuilder(s)) = self.draft.overlay_mut() else {
            return;
        };
        match s.field {
            BuilderField::Interval => {
                let cur = s.interval as i32;
                s.interval = (cur + delta).max(1) as u32;
            }
            BuilderField::Unit => {
                let order = REC_UNIT_ORDER;
                let cur = order.iter().position(|u| *u == s.unit).unwrap_or(1) as i32;
                let n = order.len() as i32;
                let next = ((cur + delta).rem_euclid(n)) as usize;
                s.unit = order[next];
            }
            BuilderField::Mode => {
                s.strict = !s.strict;
            }
        }
    }

    pub fn recurrence_accept(&mut self) {
        let Some(DraftOverlay::RecurrenceBuilder(s)) = self.draft.overlay() else {
            return;
        };
        let value = format_rec_value(s);
        let anchor = s.anchor;
        self.draft.set_overlay(None);
        if let Some(a) = anchor {
            strip_trigger_literal(self, "rec", a);
        }
        self.apply_kv("rec", Some(&value));
    }

    pub fn recurrence_cancel(&mut self) {
        self.draft.set_overlay(None);
    }
}

/// Format a builder state as the value portion of a `rec:` token (e.g. `1w`,
/// `+2m`). Used by `recurrence_accept` and by the live preview line.
#[must_use]
pub fn format_rec_value(state: &RecurrenceBuilderState) -> String {
    let prefix = if state.strict { "+" } else { "" };
    let unit = match state.unit {
        RecUnit::Day => "d",
        RecUnit::BusinessDay => "b",
        RecUnit::Week => "w",
        RecUnit::Month => "m",
        RecUnit::Year => "y",
    };
    format!("{prefix}{}{unit}", state.interval)
}

/// "Next occurrence" for the recurrence-builder preview line. Computed via
/// the same `recurrence::advance` used by the completion-spawn path, anchored
/// on the app's `today` so the value is identical to what the user would see
/// after marking a task done now.
#[must_use]
pub fn recurrence_next_preview(state: &RecurrenceBuilderState, today: &str) -> Option<NaiveDate> {
    let spec = RecSpec {
        strict: state.strict,
        n: state.interval,
        unit: state.unit,
    };
    let date = NaiveDate::parse_from_str(today, "%Y-%m-%d").ok()?;
    recurrence::advance(date, &spec)
}
