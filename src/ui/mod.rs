use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{App, Mode, View};
use crate::ui::dialog::{calendar_cells, calendar_footer, format_focused, month_name};

pub mod archive;
pub mod command_palette;
pub mod detail;
pub mod dialog;
pub mod empty;
pub mod filters;
pub mod header;
pub mod help;
pub mod hyperlinks;
pub mod list;
pub mod logo;
pub mod settings;
pub mod share;
pub mod status;
pub mod task_row;
pub mod theme_picker;
pub mod title;
pub mod welcome;

// Pane and overlay sizing. Promoted out of inline literals so the three
// `MIN_BODY_W` references below stay in sync, and so tweaking a sidebar
// width is a one-line change.
const LEFT_PANE_W: u16 = 26;
const RIGHT_PANE_W: u16 = 34;
const MIN_BODY_W: u16 = 40;

const DIALOG_H: u16 = 8;
const DIALOG_MIN_W: u16 = 40;
const DIALOG_MAX_W: u16 = 100;

const HELP_MAX_H: u16 = 40;
const HELP_MIN_W: u16 = 76;
const HELP_MAX_W: u16 = 120;

const PROMPT_H: u16 = 5;
const PROMPT_MAX_W: u16 = 50;

const PALETTE_MAX_H: u16 = 20;
const PALETTE_MIN_W: u16 = 50;
const PALETTE_MAX_W: u16 = 80;

pub fn draw(frame: &mut Frame, app: &App) {
    let theme = app.theme();
    let area = frame.area();

    // Paint full background.
    frame.render_widget(Block::default().style(Style::default().bg(theme.bg)), area);

    let bottom = u16::from(app.prefs.layout.status_bar);
    let [body_area, bottom_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(bottom)]).areas(area);

    // Determine pane widths. Sidebars apply to every view; navigation +
    // detail pane track the cursor regardless of which view is active.
    //
    // We compute exact Column widths instead of relying on ratatui's
    // constraint solver, which can zero out `Length` or `Max` panes when
    // the total exceeds the terminal width.  Sidebars get a guaranteed
    // minimum so they never become invisible; extra space goes to the
    // centre body (up to its desired width) and then to the sidebars.
    let show_left = app.prefs.layout.left;
    let show_right = app.prefs.layout.right;

    // Minimum widths for sidebars to remain useful on narrow terminals.
    const MIN_LEFT_W: u16 = 12;
    const MIN_RIGHT_W: u16 = 16;

    let left_desired = if show_left { LEFT_PANE_W } else { 0 };
    let right_desired = if show_right { RIGHT_PANE_W } else { 0 };
    let left_min = if show_left { MIN_LEFT_W } else { 0 };
    let right_min = if show_right { MIN_RIGHT_W } else { 0 };

    let available = body_area.width;
    let total_min = left_min + MIN_BODY_W + right_min;

    let (left_w, center_w, right_w) = if available <= total_min {
        // Even minimums barely fit — give sidebars their floor, centre
        // gets whatever is left.
        let after_sides = available.saturating_sub(left_min + right_min);
        (left_min, after_sides, right_min)
    } else {
        // Minimums fit.  Start there, then grow sidebars toward their
        // desired widths before giving remaining space to the centre.
        let mut extra = available - total_min;
        let mut left = left_min;
        let mut right = right_min;

        if left < left_desired {
            let need = left_desired - left;
            let take = need.min(extra);
            left += take;
            extra -= take;
        }
        if right < right_desired {
            let need = right_desired - right;
            let take = need.min(extra);
            right += take;
            extra -= take;
        }
        (left, MIN_BODY_W + extra, right)
    };

    let constraints: Vec<Constraint> = match (show_left, show_right) {
        (true, true) => vec![
            Constraint::Length(left_w),
            Constraint::Length(center_w),
            Constraint::Length(right_w),
        ],
        (true, false) => vec![Constraint::Length(left_w), Constraint::Length(center_w)],
        (false, true) => vec![Constraint::Length(center_w), Constraint::Length(right_w)],
        (false, false) => vec![Constraint::Length(center_w)],
    };
    let chunks = Layout::horizontal(constraints).split(body_area);

    let (left_area, center_area, right_area) = match (show_left, show_right) {
        (true, true) => (Some(chunks[0]), chunks[1], Some(chunks[2])),
        (true, false) => (Some(chunks[0]), chunks[1], None),
        (false, true) => (None, chunks[0], Some(chunks[1])),
        (false, false) => (None, chunks[0], None),
    };

    if let Some(la) = left_area {
        filters::render(frame, la, app);
    }
    match app.view() {
        View::List => list::render(frame, center_area, app),
        View::Archive => archive::render(frame, center_area, app),
        View::Timesheet => render_timesheet(frame, center_area, app),
    }
    if let Some(ra) = right_area {
        detail::render(frame, ra, app);
    }

    if app.prefs.layout.status_bar {
        if app.nav.mode == Mode::Search {
            status::render_command_line(frame, bottom_area, app);
        } else {
            status::render(frame, bottom_area, app);
        }
    }

    // Overlays
    match app.nav.mode {
        Mode::Insert => {
            let dlg_w: u16 = (u32::from(center_area.width) * 4 / 5)
                .clamp(u32::from(DIALOG_MIN_W), u32::from(DIALOG_MAX_W))
                as u16;
            let dlg = centered_in(area, dlg_w, DIALOG_H);
            frame.render_widget(Clear, dlg);
            dialog::render(frame, dlg, app);
            // At most one overlay shows at a time. The autocomplete popup is
            // suppressed while a metadata picker is open so we don't stack
            // two floating panels in the same spot.
            if !dialog::render_overlay(frame, dlg, area, app) {
                dialog::render_autocomplete(frame, dlg, area, app);
            }
        }
        Mode::Help => {
            let h: u16 = area.height.saturating_sub(3).min(HELP_MAX_H);
            let w: u16 = (u32::from(area.width) * 9 / 10)
                .clamp(u32::from(HELP_MIN_W), u32::from(HELP_MAX_W))
                as u16;
            let r = centered_in(area, w, h);
            frame.render_widget(Clear, r);
            help::render(frame, r, app);
        }
        Mode::Settings => {
            frame.render_widget(Clear, body_area);
            settings::render(frame, body_area, app);
        }
        // Nudge prompts stack on top of Settings so the user sees the
        // settings table *behind* the edit dialog, rather than having the
        // settings close/reopen around the prompt.
        Mode::PromptIdleNudge | Mode::PromptLongTimerNudge => {
            frame.render_widget(Clear, body_area);
            settings::render(frame, body_area, app);
            let w: u16 = PROMPT_MAX_W.min(area.width.saturating_sub(4));
            let r = centered_in(area, w, PROMPT_H);
            frame.render_widget(Clear, r);
            dialog::render_prompt(frame, r, app);
        }
        // Rename-project prompt stacks on top of the project management
        // view so the user sees the project list behind the edit dialog.
        Mode::PromptRenameProject => {
            frame.render_widget(Clear, body_area);
            render_manage_projects(frame, body_area, app);
            let w: u16 = PROMPT_MAX_W.min(area.width.saturating_sub(4));
            let r = centered_in(area, w, PROMPT_H);
            frame.render_widget(Clear, r);
            dialog::render_prompt(frame, r, app);
        }
        Mode::PromptProject | Mode::PromptContext | Mode::PromptSaveFilter
        | Mode::PromptAddTime => {
            let w: u16 = PROMPT_MAX_W.min(area.width.saturating_sub(4));
            let r = centered_in(area, w, PROMPT_H);
            frame.render_widget(Clear, r);
            dialog::render_prompt(frame, r, app);
            if matches!(app.nav.mode, Mode::PromptProject | Mode::PromptContext) {
                dialog::render_autocomplete(frame, r, area, app);
            }
        }
        Mode::CommandPalette => {
            let h: u16 = area.height.saturating_sub(4).min(PALETTE_MAX_H);
            let w: u16 = (u32::from(area.width) * 3 / 5)
                .clamp(u32::from(PALETTE_MIN_W), u32::from(PALETTE_MAX_W))
                as u16;
            let r = centered_in(area, w, h);
            frame.render_widget(Clear, r);
            command_palette::render(frame, r, app);
        }
        Mode::Share => {
            let (w, h) = share::size_for(app);
            let r = centered_in(area, w, h);
            frame.render_widget(Clear, r);
            share::render(frame, r, app);
        }
        Mode::PickTheme => {
            let h: u16 = area.height.saturating_sub(4).min(PALETTE_MAX_H);
            let w: u16 = (u32::from(area.width) * 3 / 5)
                .clamp(u32::from(PALETTE_MIN_W), u32::from(PALETTE_MAX_W))
                as u16;
            let r = centered_in(area, w, h);
            frame.render_widget(Clear, r);
            theme_picker::render(frame, r, app);
        }
        Mode::Welcome => {
            let r = centered_in(area, welcome::WIDTH, welcome::HEIGHT);
            frame.render_widget(Clear, r);
            welcome::render(frame, r, app);
        }
        Mode::PickTimesheetDate => {
            render_timesheet_calendar(frame, area, app);
        }
        Mode::IdleNudge => {
            let r = centered_in(area, 60, 6);
            frame.render_widget(Clear, r);
            let theme = app.theme();
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.accent).bg(theme.panel))
                .title(" ⏰ Idle Nudge ")
                .style(Style::default().bg(theme.panel));
            let inner = block.inner(r);
            frame.render_widget(block, r);
            let lines = vec![
                Line::from(Span::styled(
                    "No timer running!",
                    Style::default().fg(theme.fg).bg(theme.panel).add_modifier(Modifier::BOLD),
                )),
                Line::raw(""),
                Line::from(Span::styled(
                    "[N]ew entry  [D]ismiss",
                    Style::default().fg(theme.dim).bg(theme.panel),
                )),
            ];
            frame.render_widget(Paragraph::new(lines).centered(), inner);
        }
        Mode::ManualEntryChoice => {
            let r = centered_in(area, 60, 6);
            frame.render_widget(Clear, r);
            let theme = app.theme();
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.accent).bg(theme.panel))
                .title(" ✏ Manual Time Entry ")
                .style(Style::default().bg(theme.panel));
            let inner = block.inner(r);
            frame.render_widget(block, r);
            let lines = vec![
                Line::from(Span::styled(
                    "How would you like to describe this entry?",
                    Style::default().fg(theme.fg).bg(theme.panel).add_modifier(Modifier::BOLD),
                )),
                Line::raw(""),
                Line::from(Span::styled(
                    "[N]ew blank entry  [A]dd to current task  [Esc] cancel",
                    Style::default().fg(theme.dim).bg(theme.panel),
                )),
            ];
            frame.render_widget(Paragraph::new(lines).centered(), inner);
        }
        Mode::ManageProjects => {
            frame.render_widget(Clear, body_area);
            render_manage_projects(frame, body_area, app);
        }
        _ => {}
    }
    // OSC 8 hyperlinks are applied post-draw by the caller (see
    // `hyperlinks::collect` + `emit_overlay`). Doing it inside the buffer
    // breaks ratatui's diff width calculation — keep cell symbols pristine.
}

/// Render the timesheet inline in the center area so sidebars remain fully
/// visible.
fn render_timesheet(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();

    let display = app.timesheet_date_display();
    let view_label = if app.timesheet.weekly {
        Span::styled(
            format!(" {display} (week) "),
            Style::default()
                .fg(theme.bg)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" {display} "),
            Style::default()
                .fg(theme.bg)
                .bg(theme.today)
                .add_modifier(Modifier::BOLD),
        )
    };
    let sort_label = Span::styled(
        format!(" {} ", app.timesheet.sort.label()),
        Style::default()
            .fg(theme.dim)
            .bg(theme.panel),
    );
    let title = Line::from(vec![
        Span::raw(" Timesheet "),
        view_label,
        sort_label,
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(title)
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let groups = app.build_timesheet_groups();

    let mut lines: Vec<Line> = Vec::new();
    if groups.is_empty() {
        let msg = if app.filter().search.is_empty() {
            "  No time entries for this period."
        } else {
            "  No entries match your search."
        };
        lines.push(Line::from(Span::styled(
            msg,
            Style::default().fg(theme.dim).bg(theme.panel),
        )));
    } else {
        // Narrative-level cursor: resolve to the group that contains the
        // narrative under the cursor so we can highlight the correct group.
        let selected = app.timesheet_narrative_at(app.timesheet.cursor);
        let selected_group = selected.map(|(gi, _, _)| gi);
        let selected_narrative = selected.map(|(_, ni, _)| ni);
        let mut grand_total: u64 = 0;
        let mut billable_total: u64 = 0;
        let mut dnb_total: u64 = 0;
        let mut day_total: u64 = 0;
        // Billable tenths (per-group rounding, summed): 1 min × 5 matters = 0.5h.
        let mut grand_billable_tenths: u64 = 0;
        let mut billable_billable_tenths: u64 = 0;
        let mut dnb_billable_tenths: u64 = 0;
        let mut day_billable_tenths: u64 = 0;
        let mut last_date: Option<&str> = None;
        for (i, entry) in groups.iter().enumerate() {
            let is_selected = selected_group == Some(i);

            // Emit date header when date changes (visible in weekly + date-sort views).
            if (app.timesheet.weekly
                || matches!(app.timesheet.sort, crate::app::TimesheetSort::Date))
                && last_date != Some(entry.date.as_str())
            {
                // Flush the previous day's subtotal before the new header.
                if last_date.is_some() {
                    lines.push(Line::from(Span::styled(
                        format!("    ──  {} ({})", crate::app::format_duration(day_total), crate::app::format_billable_tenths(day_billable_tenths)),
                        Style::default()
                            .fg(theme.dim)
                            .bg(theme.panel),
                    )));
                    lines.push(Line::raw(""));
                    day_total = 0;
                    day_billable_tenths = 0;
                }
                let date_dow = chrono::NaiveDate::parse_from_str(
                    &entry.date,
                    "%Y-%m-%d",
                ).map_or_else(|_| entry.date.clone(), |d| d.format("%a %Y-%m-%d").to_string());
                lines.push(Line::from(Span::styled(
                    format!("  {date_dow}"),
                    Style::default()
                        .fg(theme.today)
                        .bg(theme.panel)
                        .add_modifier(Modifier::BOLD),
                )));
                last_date = Some(&entry.date);
            }

            let formatted = crate::app::format_duration(entry.total_secs);
            let billable_str = crate::app::format_billable(entry.total_secs);
            // Copy-flash: briefly highlight the entire group white after copy.
            let copy_flash_active = app
                .timesheet.copy_flash
                .is_some_and(|(gi, t)| gi == i && t.elapsed() < crate::app::FLASH_TTL);
            let group_style = if copy_flash_active {
                Style::default()
                    .fg(theme.bg)
                    .bg(theme.fg)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default()
                    .fg(theme.accent)
                    .bg(theme.selection)
                    .add_modifier(Modifier::BOLD)
            } else if !entry.billable {
                Style::default().fg(theme.dim).bg(theme.panel)
            } else {
                Style::default()
                    .fg(theme.accent)
                    .bg(theme.panel)
                    .add_modifier(Modifier::BOLD)
            };
            let dnb_suffix = if entry.billable { "" } else { " (DNB)" };
            lines.push(Line::from(Span::styled(
                format!("  {}  —  {} ({billable_str}){dnb_suffix}", entry.key, formatted),
                group_style,
            )));

            for (ni, n) in entry.narratives.iter().enumerate() {
                let is_narr_cursor = is_selected && selected_narrative == Some(ni);
                let (is_done, is_archived) = entry
                    .task_indices
                    .get(ni)
                    .map_or((false, false), |r| match r {
                        crate::app::TimesheetTaskRef::Active(abs) => {
                            (app.tasks().get(*abs).is_some_and(|t| t.done), false)
                        }
                        crate::app::TimesheetTaskRef::Archived(_) => (false, true),
                    });
                let status_suffix = if is_archived {
                    " (archived)"
                } else if is_done {
                    " (done)"
                } else {
                    ""
                };
                let narr_style = if copy_flash_active {
                    Style::default().fg(theme.bg).bg(theme.fg)
                } else if is_selected {
                    if is_narr_cursor {
                        Style::default()
                            .fg(theme.fg)
                            .bg(theme.selection)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.dim).bg(theme.selection)
                    }
                } else if is_archived || is_done || !entry.billable {
                    Style::default().fg(theme.dim).bg(theme.panel)
                } else {
                    Style::default().fg(theme.fg).bg(theme.panel)
                };
                lines.push(Line::from(Span::styled(
                    format!("    • {n}{status_suffix}"),
                    narr_style,
                )));
            }
            let tenths = entry.total_secs.div_ceil(360);
            grand_total += entry.total_secs;
            grand_billable_tenths += tenths;
            day_total += entry.total_secs;
            day_billable_tenths += tenths;
            if entry.billable {
                billable_total += entry.total_secs;
                billable_billable_tenths += tenths;
            } else {
                dnb_total += entry.total_secs;
                dnb_billable_tenths += tenths;
            }
        }
        // Flush the final day's subtotal (if date headers were shown).
        if (app.timesheet.weekly
            || matches!(app.timesheet.sort, crate::app::TimesheetSort::Date))
            && last_date.is_some()
        {
            lines.push(Line::from(Span::styled(
                format!("    ──  {} ({})", crate::app::format_duration(day_total), crate::app::format_billable_tenths(day_billable_tenths)),
                Style::default()
                    .fg(theme.dim)
                    .bg(theme.panel),
            )));
            lines.push(Line::raw(""));
        }
        // Grand total with billable / DNB split.
        lines.push(Line::raw(""));
        let search_note = if app.filter().search.is_empty() {
            ""
        } else {
            " (filtered)"
        };
        let billable_str = crate::app::format_billable_tenths(billable_billable_tenths);
        lines.push(Line::from(Span::styled(
            format!(
                "  Billable: {} ({billable_str})",
                crate::app::format_duration(billable_total),
            ),
            Style::default()
                .fg(theme.accent)
                .bg(theme.panel)
                .add_modifier(Modifier::BOLD),
        )));
        if dnb_total > 0 {
            let dnb_str = crate::app::format_billable_tenths(dnb_billable_tenths);
            lines.push(Line::from(Span::styled(
                format!("  DNB:      {} ({dnb_str})", crate::app::format_duration(dnb_total)),
                Style::default()
                    .fg(theme.dim)
                    .bg(theme.panel)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        let total_str = crate::app::format_billable_tenths(grand_billable_tenths);
        lines.push(Line::from(Span::styled(
            format!("  Total:    {} ({total_str}){search_note}", crate::app::format_duration(grand_total)),
            Style::default()
                .fg(theme.accent)
                .bg(theme.panel)
                .add_modifier(Modifier::BOLD),
        )));
    }

    // Scroll if content exceeds space
    let [_pad_top, body_rect] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)])
            .areas(inner);
    let max_lines = body_rect.height as usize;
    let visible: Vec<Line> = lines.into_iter().take(max_lines).collect();
    frame.render_widget(Paragraph::new(visible), body_rect);
}

/// Render a centered calendar overlay for the timesheet date picker.
fn render_timesheet_calendar(frame: &mut Frame, area: Rect, app: &App) {
    use chrono::{Datelike, NaiveDate};
    let theme = app.theme();
    let focused = app.timesheet.calendar_focus;
    let today = NaiveDate::parse_from_str(app.today(), "%Y-%m-%d").ok();
    let first_of_month =
        NaiveDate::from_ymd_opt(focused.year(), focused.month(), 1).unwrap_or(focused);

    let popup_w: u16 = 50u16.min(area.width.max(40));
    let popup_h: u16 = 14;
    let r = centered_in(area, popup_w, popup_h);
    frame.render_widget(Clear, r);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(Line::from(Span::styled(
            " JUMP TO DATE ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )))
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(r);
    frame.render_widget(block, r);

    let mut lines: Vec<Line> = Vec::new();
    // Input line: show typed date (or placeholder when empty).
    let input_text = if app.timesheet.date_input.is_empty() {
        format!("{}  ", format_focused(focused))
    } else {
        format!("{}█ ", app.timesheet.date_input)
    };
    lines.push(
        Line::from(vec![
            Span::raw("  "),
            Span::styled("date: ", Style::default().fg(theme.dim)),
            Span::styled(
                input_text,
                Style::default()
                    .fg(theme.fg)
                    .add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::default().bg(theme.panel)),
    );
    lines.push(Line::raw("").style(Style::default().bg(theme.panel)));
    // Header: « Month YYYY »
    let header = Line::from(vec![
        Span::raw("  "),
        Span::styled("‹  ", Style::default().fg(theme.dim)),
        Span::styled(
            format!("{} {}", month_name(focused.month()), focused.year()),
            Style::default().fg(theme.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ›", Style::default().fg(theme.dim)),
    ])
    .style(Style::default().bg(theme.panel));
    lines.push(header);
    // Weekday row.
    let dow_header = if app.week_start == crate::app::WeekStart::Sunday {
        Span::styled(
            "  S   M   T   W   T   F   S ",
            Style::default().fg(theme.dim),
        )
    } else {
        Span::styled(
            "  M   T   W   T   F   S   S ",
            Style::default().fg(theme.dim),
        )
    };
    let dow = Line::from(vec![Span::raw("  "), dow_header]).style(Style::default().bg(theme.panel));
    lines.push(dow);
    // Calendar grid.
    for row in calendar_cells(first_of_month, app.week_start) {
        let mut spans: Vec<Span> = vec![Span::raw("  ")];
        for cell in row {
            match cell {
                None => spans.push(Span::styled("    ", Style::default().bg(theme.panel))),
                Some(n) => {
                    let date = NaiveDate::from_ymd_opt(focused.year(), focused.month(), n);
                    let is_today = today == date;
                    let is_focus = focused.day() == n;
                    let mut style = Style::default().fg(theme.fg).bg(theme.panel);
                    if is_today {
                        style = style.fg(theme.today);
                    }
                    if is_focus {
                        style = style.bg(theme.cursor).add_modifier(Modifier::BOLD);
                    }
                    spans.push(Span::styled(format!(" {n:>2} "), style));
                }
            }
        }
        lines.push(Line::from(spans).style(Style::default().bg(theme.panel)));
    }
    // Spacer + focused date label + footer.
    lines.push(Line::raw("").style(Style::default().bg(theme.panel)));
    lines.push(
        Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format_focused(focused),
                Style::default().fg(theme.due).add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::default().bg(theme.panel)),
    );
    lines.push(calendar_footer(theme));

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        inner,
    );
}

/// Render the project management overlay (press `P` from Normal mode).
fn render_manage_projects(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let sort_label = Span::styled(
        format!(" {} ", app.project_manager.project_sort.label()),
        Style::default().fg(theme.dim).bg(theme.panel),
    );
    let title = Line::from(vec![
        Span::raw(" Project Management "),
        sort_label,
    ]);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border).bg(theme.panel))
        .title(title)
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let projects = app.filtered_projects();
    // Clamp cursor to filtered list bounds.
    let cursor = app.nav.cursor.min(projects.len().saturating_sub(1));

    let mut lines: Vec<Line> = Vec::new();

    if projects.is_empty() {
        let msg = if app.filter().search.is_empty() {
            "  No projects found."
        } else {
            "  No projects match your search."
        };
        lines.push(Line::from(Span::styled(
            msg,
            Style::default().fg(theme.dim).bg(theme.panel),
        )));
    } else {
        for (i, name) in projects.iter().enumerate() {
            let archived = app.is_project_archived(name);
            let is_cursor = i == cursor;
            let mut style = Style::default().bg(theme.panel);
            if is_cursor {
                style = style.bg(theme.selection).add_modifier(Modifier::BOLD);
                if archived {
                    style = style.fg(theme.dim);
                } else {
                    style = style.fg(theme.fg);
                }
            } else if archived {
                style = style.fg(theme.dim);
            } else {
                style = style.fg(theme.fg);
            }
            let cursor_mark = if is_cursor { ">" } else { " " };
            let status = if archived { " (archived)" } else { "" };
            lines.push(Line::from(Span::styled(
                format!(" {cursor_mark} +{name}{status}"),
                style,
            )));
        }
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

pub(crate) fn centered_in(parent: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(parent.width);
    let h = h.min(parent.height);
    let x = parent.x + (parent.width - w) / 2;
    let y = parent.y + (parent.height - h) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

pub(crate) fn fill_bg(frame: &mut Frame, area: Rect, style: Style) {
    frame.render_widget(Block::default().style(style), area);
}

pub(crate) fn density_blank_lines(d: crate::app::Density) -> usize {
    match d {
        crate::app::Density::Compact => 0,
        crate::app::Density::Comfortable => 1,
        crate::app::Density::Cozy => 2,
    }
}

/// Compute the new vertical scroll offset for a paragraph-backed list so the
/// cursor row stays inside the viewport. `prev` is the previous frame's offset,
/// `cursor_line` is the line index of the cursor (or `None` if there's no
/// cursor row in the current build, e.g. when the list is empty). `height` is
/// the viewport height in rows; `total` is the total line count.
pub(crate) fn keep_cursor_visible(
    prev: u16,
    cursor_line: Option<usize>,
    height: u16,
    total: usize,
) -> u16 {
    let h = usize::from(height);
    if h == 0 || total == 0 {
        return 0;
    }
    let max_offset = total.saturating_sub(h);
    let prev = usize::from(prev).min(max_offset);
    let new = match cursor_line {
        Some(cl) if cl < prev => cl,
        Some(cl) if cl >= prev + h => cl + 1 - h,
        _ => prev,
    };
    new.min(max_offset).min(usize::from(u16::MAX)) as u16
}

#[cfg(test)]
mod tests {
    use super::keep_cursor_visible;

    #[test]
    fn no_scroll_when_content_fits() {
        assert_eq!(keep_cursor_visible(0, Some(5), 10, 8), 0);
        assert_eq!(keep_cursor_visible(0, Some(7), 10, 8), 0);
    }

    #[test]
    fn scrolls_down_when_cursor_below_viewport() {
        // viewport rows 0..5, cursor at line 7 -> offset = 7 - 5 + 1 = 3
        assert_eq!(keep_cursor_visible(0, Some(7), 5, 20), 3);
    }

    #[test]
    fn scrolls_up_when_cursor_above_viewport() {
        // prev offset 10, cursor at line 3 -> offset = 3
        assert_eq!(keep_cursor_visible(10, Some(3), 5, 20), 3);
    }

    #[test]
    fn keeps_previous_offset_when_cursor_in_viewport() {
        // prev 5, cursor at line 7, height 5 -> 7 in [5, 10), stays 5
        assert_eq!(keep_cursor_visible(5, Some(7), 5, 20), 5);
    }

    #[test]
    fn clamps_to_max_offset_when_previous_exceeds_it() {
        // total shrank since last frame; previous offset 50 is now too large.
        assert_eq!(keep_cursor_visible(50, None, 5, 8), 3);
    }

    #[test]
    fn handles_degenerate_inputs() {
        assert_eq!(keep_cursor_visible(0, None, 0, 100), 0);
        assert_eq!(keep_cursor_visible(0, Some(0), 5, 0), 0);
    }
}
