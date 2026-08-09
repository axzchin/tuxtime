//! Nudge task picker: the centered list overlay shown when the user presses
//! `S` (start timer) or `M` (add time) from the idle-nudge popup. The whole
//! point is a *conscious* choice — a nudge must never start a timer on a
//! random task just because it happens to be under the list cursor.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::{App, NudgePickAction};
use crate::ui::{msgbox, overlay};

pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let r = overlay::palette_rect(area);
    frame.render_widget(Clear, r);

    let (title, header) = match app.session.nudge_picker.as_ref().map(|p| p.action) {
        Some(NudgePickAction::StartTimer) => (" ▶ START TIMER ", "start timer on:"),
        Some(NudgePickAction::AddTime) => (" ⏱ ADD TIME ", "add time to:"),
        None => (" PICK TASK ", "pick a task:"),
    };
    let inner = msgbox::frame_box(
        frame,
        r,
        theme.accent,
        theme.panel,
        Line::from(Span::styled(
            title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
    );

    let bg = Style::default().bg(theme.panel).fg(theme.fg);

    let [list_area, footer_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(inner);

    let Some(picker) = app.session.nudge_picker.as_ref() else {
        return;
    };
    let list = &picker.abs_list;
    let cursor = picker.cursor.min(list.len().saturating_sub(1));

    let list_h = usize::from(list_area.height);
    if list.is_empty() {
        let line = Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "no open tasks — create one first",
                Style::default().fg(theme.dim),
            ),
        ])
        .style(bg);
        frame.render_widget(Paragraph::new(line).style(bg), list_area);
    } else {
        // Window the list so the highlight stays on-screen.
        let start = if cursor < list_h { 0 } else { cursor + 1 - list_h };
        let end = (start + list_h).min(list.len());
        let row_w = usize::from(list_area.width).saturating_sub(1);
        let lines: Vec<Line> = list[start..end]
            .iter()
            .enumerate()
            .map(|(i, &abs)| {
                let is_sel = start + i == cursor;
                let t = &app.store.tasks()[abs];
                let body = crate::todo::body_only_from_clean(&t.clean_raw);
                let proj = t
                    .projects
                    .first()
                    .map(|p| format!(" +{p}"))
                    .unwrap_or_default();
                let act = t
                    .contexts
                    .first()
                    .map(|c| format!(" @{c}"))
                    .unwrap_or_default();
                // Show the timer state for running tasks so the user can see
                // what's live while choosing.
                let state = if app.is_timer_running_on(abs) {
                    "  ▶ running".to_string()
                } else if t.dur.is_some_and(|d| d > 0) {
                    format!(
                        "  ({})",
                        crate::app::format_duration(
                            t.dur.unwrap_or(0),
                            app.prefs.rounding_increment
                        )
                    )
                } else {
                    String::new()
                };
                let row: String = format!(
                    " {}{}{}{}{}",
                    if is_sel { "▶" } else { " " },
                    body,
                    proj,
                    act,
                    state
                );
                let row: String = row.chars().take(row_w).collect();
                let style = if is_sel {
                    Style::default()
                        .fg(theme.fg)
                        .bg(theme.selection)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.fg).bg(theme.panel)
                };
                Line::from(Span::styled(row, style)).style(bg)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines).style(bg), list_area);
    }

    let footer = Line::from(vec![
        Span::raw("  "),
        Span::styled(header, Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
        Span::styled("  j/k ", Style::default().fg(theme.dim)),
        Span::styled("navigate · ", Style::default().fg(theme.dim)),
        Span::styled("Enter", Style::default().fg(theme.dim).add_modifier(Modifier::BOLD)),
        Span::styled(" select · ", Style::default().fg(theme.dim)),
        Span::styled("Esc", Style::default().fg(theme.dim).add_modifier(Modifier::BOLD)),
        Span::styled(" back", Style::default().fg(theme.dim)),
    ])
    .style(bg);
    frame.render_widget(Paragraph::new(footer).style(bg), footer_area);
}
