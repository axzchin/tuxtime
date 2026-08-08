//! First-run welcome overlay: shown when `tuxtime` is launched with no target
//! and no `./todo.txt` exists. Offers to create a `./todo.txt` here or open
//! the bundled sample. Key handling lives in `handle_welcome` (main.rs);
//! `q`/`Esc` quits without creating anything. Sizing lives in `overlay`;
//! `render` fills whatever rect it is given.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::ui::msgbox::{frame_box, pad_right};

const CHOICES: &[(&str, &str)] = &[
    ("c", "create ./todo.txt here"),
    ("s", "open the sample"),
    ("q", "quit"),
];

/// Render the welcome box, filling `area`. The caller is responsible for
/// centering and clearing.
pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();

    let inner = frame_box(
        frame,
        area,
        theme.border,
        theme.panel,
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "tuxtime",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · welcome ", Style::default().fg(theme.dim)),
        ]),
    );

    let mut lines: Vec<Line> = Vec::new();
    if inner.width >= super::logo::WIDTH {
        lines.extend(super::logo::centered_lines(theme, inner.width));
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(Span::styled(
        "  no todo.txt in this folder yet".to_string(),
        Style::default().fg(theme.fg),
    )));
    lines.push(Line::raw(""));
    for (k, label) in CHOICES {
        lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled(
                pad_right(k, 4),
                Style::default()
                    .fg(theme.context)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(label.to_string(), Style::default().fg(theme.fg)),
        ]));
    }

    let para = Paragraph::new(lines).style(Style::default().bg(theme.panel).fg(theme.fg));
    frame.render_widget(para, inner);
}
