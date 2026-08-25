use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Block;

use crate::app::{App, Mode, Screen, View};

pub mod archive;
pub mod calendar_utils;
pub mod command_palette;
pub mod detail;
pub mod dialog;
pub mod empty;
pub mod filters;
pub mod header;
pub mod help;
pub(crate) mod hints;
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
        if app.nav.mode == Mode::Screen(Screen::Search) {
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

/// Number of rendered rows a [`Line`] occupies when a [`Paragraph`] wraps it
/// with `Wrap { trim: false }` at `width` columns. The timesheet renders with
/// word-wrap so long narratives stay fully visible, which means its scroll
/// offset lives in *wrapped-row* space, not source-line space: to keep the
/// cursor narrative on screen we must know how many rows every earlier line
/// consumed.
///
/// This mirrors ratatui's `WordWrapper` (`ratatui_widgets::reflow`) exactly
/// — the same grapheme stream, the same whitespace/overflow rules — so the
/// count agrees with what `Paragraph` actually renders. Width-only
/// bookkeeping; no graphemes are stored.
pub(crate) fn wrapped_row_count(line: &Line, width: u16) -> u16 {
    use std::collections::VecDeque;
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    if width == 0 {
        return 0;
    }
    let max = width;
    let mut rows = 0u16;
    // Vec-emptiness mirrors (a Vec can hold zero-width graphemes, so width
    // alone can't tell whether it's empty).
    let mut pending_line_has = false;
    let mut pending_word_has = false;
    let mut pending_ws_has = false;
    let mut line_width = 0u16;
    let mut word_width = 0u16;
    let mut whitespace_width = 0u16;
    let mut non_ws_prev = false;
    // Widths of the queued whitespace graphemes, in order — the flush path
    // pops from the front, so a deque (not a sum) is needed.
    let mut pending_ws: VecDeque<u16> = VecDeque::new();

    let graphemes = line
        .spans
        .iter()
        .flat_map(|span| span.content.graphemes(true));
    for g in graphemes {
        // Matches `StyledGrapheme::is_whitespace`: ZWSP counts, NBSP does not.
        let is_ws = g == "\u{200B}" || (g.chars().all(char::is_whitespace) && g != "\u{00A0}");
        let symbol_width = UnicodeWidthStr::width(g) as u16;
        // Symbols wider than the line are dropped by the wrapper.
        if symbol_width > max {
            continue;
        }

        let word_found = non_ws_prev && is_ws;
        // trim=false: only the untrimmed overflow rule fires.
        let untrimmed_overflow =
            !pending_line_has && word_width + whitespace_width + symbol_width > max;
        if word_found || untrimmed_overflow {
            // trim=false: always append the queued whitespace, then the word.
            pending_line_has = pending_line_has || pending_ws_has || pending_word_has;
            line_width += whitespace_width;
            whitespace_width = 0;
            pending_ws.clear();
            pending_ws_has = false;
            line_width += word_width;
            word_width = 0;
            pending_word_has = false;
        }

        let line_full = line_width >= max;
        let pending_word_overflow =
            symbol_width > 0 && line_width + whitespace_width + word_width >= max;
        if line_full || pending_word_overflow {
            rows += 1;
            let mut remaining = max.saturating_sub(line_width);
            pending_line_has = false;
            line_width = 0;
            // Remove leading whitespace that would overflow the flushed row.
            while let Some(w) = pending_ws.front() {
                if *w > remaining {
                    break;
                }
                whitespace_width -= w;
                remaining -= w;
                pending_ws.pop_front();
            }
            pending_ws_has = !pending_ws.is_empty();
            if is_ws && !pending_ws_has {
                // This whitespace grapheme is dropped entirely (it would
                // start the next row); `non_ws_prev` is deliberately
                // unchanged, mirroring the wrapper's `continue`.
                continue;
            }
        }

        if is_ws {
            whitespace_width += symbol_width;
            pending_ws.push_back(symbol_width);
            pending_ws_has = true;
        } else {
            word_width += symbol_width;
            pending_word_has = true;
        }
        non_ws_prev = !is_ws;
    }

    // Tail: with trim=false the queued whitespace is always flushed into the
    // pending line, then the word, then the line is emitted if non-empty.
    let flushed_any = pending_line_has || pending_ws_has || pending_word_has;
    if flushed_any {
        rows += 1;
    }
    // An empty source line still occupies one rendered row.
    if rows == 0 {
        rows = 1;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::{keep_cursor_visible, wrapped_row_count, Line};

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

    // ── wrapped_row_count ────────────────────────────────────────────

    fn line(s: &str) -> Line<'static> {
        Line::raw(s.to_string())
    }

    #[test]
    fn short_lines_occupy_one_row() {
        assert_eq!(wrapped_row_count(&line("hello"), 20), 1);
        assert_eq!(wrapped_row_count(&line(""), 20), 1, "blank line is a row");
        assert_eq!(wrapped_row_count(&line("  "), 20), 1);
    }

    #[test]
    fn zero_width_renders_nothing() {
        // `WordWrapper::next_line` bails when max_line_width == 0.
        assert_eq!(wrapped_row_count(&line("hello"), 0), 0);
    }

    #[test]
    fn wraps_at_word_boundaries() {
        // "one two three four" wraps at word boundaries: at width 10 →
        // "one two" + "three four" (2 rows), at width 8 → "one two" +
        // "three" + "four" (3 rows).
        let s = "one two three four";
        assert_eq!(wrapped_row_count(&line(s), 10), 2);
        assert_eq!(wrapped_row_count(&line(s), 8), 3);
        assert_eq!(wrapped_row_count(&line(s), 4), 5);
    }

    #[test]
    fn long_single_word_stays_whole() {
        // trim=false never breaks a word: a 12-char word at width 5 overflows
        // whole. The word's own row is emitted when it would overflow the
        // pane ("abcdef"), and each further mid-word overflow emits the row
        // so far — blank at the word's start, hence the empty row.
        assert_eq!(wrapped_row_count(&line("abcdefghijkl"), 5), 3);
        // "ab" / "abcdef" / (blank) / "kl" — the untrimmed-overflow flush
        // at 'f' emits "abcdef" as a real (exactly-fitting) row.
        assert_eq!(wrapped_row_count(&line("ab abcdefghijkl"), 5), 4);
        // A word narrower than the pane: clean two-row wrap.
        assert_eq!(wrapped_row_count(&line("abcd efgh"), 4), 2);
    }

    #[test]
    fn exact_fit_is_single_row() {
        // A line exactly as wide as the pane must not wrap.
        let s = "1234567890";
        assert_eq!(wrapped_row_count(&line(s), 10), 1);
        let s = "1234567890 x";
        assert_eq!(wrapped_row_count(&line(s), 10), 2);
    }

    #[test]
    fn unicode_width_counts_wide_chars() {
        // Two CJK chars (width 2 each) = 4 columns, so they fit on one row at
        // width 5; at width 3 the pair (4 wide) is wider than the pane and,
        // like any unbreakable word, overflows whole onto a single row.
        assert_eq!(wrapped_row_count(&line("汉字"), 5), 1);
        assert_eq!(wrapped_row_count(&line("汉字"), 3), 1);
        // 4 wide + words: "汉字" / "and" / "more".
        assert_eq!(wrapped_row_count(&line("汉字 and more"), 5), 3);
    }

    /// The row count must agree with what ratatui's `Paragraph` + `Wrap`
    /// actually renders, or the timesheet scroll math would drift off-screen.
    /// Render each case into a scratch buffer and count non-blank rows.
    #[test]
    fn wrapped_row_count_matches_ratatui_render() {
        use ratatui::backend::TestBackend;
        use ratatui::layout::Rect;
        use ratatui::widgets::{Paragraph, Wrap};
        use ratatui::Terminal;

        fn rendered_rows(s: &str, width: u16) -> usize {
            let backend = TestBackend::new(width.max(1), 50);
            let mut terminal = Terminal::new(backend).expect("term");
            terminal
                .draw(|f| {
                    f.render_widget(
                        Paragraph::new(Line::raw(s.to_string())).wrap(Wrap { trim: false }),
                        Rect::new(0, 0, width, 50),
                    );
                })
                .expect("draw");
            let buf = terminal.backend().buffer();
            let mut rows = 0usize;
            for y in 0..buf.area.height {
                if (0..buf.area.width).any(|x| buf[(x, y)].symbol() != " ") {
                    rows += 1;
                }
            }
            rows
        }

        // Cases whose wrapped output ends on a non-blank row, so the buffer
        // row count is unambiguous. Blank wrapped rows (empty source lines,
        // unbreakable words wider than the pane) are asserted separately —
        // the buffer cannot distinguish them from empty space.
        let cases: &[(&str, u16)] = &[
            ("hello", 20),
            ("one two three four", 10),
            ("one two three four", 8),
            ("1234567890", 10),
            ("1234567890 x", 10),
            ("汉字", 5),
            ("a b c d e f g h i j", 5),
            ("word word word word word word", 7),
            ("x", 1),
            ("aa bb cc", 2),
            ("one  two   three", 6),
        ];
        for (s, w) in cases {
            let count = wrapped_row_count(&line(s), *w);
            let actual = rendered_rows(s, *w);
            assert_eq!(
                usize::from(count),
                actual,
                "row count for {s:?} at width {w} must match ratatui's wrap"
            );
        }
    }
}
