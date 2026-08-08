//! Slash menu overlay: the `/` command popup that lists metadata options.

use super::App;
use super::DraftOverlay;
use crate::app::draft::prev_char_boundary;

// ---------------------------------------------------------------------------
// Catalog
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashKind {
    Due,
    Recurrence,
    Threshold,
    Priority,
    Project,
    Context,
    Duration,
}

#[derive(Debug, Clone, Copy)]
pub struct SlashEntry {
    pub label: &'static str,
    pub description: &'static str,
    pub cmd: &'static str,
    pub kind: SlashKind,
}

/// Order matches the mockup. The slash menu renders entries in this order
/// when the filter is empty; `slash_matches` re-sorts only via the filter.
pub const SLASH_ENTRIES: &[SlashEntry] = &[
    SlashEntry {
        label: "Due date",
        description: "when this needs doing",
        cmd: "/due",
        kind: SlashKind::Due,
    },
    SlashEntry {
        label: "Recurrence",
        description: "repeat after completing",
        cmd: "/rec",
        kind: SlashKind::Recurrence,
    },
    SlashEntry {
        label: "Threshold",
        description: "hide until this date",
        cmd: "/t",
        kind: SlashKind::Threshold,
    },
    SlashEntry {
        label: "Priority",
        description: "A · B · C",
        cmd: "/prio",
        kind: SlashKind::Priority,
    },
    SlashEntry {
        label: "+ Project",
        description: "attach to a project",
        cmd: "/proj",
        kind: SlashKind::Project,
    },
    SlashEntry {
        label: "@ Context",
        description: "tool, place or person",
        cmd: "/ctx",
        kind: SlashKind::Context,
    },
    SlashEntry {
        label: "Duration",
        description: "time spent: 90 (min), 1.5h, 14:30, 9am",
        cmd: "/dur",
        kind: SlashKind::Duration,
    },
];

#[derive(Debug, Clone)]
pub struct SlashMenuState {
    /// Byte offset of the `/` in the draft. Filter text is `draft[anchor+1..cursor]`.
    pub anchor: usize,
    /// Index into the *filtered* entry list. Reset on every filter change.
    pub selected: usize,
}

impl App {
    /// After a `/` was just inserted into the draft, decide whether to open
    /// the slash menu. Triggers only at BOL or right after whitespace, so a
    /// `/` mid-URL (e.g. `https://example.com`) does not pop the menu.
    /// No-op when an overlay is already open.
    pub fn maybe_open_slash_menu(&mut self) {
        if self.draft.overlay().is_some() {
            return;
        }
        let text = self.draft.text();
        let cursor = self.draft.cursor();
        if cursor == 0 {
            return;
        }
        let slash_pos = cursor - 1;
        if text.as_bytes().get(slash_pos) != Some(&b'/') {
            return;
        }
        if slash_pos > 0 {
            let prev = prev_char_boundary(text, slash_pos);
            let prev_char = text[prev..slash_pos].chars().next();
            if !matches!(prev_char, Some(c) if c.is_whitespace()) {
                return;
            }
        }
        self.draft
            .set_overlay(Some(DraftOverlay::SlashMenu(SlashMenuState {
                anchor: slash_pos,
                selected: 0,
            })));
    }

    /// After a `:` was just inserted, decide whether to auto-open a metadata
    /// picker. Mirrors `maybe_open_slash_menu`: triggers only when the chars
    /// immediately before the colon are one of the recognised keys (`due`,
    /// `t`, `rec`) and the char before *that* is whitespace or BOL. So
    /// `Recipe:` and `Mydue:` don't fire; ` due:` and `rec:` at BOL do.
    pub fn maybe_open_kv_overlay(&mut self) {
        if self.draft.overlay().is_some() {
            return;
        }
        let cursor = self.draft.cursor();
        if cursor == 0 {
            return;
        }
        let text = self.draft.text();
        if text.as_bytes().get(cursor - 1) != Some(&b':') {
            return;
        }
        let colon_pos = cursor - 1;
        // Try longest keys first so `rec` doesn't shadow a hypothetical `re`.
        for (key, kind) in [
            ("rec", KvKind::Rec),
            ("due", KvKind::Due),
            ("t", KvKind::T),
            ("dur", KvKind::Dur),
        ] {
            if let Some(key_start) = match_key_before(text, colon_pos, key) {
                match kind {
                    KvKind::Due => {
                        self.open_calendar_anchored(super::CalendarTarget::Due, Some(key_start));
                    }
                    KvKind::T => {
                        self.open_calendar_anchored(
                            super::CalendarTarget::Threshold,
                            Some(key_start),
                        );
                    }
                    KvKind::Rec => {
                        self.open_recurrence_builder_anchored(Some(key_start));
                    }
                    KvKind::Dur => {
                        self.open_duration_picker_anchored(Some(key_start));
                    }
                }
                return;
            }
        }
    }

    /// Validate that the slash menu's anchor is still consistent with the
    /// current buffer. Drops the menu when the `/` was deleted, the cursor
    /// moved before it, or a space was typed after it (a slash command is a
    /// single whitespace-delimited token, so `Option A / B` is prose, not a
    /// command). Called after every text-edit key in handle_insert so the
    /// popup goes away when the user backspaces over its trigger or types past
    /// it.
    pub fn slash_menu_revalidate(&mut self) {
        let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay() else {
            return;
        };
        let anchor = state.anchor;
        let text = self.draft.text();
        let still_slash = text.as_bytes().get(anchor) == Some(&b'/');
        let cursor = self.draft.cursor();
        let cursor_ok = cursor > anchor;
        // A slash command is a single whitespace-delimited token. If a space
        // appears after the `/` (e.g. prose like "Option A / B"), it isn't a
        // command — drop the menu so Enter saves the todo instead of being
        // swallowed by `slash_accept`.
        let filter_has_space = still_slash
            && cursor_ok
            && cursor <= text.len()
            && text[anchor + 1..cursor]
                .bytes()
                .any(|b| b.is_ascii_whitespace());
        if !still_slash || !cursor_ok || filter_has_space {
            self.draft.set_overlay(None);
        }
    }

    /// Filter text typed after the `/`. Empty when the menu just opened.
    pub fn slash_filter(&self) -> &str {
        let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay() else {
            return "";
        };
        let cursor = self.draft.cursor().min(self.draft.text().len());
        let start = (state.anchor + 1).min(cursor);
        &self.draft.text()[start..cursor]
    }

    /// Entries that match the current filter, in display order. Case-insensitive
    /// substring match against label and command. When the filter is empty,
    /// every entry passes.
    pub fn slash_matches(&self) -> Vec<&'static SlashEntry> {
        let filter = self.slash_filter().to_lowercase();
        SLASH_ENTRIES
            .iter()
            .filter(|e| {
                if filter.is_empty() {
                    return true;
                }
                e.label.to_lowercase().contains(&filter) || e.cmd.contains(&filter)
            })
            .collect()
    }

    /// Index of the currently-highlighted match, clamped to the filtered list.
    pub fn slash_selected(&self) -> usize {
        let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay() else {
            return 0;
        };
        let n = self.slash_matches().len();
        if n == 0 { 0 } else { state.selected.min(n - 1) }
    }

    pub fn slash_step(&mut self, forward: bool) {
        let n = self.slash_matches().len();
        if n == 0 {
            return;
        }
        if let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay_mut() {
            let cur = state.selected.min(n - 1);
            state.selected = if forward {
                (cur + 1) % n
            } else {
                (cur + n - 1) % n
            };
        }
    }

    /// Cancel the slash menu and remove the `/filter` literal from the buffer.
    pub fn slash_cancel(&mut self) {
        let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay() else {
            return;
        };
        let anchor = state.anchor;
        let cursor = self.draft.cursor();
        let end = cursor.max(anchor);
        self.draft.set_overlay(None);
        if anchor <= self.draft.text().len() && end <= self.draft.text().len() {
            self.draft.replace_token(anchor, end, "");
        }
    }

    /// Accept the highlighted entry. Removes the `/filter` literal and then
    /// either opens a second overlay (Due/Rec/Threshold/Priority) or inserts
    /// a sigil so the existing autocomplete popup takes over (Project/Context).
    pub fn slash_accept(&mut self) {
        let Some(DraftOverlay::SlashMenu(state)) = self.draft.overlay() else {
            return;
        };
        let anchor = state.anchor;
        let cursor = self.draft.cursor();
        let matches = self.slash_matches();
        if matches.is_empty() {
            return;
        }
        let idx = state.selected.min(matches.len() - 1);
        let kind = matches[idx].kind;
        // Drop the `/filter` literal first so subsequent inserts land at the
        // anchor without colliding with leftover trigger chars.
        let end = cursor.max(anchor);
        self.draft.set_overlay(None);
        self.draft.replace_token(anchor, end, "");
        // Dispatch.
        match kind {
            SlashKind::Due => self.open_calendar(super::CalendarTarget::Due),
            SlashKind::Threshold => self.open_calendar(super::CalendarTarget::Threshold),
            SlashKind::Recurrence => self.open_recurrence_builder(),
            SlashKind::Priority => self.open_priority_chooser(),
            SlashKind::Project => self.insert_sigil_at_cursor('+'),
            SlashKind::Context => self.insert_sigil_at_cursor('@'),
            SlashKind::Duration => self.insert_text_at_cursor("dur:"),
        }
    }

    fn insert_sigil_at_cursor(&mut self, sigil: char) {
        self.insert_text_at_cursor(&sigil.to_string());
    }

    /// Insert arbitrary text at the current cursor position, with a leading
    /// space if the cursor isn't at BOL or after whitespace. Used by the
    /// slash menu to insert `dur:` (and potentially other prefix literals).
    fn insert_text_at_cursor(&mut self, text: &str) {
        let pos = self.draft.cursor();
        let needs_space = pos > 0
            && self
                .draft
                .text()
                .as_bytes()
                .get(pos - 1)
                .copied()
                .is_some_and(|b| !b.is_ascii_whitespace());
        let insert = if needs_space {
            format!(" {text}")
        } else {
            text.to_string()
        };
        self.draft.replace_token(pos, pos, &insert);
    }
}

// ---------------------------------------------------------------------------
// kv-trigger helpers (used only by `maybe_open_kv_overlay`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum KvKind {
    Due,
    T,
    Rec,
    Dur,
}

/// True when `text[colon_pos - key.len() .. colon_pos] == key` and the char
/// before that range is whitespace or BOL. Returns the byte offset where the
/// key starts so the caller can record it as the trigger anchor.
fn match_key_before(text: &str, colon_pos: usize, key: &str) -> Option<usize> {
    if colon_pos < key.len() {
        return None;
    }
    let key_start = colon_pos - key.len();
    // `key_start` is a byte arithmetic — if the char immediately before the
    // colon is multi-byte, it may land mid-codepoint and `text.get` returns
    // None instead of panicking on a direct slice.
    if text.get(key_start..colon_pos) != Some(key) {
        return None;
    }
    if key_start == 0 {
        return Some(key_start);
    }
    let prev = prev_char_boundary(text, key_start);
    let prev_char = text.get(prev..key_start).and_then(|s| s.chars().next())?;
    if prev_char.is_whitespace() {
        Some(key_start)
    } else {
        None
    }
}
