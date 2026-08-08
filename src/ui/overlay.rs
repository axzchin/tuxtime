//! Overlay sizing: the constants and rect math for every floating overlay
//! (add/edit dialog, help, welcome, text prompts, command palette, theme
//! picker, prompt message boxes, day-boundary prompt). `draw` in `ui::mod`
//! renders each overlay into the rect computed here, so every overlay's
//! footprint — width, height, and clamping rules — lives in one place.

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

/// Static prompt message boxes (idle nudge, manual-entry choice).
const MESSAGE_W: u16 = 60;
const MESSAGE_H: u16 = 6;

/// Day-boundary prompt: fixed width, height grows with the wrapped narrative.
const DAY_BOUNDARY_W: u16 = 68;
const DAY_BOUNDARY_MIN_H: u16 = 7;

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

/// The static prompt message boxes (idle nudge, manual-entry choice).
#[must_use]
pub(crate) fn message_rect(parent: Rect) -> Rect {
    centered_in(parent, MESSAGE_W, MESSAGE_H)
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
}
