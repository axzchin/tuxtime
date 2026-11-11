//! Timesheet view renderer and date-picker overlay. Extracted from [`super::mod`].

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use chrono::Timelike;

use crate::app::{App, View};
use crate::ui::calendar_utils::{calendar_cells, calendar_footer, format_focused, month_name};
use crate::ui::msgbox;
use crate::ui::overlay::timesheet_calendar_rect;
use crate::ui::task_row::{bill_badge_style, dur_badge_style};
use crate::ui::wrapped_row_count;

/// Plain `1h 5m` / `45m` without the billable parenthetical that
/// [`crate::app::format_duration`] embeds. Used wherever the invoice-correct
/// units figure is appended separately (group rows, day subtotals, the pinned
/// footer) so a billable number never appears twice on one line.
fn h_m(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}m", secs / 60)
    }
}

/// Word-wrap `text` into chunks at most `width` display columns wide, keeping
/// words whole and breaking only a single word that still exceeds the width.
/// Runs of whitespace collapse to single spaces. The timesheet uses this to
/// pre-wrap narrative rows so continuation lines hang indented under the
/// narrative text — ratatui's own word-wrap would drop them flush at the
/// pane's left edge, breaking the entry's visual indent.
fn wrap_chunks(text: &str, width: usize) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;

    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0usize;
    for word in text.split(' ') {
        if word.is_empty() {
            continue;
        }
        let word_w = UnicodeWidthStr::width(word);
        if word_w > width {
            if !cur.is_empty() {
                chunks.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            // A single overlong word (URL, path) is broken at grapheme
            // boundaries so it can never overflow the pane.
            let mut piece = String::new();
            let mut piece_w = 0usize;
            for g in word.graphemes(true) {
                let w = UnicodeWidthStr::width(g);
                if piece_w + w > width && !piece.is_empty() {
                    chunks.push(std::mem::take(&mut piece));
                    piece_w = 0;
                }
                piece.push_str(g);
                piece_w += w;
            }
            if !piece.is_empty() {
                chunks.push(piece);
            }
            continue;
        }
        let sep = usize::from(!cur.is_empty());
        if cur_w + sep + word_w > width {
            chunks.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        if !cur.is_empty() {
            cur.push(' ');
            cur_w += 1;
        }
        cur.push_str(word);
        cur_w += word_w;
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    chunks
}

pub(crate) fn render_timesheet(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();

    let display = app.timesheet_date_display();
    let view_label = if app.timesheet.weekly {
        Span::styled(
            format!(" {display} (week) "),
            Style::default()
                .fg(theme.bg)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" {display} "),
            Style::default()
                .fg(theme.bg)
                .bg(theme.today)
                .add_modifier(Modifier::BOLD),
        )
    };
    let sort_label = Span::styled(
        format!(" {} ", app.timesheet.sort.label()),
        Style::default().fg(theme.dim).bg(theme.panel),
    );
    let billing_filter = if app.timesheet.dnb_only {
        Span::styled(" DNB only ", Style::default().fg(theme.bg).bg(theme.due))
    } else {
        Span::raw("")
    };
    let title = Line::from(vec![
        Span::raw(" Timesheet "),
        view_label,
        sort_label,
        billing_filter,
    ]);
    let inner = msgbox::frame_box(frame, area, theme.border, theme.panel, title);
    let [_pad_top, body_rect] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    // The paragraph wraps at the pane's inner width; narrative rows are
    // pre-wrapped to that width so continuation lines can hang indented.
    let pane_w = usize::from(body_rect.width);

    let groups = app.build_timesheet_groups();
    // Billable rounding increment (decimal hours) from prefs — 0.1 default,
    // 0.25 for fifteen-minute billing, 0 for exact.
    let increment = app.prefs.rounding_increment;

    let mut lines: Vec<Line> = Vec::new();
    // Source-line span of the cursor narrative (first wrapped row .. one past
    // the last), resolved later into *wrapped-row* positions for the scroll
    // math so the whole entry can be kept on screen.
    let mut cursor_span: Option<(usize, usize)> = None;
    // Source lines where a visual block begins (date header, group header,
    // narrative bullet). The scroll offset snaps to these so the top row of
    // the viewport is never a wrapped continuation line.
    let mut block_starts: Vec<usize> = Vec::new();
    // The grand-total block is kept out of `lines` so it can be pinned below
    // the scrollable entry list instead of scrolling off short terminals.
    let mut footer: Vec<Line> = Vec::new();
    if groups.is_empty() {
        let msg = if app.timesheet.dnb_only {
            "  No non-billable (DNB) entries for this period."
        } else if app.filter().search.is_empty() {
            "  No time entries for this period."
        } else {
            "  No entries match your search."
        };
        lines.push(Line::from(Span::styled(
            msg,
            Style::default().fg(theme.dim).bg(theme.panel),
        )));
    } else {
        // Narrative-level cursor: resolve to the group that contains the
        // narrative under the cursor so we can highlight the correct group.
        let selected = app.timesheet_narrative_at(app.timesheet.cursor);
        let selected_group = selected.map(|(gi, _, _)| gi);
        let selected_narrative = selected.map(|(_, ni, _)| ni);
        // Grand/billable/DNB totals come from the domain helper; the renderer
        // only tracks the per-day subtotal it flushes at each date header.
        let totals = crate::app::timesheet_totals(&groups, increment);
        let mut day_total: u64 = 0;
        let mut day_billable_units: u64 = 0;
        let mut last_date: Option<&str> = None;
        for (i, entry) in groups.iter().enumerate() {
            let is_selected = selected_group == Some(i);

            // Emit date header when date changes (visible in weekly + date-sort views).
            if (app.timesheet.weekly
                || matches!(app.timesheet.sort, crate::app::TimesheetSort::Date))
                && last_date != Some(entry.date.as_str())
            {
                // Flush the previous day's subtotal before the new header.
                if last_date.is_some() {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "    ──  {} ({})",
                            h_m(day_total),
                            crate::app::format_billable_units(day_billable_units, increment)
                        ),
                        Style::default().fg(theme.dim).bg(theme.panel),
                    )));
                    lines.push(Line::raw(""));
                    day_total = 0;
                    day_billable_units = 0;
                }
                let date_dow = chrono::NaiveDate::parse_from_str(&entry.date, "%Y-%m-%d")
                    .map_or_else(
                        |_| entry.date.clone(),
                        |d| d.format("%a %Y-%m-%d").to_string(),
                    );
                block_starts.push(lines.len());
                lines.push(Line::from(Span::styled(
                    format!("  {date_dow}"),
                    Style::default()
                        .fg(theme.today)
                        .bg(theme.panel)
                        .add_modifier(Modifier::BOLD),
                )));
                last_date = Some(&entry.date);
            }

            let formatted = h_m(entry.total_secs);
            let billable_str = crate::app::format_billable(entry.total_secs, increment);
            // Copy-flash: briefly highlight the entire group white after copy.
            let copy_flash_active = app
                .timesheet
                .copy_flash
                .is_some_and(|(gi, t)| gi == i && t.elapsed() < crate::app::FLASH_TTL);
            let group_style = if copy_flash_active {
                Style::default()
                    .fg(theme.bg)
                    .bg(theme.fg)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default()
                    .fg(theme.accent)
                    .bg(theme.selection)
                    .add_modifier(Modifier::BOLD)
            } else if !entry.billable {
                Style::default().fg(theme.dim).bg(theme.panel)
            } else {
                Style::default()
                    .fg(theme.accent)
                    .bg(theme.panel)
                    .add_modifier(Modifier::BOLD)
            };
            // Keep duration and billing status as compact chips, matching
            // list/archive rows. The billable-unit parenthetical is appended
            // once (the badge carries plain h/m) so the figure isn't doubled.
            let group_prefix = format!("  {}  — ", entry.key);
            let mut group_spans = vec![Span::styled(group_prefix, group_style)];
            if copy_flash_active {
                group_spans.push(Span::styled(
                    format!(" {formatted} "),
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.fg)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                group_spans.push(Span::styled(
                    format!(" {formatted} "),
                    dur_badge_style(false, theme, app.prefs.badge_theme),
                ));
            }
            // The rounded billable figure is shown for every group, billable
            // or not: dropping it for DNB left the header without the figure
            // the detail pane reports as `group dur`, so the group's time was
            // only findable in the sidebar.
            group_spans.push(Span::styled(format!(" ({billable_str})"), group_style));
            if !entry.billable {
                group_spans.push(Span::raw(" "));
                let dnb_style = if copy_flash_active {
                    Style::default()
                        .fg(theme.bg)
                        .bg(theme.fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    bill_badge_style(false, theme, app.prefs.badge_theme)
                };
                group_spans.push(Span::styled("DNB", dnb_style));
            }
            let group_line = Line::from(group_spans);
            block_starts.push(lines.len());
            lines.push(group_line);

            for (ni, n) in entry.narratives.iter().enumerate() {
                let is_narr_cursor = is_selected && selected_narrative == Some(ni);
                let (is_done, is_archived) =
                    entry
                        .task_indices
                        .get(ni)
                        .map_or((false, false), |r| match r {
                            crate::app::TimesheetTaskRef::Active(abs) => {
                                (app.tasks().get(*abs).is_some_and(|t| t.done), false)
                            }
                            crate::app::TimesheetTaskRef::Archived(_) => (false, true),
                        });
                let status_suffix = if is_archived {
                    " (archived)"
                } else if is_done {
                    " (done)"
                } else {
                    ""
                };
                let narr_style = if copy_flash_active {
                    Style::default().fg(theme.bg).bg(theme.fg)
                } else if is_selected {
                    if is_narr_cursor {
                        // Full-line highlight, matching the list view's
                        // cursor row: `theme.cursor` background over the whole
                        // line (not just the bullet) so the current narrative
                        // is unmistakable, with `theme.fg` text for contrast.
                        Style::default()
                            .fg(theme.fg)
                            .bg(theme.cursor)
                            .add_modifier(Modifier::BOLD)
                    } else if is_archived || is_done || !entry.billable {
                        // Dim keeps a single meaning: completed / archived /
                        // non-billable. "Which narrative am I on" is the
                        // triangle + full-line highlight, so open siblings
                        // stay at the normal foreground instead of looking
                        // done.
                        Style::default().fg(theme.dim).bg(theme.selection)
                    } else {
                        Style::default().fg(theme.fg).bg(theme.selection)
                    }
                } else if is_archived || is_done || !entry.billable {
                    Style::default().fg(theme.dim).bg(theme.panel)
                } else {
                    Style::default().fg(theme.fg).bg(theme.panel)
                };
                // The cursor narrative's bullet becomes a `▸` indicator
                // triangle (the list view's cursor glyph), accent-colored so
                // it reads at a glance on the highlighted row. Same column as
                // the `•` bullets — both are single-width, so nothing shifts.
                let bullet = if is_narr_cursor {
                    if copy_flash_active {
                        // Join the group's white flash: keeping the cursor
                        // background here would leave a dark box around the
                        // triangle while every other cell inverts.
                        Span::styled("▸", narr_style)
                    } else {
                        // The triangle keeps the cursor background explicitly:
                        // with only an accent foreground it would inherit the
                        // paragraph's panel background and punch a gap in the
                        // full-line highlight.
                        Span::styled(
                            "▸",
                            Style::default()
                                .fg(theme.accent)
                                .bg(theme.cursor)
                                .add_modifier(Modifier::BOLD),
                        )
                    }
                } else {
                    Span::styled("•", narr_style)
                };
                // Per-task duration as a quiet secondary label, shown only
                // when the group merges several tasks — a single-task group's
                // badge already is that task's time, so a second figure would
                // just duplicate it. Dim plain text (no chip, no billable
                // parenthetical) keeps the loud accent badge on the group
                // header visually primary. Right-padded to a fixed-width
                // block so the per-task figures read as a tidy column.
                let dur_block: Option<(String, Style)> = if entry.task_indices.len() > 1
                    && let Some(d) = entry
                        .task_indices
                        .get(ni)
                        .and_then(|r| app.timesheet_task(*r).and_then(|t| t.dur))
                {
                    let dur_text = if d >= 3600 {
                        format!("{}h {}m", d / 3600, (d % 3600) / 60)
                    } else {
                        format!("{}m", d / 60)
                    };
                    let dur_style = if copy_flash_active {
                        Style::default().fg(theme.bg).bg(theme.fg)
                    } else if is_narr_cursor {
                        // The duration sits on the highlighted row and
                        // joins the narrative's highlight: bright text on
                        // the cursor background (not dim, which would read
                        // as a secondary label mid-highlight).
                        Style::default()
                            .fg(theme.fg)
                            .bg(theme.cursor)
                            .add_modifier(Modifier::BOLD)
                    } else if is_selected {
                        Style::default().fg(theme.dim).bg(theme.selection)
                    } else {
                        Style::default().fg(theme.dim).bg(theme.panel)
                    };
                    Some((format!(" {:>7}", dur_text), dur_style))
                } else {
                    None
                };
                // Pre-wrap the narrative so wrapped continuation rows hang
                // indented under the text (column 6, aligned with the first
                // line's narrative) instead of snapping flush to the pane's
                // left edge. Each emitted row fits the pane width, so the
                // paragraph's own word-wrap never re-triggers on them.
                let chunks = wrap_chunks(&format!("{n}{status_suffix}"), pane_w.saturating_sub(6));
                let last_chunk_w = chunks
                    .last()
                    .map_or(0, |c| unicode_width::UnicodeWidthStr::width(c.as_str()));
                // The duration rides the last text row when it fits, else it
                // gets its own (still indented) continuation row.
                let dur_on_last = dur_block.as_ref().is_some_and(|(d, _)| {
                    6 + last_chunk_w + unicode_width::UnicodeWidthStr::width(d.as_str()) <= pane_w
                });
                let mut narr_rows: Vec<Line> = Vec::with_capacity(chunks.len() + 1);
                for (ci, chunk) in chunks.iter().enumerate() {
                    let last = ci + 1 == chunks.len();
                    // The first row carries the bullet; continuation rows pad
                    // to the narrative's text column. Indents are styled like
                    // the row so the full-line cursor highlight fills the
                    // leading gutter on every wrapped line.
                    let mut spans: Vec<Span> = if ci == 0 {
                        vec![
                            Span::styled("    ", narr_style),
                            bullet.clone(),
                            Span::styled(" ", narr_style),
                        ]
                    } else {
                        vec![Span::styled("      ", narr_style)]
                    };
                    spans.push(Span::styled(chunk.clone(), narr_style));
                    if last
                        && dur_on_last
                        && let Some((d, s)) = &dur_block
                    {
                        spans.push(Span::styled(d.clone(), *s));
                    }
                    narr_rows.push(Line::from(spans));
                }
                // Whitespace-only narrative: keep the bullet row so the entry
                // never disappears entirely.
                if chunks.is_empty() {
                    narr_rows.push(Line::from(vec![
                        Span::styled("    ", narr_style),
                        bullet,
                        Span::styled(" ", narr_style),
                    ]));
                }
                if !dur_on_last && let Some((d, s)) = &dur_block {
                    narr_rows.push(Line::from(vec![
                        Span::styled("      ", narr_style),
                        Span::styled(d.clone(), *s),
                    ]));
                }
                // No trailing padding: the highlight stops at the end of the
                // narrative (or the per-task duration in a multi-task group),
                // so the cursor bar is exactly as wide as the row's content
                // instead of bleeding to the pane's right edge.
                block_starts.push(lines.len());
                if is_narr_cursor {
                    cursor_span = Some((lines.len(), lines.len() + narr_rows.len()));
                }
                lines.append(&mut narr_rows);
            }
            let units = crate::app::billable_units(entry.total_secs, increment);
            day_total += entry.total_secs;
            day_billable_units += units;
        }
        // Flush the final day's subtotal (if date headers were shown).
        if (app.timesheet.weekly || matches!(app.timesheet.sort, crate::app::TimesheetSort::Date))
            && last_date.is_some()
        {
            lines.push(Line::from(Span::styled(
                format!(
                    "    ──  {} ({})",
                    h_m(day_total),
                    crate::app::format_billable_units(day_billable_units, increment)
                ),
                Style::default().fg(theme.dim).bg(theme.panel),
            )));
            lines.push(Line::raw(""));
        }
        // Grand total with billable / DNB split. Built into `footer` so the
        // billable figure stays pinned while the entry list above scrolls.
        footer.push(Line::raw(""));
        let search_note = if app.filter().search.is_empty() {
            ""
        } else {
            " (filtered)"
        };
        let billable_str = crate::app::format_billable_units(totals.billable_units, increment);
        footer.push(Line::from(Span::styled(
            format!(
                "  Billable:     {} ({billable_str})",
                h_m(totals.billable_secs),
            ),
            Style::default()
                .fg(theme.accent)
                .bg(theme.panel)
                .add_modifier(Modifier::BOLD),
        )));
        if totals.non_billable_secs > 0 {
            let dnb_str = crate::app::format_billable_units(totals.non_billable_units, increment);
            footer.push(Line::from(Span::styled(
                format!(
                    "  Non-billable: {} ({dnb_str})",
                    h_m(totals.non_billable_secs)
                ),
                Style::default()
                    .fg(theme.dim)
                    .bg(theme.panel)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        let total_str = crate::app::format_billable_units(totals.total_units, increment);
        footer.push(Line::from(Span::styled(
            format!(
                "  Total:        {} ({total_str}){search_note}",
                h_m(totals.total_secs)
            ),
            Style::default()
                .fg(theme.accent)
                .bg(theme.panel)
                .add_modifier(Modifier::BOLD),
        )));
        // End-of-day coverage: how much of the configured workday this day's
        // entries account for (daily view only — weekly spans many days).
        // Bare h/m formatting here; the billable parens are for copy, not
        // for an audit line.
        if !app.timesheet.weekly {
            let now = chrono::Local::now();
            let now_min = now.hour() * 60 + now.minute();
            if let Some((span_secs, unaccounted, in_progress)) =
                app.workday_coverage(&app.timesheet.date, totals.total_secs, now_min, app.today())
            {
                let fmt = |s: u64| format!("{}h {}m", s / 3600, (s % 3600) / 60);
                let suffix = if in_progress {
                    " — day in progress"
                } else {
                    ""
                };
                footer.push(Line::from(Span::styled(
                    format!(
                        "  Unaccounted: {} of {}{suffix}",
                        fmt(unaccounted),
                        fmt(span_secs)
                    ),
                    Style::default()
                        .fg(theme.dim)
                        .bg(theme.panel)
                        .add_modifier(Modifier::BOLD),
                )));
            }
        }
    }

    // Scroll if content exceeds space. The footer stays pinned: the entry list
    // gets whatever height remains after the footer's own rows are reserved,
    // so the billable/total figures can never be clipped away.
    let footer_h = footer.len() as u16;
    let [list_rect, footer_rect] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(footer_h)]).areas(body_rect);
    // The paragraph word-wraps long narratives at the pane's inner width, so
    // the scroll offset lives in *wrapped-row* space: the cursor narrative's
    // source-line index maps to a rendered row only after summing how many
    // rows each earlier line consumed (long narratives take several).
    let list_width = list_rect.width;
    // `row_offsets[i]` is the first rendered row of source line `i`, with a
    // sentinel at the end so a span ending at `lines.len()` maps cleanly.
    let mut row_offsets: Vec<usize> = Vec::with_capacity(lines.len() + 1);
    let mut total_rows: usize = 0;
    for line in &lines {
        row_offsets.push(total_rows);
        total_rows += usize::from(wrapped_row_count(line, list_width));
    }
    row_offsets.push(total_rows);
    let last_offset = row_offsets.len() - 1;
    let cursor_rows = cursor_span.map(|(start, end)| {
        (
            row_offsets[start.min(last_offset)],
            row_offsets[end.min(last_offset)],
        )
    });
    let boundaries: Vec<usize> = block_starts
        .iter()
        .map(|s| row_offsets[(*s).min(last_offset)])
        .collect();
    let scroll_cell = &app.nav.view_scroll[View::Timesheet];
    let scroll = entry_scroll(
        scroll_cell.get(),
        cursor_rows,
        &boundaries,
        list_rect.height,
        total_rows,
    );
    scroll_cell.set(scroll);
    // Long narratives word-wrap at the pane's inner width instead of clipping
    // at the border: the right margin is the detail pane's boundary, so the
    // duration badge and `(done)` status at the end of a long line stay
    // visible instead of being cut off. The footer is a separate paragraph,
    // so wrapping here can never push the totals off-screen.
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        list_rect,
    );
    if !footer.is_empty() {
        frame.render_widget(Paragraph::new(footer), footer_rect);
    }
}

/// Scroll offset (in wrapped rows) for the timesheet entry list.
///
/// The list scrolls in whole entries: the viewport's first row is always a
/// block start (date header, group header, or narrative bullet) and the
/// cursor's entry — its bullet plus every wrapped continuation row — is kept
/// entirely on screen. An entry therefore never appears cut off at the top or
/// bottom edge; when the block sizes don't divide the viewport evenly the list
/// ends a row or two early, leaving blank rows above the pinned totals rather
/// than a half-visible narrative.
///
/// An entry taller than the viewport can't fit whole; it is pinned to its
/// first row (its banner or bullet) so the reader sees where it begins.
fn entry_scroll(
    prev: u16,
    cursor_rows: Option<(usize, usize)>,
    boundaries: &[usize],
    height: u16,
    total: usize,
) -> u16 {
    let h = usize::from(height);
    if h == 0 || total == 0 {
        return 0;
    }
    let prev = usize::from(prev);
    let Some((start, end)) = cursor_rows else {
        // No cursor entry (empty list or all groups filtered out): keep the
        // viewport inside the content so the "no entries" line stays visible.
        return u16::try_from(prev.min(total.saturating_sub(h))).unwrap_or(u16::MAX);
    };
    if end <= start {
        return u16::try_from(prev).unwrap_or(u16::MAX);
    }
    // Taller than the viewport: pin the entry's first row so the reader sees
    // where it starts instead of an arbitrary middle slice.
    if end - start >= h {
        return u16::try_from(start).unwrap_or(u16::MAX);
    }
    // Already on screen: leave the offset alone so a redraw (or a toggle that
    // re-sorts nothing) doesn't shift the view under the reader.
    if start >= prev && end <= prev + h {
        return u16::try_from(prev).unwrap_or(u16::MAX);
    }
    // Otherwise pick the smallest block start that still shows the entry's
    // last row, so the top row belongs to a whole entry.
    let min_start = end.saturating_sub(h);
    let offset = boundaries
        .iter()
        .copied()
        .find(|b| *b >= min_start)
        .map_or(start, |b| b.min(start));
    u16::try_from(offset).unwrap_or(u16::MAX)
}

/// Render a centered calendar overlay for the timesheet date picker.
pub(crate) fn render_timesheet_calendar(frame: &mut Frame, area: Rect, app: &App) {
    use chrono::{Datelike, NaiveDate};
    let theme = app.theme();
    let focused = app.timesheet.calendar_focus;
    let today = NaiveDate::parse_from_str(app.today(), "%Y-%m-%d").ok();
    let first_of_month =
        NaiveDate::from_ymd_opt(focused.year(), focused.month(), 1).unwrap_or(focused);

    let r = timesheet_calendar_rect(area);
    frame.render_widget(Clear, r);

    let inner = msgbox::frame_box(
        frame,
        r,
        theme.border,
        theme.panel,
        Line::from(Span::styled(
            " JUMP TO DATE ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
    );

    let mut lines: Vec<Line> = Vec::new();
    // Input line: show typed date (or placeholder when empty).
    let input_text = if app.timesheet.date_input.is_empty() {
        format!("{}  ", format_focused(focused))
    } else {
        format!("{}█ ", app.timesheet.date_input)
    };
    lines.push(
        Line::from(vec![
            Span::raw("  "),
            Span::styled("date: ", Style::default().fg(theme.dim)),
            Span::styled(
                input_text,
                Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::default().bg(theme.panel)),
    );
    lines.push(Line::raw("").style(Style::default().bg(theme.panel)));
    // Header: « Month YYYY »
    let header = Line::from(vec![
        Span::raw("  "),
        Span::styled("‹  ", Style::default().fg(theme.dim)),
        Span::styled(
            format!("{} {}", month_name(focused.month()), focused.year()),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ›", Style::default().fg(theme.dim)),
    ])
    .style(Style::default().bg(theme.panel));
    lines.push(header);
    // Weekday row.
    let dow_header = if app.env.week_start == crate::app::WeekStart::Sunday {
        Span::styled(
            "  S   M   T   W   T   F   S ",
            Style::default().fg(theme.dim),
        )
    } else {
        Span::styled(
            "  M   T   W   T   F   S   S ",
            Style::default().fg(theme.dim),
        )
    };
    let dow = Line::from(vec![Span::raw("  "), dow_header]).style(Style::default().bg(theme.panel));
    lines.push(dow);
    // Calendar grid.
    for row in calendar_cells(first_of_month, app.env.week_start) {
        let mut spans: Vec<Span> = vec![Span::raw("  ")];
        for cell in row {
            match cell {
                None => spans.push(Span::styled("    ", Style::default().bg(theme.panel))),
                Some(n) => {
                    let date = NaiveDate::from_ymd_opt(focused.year(), focused.month(), n);
                    let is_today = today == date;
                    let is_focus = focused.day() == n;
                    let mut style = Style::default().fg(theme.fg).bg(theme.panel);
                    if is_today {
                        style = style.fg(theme.today);
                    }
                    if is_focus {
                        style = style.bg(theme.cursor).add_modifier(Modifier::BOLD);
                    }
                    spans.push(Span::styled(format!(" {n:>2} "), style));
                }
            }
        }
        lines.push(Line::from(spans).style(Style::default().bg(theme.panel)));
    }
    // Spacer + focused date label + footer.
    lines.push(Line::raw("").style(Style::default().bg(theme.panel)));
    lines.push(
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format_focused(focused),
                Style::default().fg(theme.due).add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::default().bg(theme.panel)),
    );
    lines.push(calendar_footer(theme));

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}

#[cfg(test)]
mod tests {
    use super::{entry_scroll, wrap_chunks};

    #[test]
    fn short_text_is_one_chunk() {
        assert_eq!(wrap_chunks("short narrative", 30), vec!["short narrative"]);
    }

    #[test]
    fn wraps_at_word_boundaries() {
        assert_eq!(
            wrap_chunks("one two three four", 10),
            vec!["one two", "three four"]
        );
        assert_eq!(
            wrap_chunks("one two three four", 8),
            vec!["one two", "three", "four"]
        );
    }

    #[test]
    fn runs_of_spaces_collapse() {
        assert_eq!(wrap_chunks("one  two   three", 30), vec!["one two three"]);
    }

    /// Every emitted chunk must fit its width budget, or the paragraph's own
    /// word-wrap would re-trigger and break the hanging indent.
    #[test]
    fn chunks_never_exceed_width() {
        for s in [
            "one two three four five six seven",
            "a b c d e f g h i j k l m",
            "draft the opposition brief for the Smith versus Jones",
        ] {
            for w in 1..=15 {
                for chunk in wrap_chunks(s, w) {
                    assert!(
                        unicode_width::UnicodeWidthStr::width(chunk.as_str()) <= w,
                        "chunk {chunk:?} wider than {w} for {s:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn overlong_word_is_broken_not_overflowed() {
        let chunks = wrap_chunks("prefix abcdefghijklmnop suffix", 6);
        assert!(
            chunks
                .iter()
                .all(|c| unicode_width::UnicodeWidthStr::width(c.as_str()) <= 6),
            "chunks: {chunks:?}"
        );
        assert!(
            chunks.iter().any(|c| c.starts_with("abc")),
            "the long word must be split into visible pieces: {chunks:?}"
        );
    }

    #[test]
    fn wide_chars_count_by_display_width() {
        // 汉字 is 4 display columns: one chunk at width 5, split per glyph
        // only when the budget demands it.
        assert_eq!(wrap_chunks("汉字", 5), vec!["汉字"]);
        assert_eq!(wrap_chunks("汉字", 4), vec!["汉字"]);
        assert_eq!(wrap_chunks("汉字", 2), vec!["汉", "字"]);
    }

    // ── entry_scroll ─────────────────────────────────────────────────

    #[test]
    fn pins_the_viewport_to_the_cursor_entry_start() {
        // 4-row viewport, cursor entry on rows 10..13: its whole block fits
        // when the viewport starts at the entry itself.
        assert_eq!(entry_scroll(0, Some((10, 13)), &[0, 10], 4, 40), 10);
        assert_eq!(entry_scroll(10, Some((10, 13)), &[0, 10], 4, 40), 10);
    }

    #[test]
    fn leaves_the_offset_alone_while_the_entry_is_visible() {
        // Entry rows 11..13 sit whole inside the 4-row viewport starting at 10.
        assert_eq!(entry_scroll(10, Some((11, 13)), &[0, 10], 4, 40), 10);
    }

    #[test]
    fn end_of_list_ends_whole_entries_early_rather_than_clipping() {
        // 30 entries of 3 rows after a 1-row header: the last entry owns rows
        // 88..91. A 7-row viewport can't start on a block boundary and still
        // reach row 90, so it starts at the boundary 85 and leaves one blank
        // row instead of cutting the entry's tail.
        let boundaries: Vec<usize> = (0..30).map(|i| 1 + 3 * i).collect();
        assert_eq!(entry_scroll(0, Some((88, 91)), &boundaries, 7, 88 + 3), 85);
    }

    #[test]
    fn overlong_entry_is_pinned_to_its_first_row() {
        // Taller than the 4-row viewport: show where the entry begins rather
        // than its tail.
        assert_eq!(entry_scroll(8, Some((5, 20)), &[0, 5], 4, 40), 5);
    }

    #[test]
    fn no_cursor_entry_or_tiny_viewport_stays_inside_the_content() {
        // No cursor: clamp a stale offset so the content (e.g. the "no
        // entries" line) can't be scrolled past.
        assert_eq!(entry_scroll(3, None, &[0, 2], 4, 40), 3);
        assert_eq!(entry_scroll(80, None, &[0, 2], 4, 10), 6);
        assert_eq!(entry_scroll(0, Some((1, 2)), &[0], 0, 40), 0);
        assert_eq!(entry_scroll(0, Some((1, 2)), &[0], 4, 0), 0);
    }
}
