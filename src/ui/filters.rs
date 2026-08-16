use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, View, format_compact_duration, ordered_unique};
use crate::search::subseq_match_ci;
use crate::theme::Theme;
use crate::todo;

/// Render the left sidebar. The content is view-aware: the list/archive
/// views show the project/context/saved-filter list (task counts), while the
/// timesheet shows a per-matter billing snapshot for the current period —
/// billable hours per project plus the non-billable split, with the active
/// project filter highlighted.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    super::fill_bg(frame, area, Style::default().bg(theme.panel));

    if matches!(app.view(), View::Timesheet) {
        render_timesheet_side(frame, area, app, theme);
    } else {
        render_filter_side(frame, area, app, theme);
    }
}

/// List/archive sidebar: PROJECTS, CONTEXTS, SAVED filters with task counts.
fn render_filter_side(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let projects = ordered_unique(app.tasks(), |t| &t.projects);
    let contexts = ordered_unique(app.tasks(), |t| &t.contexts);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(line_pad(
        theme,
        vec![Span::styled(
            " FILTERS",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        )],
    ));
    lines.push(line_pad(theme, vec![Span::raw(" ")]));
    lines.push(line_pad(
        theme,
        vec![Span::styled(
            " PROJECTS",
            Style::default()
                .fg(theme.project)
                .add_modifier(Modifier::BOLD),
        )],
    ));
    if projects.is_empty() {
        lines.push(hint_row(theme, "+project", theme.project));
    } else {
        for (name, count) in &projects {
            let active = app.filter.project.as_deref() == Some(name.as_str());
            lines.push(filter_row(theme, "+", name, *count, active, theme.project));
        }
    }
    lines.push(line_pad(theme, vec![Span::raw(" ")]));
    lines.push(line_pad(
        theme,
        vec![Span::styled(
            " CONTEXTS",
            Style::default()
                .fg(theme.context)
                .add_modifier(Modifier::BOLD),
        )],
    ));
    if contexts.is_empty() {
        lines.push(hint_row(theme, "@context", theme.context));
    } else {
        for (name, count) in &contexts {
            let active = app.filter.context.as_deref() == Some(name.as_str());
            lines.push(filter_row(theme, "@", name, *count, active, theme.context));
        }
    }

    let saved = app.saved_filters();
    if !saved.is_empty() {
        lines.push(line_pad(theme, vec![Span::raw(" ")]));
        lines.push(line_pad(
            theme,
            vec![Span::styled(
                " SAVED",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )],
        ));
        for f in saved {
            let active = app.filter().search == f.query;
            let count = app
                .tasks()
                .iter()
                .filter(|t| {
                    !t.done
                        && subseq_match_ci(todo::body_after_priority(&t.raw), &f.query).is_some()
                })
                .count();
            lines.push(filter_row(theme, "", &f.name, count, active, theme.accent));
        }
    }
    let para = Paragraph::new(lines).style(Style::default().bg(theme.panel).fg(theme.fg));
    frame.render_widget(para, area);
}

/// Timesheet sidebar: per-matter billable hours for the current period.
/// Rows are projects with their billable time (the active project filter is
/// highlighted); a NON-BILLABLE section shows the DNB split per project; the
/// TOTALS block summarizes the period's billable vs non-billable hours.
fn render_timesheet_side(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let totals = app.timesheet_project_totals();
    let (total, billable, non_billable) = app.timesheet_period_totals();

    let mut lines: Vec<Line> = Vec::new();
    lines.push(line_pad(
        theme,
        vec![Span::styled(
            " FILTERS",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        )],
    ));
    lines.push(line_pad(theme, vec![Span::raw(" ")]));
    lines.push(line_pad(
        theme,
        vec![Span::styled(
            " MATTERS · BILLABLE",
            Style::default()
                .fg(theme.project)
                .add_modifier(Modifier::BOLD),
        )],
    ));
    if totals.is_empty() {
        lines.push(hint_row(theme, "no time", theme.dim));
    } else {
        for (name, bill, _dnb) in &totals {
            let active = app.filter.project.as_deref() == Some(name.as_str());
            lines.push(matter_row(theme, name.clone(), *bill, active));
        }
    }
    let has_dnb = totals.iter().any(|(_, _, dnb)| *dnb > 0);
    if has_dnb {
        lines.push(line_pad(theme, vec![Span::raw(" ")]));
        lines.push(line_pad(
            theme,
            vec![Span::styled(
                " NON-BILLABLE",
                Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
            )],
        ));
        for (name, _bill, dnb) in &totals {
            if *dnb > 0 {
                lines.push(matter_row(theme, name.clone(), *dnb, false));
            }
        }
    }
    lines.push(line_pad(theme, vec![Span::raw(" ")]));
    lines.push(line_pad(
        theme,
        vec![Span::styled(
            " TOTALS",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )],
    ));
    lines.push(total_row(theme, "billable", billable));
    lines.push(total_row(theme, "non-billable", non_billable));
    lines.push(total_row(theme, "total", total));
    let para = Paragraph::new(lines).style(Style::default().bg(theme.panel).fg(theme.fg));
    frame.render_widget(para, area);
}

/// A timesheet matter row: `+name` padded, then the compact billable
/// duration right-aligned. The active project filter gets the ▸ marker and
/// the selected background.
fn matter_row(theme: &Theme, name: String, secs: u64, active: bool) -> Line<'static> {
    let bg = if active { theme.selected } else { theme.panel };
    let prefix = if active { "▸ " } else { "  " };
    let sigil = if name == "(no project)" { "" } else { "+" };
    let mut padded = format!("{sigil}{name}");
    if padded.chars().count() < 16 {
        let pad = 16 - padded.chars().count();
        padded.push_str(&" ".repeat(pad));
    }
    let dur = format_compact_duration(secs);
    Line::from(vec![
        Span::raw(" "),
        Span::styled(prefix.to_string(), Style::default().fg(theme.accent)),
        Span::styled(padded, Style::default().fg(theme.project)),
        Span::styled(format!("{dur:>7}"), Style::default().fg(theme.fg)),
    ])
    .style(Style::default().bg(bg))
}

/// A totals row: dim label + right-aligned compact duration.
fn total_row(theme: &Theme, label: &str, secs: u64) -> Line<'static> {
    let dur = format_compact_duration(secs);
    let mut padded = label.to_string();
    if padded.chars().count() < 16 {
        let pad = 16 - padded.chars().count();
        padded.push_str(&" ".repeat(pad));
    }
    Line::from(vec![
        Span::raw(" "),
        Span::raw("  "),
        Span::styled(padded, Style::default().fg(theme.dim)),
        Span::styled(format!("{dur:>7}"), Style::default().fg(theme.fg)),
    ])
    .style(Style::default().bg(theme.panel))
}

fn filter_row<'a>(
    theme: &Theme,
    sigil: &str,
    name: &'a str,
    count: usize,
    active: bool,
    sigil_color: ratatui::style::Color,
) -> Line<'a> {
    let bg = if active { theme.selected } else { theme.panel };
    let prefix = if active { "▸ " } else { "  " };
    let mut padded = format!("{sigil}{name}");
    if padded.chars().count() < 16 {
        let pad = 16 - padded.chars().count();
        padded.push_str(&" ".repeat(pad));
    }
    Line::from(vec![
        Span::raw(" "),
        Span::styled(prefix.to_string(), Style::default().fg(theme.accent)),
        Span::styled(padded, Style::default().fg(sigil_color)),
        Span::styled(format!("{count:>3}"), Style::default().fg(theme.dim)),
    ])
    .style(Style::default().bg(bg))
}

fn hint_row<'a>(theme: &Theme, token: &'a str, token_color: ratatui::style::Color) -> Line<'a> {
    Line::from(vec![
        Span::raw("   "),
        Span::styled("tag with ", Style::default().fg(theme.dim)),
        Span::styled(token, Style::default().fg(token_color)),
    ])
    .style(Style::default().bg(theme.panel))
}

fn line_pad<'a>(theme: &Theme, spans: Vec<Span<'a>>) -> Line<'a> {
    Line::from(spans).style(Style::default().bg(theme.panel))
}
