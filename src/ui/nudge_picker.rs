//! Nudge task selection banner: a thin strip drawn over the *real list view*
//! when the user presses `S` (start timer) or `M` (add time) from the
//! idle-nudge popup. The list itself — navigation, search, filters, detail
//! pane — is the selection surface; this strip just announces the mode and
//! what the commit keys do, so the user always knows a choice is pending.
//!
//! The banner sits at the bottom of the body (right above the status bar):
//! the cursor's row lives at the top of the list, so the highlight stays
//! visible while choosing.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::{App, NudgePickAction};

pub(crate) fn render_banner(frame: &mut Frame, body_area: Rect, app: &App) {
    let theme = app.theme();
    let (title, hint) = match app.session.nudge_picker.as_ref().map(|p| p.action) {
        Some(NudgePickAction::StartTimer) => (
            "▶ START TIMER",
            "pick a task — Enter starts the timer on the highlighted row · / search · Esc back",
        ),
        Some(NudgePickAction::AddTime) => (
            "⏱ ADD TIME",
            "pick a task — Enter adds time to the highlighted row · / search · Esc back",
        ),
        None => ("PICK TASK", "Enter select · Esc back"),
    };
    if body_area.height == 0 {
        return;
    }
    let rect = Rect::new(
        body_area.x,
        body_area.y + body_area.height - 1,
        body_area.width,
        1,
    );
    frame.render_widget(Clear, rect);
    let line = Line::from(vec![
        Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme.bg)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {hint}"),
            Style::default().fg(theme.bg).bg(theme.accent),
        ),
    ])
    .style(Style::default().bg(theme.accent));
    frame.render_widget(Paragraph::new(line), rect);
}
