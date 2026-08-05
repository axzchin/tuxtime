//! Timesheet view renderer and date-picker overlay. Extracted from [`super::mod`].

use chrono::{Datelike, NaiveDate};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{App, TimesheetSort, TimesheetTaskRef, FLASH_TTL, WeekStart};
use crate::app::{format_billable, format_billable_tenths, format_duration};
use crate::ui::dialog::{calendar_cells, calendar_footer, format_focused, month_name};
use crate::ui::centered_in;

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
        Style::default()
            .fg(theme.dim)
            .bg(theme.panel),
    );
    let title = Line::from(vec![
        Span::raw(" Timesheet "),
        view_label,
        sort_label,
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(title)
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let groups = app.build_timesheet_groups();

    let mut lines: Vec<Line> = Vec::new();
    if groups.is_empty() {
        let msg = if app.filter().search.is_empty() {
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
        let mut grand_total: u64 = 0;
        let mut billable_total: u64 = 0;
        let mut dnb_total: u64 = 0;
        let mut day_total: u64 = 0;
        // Billable tenths (per-group rounding, summed): 1 min × 5 matters = 0.5h.
        let mut grand_billable_tenths: u64 = 0;
        let mut billable_billable_tenths: u64 = 0;
        let mut dnb_billable_tenths: u64 = 0;
        let mut day_billable_tenths: u64 = 0;
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
                        format!("    ──  {} ({})", crate::app::format_duration(day_total), crate::app::format_billable_tenths(day_billable_tenths)),
                        Style::default()
                            .fg(theme.dim)
                            .bg(theme.panel),
                    )));
                    lines.push(Line::raw(""));
                    day_total = 0;
                    day_billable_tenths = 0;
                }
                let date_dow = chrono::NaiveDate::parse_from_str(
                    &entry.date,
                    "%Y-%m-%d",
                ).map_or_else(|_| entry.date.clone(), |d| d.format("%a %Y-%m-%d").to_string());
                lines.push(Line::from(Span::styled(
                    format!("  {date_dow}"),
                    Style::default()
                        .fg(theme.today)
                        .bg(theme.panel)
                        .add_modifier(Modifier::BOLD),
                )));
                last_date = Some(&entry.date);
            }

            let formatted = crate::app::format_duration(entry.total_secs);
            let billable_str = crate::app::format_billable(entry.total_secs);
            // Copy-flash: briefly highlight the entire group white after copy.
            let copy_flash_active = app
                .timesheet.copy_flash
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
            let dnb_suffix = if entry.billable { "" } else { " (DNB)" };
            lines.push(Line::from(Span::styled(
                format!("  {}  —  {} ({billable_str}){dnb_suffix}", entry.key, formatted),
                group_style,
            )));

            for (ni, n) in entry.narratives.iter().enumerate() {
                let is_narr_cursor = is_selected && selected_narrative == Some(ni);
                let (is_done, is_archived) = entry
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
                        Style::default()
                            .fg(theme.fg)
                            .bg(theme.selection)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.dim).bg(theme.selection)
                    }
                } else if is_archived || is_done || !entry.billable {
                    Style::default().fg(theme.dim).bg(theme.panel)
                } else {
                    Style::default().fg(theme.fg).bg(theme.panel)
                };
                lines.push(Line::from(Span::styled(
                    format!("    • {n}{status_suffix}"),
                    narr_style,
                )));
            }
            let tenths = entry.total_secs.div_ceil(360);
            grand_total += entry.total_secs;
            grand_billable_tenths += tenths;
            day_total += entry.total_secs;
            day_billable_tenths += tenths;
            if entry.billable {
                billable_total += entry.total_secs;
                billable_billable_tenths += tenths;
            } else {
                dnb_total += entry.total_secs;
                dnb_billable_tenths += tenths;
            }
        }
        // Flush the final day's subtotal (if date headers were shown).
        if (app.timesheet.weekly
            || matches!(app.timesheet.sort, crate::app::TimesheetSort::Date))
            && last_date.is_some()
        {
            lines.push(Line::from(Span::styled(
                format!("    ──  {} ({})", crate::app::format_duration(day_total), crate::app::format_billable_tenths(day_billable_tenths)),
                Style::default()
                    .fg(theme.dim)
                    .bg(theme.panel),
            )));
            lines.push(Line::raw(""));
        }
        // Grand total with billable / DNB split.
        lines.push(Line::raw(""));
        let search_note = if app.filter().search.is_empty() {
            ""
        } else {
            " (filtered)"
        };
        let billable_str = crate::app::format_billable_tenths(billable_billable_tenths);
        lines.push(Line::from(Span::styled(
            format!(
                "  Billable: {} ({billable_str})",
                crate::app::format_duration(billable_total),
            ),
            Style::default()
                .fg(theme.accent)
                .bg(theme.panel)
                .add_modifier(Modifier::BOLD),
        )));
        if dnb_total > 0 {
            let dnb_str = crate::app::format_billable_tenths(dnb_billable_tenths);
            lines.push(Line::from(Span::styled(
                format!("  DNB:      {} ({dnb_str})", crate::app::format_duration(dnb_total)),
                Style::default()
                    .fg(theme.dim)
                    .bg(theme.panel)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        let total_str = crate::app::format_billable_tenths(grand_billable_tenths);
        lines.push(Line::from(Span::styled(
            format!("  Total:    {} ({total_str}){search_note}", crate::app::format_duration(grand_total)),
            Style::default()
                .fg(theme.accent)
                .bg(theme.panel)
                .add_modifier(Modifier::BOLD),
        )));
    }

    // Scroll if content exceeds space
    let [_pad_top, body_rect] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)])
            .areas(inner);
    let max_lines = body_rect.height as usize;
    let visible: Vec<Line> = lines.into_iter().take(max_lines).collect();
    frame.render_widget(Paragraph::new(visible), body_rect);
}

/// Render a centered calendar overlay for the timesheet date picker.
pub(crate) fn render_timesheet_calendar(frame: &mut Frame, area: Rect, app: &App) {
    use chrono::{Datelike, NaiveDate};
    let theme = app.theme();
    let focused = app.timesheet.calendar_focus;
    let today = NaiveDate::parse_from_str(app.today(), "%Y-%m-%d").ok();
    let first_of_month =
        NaiveDate::from_ymd_opt(focused.year(), focused.month(), 1).unwrap_or(focused);

    let popup_w: u16 = 50u16.min(area.width.max(40));
    let popup_h: u16 = 14;
    let r = centered_in(area, popup_w, popup_h);
    frame.render_widget(Clear, r);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(Line::from(Span::styled(
            " JUMP TO DATE ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(r);
    frame.render_widget(block, r);

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
                Style::default()
                    .fg(theme.fg)
                    .add_modifier(Modifier::BOLD),
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
    let dow_header = if app.week_start == crate::app::WeekStart::Sunday {
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
    for row in calendar_cells(first_of_month, app.week_start) {
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

