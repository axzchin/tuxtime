use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::App;
use crate::theme::Theme;
use crate::ui::msgbox::{frame_box, pad_right};
use crate::ui::overlay::{INPUT_PREFIX_OFFSET, anchored_below};

pub(super) fn render_slash_menu(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) {
    let theme = app.theme();
    let matches = app.slash_matches();
    if matches.is_empty() {
        return;
    }
    let selected = app.slash_selected();

    // Width: longest label + spacer + longest description + spacer + cmd.
    let label_w = matches
        .iter()
        .map(|e| e.label.chars().count())
        .max()
        .unwrap_or(0);
    let desc_w = matches
        .iter()
        .map(|e| e.description.chars().count())
        .max()
        .unwrap_or(0);
    let cmd_w = matches
        .iter()
        .map(|e| e.cmd.chars().count())
        .max()
        .unwrap_or(0);
    let content_w = label_w + 4 + desc_w + 4 + cmd_w + 4; // padding/spacers
    // Wider than the dialog on purpose so the footer hint fits — anchor
    // placement clamps to the screen edge below.
    let popup_w: u16 = (content_w as u16).max(60).min(screen.width.max(40));
    // Title row + entries + spacer + footer + 2 borders.
    let popup_h: u16 = matches.len() as u16 + 5;

    let area = anchored_below(dlg, screen, popup_w, popup_h, INPUT_PREFIX_OFFSET);
    frame.render_widget(Clear, area);
    // The slash menu has no title line; pass an empty one so the shared
    // `frame_box` still draws the same bordered panel.
    let inner = frame_box(frame, area, theme.border, theme.panel, Line::raw(""));

    let mut lines: Vec<Line> = Vec::new();
    lines.push(
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "ATTACH METADATA",
                Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::default().bg(theme.panel)),
    );
    for (i, entry) in matches.iter().enumerate() {
        let is_sel = i == selected;
        let bg = if is_sel { theme.cursor } else { theme.panel };
        let label_style = if is_sel {
            Style::default()
                .fg(theme.fg)
                .bg(bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.fg).bg(bg)
        };
        let desc_style = Style::default().fg(theme.dim).bg(bg);
        let cmd_style = Style::default().fg(theme.dim).bg(bg);

        // Right-align the /cmd by padding between description and cmd.
        let label_w_pad = label_w + 2;
        let desc_w_pad = desc_w + 2;
        let total = inner.width as usize;
        let used = 2 + label_w_pad + desc_w_pad + entry.cmd.chars().count() + 1;
        let pad = total.saturating_sub(used);

        let label_padded = pad_right(entry.label, label_w_pad);
        let desc_padded = pad_right(entry.description, desc_w_pad);
        lines.push(
            Line::from(vec![
                Span::styled("  ", Style::default().bg(bg)),
                Span::styled(label_padded, label_style),
                Span::styled(desc_padded, desc_style),
                Span::styled(" ".repeat(pad), Style::default().bg(bg)),
                Span::styled(entry.cmd.to_string(), cmd_style),
                Span::styled(" ", Style::default().bg(bg)),
            ])
            .style(Style::default().bg(bg)),
        );
    }
    lines.push(Line::raw("").style(Style::default().bg(theme.panel)));
    lines.push(slash_footer(theme));

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}

fn slash_footer<'a>(theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "↑↓",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" pick · ", Style::default().fg(theme.dim)),
        Span::styled(
            "Enter",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" insert · ", Style::default().fg(theme.dim)),
        Span::styled(
            "Esc",
            Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" dismiss · type to filter", Style::default().fg(theme.dim)),
    ])
    .style(Style::default().bg(theme.panel))
}
