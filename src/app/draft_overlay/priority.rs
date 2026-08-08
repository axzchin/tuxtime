//! Priority chooser overlay: A / B / C / clear.

use super::App;
use super::DraftOverlay;

#[derive(Debug, Clone)]
pub struct PriorityChooserState {
    /// 0=A, 1=B, 2=C, 3=clear.
    pub selected: u8,
}

impl App {
    pub fn open_priority_chooser(&mut self) {
        let existing = find_priority(self.draft.text());
        let selected = match existing {
            Some('A') => 0,
            Some('B') => 1,
            Some('C') => 2,
            _ => 0,
        };
        self.draft
            .set_overlay(Some(DraftOverlay::PriorityChooser(PriorityChooserState {
                selected,
            })));
    }

    pub fn priority_state(&self) -> Option<&PriorityChooserState> {
        match self.draft.overlay()? {
            DraftOverlay::PriorityChooser(s) => Some(s),
            _ => None,
        }
    }

    pub fn priority_step(&mut self, forward: bool) {
        let Some(DraftOverlay::PriorityChooser(s)) = self.draft.overlay_mut() else {
            return;
        };
        let n: i32 = 4; // A, B, C, clear
        let cur = i32::from(s.selected);
        let next = (cur + if forward { 1 } else { -1 }).rem_euclid(n);
        s.selected = next as u8;
    }

    pub fn priority_accept(&mut self) {
        let Some(DraftOverlay::PriorityChooser(s)) = self.draft.overlay() else {
            return;
        };
        let pri = match s.selected {
            0 => Some('A'),
            1 => Some('B'),
            2 => Some('C'),
            _ => None,
        };
        self.draft.set_overlay(None);
        self.apply_priority(pri);
    }

    pub fn priority_cancel(&mut self) {
        self.draft.set_overlay(None);
    }
}

/// Leading priority letter, if the line starts with `(X) `.
fn find_priority(text: &str) -> Option<char> {
    let bytes = text.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'('
        && bytes[1].is_ascii_uppercase()
        && bytes[2] == b')'
        && bytes[3] == b' '
    {
        Some(bytes[1] as char)
    } else {
        None
    }
}
