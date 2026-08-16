//! Timesheet view key handler. Route keys inside the timesheet view to
//! date navigation, narrative-level movement, edit, archive, billable
//! toggle, and clipboard operations.

use crate::app::{App, Mode, Picker, Screen, TimesheetTaskRef, View, format_billable};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

#[allow(clippy::too_many_lines)]
pub(crate) fn handle_timesheet_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') => {
            app.set_view(View::List);
        }
        KeyCode::Char('w') => {
            app.timesheet.weekly = true;
            app.timesheet.cursor = 0;
            app.timesheet.invalidate_cache();
        }
        KeyCode::Char('d') => {
            app.timesheet.weekly = false;
            app.timesheet.cursor = 0;
            app.timesheet.invalidate_cache();
        }
        KeyCode::Char('s') => {
            app.timesheet.sort = app.timesheet.sort.next();
            app.timesheet.cursor = 0;
            app.timesheet.invalidate_cache();
            app.flash(format!("sort: {}", app.timesheet.sort.label()));
        }
        // Day navigation
        KeyCode::Char('h') | KeyCode::Left => {
            app.timesheet_shift_days(-1);
            app.flash(app.timesheet_date_display());
        }
        KeyCode::Char('l') | KeyCode::Right => {
            app.timesheet_shift_days(1);
            app.flash(app.timesheet_date_display());
        }
        // Week navigation
        KeyCode::Char('H') => {
            app.timesheet_shift_days(-7);
            app.flash(app.timesheet_date_display());
        }
        KeyCode::Char('L') => {
            app.timesheet_shift_days(7);
            app.flash(app.timesheet_date_display());
        }
        // Jump to today
        KeyCode::Char('t') => {
            app.timesheet_goto_today();
            app.flash(format!("today ({})", app.timesheet_date_display()));
        }
        // Filter the timesheet by project (f) or context (F), reusing the
        // list-view pickers so the filter previews live on the timesheet.
        KeyCode::Char('f') => app.enter_pick_project(),
        KeyCode::Char('F') => app.enter_pick_context(),
        // g — open calendar to jump to a specific date.
        KeyCode::Char('g') => {
            if let Ok(d) = chrono::NaiveDate::parse_from_str(&app.timesheet.date, "%Y-%m-%d") {
                app.timesheet.calendar_focus = d;
            }
            app.timesheet.date_input.clear();
            app.nav.mode = Mode::Picker(Picker::TimesheetDate);
        }
        // Narrative-level navigation: j/k moves between individual narratives.
        KeyCode::Char('j') | KeyCode::Down => {
            let count = app.timesheet_narrative_count();
            if count > 0 {
                app.timesheet.cursor = (app.timesheet.cursor + 1).min(count - 1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.timesheet.cursor = app.timesheet.cursor.saturating_sub(1);
        }
        KeyCode::Char('b') => {
            if let Some((_gi, _ni, task_ref)) = app.timesheet_narrative_at(app.timesheet.cursor) {
                match task_ref {
                    TimesheetTaskRef::Active(abs) => {
                        app.toggle_billable_at(abs);
                        let count = app.timesheet_narrative_count();
                        app.timesheet.cursor = app.timesheet.cursor.min(count.saturating_sub(1));
                    }
                    TimesheetTaskRef::Archived(_) => {
                        app.flash(
                            "cannot toggle billable on archived entry — press a to unarchive",
                        );
                    }
                }
            }
        }
        KeyCode::Char('x') => {
            if let Some((_gi, _ni, TimesheetTaskRef::Active(abs))) =
                app.timesheet_narrative_at(app.timesheet.cursor)
            {
                app.toggle_complete(abs);
            }
        }
        KeyCode::Char('a') => {
            if let Some((_gi, _ni, task_ref)) = app.timesheet_narrative_at(app.timesheet.cursor) {
                match task_ref {
                    TimesheetTaskRef::Active(abs) => {
                        app.archive_one(abs);
                        let count = app.timesheet_narrative_count();
                        app.timesheet.cursor = app.timesheet.cursor.min(count.saturating_sub(1));
                    }
                    TimesheetTaskRef::Archived(abs) => {
                        app.unarchive(abs);
                    }
                }
            }
        }
        KeyCode::Char('c') => {
            let groups = app.build_timesheet_groups();
            if groups.is_empty() {
                app.flash("no entries to copy");
                return;
            }
            let Some((gi, _, _)) = app.timesheet_narrative_at(app.timesheet.cursor) else {
                app.flash("no entries to copy");
                return;
            };
            let entry = &groups[gi];
            if entry.narratives.is_empty() {
                app.flash("no narratives to copy");
            } else {
                let prefix = if entry.billable { "" } else { "DNB - " };
                let joined = entry.narratives.join("; ");
                let payload = format!("{prefix}{joined}");
                let key_label = if entry.billable {
                    entry.key.clone()
                } else {
                    format!("{} (DNB)", entry.key)
                };
                match crate::clipboard::copy(&payload) {
                    Ok(()) => {
                        app.timesheet.copy_flash = Some((gi, std::time::Instant::now()));
                        app.flash(format!("copied narrative for {key_label}"));
                    }
                    Err(e) => app.flash(format!("copy failed: {e}")),
                }
            }
        }
        KeyCode::Char('y') => {
            let groups = app.build_timesheet_groups();
            if groups.is_empty() {
                app.flash("no entries to copy");
                return;
            }
            let Some((gi, _, _)) = app.timesheet_narrative_at(app.timesheet.cursor) else {
                app.flash("no entries to copy");
                return;
            };
            let entry = &groups[gi];
            let billable = format_billable(entry.total_secs, app.prefs.rounding_increment);
            let key_label = if entry.billable {
                entry.key.clone()
            } else {
                format!("{} (DNB)", entry.key)
            };
            match crate::clipboard::copy(&billable) {
                Ok(()) => {
                    app.timesheet.copy_flash = Some((gi, std::time::Instant::now()));
                    app.flash(format!("copied {billable} for {key_label}"));
                }
                Err(e) => app.flash(format!("copy failed: {e}")),
            }
        }
        KeyCode::Char('C') => {
            let groups = app.build_timesheet_groups();
            if groups.is_empty() {
                app.flash("no entries to copy");
                return;
            }
            let Some((gi, _, _)) = app.timesheet_narrative_at(app.timesheet.cursor) else {
                app.flash("no entries to copy");
                return;
            };
            let entry = &groups[gi];
            if entry.narratives.is_empty() {
                app.flash("no narratives to copy");
            } else {
                let prefix = if entry.billable { "" } else { "DNB - " };
                let joined = entry.narratives.join("; ");
                let billable = format_billable(entry.total_secs, app.prefs.rounding_increment);
                let payload = format!("{prefix}{joined} ({billable})");
                let key_label = if entry.billable {
                    entry.key.clone()
                } else {
                    format!("{} (DNB)", entry.key)
                };
                match crate::clipboard::copy(&payload) {
                    Ok(()) => {
                        app.timesheet.copy_flash = Some((gi, std::time::Instant::now()));
                        app.flash(format!("copied {billable} for {key_label}"));
                    }
                    Err(e) => app.flash(format!("copy failed: {e}")),
                }
            }
        }
        KeyCode::Enter => {
            let Some((_gi, _ni, task_ref)) = app.timesheet_narrative_at(app.timesheet.cursor)
            else {
                return;
            };
            match task_ref {
                TimesheetTaskRef::Active(abs) => {
                    if let Some(raw) = app.task_raw(abs) {
                        app.selection.enter_edit(abs);
                        app.draft_set(raw);
                        app.session.manual_time_entry = false;
                        app.nav.mode = Mode::Screen(Screen::Insert);
                    }
                }
                TimesheetTaskRef::Archived(_) => {
                    app.flash("cannot edit archived entry — press a to unarchive");
                }
            }
        }
        _ => {}
    }
}
