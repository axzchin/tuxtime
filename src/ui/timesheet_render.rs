//! Timesheet view renderer and date-picker overlay. Extracted from [`super::mod`].

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use chrono::Timelike;

use crate::app::App;
use crate::ui::calendar_utils::{calendar_cells, calendar_footer, format_focused, month_name};
use crate::ui::msgbox;
use crate::ui::overlay::timesheet_calendar_rect;

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
    let title = Line::from(vec![Span::raw(" Timesheet "), view_label, sort_label]);
    let inner = msgbox::frame_box(frame, area, theme.border, theme.panel, title);

    let groups = app.build_timesheet_groups();
    // Billable rounding increment (decimal hours) from prefs — 0.1 default,
    // 0.25 for fifteen-minute billing, 0 for exact.
    let increment = app.prefs.rounding_increment;

    let mut lines: Vec<Line> = Vec::new();
    // The grand-total block is kept out of `lines` so it can be pinned below
    // the scrollable entry list instead of scrolling off short terminals.
    let mut footer: Vec<Line> = Vec::new();
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
                            crate::app::format_duration(day_total, increment),
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
                lines.push(Line::from(Span::styled(
                    format!("  {date_dow}"),
                    Style::default()
                        .fg(theme.today)
                        .bg(theme.panel)
                        .add_modifier(Modifier::BOLD),
                )));
                last_date = Some(&entry.date);
            }

            let formatted = crate::app::format_duration(entry.total_secs, increment);
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
            let dnb_suffix = if entry.billable { "" } else { " (DNB)" };
            lines.push(Line::from(Span::styled(
                format!(
                    "  {}  —  {} ({billable_str}){dnb_suffix}",
                    entry.key, formatted
                ),
                group_style,
            )));

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
                    crate::app::format_duration(day_total, increment),
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
                "  Billable: {} ({billable_str})",
                crate::app::format_duration(totals.billable_secs, increment),
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
                    "  DNB:      {} ({dnb_str})",
                    crate::app::format_duration(totals.non_billable_secs, increment)
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
                "  Total:    {} ({total_str}){search_note}",
                crate::app::format_duration(totals.total_secs, increment)
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
    let [_pad_top, body_rect] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    let footer_h = footer.len() as u16;
    let [list_rect, footer_rect] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(footer_h)]).areas(body_rect);
    let max_lines = list_rect.height as usize;
    let visible: Vec<Line> = lines.into_iter().take(max_lines).collect();
    frame.render_widget(Paragraph::new(visible), list_rect);
    if !footer.is_empty() {
        frame.render_widget(Paragraph::new(footer), footer_rect);
    }
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
