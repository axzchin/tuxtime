use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::{App, DURATION_PRESETS};
use crate::ui::{msgbox, overlay};

pub(super) fn render_duration_picker(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) {
    let theme = app.theme();
    let Some(state) = app.duration_state() else {
        return;
    };
    let presets = DURATION_PRESETS;
    let area = overlay::duration_popup_rect(dlg, screen, presets.len());
    frame.render_widget(Clear, area);
    let inner = msgbox::frame_box(
        frame,
        area,
        theme.border,
        theme.panel,
        Line::from(Span::styled(
            " DURATION ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
    );

    let mut lines: Vec<Line> = Vec::new();
    for (i, &(label, desc, _secs)) in presets.iter().enumerate() {
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
                    Style::default().fg(theme.accent).bg(bg).add_modifier(m),
                ),
                Span::styled(format!("  ({desc})"), Style::default().fg(theme.dim).bg(bg)),
                Span::styled(
                    " ".repeat(
                        (inner.width as usize).saturating_sub(4 + label.len() + desc.len() + 4),
                    ),
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
            Span::styled(" insert · ", Style::default().fg(theme.dim)),
            Span::styled(
                "Esc",
                Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" type manually", Style::default().fg(theme.dim)),
        ])
        .style(Style::default().bg(theme.panel)),
    );

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}
