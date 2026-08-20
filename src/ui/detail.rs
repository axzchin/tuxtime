use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, BadgeTheme, View, format_billable};
use crate::theme::Theme;
use crate::todo::Task;
use crate::ui::msgbox::wrap_words;
use crate::ui::task_row::{
    bill_badge_style, due_label, due_token_style, is_url_token, url_token_style,
};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    super::fill_bg(frame, area, Style::default().bg(theme.panel));

    // Wrap to the actual pane width minus 1-char left padding and 1-char
    // safety margin on the right. Floor at 16 so a tiny pane still wraps.
    let wrap_w = (area.width as usize).saturating_sub(2).max(16);
    let style = Style::default().bg(theme.panel).fg(theme.fg);
    if matches!(app.view(), View::Timesheet) {
        // The period totals (billable / non-billable / total) are pinned at
        // the top so the billable figure can never be clipped by a short
        // terminal; the entry + narrative below scrolls independently.
        let (totals, body) = build_timesheet_content(theme, app, wrap_w);
        let totals_h = totals.len() as u16;
        let [totals_area, body_area] =
            Layout::vertical([Constraint::Length(totals_h), Constraint::Min(0)]).areas(area);
        frame.render_widget(Paragraph::new(totals).style(style), totals_area);
        // Scroll the narrative. The offset is keyed to the timesheet cursor so
        // moving to a different entry resets to the top; it's clamped here (and
        // written back) so Ctrl-d/Ctrl-u in the handler read a sane value.
        let (for_cursor, raw_scroll) = app.nav.detail_scroll.get();
        let scroll = if for_cursor == app.timesheet.cursor {
            raw_scroll
        } else {
            0
        };
        let max_scroll = body.len().saturating_sub(body_area.height as usize) as u16;
        let scroll = scroll.min(max_scroll);
        app.nav.detail_scroll.set((app.timesheet.cursor, scroll));
        frame.render_widget(
            Paragraph::new(body).style(style).scroll((scroll, 0)),
            body_area,
        );
    } else {
        let lines = build_lines(theme, app.cur_task(), app.today(), wrap_w);
        frame.render_widget(Paragraph::new(lines).style(style), area);
    }
}

/// Split the timesheet sidebar into its pinned totals block and the
/// (clippable) entry + narrative body, mirroring how the centre list pins its
/// grand-total footer. Keeping the two apart means the billable figure is
/// structurally guaranteed to render first regardless of how short the pane
/// is or how long the narrative wraps.
fn build_timesheet_content<'a>(
    theme: &Theme,
    app: &'a App,
    wrap_w: usize,
) -> (Vec<Line<'a>>, Vec<Line<'a>>) {
    (
        build_timesheet_totals(theme, app),
        build_timesheet_body(theme, app, wrap_w),
    )
}

/// Pinned header: the period's billable / non-billable / total figures.
fn build_timesheet_totals<'a>(theme: &Theme, app: &'a App) -> Vec<Line<'a>> {
    let increment = app.prefs.rounding_increment;
    let (total, billable, non_billable) = app.timesheet_period_totals();
    vec![
        line_panel(
            theme,
            vec![Span::styled(
                " DETAIL · TIMESHEET",
                Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
            )],
        ),
        line_panel(theme, vec![Span::raw(" ")]),
        line_panel(
            theme,
            vec![Span::styled(" PERIOD", Style::default().fg(theme.accent))],
        ),
        label_value_row(theme, "billable", format_billable(billable, increment)),
        label_value_row(
            theme,
            "non-billable",
            format_billable(non_billable, increment),
        ),
        label_value_row(theme, "total", format_billable(total, increment)),
        line_panel(theme, vec![Span::raw(" ")]),
    ]
}

/// Body: the entry under the timesheet cursor — its date, project+activity
/// key, duration and billable status — followed by the wrapped narrative.
fn build_timesheet_body<'a>(theme: &Theme, app: &'a App, wrap_w: usize) -> Vec<Line<'a>> {
    // All rendered strings are owned so `rows` never references the local
    // `groups` (which is built from `app` and dropped at the end).
    let Some((gi, ni, _)) = app.timesheet_narrative_at(app.timesheet.cursor) else {
        return vec![line_panel(
            theme,
            vec![Span::styled(
                " (no entries)",
                Style::default().fg(theme.dim),
            )],
        )];
    };
    let groups = app.build_timesheet_groups();
    let Some(entry) = groups.get(gi) else {
        return vec![];
    };
    let (date, key, total_secs, billable, narrative, entry_narrative_count) = (
        entry.date.clone(),
        entry.key.clone(),
        entry.total_secs,
        entry.billable,
        entry.narratives.get(ni).cloned(),
        entry.narratives.len(),
    );
    let mut rows: Vec<Line<'a>> = vec![
        line_panel(
            theme,
            vec![Span::styled(" ENTRY", Style::default().fg(theme.accent))],
        ),
        line_panel(
            theme,
            vec![Span::styled(date, Style::default().fg(theme.fg))],
        ),
        line_panel(
            theme,
            vec![Span::styled(key, Style::default().fg(theme.project))],
        ),
    ];
    let increment = app.prefs.rounding_increment;
    let dur = format_billable(total_secs, increment);
    // The duration is the *group* total (all narratives sharing this
    // project+activity+day), so label it as such rather than implying it
    // belongs to the single narrative under the cursor. Keep it as a chip
    // so the detail pane uses the same visual language as task rows.
    rows.push(label_value_duration_row(
        theme,
        "group dur",
        dur,
        if billable { "billable" } else { "" },
        (!billable).then_some("DNB"),
        app.prefs.badge_theme,
    ));
    if let Some(narrative) = narrative {
        rows.push(line_panel(theme, vec![Span::raw(" ")]));
        let total_in_group = entry_narrative_count;
        rows.push(line_panel(
            theme,
            vec![Span::styled(
                format!(" NARRATIVE {}/{} ", ni + 1, total_in_group),
                Style::default().fg(theme.accent),
            )],
        ));
        for chunk in wrap_words(&narrative, wrap_w.saturating_sub(2)) {
            rows.push(line_panel(
                theme,
                vec![Span::styled(
                    format!("  {}", chunk.join(" ")),
                    Style::default().fg(theme.fg),
                )],
            ));
        }
    }
    rows
}

fn label_value_row<'a>(theme: &Theme, label: &'a str, value: String) -> Line<'a> {
    let mut padded = format!(" {label}");
    if padded.chars().count() < 14 {
        padded.push_str(&" ".repeat(14 - padded.chars().count()));
    }
    line_panel(
        theme,
        vec![
            Span::styled(padded, Style::default().fg(theme.dim)),
            Span::styled(value, Style::default().fg(theme.fg)),
        ],
    )
}

fn label_value_duration_row<'a>(
    theme: &Theme,
    label: &'a str,
    duration: String,
    suffix: &'a str,
    badge: Option<&'static str>,
    badge_theme: BadgeTheme,
) -> Line<'a> {
    let mut padded = format!(" {label}");
    if padded.chars().count() < 14 {
        padded.push_str(&" ".repeat(14 - padded.chars().count()));
    }
    let mut spans = vec![
        Span::styled(padded, Style::default().fg(theme.dim)),
        Span::styled(
            format!(" {duration} "),
            crate::ui::task_row::dur_badge_style(false, theme, badge_theme),
        ),
    ];
    if !suffix.is_empty() {
        spans.push(Span::styled(
            format!(" · {suffix}"),
            Style::default().fg(theme.fg),
        ));
    }
    if let Some(badge) = badge {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            badge,
            bill_badge_style(false, theme, badge_theme),
        ));
    }
    line_panel(theme, spans)
}

fn build_lines<'a>(
    theme: &Theme,
    task: Option<&'a Task>,
    today: &'a str,
    wrap_w: usize,
) -> Vec<Line<'a>> {
    let mut rows: Vec<Line> = Vec::new();
    rows.push(line_panel(
        theme,
        vec![Span::styled(
            " DETAIL",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        )],
    ));
    rows.push(line_panel(theme, vec![Span::raw(" ")]));
    let Some(t) = task else {
        rows.push(line_panel(
            theme,
            vec![Span::styled(" (no task)", Style::default().fg(theme.dim))],
        ));
        return rows;
    };

    let priority_value = if let Some(p) = t.priority {
        Span::styled(
            format!("({p})"),
            Style::default()
                .fg(theme.priority_color(p))
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" priority  ", Style::default().fg(theme.dim)),
            priority_value,
        ],
    ));
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" created   ", Style::default().fg(theme.dim)),
            Span::styled(
                t.created_date.as_deref().unwrap_or("—"),
                Style::default().fg(theme.fg),
            ),
        ],
    ));
    if let Some(due) = &t.due {
        rows.push(line_panel(
            theme,
            vec![
                Span::styled(" due       ", Style::default().fg(theme.dim)),
                Span::styled(due.as_str(), Style::default().fg(theme.fg)),
                Span::raw("  "),
                Span::styled(due_label(due, today), Style::default().fg(theme.overdue)),
            ],
        ));
    }
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" projects  ", Style::default().fg(theme.dim)),
            Span::styled(
                t.projects
                    .iter()
                    .map(|p| format!("+{p}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                Style::default().fg(theme.project),
            ),
        ],
    ));
    rows.push(line_panel(
        theme,
        vec![
            Span::styled(" contexts  ", Style::default().fg(theme.dim)),
            Span::styled(
                t.contexts
                    .iter()
                    .map(|c| format!("@{c}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                Style::default().fg(theme.context),
            ),
        ],
    ));

    // Rendering notes line by line
    if !t.notes.is_empty() {
        rows.push(line_panel(
            theme,
            vec![Span::styled(" notes", Style::default().fg(theme.dim))],
        ));
        for note in &t.notes {
            let chunks = wrap_words(note, wrap_w.saturating_sub(4));
            for (i, chunk) in chunks.into_iter().enumerate() {
                let prefix = if i == 0 { "   - " } else { "     " };
                rows.push(line_panel(
                    theme,
                    vec![Span::styled(
                        format!("{prefix}{}", chunk.join(" ")),
                        Style::default().fg(theme.fg),
                    )],
                ));
            }
        }
    }

    if t.done {
        rows.push(line_panel(
            theme,
            vec![
                Span::styled(" done      ", Style::default().fg(theme.dim)),
                Span::styled(
                    t.done_date.as_deref().unwrap_or(""),
                    Style::default().fg(theme.done),
                ),
            ],
        ));
    }
    rows.push(line_panel(theme, vec![Span::raw(" ")]));
    rows.push(line_panel(
        theme,
        vec![Span::styled(
            " RAW",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        )],
    ));
    rows.push(line_panel(theme, vec![Span::raw(" ")]));
    let mut state = RawWalk::default();
    for chunk in wrap_words(&t.raw, wrap_w) {
        let mut spans: Vec<Span> = vec![Span::raw(" ")];
        let mut words = chunk.into_iter();
        if let Some(first) = words.next() {
            spans.push(style_raw_token(first, t, today, theme, &mut state));
        }
        for w in words {
            spans.push(Span::raw(" "));
            spans.push(style_raw_token(w, t, today, theme, &mut state));
        }
        rows.push(line_panel(theme, spans));
    }
    rows
}

#[derive(Default)]
struct RawWalk {
    done_marker_consumed: bool,
    priority_consumed: bool,
}

fn style_raw_token<'a>(
    token: &'a str,
    task: &Task,
    today: &str,
    theme: &Theme,
    state: &mut RawWalk,
) -> Span<'a> {
    if task.done && !state.done_marker_consumed {
        state.done_marker_consumed = true;
        if token == "x" {
            return Span::styled(token, Style::default().fg(theme.done));
        }
    }
    if !state.priority_consumed
        && let Some(p) = task.priority
        && token.len() == 3
        && token.as_bytes()[0] == b'('
        && token.as_bytes()[1] == p as u8
        && token.as_bytes()[2] == b')'
    {
        state.priority_consumed = true;
        return Span::styled(
            token,
            Style::default()
                .fg(theme.priority_color(p))
                .add_modifier(Modifier::BOLD),
        );
    }
    if let Some(rest) = token.strip_prefix("due:") {
        return Span::styled(token, due_token_style(task.done, rest, today, theme));
    }
    if is_url_token(token) {
        return Span::styled(token, url_token_style(task.done, theme));
    }
    if token.len() > 1 && token.starts_with('+') {
        return Span::styled(token, Style::default().fg(theme.project));
    }
    if token.len() > 1 && token.starts_with('@') {
        return Span::styled(token, Style::default().fg(theme.context));
    }
    Span::styled(token, Style::default().fg(theme.fg))
}

fn line_panel<'a>(theme: &Theme, spans: Vec<Span<'a>>) -> Line<'a> {
    Line::from(spans).style(Style::default().bg(theme.panel))
}
