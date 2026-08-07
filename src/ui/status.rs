use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, DialogInputMode, Mode, View};
use crate::ui::dialog::draft_cursor_spans;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let mut mode_label: std::borrow::Cow<'static, str> = match app.nav.mode {
        Mode::Normal => "NORMAL".into(),
        Mode::Insert => match app.draft.input_mode() {
            DialogInputMode::Normal => "NORMAL",
            DialogInputMode::Insert => "INSERT",
        }
        .into(),
        Mode::Search => "SEARCH".into(),
        Mode::Visual => "VISUAL".into(),
        Mode::Help => "HELP".into(),
        Mode::Settings => "SETTINGS".into(),
        Mode::PromptProject => "PROJECT".into(),
        Mode::PromptContext => "CONTEXT".into(),
        Mode::PickProject => "PICK +PROJECT".into(),
        Mode::PickContext => "PICK @CONTEXT".into(),
        Mode::PickSavedFilter => "PICK FILTER".into(),
        Mode::PromptSaveFilter => "SAVE FILTER".into(),
        Mode::PromptAddTime => "ADD TIME".into(),
        Mode::PromptIdleNudge => "IDLE NUDGE".into(),
        Mode::PromptLongTimerNudge => "LONG TIMER NUDGE".into(),
        Mode::PickTimesheetDate => "JUMP TO DATE".into(),
        Mode::CommandPalette => "COMMAND".into(),
        Mode::Share => "SHARE".into(),
        Mode::PickTheme => "PICK THEME".into(),
        Mode::Welcome => "WELCOME".into(),
        Mode::IdleNudge => "IDLE NUDGE".into(),
        Mode::ManualEntryChoice => "MANUAL ENTRY".into(),
        Mode::ManageProjects => "PROJECTS".into(),
        Mode::PromptRenameProject => "RENAME PROJECT".into(),
        Mode::PromptDayBoundary => "DAY BOUNDARY".into(),
    };
    if matches!(app.nav.view, View::Timesheet) {
        mode_label = "TIMESHEET".into();
    }
    if matches!(app.nav.view, View::Archive) {
        mode_label = "ARCHIVE".into();
    }
    if let Some(f) = app.flash_active() {
        mode_label = format!("{mode_label} · {f}").into();
    }

    let hint = if app.timer_running() {
        if let Some(task) = app.active_timer_task() {
            let elapsed = app.timer_elapsed_secs().unwrap_or(0);
            let h = elapsed / 3600;
            let m = (elapsed % 3600) / 60;
            let s = elapsed % 60;
            let proj = task
                .projects
                .first()
                .map(|p| format!("+{p} "))
                .unwrap_or_default();
            let act = task
                .contexts
                .first()
                .map(|a| format!("@{a} "))
                .unwrap_or_default();
            let body = crate::todo::body_only(&task.raw);
            let mut time_str = format!("▶ {proj}{act} {h:02}:{m:02}:{s:02}  {body}");
            if app.session.long_timer_nudge_active {
                time_str = format!("⏰ {time_str}  —  timer running long!");
            }
            time_str
        } else {
            "▶ timer running".to_string()
        }
    } else {
        match app.nav.mode {
            Mode::Insert => match app.draft.input_mode() {
                DialogInputMode::Normal => {
                    if app.session.manual_time_entry {
                        "h/l navigate · w/b/e word · i/a insert · dur:90 (min) dur:1.5h dur:14:30 dur:9am → Enter save · C-Enter save+start".to_string()
                    } else {
                        "h/l navigate · w/b/e word · i/a insert · Enter save · C-Enter save+start · Esc cancel".to_string()
                    }
                }
                DialogInputMode::Insert => {
                    if app.session.manual_time_entry {
                        "Enter save · C-Enter save+start · Esc normal — type a duration after dur:"
                            .to_string()
                    } else {
                        "Enter save · C-Enter save+start · Esc normal".to_string()
                    }
                }
            },
            Mode::Visual => "space toggle · x complete · dd delete · Esc cancel".to_string(),
            Mode::Help => "? close help".to_string(),
            Mode::Settings => {
                "Esc/ ,/ q dismiss  ·  i idle nudge  ·  l long timer nudge".to_string()
            }
            Mode::PromptProject => "type +project name · Enter save · Esc cancel".to_string(),
            Mode::PromptContext => "type @context name · Enter toggle · Esc cancel".to_string(),
            Mode::PickProject => "j/k or ↑↓ cycle projects · Enter keep · Esc clear".to_string(),
            Mode::PickContext => "j/k or ↑↓ cycle contexts · Enter keep · Esc clear".to_string(),
            Mode::PickSavedFilter => {
                "j/k or ↑↓ cycle filters · Enter keep · Esc revert".to_string()
            }
            Mode::PromptSaveFilter => "type a filter name · Enter save · Esc cancel".to_string(),
            Mode::PromptAddTime => {
                "type duration (e.g. 30, 1.5, 14:30) · Enter add · Esc cancel".to_string()
            }
            Mode::PromptIdleNudge => "type minutes · Enter save · Esc cancel".to_string(),
            Mode::PromptLongTimerNudge => "type minutes · Enter save · Esc cancel".to_string(),
            Mode::PickTimesheetDate => {
                "hjkl/arrows navigate  ·  type date  ·  Enter select  ·  Esc cancel  ·  t today"
                    .to_string()
            }
            Mode::CommandPalette => "type to filter · Enter run · Esc cancel".to_string(),
            Mode::Share => "scan the QR · any key dismisses".to_string(),
            Mode::Welcome => "c create ./todo.txt · s open sample · q quit".to_string(),
            Mode::ManageProjects => {
                let all = app.all_projects();
                let archived_count = all.iter().filter(|n| app.is_project_archived(n)).count();
                let total = all.len();
                let needle = app.filter().search.to_lowercase();
                let filtered: Vec<&String> = if needle.is_empty() {
                    all.iter().collect()
                } else {
                    all.iter()
                        .filter(|n| n.to_lowercase().contains(&needle))
                        .collect()
                };
                let sort = app.project_manager.project_sort.label();
                let base = format!(
                    "j/k nav · x archive · r rename · s sort ({sort}) · / search · Esc/P back"
                );
                if needle.is_empty() {
                    format!(
                        "{base}  —  {total} projects{archived}",
                        archived = if archived_count > 0 {
                            format!(", {archived_count} archived")
                        } else {
                            String::new()
                        }
                    )
                } else {
                    format!("{base}  —  /{needle} ({}/{total})", filtered.len())
                }
            }
            Mode::PromptRenameProject => "type new name · Enter rename · Esc cancel".to_string(),
            _ => {
                if matches!(app.nav.view, View::Timesheet) {
                    "j/k navigate  ·  Enter edit  ·  b billable  ·  a archive toggle  ·  c copy text  ·  y copy time  ·  C copy both  ·  h/l ±day  ·  H/L ±week  ·  w/d view  ·  s sort  ·  / search  ·  g date  ·  t today  ·  Esc/V/q back".to_string()
                } else {
                    "j/k · n new · t timer · T interrupt · x done · / search · ? help · u undo · q quit".to_string()
                }
            }
        }
    };

    let mut right_parts = Vec::new();
    if app.nav.mode == Mode::ManageProjects {
        let all = app.all_projects();
        let archived = all.iter().filter(|n| app.is_project_archived(n)).count();
        right_parts.push(format!("{} projects · {} archived", all.len(), archived));
    } else if matches!(app.nav.view, View::Archive) {
        right_parts.push(format!("{} archived", app.archive().len()));
    } else {
        right_parts.push(format!("{} open", app.visible_indices().len()));
    }
    if !app.selection.is_empty() {
        right_parts.push(format!("{} selected", app.selection.len()));
    }
    right_parts.push(app.today().to_string());
    right_parts.push(concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION")).to_string());
    // Track where the update suffix would slot in so we can paint it in the
    // accent color (the rest of the right text is dim).
    let update_suffix = app
        .update_available()
        .map(|tag| format!(" · ↑ {tag} (tuxtime update)"));
    let right_text = right_parts.join(" · ");

    // Append a chord indicator (e.g. " g…") so two-key sequences like gg/dd/fp
    // give visible feedback on the first press. Only shown while armed.
    let chord_suffix = app
        .chord
        .active()
        .map(|c| format!(" {c}…"))
        .unwrap_or_default();
    // Layout: mode chip on left, hint in middle, right text right-aligned.
    let chip_text = format!(" {mode_label}{chord_suffix} ");
    let chip_w = chip_text.chars().count() as u16;
    let update_w = update_suffix
        .as_deref()
        .map_or(0, |s| s.chars().count() as u16);
    let right_w = right_text.chars().count() as u16 + update_w + 1;
    let middle_w = area.width.saturating_sub(chip_w).saturating_sub(right_w);

    let [chip_area, mid_area, right_area] = Layout::horizontal([
        Constraint::Length(chip_w),
        Constraint::Length(middle_w),
        Constraint::Length(right_w),
    ])
    .areas(area);

    let chip = Paragraph::new(Span::styled(
        chip_text,
        Style::default()
            .bg(theme.mode_bg)
            .fg(theme.mode_fg)
            .add_modifier(Modifier::BOLD),
    ))
    .style(Style::default().bg(theme.statusbar));
    frame.render_widget(chip, chip_area);

    let mid_line = Line::from(vec![
        Span::raw("  "),
        Span::styled(hint, Style::default().fg(theme.status_fg)),
    ])
    .style(Style::default().bg(theme.statusbar));
    frame.render_widget(
        Paragraph::new(mid_line).style(Style::default().bg(theme.statusbar)),
        mid_area,
    );

    let right_line = if let Some(suffix) = update_suffix {
        Line::from(vec![
            Span::styled(right_text, Style::default().fg(theme.dim)),
            Span::styled(
                suffix,
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default().fg(theme.dim)),
        ])
        .style(Style::default().bg(theme.statusbar))
    } else {
        Line::from(Span::styled(
            format!("{right_text} "),
            Style::default().fg(theme.dim),
        ))
        .style(Style::default().bg(theme.statusbar))
    };
    frame.render_widget(
        Paragraph::new(right_line)
            .style(Style::default().bg(theme.statusbar))
            .right_aligned(),
        right_area,
    );
}

pub fn render_command_line(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let visible_count = app.visible_indices().len();
    let suggestion = format!("  {visible_count} matches · Enter accept · Esc cancel");
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            "/",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    spans.extend(draft_cursor_spans(
        app.draft.text(),
        app.draft.cursor(),
        theme.fg,
        theme.bg,
    ));
    spans.push(Span::styled(suggestion, Style::default().fg(theme.dim)));
    let line = Line::from(spans).style(Style::default().bg(theme.bg));
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme.bg)),
        area,
    );
}
