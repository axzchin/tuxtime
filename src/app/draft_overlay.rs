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

use chrono::{Days, Months, NaiveDate};
use std::ops::Range;

use super::App;
use crate::recurrence::{self, RecSpec, RecUnit};
use crate::threshold::{self, ThresholdSpec};

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

// ---------------------------------------------------------------------------
// Overlay state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SlashMenuState {
    /// Byte offset of the `/` in the draft. Filter text is `draft[anchor+1..cursor]`.
    pub anchor: usize,
    /// Index into the *filtered* entry list. Reset on every filter change.
    pub selected: usize,
}

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

#[derive(Debug, Clone)]
pub struct PriorityChooserState {
    /// 0=A, 1=B, 2=C, 3=clear.
    pub selected: u8,
}

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
    pub    fn kind(&self) -> OverlayKind {
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
// Slash menu — open / filter / cancel / accept
// ---------------------------------------------------------------------------

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
            let prev = super::draft::prev_char_boundary(text, slash_pos);
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
        for (key, kind) in [("rec", KvKind::Rec), ("due", KvKind::Due), ("t", KvKind::T), ("dur", KvKind::Dur)] {
            if let Some(key_start) = match_key_before(text, colon_pos, key) {
                match kind {
                    KvKind::Due => {
                        self.open_calendar_anchored(CalendarTarget::Due, Some(key_start));
                    }
                    KvKind::T => {
                        self.open_calendar_anchored(CalendarTarget::Threshold, Some(key_start));
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
            SlashKind::Due => self.open_calendar(CalendarTarget::Due),
            SlashKind::Threshold => self.open_calendar(CalendarTarget::Threshold),
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
// Calendar
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Recurrence builder
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Priority chooser
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Duration picker (time presets)
// ---------------------------------------------------------------------------

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
// Pure helpers
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
    let prev = super::draft::prev_char_boundary(text, key_start);
    let prev_char = text.get(prev..key_start).and_then(|s| s.chars().next())?;
    if prev_char.is_whitespace() {
        Some(key_start)
    } else {
        None
    }
}

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
