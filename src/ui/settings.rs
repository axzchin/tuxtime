use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::ui::header;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    super::fill_bg(frame, area, Style::default().bg(theme.bg));

    let [header_area, _sp, body_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(area);

    header::render(
        frame,
        header_area,
        theme,
        header::HeaderProps {
            title: Some("settings"),
            // title: None,
            // file: "settings",
            count: app.tasks().len(),
            sort: app.sort_label(),
            filter: None,
        },
    );

    let mut lines: Vec<Line> = Vec::new();
    let density = match app.prefs.density {
        crate::app::Density::Compact => "compact",
        crate::app::Density::Comfortable => "comfortable",
        crate::app::Density::Cozy => "cozy",
    };
    let on = |b: bool| if b { "on" } else { "off" };

    let config_path = app
        .env
        .config_path
        .as_ref()
        .map_or_else(|| "(unavailable)".into(), |p| p.display().to_string());

    let items: &[(&str, Option<String>)] = &[
        ("FILES", None),
        ("  todo file", Some(app.env.file_path.display().to_string())),
        ("  config file", Some(config_path)),
        ("", Some(String::new())),
        ("DISPLAY", None),
        ("  theme", Some(format!("{} ▾  (Z to pick)", theme.name))),
        ("  density", Some(format!("{density} ▾  (D to cycle)"))),
        (
            "  line numbers",
            Some(format!("{}  (L to toggle)", on(app.prefs.layout.line_num))),
        ),
        ("  status bar", Some(on(app.prefs.layout.status_bar).into())),
        (
            "  filter sidebar",
            Some(format!("{}  ([ to toggle)", on(app.prefs.layout.left))),
        ),
        (
            "  detail sidebar",
            Some(format!("{}  (] to toggle)", on(app.prefs.layout.right))),
        ),
        (
            "  show done in list",
            Some(format!("{}  (H to toggle)", on(app.prefs.show_done))),
        ),
        (
            "  show future in list",
            Some(format!("{}  (F to toggle)", on(app.prefs.show_future))),
        ),
        (
            "  duration badge inline",
            Some(format!(
                "{}  (I to toggle)",
                on(app.prefs.show_duration_inline)
            )),
        ),
        (
            "  prefill + on new task",
            Some(format!(
                "{}  (+ to toggle)",
                on(app.prefs.prefill_plus_new)
            )),
        ),
        (
            "  edit cursor at narrative",
            Some(format!(
                "{}  (e to toggle)",
                if app.prefs.edit_cursor_narrative_start {
                    "start"
                } else {
                    "end"
                }
            )),
        ),
        (
            "  prompt if done w/o time",
            Some(format!(
                "{}  (x to toggle)",
                on(app.prefs.prompt_complete_no_time)
            )),
        ),
        (
            "  enter timer",
            Some(format!(
                "{}  (t to toggle)",
                if app.prefs.enter_timer_toggle {
                    "toggle"
                } else {
                    "start-only"
                }
            )),
        ),
        (
            "  log date inline",
            Some(format!("{}  (O to toggle)", on(app.prefs.show_log_inline))),
        ),
        (
            "  badge theme",
            Some(format!("{}  (B to cycle)", app.prefs.badge_theme)),
        ),
        ("", Some(String::new())),
        ("BEHAVIOR", None),
        (
            "  default sort",
            Some(format!("{} (S to cycle)", app.sort_label())),
        ),
        (
            "  idle nudge",
            Some(format!(
                "{} min  (i to change)",
                app.idle_nudge_seconds() / 60
            )),
        ),
        (
            "  long timer nudge",
            Some(format!(
                "{} min  (l to change)",
                app.long_timer_nudge_seconds() / 60
            )),
        ),
        (
            "  rounding increment",
            Some(format!(
                "{}  (r to cycle)",
                crate::app::rounding_increment_label(app.prefs.rounding_increment)
            )),
        ),
        ("", Some(String::new())),
        ("KEYBINDINGS", None),
        ("  ", Some("press ? for the full list".into())),
    ];

    for (k, v) in items {
        match v {
            None => {
                lines.push(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        k.to_string(),
                        Style::default()
                            .fg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            }
            Some(val) if k.is_empty() => {
                lines.push(Line::raw(" "));
                let _ = val;
            }
            Some(val) => {
                let mut padded = k.to_string();
                let len = padded.chars().count();
                if len < 30 {
                    padded.push_str(&" ".repeat(30 - len));
                }
                lines.push(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(padded, Style::default().fg(theme.fg)),
                    Span::styled(val.clone(), Style::default().fg(theme.dim)),
                ]));
            }
        }
    }

    let para = Paragraph::new(lines).style(Style::default().bg(theme.bg).fg(theme.fg));
    frame.render_widget(para, body_area);
}
