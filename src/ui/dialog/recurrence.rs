use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::{App, BuilderField, REC_UNIT_ORDER};
use crate::theme::Theme;
use crate::ui::calendar_utils::format_focused;
use crate::ui::{msgbox, overlay};

pub(super) fn render_recurrence_builder(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) {
    let theme = app.theme();
    let Some(state) = app.recurrence_state() else {
        return;
    };
    let area = overlay::recurrence_popup_rect(dlg, screen);
    frame.render_widget(Clear, area);
    let inner = msgbox::frame_box(
        frame,
        area,
        theme.border,
        theme.panel,
        Line::from(Span::styled(
            " ↻ REPEAT ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
    );

    let pill = |label: &str, focused: bool, theme: &Theme| -> Span<'static> {
        let bg = if focused { theme.cursor } else { theme.panel };
        let fg = theme.fg;
        let m = if focused {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        Span::styled(
            format!(" {label} "),
            Style::default().fg(fg).bg(bg).add_modifier(m),
        )
    };

    let interval_focus = state.field == BuilderField::Interval;
    let unit_focus = state.field == BuilderField::Unit;
    let mode_focus = state.field == BuilderField::Mode;

    // every {N} day/business/week/month/year — single source of truth in
    // REC_UNIT_ORDER so the cycle and render can't drift apart.
    let mut every_spans: Vec<Span> = vec![
        Span::raw("  "),
        Span::styled("every ", Style::default().fg(theme.dim)),
        pill(&state.interval.to_string(), interval_focus, theme),
        Span::raw("  "),
    ];
    for unit in REC_UNIT_ORDER.iter().copied() {
        let sel = state.unit == unit;
        let label = match unit {
            crate::recurrence::RecUnit::Day => "day",
            crate::recurrence::RecUnit::Week => "week",
            crate::recurrence::RecUnit::Month => "month",
            crate::recurrence::RecUnit::Year => "year",
            crate::recurrence::RecUnit::BusinessDay => "business",
        };
        let focused = unit_focus && sel;
        let style = if sel {
            Style::default()
                .fg(theme.fg)
                .bg(if focused { theme.accent } else { theme.cursor })
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.dim).bg(theme.panel)
        };
        every_spans.push(Span::styled(format!(" {label} "), style));
    }
    let line1 = Line::from(every_spans).style(Style::default().bg(theme.panel));

    // mode  strict / after-complete    next: ...
    let strict_label = " strict ";
    let after_label = " after-complete ";
    let mode_bg_strict = if state.strict {
        theme.cursor
    } else {
        theme.panel
    };
    let mode_bg_after = if state.strict {
        theme.panel
    } else {
        theme.cursor
    };
    let mode_emph_strict = if mode_focus && state.strict {
        theme.accent
    } else {
        mode_bg_strict
    };
    let mode_emph_after = if mode_focus && !state.strict {
        theme.accent
    } else {
        mode_bg_after
    };
    let mut line2_spans: Vec<Span> = vec![
        Span::raw("  "),
        Span::styled("mode  ", Style::default().fg(theme.dim)),
        Span::styled(
            strict_label,
            Style::default()
                .fg(theme.fg)
                .bg(mode_emph_strict)
                .add_modifier(if state.strict {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::raw(" "),
        Span::styled(
            after_label,
            Style::default()
                .fg(theme.fg)
                .bg(mode_emph_after)
                .add_modifier(if state.strict {
                    Modifier::empty()
                } else {
                    Modifier::BOLD
                }),
        ),
    ];
    let next = crate::app::recurrence_next_preview(state, app.today())
        .map_or_else(|| "—".into(), format_focused);
    let next_label = format!("next: {next}");
    // Measure the already-built left side instead of hardcoding a width that
    // silently drifts when the mode-line copy changes. The `+ 2` keeps a
    // 2-cell margin from the right border when there's room.
    let left_width: usize = line2_spans.iter().map(|s| s.content.chars().count()).sum();
    let next_pad =
        (inner.width as usize).saturating_sub(left_width + next_label.chars().count() + 2);
    line2_spans.push(Span::styled(
        " ".repeat(next_pad),
        Style::default().bg(theme.panel),
    ));
    line2_spans.push(Span::styled(next_label, Style::default().fg(theme.dim)));
    let line2 = Line::from(line2_spans).style(Style::default().bg(theme.panel));

    let value = crate::app::format_rec_value(state);
    let line_preview = Line::from(vec![
        Span::raw("  "),
        Span::styled("→ writes ", Style::default().fg(theme.dim)),
        Span::styled(
            format!("rec:{value}"),
            Style::default().fg(theme.due).add_modifier(Modifier::BOLD),
        ),
    ])
    .style(Style::default().bg(theme.panel));

    let lines = vec![
        Line::raw("").style(Style::default().bg(theme.panel)),
        line1,
        Line::raw("").style(Style::default().bg(theme.panel)),
        line2,
        Line::raw("").style(Style::default().bg(theme.panel)),
        line_preview,
        Line::raw("").style(Style::default().bg(theme.panel)),
        rec_footer(theme),
    ];

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}

fn rec_footer<'a>(theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "hjkl",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" move · ", Style::default().fg(theme.dim)),
        Span::styled(
            "+/-",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" adjust · ", Style::default().fg(theme.dim)),
        Span::styled(
            "Enter",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" save · ", Style::default().fg(theme.dim)),
        Span::styled(
            "Esc",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" cancel", Style::default().fg(theme.dim)),
    ])
    .style(Style::default().bg(theme.panel))
}
