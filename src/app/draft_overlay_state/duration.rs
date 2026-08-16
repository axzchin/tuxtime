//! Duration picker overlay (`dur:` time presets).

use super::App;
use super::{DraftOverlay, strip_trigger_literal};

/// Time-preset picker for `dur:`. Shows common billable increments.
#[derive(Debug, Clone)]
pub struct DurationPickerState {
    /// Index into the preset list.
    pub selected: usize,
    /// Optional trigger anchor, set when auto-triggered by typing `dur:`.
    pub anchor: Option<usize>,
}

/// Time presets for the duration picker: (label, description, seconds).
pub const DURATION_PRESETS: &[(&str, &str, u64)] = &[
    ("6m", "0.1h", 360),
    ("15m", "0.3h", 900),
    ("30m", "0.5h", 1800),
    ("1h", "1.0h", 3600),
    ("1.5h", "1.5h", 5400),
    ("2h", "2.0h", 7200),
];

impl App {
    pub fn open_duration_picker(&mut self) {
        self.open_duration_picker_anchored(None);
    }

    pub fn open_duration_picker_anchored(&mut self, anchor: Option<usize>) {
        self.draft
            .set_overlay(Some(DraftOverlay::DurationPicker(DurationPickerState {
                selected: 0,
                anchor,
            })));
    }

    pub fn duration_state(&self) -> Option<&DurationPickerState> {
        match self.draft.overlay()? {
            DraftOverlay::DurationPicker(s) => Some(s),
            _ => None,
        }
    }

    pub fn duration_step(&mut self, forward: bool) {
        let Some(DraftOverlay::DurationPicker(s)) = self.draft.overlay_mut() else {
            return;
        };
        let n = DURATION_PRESETS.len();
        let cur = s.selected.min(n - 1);
        s.selected = if forward {
            (cur + 1) % n
        } else {
            (cur + n - 1) % n
        };
    }

    pub fn duration_accept(&mut self) {
        let Some(DraftOverlay::DurationPicker(s)) = self.draft.overlay() else {
            return;
        };
        let anchor = s.anchor;
        let (_label, _desc, secs) = DURATION_PRESETS[s.selected];
        self.draft.set_overlay(None);
        if let Some(a) = anchor {
            strip_trigger_literal(self, "dur", a);
        }
        self.apply_kv("dur", Some(&secs.to_string()));
    }

    pub fn duration_cancel(&mut self) {
        self.draft.set_overlay(None);
    }
}
