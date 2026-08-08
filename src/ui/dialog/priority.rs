use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::App;
use crate::ui::{msgbox, overlay};

pub(super) fn render_priority_chooser(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) {
    let theme = app.theme();
    let Some(state) = app.priority_state() else {
        return;
    };
    let area = overlay::priority_popup_rect(dlg, screen);
    frame.render_widget(Clear, area);
    let inner = msgbox::frame_box(
        frame,
        area,
        theme.border,
        theme.panel,
        Line::from(Span::styled(
            " PRIORITY ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
    );

    let rows: [(u8, &str, ratatui::style::Color); 4] = [
        (0, "(A)", theme.pri_a),
        (1, "(B)", theme.pri_b),
        (2, "(C)", theme.pri_c),
        (3, "clear", theme.dim),
    ];
    let mut lines: Vec<Line> = Vec::new();
    for (i, label, color) in rows {
        let is_sel = state.selected == i;
        let bg = if is_sel { theme.cursor } else { theme.panel };
        let m = if is_sel {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };
        lines.push(
            Line::from(vec![
                Span::styled("  ", Style::default().bg(bg)),
                Span::styled(
                    label.to_string(),
                    Style::default().fg(color).bg(bg).add_modifier(m),
                ),
                Span::styled(
                    " ".repeat((inner.width as usize).saturating_sub(2 + label.chars().count())),
                    Style::default().bg(bg),
                ),
            ])
            .style(Style::default().bg(bg)),
        );
    }
    lines.push(Line::raw("").style(Style::default().bg(theme.panel)));
    lines.push(
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "jk",
                Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" move · ", Style::default().fg(theme.dim)),
            Span::styled(
                "Enter",
                Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" set", Style::default().fg(theme.dim)),
        ])
        .style(Style::default().bg(theme.panel)),
    );

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}
