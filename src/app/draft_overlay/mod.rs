//! Metadata pickers that float over the add-task dialog. Triggered by `/` in
//! Insert mode, each overlay collects a specific piece of metadata (due date,
//! recurrence, priority, threshold) or routes back to the existing
//! autocomplete popup for projects/contexts, then writes the result into the
//! draft buffer using a small set of `apply_*` helpers.
//!
//! State lives on `DraftState::overlay`. The flow is:
//!
//! 1. User types `/` at BOL or after whitespace → `maybe_open_slash_menu`
//!    installs `DraftOverlay::SlashMenu` with `anchor` = position of the `/`.
//! 2. Filter text is `draft[anchor+1..cursor]`. Up/Down navigate, Enter accepts.
//! 3. On accept, `slash_accept` drains `draft[anchor..cursor]` and either:
//!    - opens a second overlay (Calendar, RecurrenceBuilder, PriorityChooser),
//!    - or inserts a sigil (`+`/`@`) and re-arms the autocomplete popup.
//! 4. The second overlay produces a value and `apply_*` splices it into the
//!    buffer — replacing the existing token of the same kind if present,
//!    otherwise appending.
//!
//! Each overlay kind lives in its own submodule (`slash`, `calendar`,
//! `recurrence`, `priority`, `duration`) with its state struct and App
//! methods; this parent keeps the shared catalog, the `DraftOverlay` /
//! `OverlayKind` enums, the `apply_*` buffer writers, and the shared pure
//! helpers.

use chrono::NaiveDate;
use std::ops::Range;

use super::App;
use crate::threshold::{self, ThresholdSpec};

mod calendar;
mod duration;
mod priority;
mod recurrence;
mod slash;

pub use calendar::{CalendarState, CalendarTarget};
pub use duration::{DURATION_PRESETS, DurationPickerState};
pub use priority::PriorityChooserState;
pub use recurrence::{
    BuilderField, REC_UNIT_ORDER, RecurrenceBuilderState, format_rec_value, recurrence_next_preview,
};
pub use slash::{SLASH_ENTRIES, SlashEntry, SlashKind, SlashMenuState};

// ---------------------------------------------------------------------------
// Overlay state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum DraftOverlay {
    SlashMenu(SlashMenuState),
    Calendar(CalendarState),
    RecurrenceBuilder(RecurrenceBuilderState),
    PriorityChooser(PriorityChooserState),
    DurationPicker(DurationPickerState),
}

/// Discriminator-only view of `DraftOverlay`, suitable for key-dispatch matches
/// that need to free the immutable borrow before calling `&mut App` methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    SlashMenu,
    Calendar,
    RecurrenceBuilder,
    PriorityChooser,
    DurationPicker,
}

impl DraftOverlay {
    #[must_use]
    pub fn kind(&self) -> OverlayKind {
        match self {
            DraftOverlay::SlashMenu(_) => OverlayKind::SlashMenu,
            DraftOverlay::Calendar(_) => OverlayKind::Calendar,
            DraftOverlay::RecurrenceBuilder(_) => OverlayKind::RecurrenceBuilder,
            DraftOverlay::PriorityChooser(_) => OverlayKind::PriorityChooser,
            DraftOverlay::DurationPicker(_) => OverlayKind::DurationPicker,
        }
    }
}

// ---------------------------------------------------------------------------
// apply_* — write metadata back into the draft buffer
// ---------------------------------------------------------------------------

impl App {
    /// Insert/replace/remove a `key:value` token in the draft. `value = None`
    /// removes any existing token with that key. Existing tokens are replaced
    /// in place to preserve the user's body-text layout; new tokens append
    /// with a leading space.
    pub(super) fn apply_kv(&mut self, key: &str, value: Option<&str>) {
        let existing = find_kv_token_range(self.draft.text(), key);
        match (existing, value) {
            (Some(range), Some(v)) => {
                let replacement = format!("{key}:{v}");
                self.draft
                    .replace_token(range.start, range.end, &replacement);
            }
            (Some(range), None) => {
                // Delete the token. Drop one leading space if present so we
                // don't leave "  " mid-line.
                let leading_space = range.start > 0
                    && self
                        .draft
                        .text()
                        .as_bytes()
                        .get(range.start - 1)
                        .copied()
                        .is_some_and(|b| b == b' ' || b == b'\t');
                let start = if leading_space {
                    range.start - 1
                } else {
                    range.start
                };
                self.draft.replace_token(start, range.end, "");
            }
            (None, Some(v)) => {
                let (cur_len, needs_space) = {
                    let text = self.draft.text();
                    let needs_space = !text.is_empty()
                        && !text
                            .as_bytes()
                            .last()
                            .copied()
                            .is_some_and(|b| b == b' ' || b == b'\t');
                    (text.len(), needs_space)
                };
                let insert = if needs_space {
                    format!(" {key}:{v}")
                } else {
                    format!("{key}:{v}")
                };
                self.draft.replace_token(cur_len, cur_len, &insert);
            }
            (None, None) => {}
        }
    }

    /// Replace, prepend, or remove the leading `(X) ` priority token.
    pub(super) fn apply_priority(&mut self, priority: Option<char>) {
        let has_priority = {
            let bytes = self.draft.text().as_bytes();
            bytes.len() >= 4
                && bytes[0] == b'('
                && bytes[1].is_ascii_uppercase()
                && bytes[2] == b')'
                && bytes[3] == b' '
        };
        match (has_priority, priority) {
            (true, Some(p)) => {
                // Replace just the letter inside the parens — keeps cursor
                // semantics simple and avoids touching the trailing space.
                self.draft.replace_token(1, 2, &p.to_string());
            }
            (true, None) => {
                self.draft.replace_token(0, 4, "");
            }
            (false, Some(p)) => {
                let prefix = format!("({p}) ");
                self.draft.replace_token(0, 0, &prefix);
            }
            (false, None) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Shared pure helpers
// ---------------------------------------------------------------------------

/// Byte range of the first whitespace-delimited token with the form
/// `key:<non-empty value>`. Returns `None` when no such token exists.
fn find_kv_token_range(text: &str, key: &str) -> Option<Range<usize>> {
    let needle = format!("{key}:");
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let tok = &text[start..i];
        if let Some(rest) = tok.strip_prefix(&needle)
            && !rest.is_empty()
        {
            return Some(start..i);
        }
    }
    None
}

/// Value of the first `key:value` token, if any. Wraps `find_kv_token_range`
/// for the common "look up the existing value" call sites.
fn find_kv_value(text: &str, key: &str) -> Option<String> {
    let range = find_kv_token_range(text, key)?;
    let tok = &text[range];
    tok.split_once(':').map(|(_, v)| v.to_string())
}

/// Remove the `KEY:VALUE` token at `anchor` and its single leading space, if
/// any. Strips the whole token (key, colon, and any typed value) so that
/// `apply_kv` can write the canonical form without leaving a duplicate or a
/// bare value fragment in the buffer. No-op if the buffer no longer matches
/// `KEY:` at `anchor` (e.g. the user edited the line — defensive).
fn strip_trigger_literal(app: &mut App, key: &str, anchor: usize) {
    let key_with_colon = format!("{key}:");
    let text = app.draft.text();
    let key_colon_end = anchor + key_with_colon.len();
    if text.get(anchor..key_colon_end) != Some(key_with_colon.as_str()) {
        return;
    }
    // Extend past any value the user typed (up to next whitespace or EOL).
    let end = text[key_colon_end..]
        .find(|c: char| c.is_ascii_whitespace())
        .map_or(text.len(), |i| key_colon_end + i);
    let strip_start = if anchor > 0
        && text
            .as_bytes()
            .get(anchor - 1)
            .copied()
            .is_some_and(|b| b == b' ' || b == b'\t')
    {
        anchor - 1
    } else {
        anchor
    };
    app.draft.replace_token(strip_start, end, "");
}

/// Resolved threshold date for the "→ shows on {date}" hint in the calendar.
/// Mirrors `threshold::resolve` against `(due, today)` so the hint matches the
/// actual visibility filter. Unused today; reserved for future hint copy.
#[allow(dead_code)]
pub fn threshold_preview(value: &str, due: Option<&str>, today: &str) -> Option<NaiveDate> {
    let spec = threshold::parse_threshold(value)?;
    if let ThresholdSpec::Absolute(d) = spec {
        return Some(d);
    }
    threshold::resolve(&spec, due, Some(today))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "draft_overlay_tests.rs"]
mod tests;
