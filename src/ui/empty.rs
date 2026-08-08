use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::ui::msgbox::pad_right;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let r = super::overlay::empty_state_rect(area);

    let inner = crate::ui::msgbox::frame_box(
        frame,
        r,
        theme.border,
        theme.bg,
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "tuxtime",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]),
    );

    let shortcuts: &[(&str, &str)] = &[
        ("n", "add a task"),
        ("?", "show all keybindings"),
        (",", "settings"),
        ("q", "quit"),
    ];

    let mut lines: Vec<Line> = Vec::new();
    if inner.width >= super::logo::WIDTH {
        lines.extend(super::logo::centered_lines(theme, inner.width));
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(Span::styled(
        "  no tasks yet — let's get started".to_string(),
        Style::default().fg(theme.fg),
    )));
    lines.push(Line::raw(""));
    for (key, label) in shortcuts {
        lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled(
                pad_right(key, 4),
                Style::default()
                    .fg(theme.context)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(label.to_string(), Style::default().fg(theme.fg)),
        ]));
    }
    lines.push(Line::raw(""));
    let mut hint_spans = vec![
        Span::raw("   "),
        Span::styled("format: ".to_string(), Style::default().fg(theme.dim)),
    ];
    hint_spans.extend(super::dialog::format_hint_spans(theme));
    lines.push(Line::from(hint_spans));

    let para = Paragraph::new(lines).style(Style::default().bg(theme.bg).fg(theme.fg));
    frame.render_widget(para, inner);
}
