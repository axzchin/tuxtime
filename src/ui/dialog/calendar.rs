use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::{App, CalendarTarget, WeekStart};
use crate::ui::calendar_utils::{calendar_cells, calendar_footer, format_focused, month_name};
use crate::ui::msgbox;
use crate::ui::overlay::{INPUT_PREFIX_OFFSET, anchored_below};

pub(super) fn render_calendar(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) {
    let theme = app.theme();
    let Some(state) = app.calendar_state() else {
        return;
    };
    let popup_w: u16 = 50u16.min(screen.width.max(40));
    let popup_h: u16 = 13;
    let area = anchored_below(dlg, screen, popup_w, popup_h, INPUT_PREFIX_OFFSET);
    frame.render_widget(Clear, area);
    let label = match state.target {
        CalendarTarget::Due => "DUE",
        CalendarTarget::Threshold => "THRESHOLD",
    };
    let inner = msgbox::frame_box(
        frame,
        area,
        theme.border,
        theme.panel,
        Line::from(Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
    );

    use chrono::{Datelike, NaiveDate};
    let focused = state.focused;
    let today = NaiveDate::parse_from_str(app.today(), "%Y-%m-%d").ok();
    let first_of_month =
        NaiveDate::from_ymd_opt(focused.year(), focused.month(), 1).unwrap_or(focused);

    let mut lines: Vec<Line> = Vec::new();
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
    // Weekday row. Each label occupies a 4-char column ("  X ") so it lines up
    // with the day numbers below, which render as ` {:>2} `.
    let dow_header = if app.env.week_start == WeekStart::Sunday {
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
    // Up to 6 week rows; `calendar_cells` stops before any all-blank trailing row.
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
                    spans.push(Span::styled(format!(" {:>2} ", n), style));
                }
            }
        }
        lines.push(Line::from(spans).style(Style::default().bg(theme.panel)));
    }
    // Spacer + focused-date label.
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
