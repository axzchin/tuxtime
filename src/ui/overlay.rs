//! Overlay sizing: the constants and rect math for every floating overlay
//! (add/edit dialog, help, welcome, text prompts, command palette, theme
//! picker, prompt message boxes, day-boundary prompt, timesheet calendar) and
//! for the anchored popups that float below the add/edit dialog (slash menu,
//! calendar, recurrence builder, priority chooser, duration picker,
//! autocomplete). `draw` in `ui::mod` renders each overlay into the rect
//! computed here, so every overlay's footprint — width, height, and clamping
//! rules — lives in one place.

use ratatui::layout::Rect;

/// Add/edit dialog: 4/5 of the centre pane's width, within these bounds.
const DIALOG_H: u16 = 8;
const DIALOG_MIN_W: u16 = 40;
const DIALOG_MAX_W: u16 = 100;

const HELP_MAX_H: u16 = 40;
const HELP_MIN_W: u16 = 76;
const HELP_MAX_W: u16 = 120;

const WELCOME_W: u16 = 56;
const WELCOME_H: u16 = 16;

/// Single-line text prompts (project/context/save-filter/rename/nudge).
const PROMPT_H: u16 = 5;
const PROMPT_MAX_W: u16 = 50;

/// Command palette and theme picker share the same footprint.
const PALETTE_MAX_H: u16 = 20;
const PALETTE_MIN_W: u16 = 50;
const PALETTE_MAX_W: u16 = 80;

/// Prompt message boxes (idle nudge, stale timer, manual-entry choice):
/// `MESSAGE_W` wide, `MESSAGE_H` tall for short messages, growing with the
/// wrapped message (see [`message_rect_for`]).
const MESSAGE_W: u16 = 60;
const MESSAGE_H: u16 = 6;

/// Day-boundary prompt: fixed width, height grows with the wrapped narrative.
const DAY_BOUNDARY_W: u16 = 68;
const DAY_BOUNDARY_MIN_H: u16 = 7;

/// Timesheet date-picker calendar popup.
const CALENDAR_W: u16 = 50;
const CALENDAR_H: u16 = 14;

/// Columns the anchored popups shift right of the add/edit dialog to line up
/// with its input prefix (`"  › "` = 4 cols).
pub(crate) const INPUT_PREFIX_OFFSET: u16 = 4;

/// Fixed footprints of the anchored metadata pickers.
const CALENDAR_POPUP_W: u16 = 50;
const CALENDAR_POPUP_H: u16 = 13;
const RECURRENCE_POPUP_W: u16 = 60;
const RECURRENCE_POPUP_H: u16 = 9;
const PRIORITY_POPUP_W: u16 = 24;
const PRIORITY_POPUP_H: u16 = 8;
const DURATION_POPUP_W: u16 = 36;
const SLASH_MIN_W: u16 = 60;

/// Center `w` × `h` inside `parent`, clamping to the parent's bounds.
#[must_use]
pub(crate) fn centered_in(parent: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(parent.width);
    let h = h.min(parent.height);
    let x = parent.x + (parent.width - w) / 2;
    let y = parent.y + (parent.height - h) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// The add/edit dialog: 4/5 of the centre pane's width, clamped to
/// [`DIALOG_MIN_W`]..=[`DIALOG_MAX_W`], `DIALOG_H` tall.
#[must_use]
pub(crate) fn insert_dialog_rect(parent: Rect, center_width: u16) -> Rect {
    let w = (u32::from(center_width) * 4 / 5)
        .clamp(u32::from(DIALOG_MIN_W), u32::from(DIALOG_MAX_W)) as u16;
    centered_in(parent, w, DIALOG_H)
}

#[must_use]
pub(crate) fn help_rect(parent: Rect) -> Rect {
    let h = parent.height.saturating_sub(3).min(HELP_MAX_H);
    let w = (u32::from(parent.width) * 9 / 10).clamp(u32::from(HELP_MIN_W), u32::from(HELP_MAX_W))
        as u16;
    centered_in(parent, w, h)
}

#[must_use]
pub(crate) fn prompt_rect(parent: Rect) -> Rect {
    let w = PROMPT_MAX_W.min(parent.width.saturating_sub(4));
    centered_in(parent, w, PROMPT_H)
}

/// Command palette and theme picker.
#[must_use]
pub(crate) fn palette_rect(parent: Rect) -> Rect {
    let h = parent.height.saturating_sub(4).min(PALETTE_MAX_H);
    let w = (u32::from(parent.width) * 3 / 5)
        .clamp(u32::from(PALETTE_MIN_W), u32::from(PALETTE_MAX_W)) as u16;
    centered_in(parent, w, h)
}

#[must_use]
pub(crate) fn welcome_rect(parent: Rect) -> Rect {
    centered_in(parent, WELCOME_W, WELCOME_H)
}

/// Width the prompt message boxes word-wrap to (box width minus borders), so
/// the caller can count the wrapped lines before sizing the box.
#[must_use]
pub(crate) fn message_wrap_w(parent: Rect) -> usize {
    usize::from(MESSAGE_W.min(parent.width))
        .saturating_sub(2)
        .max(16)
}

/// The prompt message boxes (idle nudge, stale timer, manual-entry choice):
/// `MESSAGE_W` wide, growing with the wrapped message (`wrapped_lines` rows
/// plus borders, blank, footer), keeping [`MESSAGE_H`] for short messages and
/// capping at the parent's height.
#[must_use]
pub(crate) fn message_rect_for(parent: Rect, wrapped_lines: u16) -> Rect {
    let w = MESSAGE_W.min(parent.width);
    let h = (wrapped_lines + 4)
        .max(MESSAGE_H)
        .min(parent.height.saturating_sub(2).max(5));
    centered_in(parent, w, h)
}

/// The empty-state box: the welcome footprint, clamped with small margins so
/// the body-level panel never touches the screen edge.
#[must_use]
pub(crate) fn empty_state_rect(parent: Rect) -> Rect {
    let w = WELCOME_W.min(parent.width.saturating_sub(4));
    let h = WELCOME_H.min(parent.height.saturating_sub(2));
    centered_in(parent, w, h)
}

/// Width the day-boundary message word-wraps to (box width minus borders), so
/// the caller can count the wrapped lines before sizing the box.
#[must_use]
pub(crate) fn day_boundary_wrap_w(parent: Rect) -> usize {
    usize::from(DAY_BOUNDARY_W.min(parent.width))
        .saturating_sub(2)
        .max(16)
}

/// The anchored popup family: dialog overlays (slash menu, calendar,
/// recurrence builder, priority chooser, duration picker) and the autocomplete
/// suggestions all float just below the add/edit dialog, shifted `x_offset`
/// columns right to line up with the input prefix, clamped into `screen` so
/// they stay visible at the bottom/right edges.
#[must_use]
pub(crate) fn anchored_below(anchor: Rect, screen: Rect, w: u16, h: u16, x_offset: u16) -> Rect {
    let mut x = anchor.x + x_offset;
    let mut y = anchor.y + anchor.height;
    let max_x = screen.x + screen.width.saturating_sub(w);
    let max_y = screen.y + screen.height.saturating_sub(h);
    if x > max_x {
        x = max_x;
    }
    if y > max_y {
        y = max_y;
    }
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

/// The timesheet date-picker calendar: `CALENDAR_W` × [`CALENDAR_H`], centered
/// like the other overlays.
#[must_use]
pub(crate) fn timesheet_calendar_rect(parent: Rect) -> Rect {
    centered_in(parent, CALENDAR_W, CALENDAR_H)
}

/// The due/threshold calendar picker: fixed `CALENDAR_POPUP_W` ×
/// [`CALENDAR_POPUP_H`], anchored under the dialog.
#[must_use]
pub(crate) fn calendar_popup_rect(dlg: Rect, screen: Rect) -> Rect {
    anchored_below(
        dlg,
        screen,
        CALENDAR_POPUP_W,
        CALENDAR_POPUP_H,
        INPUT_PREFIX_OFFSET,
    )
}

/// The recurrence builder: fixed `RECURRENCE_POPUP_W` ×
/// [`RECURRENCE_POPUP_H`], anchored under the dialog.
#[must_use]
pub(crate) fn recurrence_popup_rect(dlg: Rect, screen: Rect) -> Rect {
    anchored_below(
        dlg,
        screen,
        RECURRENCE_POPUP_W,
        RECURRENCE_POPUP_H,
        INPUT_PREFIX_OFFSET,
    )
}

/// The priority chooser: fixed `PRIORITY_POPUP_W` × [`PRIORITY_POPUP_H`],
/// anchored under the dialog.
#[must_use]
pub(crate) fn priority_popup_rect(dlg: Rect, screen: Rect) -> Rect {
    anchored_below(
        dlg,
        screen,
        PRIORITY_POPUP_W,
        PRIORITY_POPUP_H,
        INPUT_PREFIX_OFFSET,
    )
}

/// The duration picker: fixed `DURATION_POPUP_W` wide, one row per preset
/// plus a spacer and footer (`+ 4` for title, blank, footer, border), anchored
/// under the dialog.
#[must_use]
pub(crate) fn duration_popup_rect(dlg: Rect, screen: Rect, presets: usize) -> Rect {
    anchored_below(
        dlg,
        screen,
        DURATION_POPUP_W,
        presets as u16 + 4,
        INPUT_PREFIX_OFFSET,
    )
}

/// The slash menu: content-driven width (longest label/description/cmd line),
/// never narrower than [`SLASH_MIN_W`], capped to the screen; one row per
/// entry plus a spacer, footer, and title (`+ 5`), anchored under the dialog.
#[must_use]
pub(crate) fn slash_popup_rect(dlg: Rect, screen: Rect, content_w: usize, entries: usize) -> Rect {
    let w = (content_w as u16)
        .max(SLASH_MIN_W)
        .min(screen.width.max(40));
    anchored_below(dlg, screen, w, entries as u16 + 5, INPUT_PREFIX_OFFSET)
}

/// The autocomplete suggestions: content-driven width (longest match, `+ 3`
/// for the leading space, sigil, and trailing space), never narrower than 16,
/// capped to the dialog's width; one row per match, anchored under the dialog.
#[must_use]
pub(crate) fn autocomplete_popup_rect(
    dlg: Rect,
    screen: Rect,
    longest_match: usize,
    matches: usize,
) -> Rect {
    let w = (longest_match as u16)
        .saturating_add(3)
        .max(16)
        .min(dlg.width.max(16));
    anchored_below(dlg, screen, w, matches as u16, INPUT_PREFIX_OFFSET)
}

/// The day-boundary prompt: `DAY_BOUNDARY_W` wide, `wrapped_lines` rows of
/// message (plus borders, blank, footer) tall, keeping [`DAY_BOUNDARY_MIN_H`]
/// for short messages and capping at the parent's height.
#[must_use]
pub(crate) fn day_boundary_rect(parent: Rect, wrapped_lines: u16) -> Rect {
    let w = DAY_BOUNDARY_W.min(parent.width);
    let h = (wrapped_lines + 4)
        .max(DAY_BOUNDARY_MIN_H)
        .min(parent.height.saturating_sub(2).max(5));
    centered_in(parent, w, h)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rect(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    #[test]
    fn centered_in_clamps_to_parent() {
        // Oversized overlay shrinks to the parent and stays centered.
        assert_eq!(
            centered_in(rect(100, 30), 300, 60),
            Rect::new(0, 0, 100, 30)
        );
        // Smaller overlay is centered with equal margins.
        assert_eq!(centered_in(rect(100, 30), 40, 6), Rect::new(30, 12, 40, 6));
    }

    #[test]
    fn insert_dialog_rect_clamps_width() {
        assert_eq!(
            insert_dialog_rect(rect(100, 30), 60),
            Rect::new(26, 11, 48, 8),
            "4/5 of a 60-wide centre pane = 48"
        );
        // Below the minimum: clamps up to DIALOG_MIN_W.
        assert_eq!(
            insert_dialog_rect(rect(100, 30), 10),
            Rect::new(30, 11, 40, 8)
        );
        // Above the maximum: clamps down to DIALOG_MAX_W.
        assert_eq!(
            insert_dialog_rect(rect(100, 30), 500),
            Rect::new(0, 11, 100, 8)
        );
    }

    #[test]
    fn prompt_rect_caps_width_and_leaves_margin() {
        assert_eq!(prompt_rect(rect(200, 30)), Rect::new(75, 12, 50, 5));
        // Narrow terminal: capped to area width - 4.
        assert_eq!(prompt_rect(rect(30, 30)), Rect::new(2, 12, 26, 5));
    }

    #[test]
    fn palette_rect_clamps() {
        // Wide terminal: 3/5 of width within [50, 80].
        assert_eq!(palette_rect(rect(200, 30)), Rect::new(60, 5, 80, 20));
        // Narrow terminal: clamps down to PALETTE_MIN_W.
        assert_eq!(palette_rect(rect(40, 30)), Rect::new(0, 5, 40, 20));
    }

    #[test]
    fn empty_state_rect_clamps_with_margins() {
        assert_eq!(empty_state_rect(rect(100, 30)), Rect::new(22, 7, 56, 16));
        // Narrow parent: shrinks to width-4 / height-2.
        assert_eq!(empty_state_rect(rect(30, 10)), Rect::new(2, 1, 26, 8));
    }

    #[test]
    fn day_boundary_rect_grows_with_wrapped_lines() {
        // One wrapped line keeps the minimum height.
        assert_eq!(
            day_boundary_rect(rect(100, 30), 1),
            Rect::new(16, 11, 68, 7)
        );
        // Four wrapped lines grow the box (8 rows, centered in 30).
        assert_eq!(
            day_boundary_rect(rect(100, 30), 4),
            Rect::new(16, 11, 68, 8)
        );
        assert_eq!(day_boundary_wrap_w(rect(100, 30)), 66);
        // Narrow terminal shrinks the width and the wrap width with it.
        assert_eq!(day_boundary_rect(rect(50, 30), 1), Rect::new(0, 11, 50, 7));
        assert_eq!(day_boundary_wrap_w(rect(50, 30)), 48);
    }

    #[test]
    fn message_rect_for_grows_with_wrapped_lines() {
        // One wrapped line keeps the original 6-row box (snapshot-stable).
        assert_eq!(message_rect_for(rect(100, 30), 1), Rect::new(20, 12, 60, 6));
        assert_eq!(message_wrap_w(rect(100, 30)), 58);
        // The stale-timer message wraps to three rows: the box grows to 7 so
        // the [k]/[s]/[d] footer is never clipped.
        assert_eq!(message_rect_for(rect(100, 30), 3), Rect::new(20, 11, 60, 7));
        // Narrow terminal shrinks the width and the wrap width with it.
        assert_eq!(message_rect_for(rect(50, 30), 3), Rect::new(0, 11, 50, 7));
        assert_eq!(message_wrap_w(rect(50, 30)), 48);
    }

    #[test]
    fn anchored_below_floats_just_under_the_anchor() {
        let screen = Rect::new(0, 0, 100, 30);
        let dlg = Rect::new(30, 11, 40, 8);
        // Aligned to the 4-col input prefix, directly below the dialog.
        assert_eq!(
            anchored_below(dlg, screen, 24, 6, 4),
            Rect::new(34, 19, 24, 6)
        );
    }

    #[test]
    fn anchored_below_clamps_at_bottom_and_right_edges() {
        let screen = Rect::new(0, 0, 50, 20);
        // A dialog hugging the bottom-right corner: the popup must shift
        // up/left so it stays fully on-screen.
        let dlg = Rect::new(40, 16, 10, 4);
        assert_eq!(
            anchored_below(dlg, screen, 16, 4, 4),
            Rect::new(34, 16, 16, 4),
            "x = 40+4 = 44, max_x = 50-16 = 34 → clamp to 34; y = 20, max_y = 20-4 = 16 → clamp to 16"
        );
    }

    #[test]
    fn timesheet_calendar_rect_centers_and_clamps() {
        assert_eq!(
            timesheet_calendar_rect(rect(100, 30)),
            Rect::new(25, 8, 50, 14)
        );
        // Narrow terminal: shrinks to the parent width, still centered.
        assert_eq!(
            timesheet_calendar_rect(rect(30, 30)),
            Rect::new(0, 8, 30, 14)
        );
    }
    #[test]
    fn popup_rects_anchor_below_the_dialog() {
        let dlg = Rect::new(30, 11, 40, 8);
        let screen = Rect::new(0, 0, 100, 30);
        // The 13-row calendar would overflow a 30-row screen at y=19, so it
        // clamps up to keep the whole popup visible; the shorter popups fit.
        assert_eq!(calendar_popup_rect(dlg, screen), Rect::new(34, 17, 50, 13));
        assert_eq!(recurrence_popup_rect(dlg, screen), Rect::new(34, 19, 60, 9));
        assert_eq!(priority_popup_rect(dlg, screen), Rect::new(34, 19, 24, 8));
        assert_eq!(
            duration_popup_rect(dlg, screen, 5),
            Rect::new(34, 19, 36, 9)
        );
    }

    #[test]
    fn slash_popup_rect_grows_to_content_and_clamps_to_screen() {
        let dlg = Rect::new(30, 11, 40, 8);
        let screen = Rect::new(0, 0, 100, 30);
        // Narrow content: kept at the minimum width.
        assert_eq!(
            slash_popup_rect(dlg, screen, 10, 4),
            Rect::new(34, 19, 60, 9)
        );
        // Wide content: grows past the minimum, then clamps x so the popup
        // (90 wide) stays inside the 100-col screen.
        assert_eq!(
            slash_popup_rect(dlg, screen, 90, 4),
            Rect::new(10, 19, 90, 9)
        );
        // Narrow screen: width caps to the screen, x clamps to the left edge,
        // and the 9-row popup clamps up to fit the 20-row screen.
        let narrow = Rect::new(0, 0, 50, 20);
        assert_eq!(
            slash_popup_rect(dlg, narrow, 90, 4),
            Rect::new(0, 11, 50, 9),
            "x and y clamp to the screen edges; width caps to the screen"
        );
    }

    #[test]
    fn autocomplete_popup_rect_sizes_and_caps() {
        let dlg = Rect::new(30, 11, 40, 8);
        let screen = Rect::new(0, 0, 100, 30);
        // Longest match 7 chars: 7 + 3 = 10, floored to 16.
        assert_eq!(
            autocomplete_popup_rect(dlg, screen, 7, 3),
            Rect::new(34, 19, 16, 3)
        );
        // Longest match 30 chars: grows, but capped to the dialog width.
        assert_eq!(
            autocomplete_popup_rect(dlg, screen, 30, 2),
            Rect::new(34, 19, 33, 2)
        );
    }
}
