use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::Block;

use crate::app::{App, Mode, View};

pub mod archive;
pub mod calendar_utils;
pub mod command_palette;
pub mod detail;
pub mod dialog;
pub mod empty;
pub mod filters;
pub mod header;
pub mod help;
pub mod hyperlinks;
pub mod list;
pub mod logo;
pub mod msgbox;
pub(crate) mod nudge_picker;
pub(crate) mod overlay;
pub(crate) mod overlays;
pub mod settings;
pub mod share;
pub mod status;
pub mod task_row;
pub mod theme_picker;
pub(crate) mod timesheet_render;
pub mod title;
pub mod welcome;

// Pane sizing. Promoted out of inline literals so the three `MIN_BODY_W`
// references below stay in sync, and so tweaking a sidebar width is a
// one-line change. (Overlay sizes live in `overlay`.)
const LEFT_PANE_W: u16 = 26;
const RIGHT_PANE_W: u16 = 34;
const MIN_BODY_W: u16 = 40;

pub fn draw(frame: &mut Frame, app: &App) {
    let theme = app.theme();
    let area = frame.area();

    // Paint full background.
    frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);

    let bottom = u16::from(app.prefs.layout.status_bar);
    let [body_area, bottom_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(bottom)]).areas(area);

    // Determine pane widths. Sidebars apply to every view; navigation +
    // detail pane track the cursor regardless of which view is active.
    //
    // We compute exact Column widths instead of relying on ratatui's
    // constraint solver, which can zero out `Length` or `Max` panes when
    // the total exceeds the terminal width.  Sidebars get a guaranteed
    // minimum so they never become invisible; extra space goes to the
    // centre body (up to its desired width) and then to the sidebars.
    let show_left = app.prefs.layout.left;
    let show_right = app.prefs.layout.right;

    // Minimum widths for sidebars to remain useful on narrow terminals.
    const MIN_LEFT_W: u16 = 12;
    const MIN_RIGHT_W: u16 = 16;

    let left_desired = if show_left { LEFT_PANE_W } else { 0 };
    let right_desired = if show_right { RIGHT_PANE_W } else { 0 };
    let left_min = if show_left { MIN_LEFT_W } else { 0 };
    let right_min = if show_right { MIN_RIGHT_W } else { 0 };

    let available = body_area.width;
    let total_min = left_min + MIN_BODY_W + right_min;

    let (left_w, center_w, right_w) = if available <= total_min {
        // Even minimums barely fit — give sidebars their floor, centre
        // gets whatever is left.
        let after_sides = available.saturating_sub(left_min + right_min);
        (left_min, after_sides, right_min)
    } else {
        // Minimums fit.  Start there, then grow sidebars toward their
        // desired widths before giving remaining space to the centre.
        let mut extra = available - total_min;
        let mut left = left_min;
        let mut right = right_min;

        if left < left_desired {
            let need = left_desired - left;
            let take = need.min(extra);
            left += take;
            extra -= take;
        }
        if right < right_desired {
            let need = right_desired - right;
            let take = need.min(extra);
            right += take;
            extra -= take;
        }
        (left, MIN_BODY_W + extra, right)
    };

    let constraints: Vec<Constraint> = match (show_left, show_right) {
        (true, true) => vec![
            Constraint::Length(left_w),
            Constraint::Length(center_w),
            Constraint::Length(right_w),
        ],
        (true, false) => vec![Constraint::Length(left_w), Constraint::Length(center_w)],
        (false, true) => vec![Constraint::Length(center_w), Constraint::Length(right_w)],
        (false, false) => vec![Constraint::Length(center_w)],
    };
    let chunks = Layout::horizontal(constraints).split(body_area);

    let (left_area, center_area, right_area) = match (show_left, show_right) {
        (true, true) => (Some(chunks[0]), chunks[1], Some(chunks[2])),
        (true, false) => (Some(chunks[0]), chunks[1], None),
        (false, true) => (None, chunks[0], Some(chunks[1])),
        (false, false) => (None, chunks[0], None),
    };

    if let Some(la) = left_area {
        filters::render(frame, la, app);
    }
    match app.view() {
        View::List => list::render(frame, center_area, app),
        View::Archive => archive::render(frame, center_area, app),
        View::Timesheet => render_timesheet(frame, center_area, app),
    }
    if let Some(ra) = right_area {
        detail::render(frame, ra, app);
    }

    if app.prefs.layout.status_bar {
        if app.nav.mode == Mode::Search {
            status::render_command_line(frame, bottom_area, app);
        } else {
            status::render(frame, bottom_area, app);
        }
    }

    // Overlays: the full z-order contract (body-replacing panels → standalone
    // boxes → anchored popups) lives in `overlays`.
    overlays::draw_overlays(frame, area, body_area, center_area.width, app);

    // OSC 8 hyperlinks are applied post-draw by the caller (see
    // `hyperlinks::collect` + `emit_overlay`). Doing it inside the buffer
    // breaks ratatui's diff width calculation — keep cell symbols pristine.
}

/// Render the timesheet inline in the center area so sidebars remain fully
/// visible.
use crate::ui::timesheet_render::render_timesheet;

pub(crate) fn fill_bg(frame: &mut Frame, area: Rect, style: Style) {
    frame.render_widget(Block::default().style(style), area);
}

pub(crate) fn density_blank_lines(d: crate::app::Density) -> usize {
    match d {
        crate::app::Density::Compact => 0,
        crate::app::Density::Comfortable => 1,
        crate::app::Density::Cozy => 2,
    }
}

/// Compute the new vertical scroll offset for a paragraph-backed list so the
/// cursor row stays inside the viewport. `prev` is the previous frame's offset,
/// `cursor_line` is the line index of the cursor (or `None` if there's no
/// cursor row in the current build, e.g. when the list is empty). `height` is
/// the viewport height in rows; `total` is the total line count.
pub(crate) fn keep_cursor_visible(
    prev: u16,
    cursor_line: Option<usize>,
    height: u16,
    total: usize,
) -> u16 {
    let h = usize::from(height);
    if h == 0 || total == 0 {
        return 0;
    }
    let max_offset = total.saturating_sub(h);
    let prev = usize::from(prev).min(max_offset);
    let new = match cursor_line {
        Some(cl) if cl < prev => cl,
        Some(cl) if cl >= prev + h => cl + 1 - h,
        _ => prev,
    };
    new.min(max_offset).min(usize::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::keep_cursor_visible;

    #[test]
    fn no_scroll_when_content_fits() {
        assert_eq!(keep_cursor_visible(0, Some(5), 10, 8), 0);
        assert_eq!(keep_cursor_visible(0, Some(7), 10, 8), 0);
    }

    #[test]
    fn scrolls_down_when_cursor_below_viewport() {
        // viewport rows 0..5, cursor at line 7 -> offset = 7 - 5 + 1 = 3
        assert_eq!(keep_cursor_visible(0, Some(7), 5, 20), 3);
    }

    #[test]
    fn scrolls_up_when_cursor_above_viewport() {
        // prev offset 10, cursor at line 3 -> offset = 3
        assert_eq!(keep_cursor_visible(10, Some(3), 5, 20), 3);
    }

    #[test]
    fn keeps_previous_offset_when_cursor_in_viewport() {
        // prev 5, cursor at line 7, height 5 -> 7 in [5, 10), stays 5
        assert_eq!(keep_cursor_visible(5, Some(7), 5, 20), 5);
    }

    #[test]
    fn clamps_to_max_offset_when_previous_exceeds_it() {
        // total shrank since last frame; previous offset 50 is now too large.
        assert_eq!(keep_cursor_visible(50, None, 5, 8), 3);
    }

    #[test]
    fn handles_degenerate_inputs() {
        assert_eq!(keep_cursor_visible(0, None, 0, 100), 0);
        assert_eq!(keep_cursor_visible(0, Some(0), 5, 0), 0);
    }
}
