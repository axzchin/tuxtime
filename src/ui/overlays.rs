//! Overlay drawing: every `Mode` that floats a panel above the base view.
//!
//! The `match` below IS the z-order — later draws land on top of earlier
//! ones, and the arm order is the contract. From bottom to top:
//!
//! 1. **Body-replacing overlays** (`Settings`, `ManageProjects`) clear the
//!    whole body area and draw full-pane content. Rendered first so
//!    everything below them in this list stacks on top.
//! 2. **Standalone boxes** — the add/edit dialog, help, welcome, command
//!    palette, theme picker, share, message boxes, timesheet calendar. Each
//!    clears its own rect and renders; no other overlay is visible beside
//!    them (except the anchors in 3).
//! 3. **Anchored popups** — slash menu, calendar, recurrence builder,
//!    priority chooser, duration picker, autocomplete. They render *after*
//!    the dialog they hang off, so they always sit on top of it. At most one
//!    shows at a time (`render_overlay` suppresses autocomplete while a
//!    metadata picker is open).
//!
//! Two modes deliberately stack a second panel: the nudge prompts draw
//! `Settings` underneath the prompt box, and the rename-project prompt draws
//! the project manager underneath — the underlying panel is cleared + drawn
//! first, then the prompt floats on top. That ordering (panel → prompt) is
//! the one place a box appears above another box in the same frame.
//!
//! Sizing and rect math live in [`overlay`](crate::ui::overlay); every box
//! here asks it for its footprint and renders into that rect.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::{App, Mode};
use crate::ui::timesheet_render::render_timesheet_calendar;
use crate::ui::{
    command_palette, dialog, help, msgbox, nudge_picker, overlay, settings, share, theme_picker,
    welcome,
};

/// Draw whatever the current mode wants on top of the base view (panes,
/// status bar). `area` is the full frame, `body_area` the centre + sidebars,
/// `center_width` the centre pane's width (the add dialog scales to it).
pub(crate) fn draw_overlays(
    frame: &mut Frame,
    area: Rect,
    body_area: Rect,
    center_width: u16,
    app: &App,
) {
    match app.nav.mode {
        Mode::Insert => {
            let dlg = overlay::insert_dialog_rect(area, center_width);
            frame.render_widget(Clear, dlg);
            dialog::render(frame, dlg, app);
            if !dialog::render_overlay(frame, dlg, area, app) {
                dialog::render_autocomplete(frame, dlg, area, app);
            }
        }
        Mode::Help => {
            let r = overlay::help_rect(area);
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
            let r = overlay::prompt_rect(area);
            frame.render_widget(Clear, r);
            dialog::render_prompt(frame, r, app);
        }
        // Rename-project prompt stacks on top of the project management
        // view so the user sees the project list behind the edit dialog.
        Mode::PromptRenameProject => {
            frame.render_widget(Clear, body_area);
            render_manage_projects(frame, body_area, app);
            let r = overlay::prompt_rect(area);
            frame.render_widget(Clear, r);
            dialog::render_prompt(frame, r, app);
        }
        Mode::PromptProject
        | Mode::PromptContext
        | Mode::PromptSaveFilter
        | Mode::PromptAddTime => {
            let r = overlay::prompt_rect(area);
            frame.render_widget(Clear, r);
            dialog::render_prompt(frame, r, app);
            if matches!(app.nav.mode, Mode::PromptProject | Mode::PromptContext) {
                dialog::render_autocomplete(frame, r, area, app);
            }
        }
        Mode::CommandPalette => {
            let r = overlay::palette_rect(area);
            frame.render_widget(Clear, r);
            command_palette::render(frame, r, app);
        }
        Mode::Share => {
            let (w, h) = share::size_for(app);
            let r = overlay::centered_in(area, w, h);
            frame.render_widget(Clear, r);
            share::render(frame, r, app);
        }
        Mode::PickTheme => {
            let r = overlay::palette_rect(area);
            frame.render_widget(Clear, r);
            theme_picker::render(frame, r, app);
        }
        Mode::Welcome => {
            let r = overlay::welcome_rect(area);
            frame.render_widget(Clear, r);
            welcome::render(frame, r, app);
        }
        Mode::PickTimesheetDate => {
            render_timesheet_calendar(frame, area, app);
        }
        Mode::IdleNudge => {
            let r = overlay::message_rect(area);
            frame.render_widget(Clear, r);
            let theme = app.theme();
            // The message speaks to the actual failure mode: a relaunch after
            // hours with nothing tracked (UntrackedDay) is a different prompt
            // than the ordinary "you stopped the timer and didn't start one".
            let message = match app.session.idle_reason {
                crate::app::IdleReason::UntrackedDay => {
                    "Nothing tracked yet today — working without a timer?"
                }
                crate::app::IdleReason::TimerStopped => "No timer running!",
            };
            msgbox::render_message_box(
                frame,
                r,
                theme,
                " ⏰ Idle Nudge ",
                message,
                "[S]tart timer  [M] add time  [N] new entry  [D]ismiss",
            );
        }
        Mode::PickNudgeTask => {
            nudge_picker::render(frame, area, app);
        }
        Mode::ReviewNudge => {
            let r = overlay::message_rect(area);
            frame.render_widget(Clear, r);
            let theme = app.theme();
            let tracked = crate::app::format_duration(
                app.today_tracked_secs(),
                app.prefs.rounding_increment,
            );
            let message = format!("You've tracked {tracked} today — anything missing?");
            msgbox::render_message_box(
                frame,
                r,
                theme,
                " ⏰ End-of-Day Review ",
                &message,
                "[V]iew timesheet  [M] add time  [s]kip for today",
            );
        }
        Mode::LongTimerNudge => {
            let r = overlay::message_rect(area);
            frame.render_widget(Clear, r);
            let theme = app.theme();
            msgbox::render_message_box(
                frame,
                r,
                theme,
                " ⏰ Long Timer Nudge ",
                "Timer has been running for a while — still tracking?",
                "[S]top timer  [D]ismiss",
            );
        }
        Mode::StaleTimer => {
            let r = overlay::message_rect(area);
            frame.render_widget(Clear, r);
            let theme = app.theme();
            let started = app
                .active_timer_task()
                .and_then(|t| t.start.as_deref().map(str::to_string))
                .unwrap_or_default();
            let elapsed = app.timer_elapsed_secs().unwrap_or(0);
            let elapsed_str = crate::app::format_duration(elapsed, app.prefs.rounding_increment);
            let message = format!(
                "Timer was running when tuxtime last closed — {elapsed_str} since {started}. \
                 Log it, keep counting, or discard the gap?"
            );
            msgbox::render_message_box(
                frame,
                r,
                theme,
                " ⏰ Stale Timer ",
                &message,
                "[K]eep counting  [S]top & log  [D]iscard gap",
            );
        }
        Mode::ManualEntryChoice => {
            let r = overlay::message_rect(area);
            frame.render_widget(Clear, r);
            let theme = app.theme();
            msgbox::render_message_box(
                frame,
                r,
                theme,
                " ✏ Manual Time Entry ",
                "How would you like to describe this entry?",
                "[N]ew blank entry  [A]dd to current task  [Esc] cancel",
            );
        }
        // Day-boundary prompt — starting a timer (or adding time) on a task
        // whose accumulated time belongs to a previous day (one line per
        // task-day). The task's narrative is shown so the user knows what
        // they're carrying forward. The narrative is word-wrapped to the box
        // width and the box grows to fit, so a long task name is never
        // clipped at the dialog edge.
        Mode::PromptDayBoundary => {
            let theme = app.theme();
            let narrative = app
                .session
                .pending_day_boundary
                .as_ref()
                .and_then(|(abs, _)| app.store.tasks().get(*abs))
                .map(|t| crate::todo::body_only_from_clean(&t.clean_raw))
                .unwrap_or_default();
            let message = format!("\"{narrative}\" has time from a previous day.");
            // Wrap to the box width and grow the box to fit; short messages
            // keep the original 7-row box so the layout doesn't jump around.
            let wrap_w = overlay::day_boundary_wrap_w(area);
            let wrapped = msgbox::wrapped_line_count(&message, wrap_w) as u16;
            let r = overlay::day_boundary_rect(area, wrapped);
            frame.render_widget(Clear, r);
            msgbox::render_message_box(
                frame,
                r,
                theme,
                " 📅 Day Boundary ",
                &message,
                "[C]ontinue same entry  [N]ew entry for today  [Esc] cancel",
            );
        }
        Mode::ManageProjects => {
            frame.render_widget(Clear, body_area);
            render_manage_projects(frame, body_area, app);
        }
        _ => {}
    }
}

fn render_manage_projects(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let sort_label = Span::styled(
        format!(" {} ", app.project_manager.project_sort.label()),
        Style::default().fg(theme.dim).bg(theme.panel),
    );
    let title = Line::from(vec![Span::raw(" Project Management "), sort_label]);
    let inner = msgbox::frame_box(frame, area, theme.border, theme.panel, title);

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
