use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{
    App, DialogInputMode, Mode, Nudge, NudgePickAction, Picker, Prompt, Screen, View,
};
use crate::ui::dialog::draft_cursor_spans;

/// Minimum columns the middle hint is guaranteed before the right block is
/// elided. Keeps the keybinding hint readable on narrow terminals instead of
/// clipping it to a handful of chars.
const MIN_HINT_W: u16 = 24;

/// Truncate `s` from the left to at most `max` chars, prefixing `…`. Used to
/// elide the status bar's dim right block (counts/date/version) so the least
/// useful leading tokens drop first and the date/version tail survives.
fn elide_left(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return "…".chars().take(max).collect();
    }
    let keep = max - 1;
    let mut out = String::with_capacity(max);
    out.push('…');
    out.extend(s.chars().skip(s.chars().count() - keep));
    out
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let mut mode_label: std::borrow::Cow<'static, str> = match app.nav.mode {
        Mode::Screen(screen) => match screen {
            Screen::Normal => "NORMAL".into(),
            Screen::Insert => match app.draft.input_mode() {
                DialogInputMode::Normal => "NORMAL",
                DialogInputMode::Insert => "INSERT",
            }
            .into(),
            Screen::Search => "SEARCH".into(),
            Screen::Visual => "VISUAL".into(),
            Screen::Help => "HELP".into(),
            Screen::Settings => "SETTINGS".into(),
            Screen::CommandPalette => "COMMAND".into(),
            Screen::Share => "SHARE".into(),
            Screen::Welcome => "WELCOME".into(),
            Screen::ManageProjects => "PROJECTS".into(),
        },
        Mode::Prompt(prompt) => match prompt {
            Prompt::Project => "PROJECT".into(),
            Prompt::Context => "CONTEXT".into(),
            Prompt::SaveFilter => "SAVE FILTER".into(),
            Prompt::AddTime => "ADD TIME".into(),
            Prompt::IdleNudge => "IDLE NUDGE".into(),
            Prompt::LongTimerNudge => "LONG TIMER NUDGE".into(),
            Prompt::RenameProject => "RENAME PROJECT".into(),
            Prompt::DayBoundary => "DAY BOUNDARY".into(),
        },
        Mode::Picker(picker) => match picker {
            Picker::Project => "PICK +PROJECT".into(),
            Picker::Context => "PICK @CONTEXT".into(),
            Picker::SavedFilter => "PICK FILTER".into(),
            Picker::Theme => "PICK THEME".into(),
            Picker::TimesheetDate => "JUMP TO DATE".into(),
            Picker::NudgeTask => match app.session.nudge_picker.as_ref().map(|p| p.action) {
                Some(NudgePickAction::StartTimer) => "START TIMER".into(),
                Some(NudgePickAction::AddTime) => "ADD TIME".into(),
                None => "PICK TASK".into(),
            },
        },
        Mode::Nudge(nudge) => match nudge {
            Nudge::Idle => "IDLE NUDGE".into(),
            Nudge::LongTimer => "LONG TIMER".into(),
            Nudge::StaleTimer => "STALE TIMER".into(),
            Nudge::Review => "END-OF-DAY".into(),
            Nudge::ManualEntryChoice => "MANUAL ENTRY".into(),
        },
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

    // The hint shown for modes without a dedicated line: the timesheet's long
    // key list when the timesheet view is active, the ordinary list hint
    // otherwise. Several modes (Normal, day-boundary, theme picker, stale
    // timer, manual-entry choice) deliberately fall back to this.
    let default_hint: &'static str = if matches!(app.nav.view, View::Timesheet) {
        "j/k navigate  ·  Enter edit  ·  b billable  ·  a archive toggle  ·  c copy text  ·  y copy time  ·  C copy both  ·  h/l ±day  ·  H/L ±week  ·  w/d view  ·  s sort  ·  f/F filter project/context  ·  / search  ·  g date  ·  t today  ·  Esc/V/q back"
    } else {
        "j/k · n new · t timer · T interrupt · x done · / search · ? help · u undo · q quit"
    };

    // The running-timer status is its own accent segment rendered ahead of
    // the hint, so the mode keybindings stay visible while a timer runs
    // (previously the timer string replaced the hint entirely).
    let timer_status = timer_status(app);
    let hint = match app.nav.mode {
        Mode::Screen(screen) => match screen {
            Screen::Insert => match app.draft.input_mode() {
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
            Screen::Visual => "space toggle · x complete · dd delete · Esc cancel".to_string(),
            Screen::Help => "? close help".to_string(),
            Screen::Settings => {
                "Esc/ ,/ q dismiss  ·  i idle nudge  ·  l long timer nudge".to_string()
            }
            Screen::CommandPalette => "type to filter · Enter run · Esc cancel".to_string(),
            Screen::Share => "scan the QR · any key dismisses".to_string(),
            Screen::Welcome => "c create ./todo.txt · s open sample · q quit".to_string(),
            Screen::ManageProjects => {
                let all = app.all_projects();
                let archived_count = all.iter().filter(|n| app.is_project_archived(n)).count();
                let total = all.len();
                let needle = app.filter().search.to_lowercase();
                let matched = app.filtered_projects().len();
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
                    format!("{base}  —  /{needle} ({matched}/{total})")
                }
            }
            Screen::Normal | Screen::Search => default_hint.to_string(),
        },
        Mode::Prompt(prompt) => match prompt {
            Prompt::Project => "type +project name · Enter save · Esc cancel".to_string(),
            Prompt::Context => "type @context name · Enter toggle · Esc cancel".to_string(),
            Prompt::SaveFilter => "type a filter name · Enter save · Esc cancel".to_string(),
            Prompt::AddTime => {
                "type duration (e.g. 30, 1.5, 14:30; -30 removes) · Enter add · Esc cancel"
                    .to_string()
            }
            Prompt::IdleNudge => "type minutes · Enter save · Esc cancel".to_string(),
            Prompt::LongTimerNudge => "type minutes · Enter save · Esc cancel".to_string(),
            Prompt::RenameProject => "type new name · Enter rename · Esc cancel".to_string(),
            Prompt::DayBoundary => default_hint.to_string(),
        },
        Mode::Picker(picker) => match picker {
            Picker::Project => "j/k or ↑↓ cycle projects · Enter keep · Esc clear".to_string(),
            Picker::Context => "j/k or ↑↓ cycle contexts · Enter keep · Esc clear".to_string(),
            Picker::SavedFilter => "j/k or ↑↓ cycle filters · Enter keep · Esc revert".to_string(),
            Picker::TimesheetDate => {
                "hjkl/arrows navigate  ·  type date  ·  Enter select  ·  Esc cancel  ·  t today"
                    .to_string()
            }
            Picker::NudgeTask => {
                let commit = match app.session.nudge_picker.as_ref().map(|p| p.action) {
                    Some(NudgePickAction::StartTimer) => "Enter start timer on highlighted",
                    Some(NudgePickAction::AddTime) => "Enter add time on highlighted",
                    None => "Enter select",
                };
                format!("{commit} · j/k navigate · / search · +/@ filter · t start · Esc back")
            }
            Picker::Theme => default_hint.to_string(),
        },
        Mode::Nudge(nudge) => match nudge {
            Nudge::Idle => "S start timer · M add time · N new entry · D dismiss".to_string(),
            Nudge::LongTimer => "S stop timer · D dismiss".to_string(),
            Nudge::StaleTimer => default_hint.to_string(),
            Nudge::Review => "V view timesheet · M add time · S skip".to_string(),
            Nudge::ManualEntryChoice => default_hint.to_string(),
        },
    };

    let mut right_parts = Vec::new();
    if app.nav.mode == Mode::Screen(Screen::ManageProjects) {
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
    // The hint gets a guaranteed minimum width — when the right block would
    // squeeze it below that, the right block is elided (from its left, so the
    // date/version tail survives) instead of clipping the keybindings.
    let chip_text = format!(" {mode_label}{chord_suffix} ");
    let chip_w = chip_text.chars().count() as u16;
    let update_w = update_suffix
        .as_deref()
        .map_or(0, |s| s.chars().count() as u16);
    let right_desired = right_text.chars().count() as u16 + update_w + 1;
    let avail = area.width.saturating_sub(chip_w);
    let middle_w = if avail.saturating_sub(right_desired) >= MIN_HINT_W {
        avail.saturating_sub(right_desired)
    } else {
        avail.saturating_sub(MIN_HINT_W.min(avail))
    };
    let right_w = avail.saturating_sub(middle_w);
    // Shrink the dim text to whatever width the right block actually got,
    // keeping the update suffix and its trailing space intact.
    let right_budget = right_w.saturating_sub(update_w).saturating_sub(1);
    let right_text = elide_left(&right_text, right_budget as usize);

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

    let mut mid_spans: Vec<Span> = Vec::with_capacity(3);
    if let Some(status) = &timer_status {
        mid_spans.push(Span::raw("  "));
        mid_spans.push(Span::styled(
            status.clone(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        mid_spans.push(Span::raw("  ·  "));
    } else {
        mid_spans.push(Span::raw("  "));
    }
    mid_spans.push(Span::styled(hint, Style::default().fg(theme.status_fg)));
    let mid_line = Line::from(mid_spans).style(Style::default().bg(theme.statusbar));
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

/// Compact running-timer indicator for the status bar: `▶ +proj @ctx 01:23:45`
/// (or `⏰ … — timer running long!` once the long-timer flag is set). The task
/// body is deliberately omitted — it's already on the task row and in the
/// detail pane — so the segment stays short enough to leave the mode hint
/// visible. Returns `None` when no timer is running.
fn timer_status(app: &App) -> Option<String> {
    if !app.timer_running() {
        return None;
    }
    let elapsed = crate::app::format_clock(app.timer_elapsed_secs().unwrap_or(0));
    let task = app.active_timer_task();
    let proj = task
        .and_then(|t| t.projects.first())
        .map(|p| format!("+{p}"))
        .unwrap_or_default();
    let act = task
        .and_then(|t| t.contexts.first())
        .map(|a| format!("@{a}"))
        .unwrap_or_default();
    let mut parts: Vec<String> = Vec::with_capacity(3);
    if !proj.is_empty() {
        parts.push(proj);
    }
    if !act.is_empty() {
        parts.push(act);
    }
    parts.push(elapsed);
    let mut s = format!("▶ {}", parts.join(" "));
    if app.session.long_timer_nudge_active {
        s = format!("⏰ {s}  —  timer running long!");
    }
    Some(s)
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

#[cfg(test)]
mod tests {
    use super::elide_left;

    #[test]
    fn elide_left_keeps_string_that_fits() {
        assert_eq!(elide_left("1 open · 2026-05-06", 30), "1 open · 2026-05-06");
    }

    #[test]
    fn elide_left_drops_leading_tokens_and_keeps_tail() {
        // The dim block is "N open · date · name version"; eliding from the
        // left must drop the count first and preserve the date/version tail.
        let out = elide_left("12 open · 2026-05-06 · tuxtime 2026.7.1", 24);
        assert_eq!(out, "…5-06 · tuxtime 2026.7.1");
        assert_eq!(out.chars().count(), 24);
    }

    #[test]
    fn elide_left_handles_tiny_budget() {
        assert_eq!(elide_left("abc", 0), "");
        assert_eq!(elide_left("abc", 1), "…");
        assert_eq!(elide_left("abc", 2), "…c");
    }
}
