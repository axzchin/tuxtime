//! Timesheet view key handler. Route keys inside the timesheet view to
//! date navigation, narrative-level movement, edit, archive, billable
//! toggle, and clipboard operations.

use crate::app::{App, Mode, Picker, Screen, TimesheetTaskRef, View, format_billable};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[allow(clippy::too_many_lines)]
pub(crate) fn handle_timesheet_keys(app: &mut App, key: KeyEvent) {
    // Ctrl-d / Ctrl-u scroll the detail sidebar's narrative. Checked before the
    // plain-key match so the CONTROL modifier distinguishes them from `d`
    // (daily view) — `u` is otherwise unbound in this view.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('d') => {
                scroll_detail(app, 5);
                return;
            }
            KeyCode::Char('u') => {
                scroll_detail(app, -5);
                return;
            }
            _ => {}
        }
    }
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
        // Show only non-billable/DNB entries. This is deliberately separate
        // from `b`, which changes an active task's billing metadata.
        KeyCode::Char('n') => {
            app.timesheet.dnb_only = !app.timesheet.dnb_only;
            app.timesheet.cursor = 0;
            app.timesheet.invalidate_cache();
            app.flash(if app.timesheet.dnb_only {
                "filter: DNB only"
            } else {
                "filter: all billing"
            });
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
        // Group-level navigation: J/K jump between groups instead of walking
        // narrative by narrative. J lands on the first narrative of the next
        // group; K on the last narrative of the previous group (mirroring
        // vim's }/{ paragraph motion over groups).
        KeyCode::Char('J') => app.timesheet_group_step(1),
        KeyCode::Char('K') => app.timesheet_group_step(-1),
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
                let joined = format_narratives_for_copy(&entry.narratives);
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
                let joined = format_narratives_for_copy(&entry.narratives);
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
                        // Same narrative-edge cursor as `e` in the list view.
                        app.draft_set_edit_pref(raw, false);
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

/// Adjust the detail sidebar's scroll offset by `delta` rows (positive scrolls
/// down). The offset is keyed to the current timesheet cursor so the renderer
/// resets it when the highlighted narrative changes; the renderer also clamps
/// it to the visible body, so the stored value never drifts past the fold.
fn scroll_detail(app: &mut App, delta: i32) {
    let (cursor, scroll) = app.nav.detail_scroll.get();
    let next = if delta >= 0 {
        scroll.saturating_add(delta as u16)
    } else {
        scroll.saturating_sub((-delta) as u16)
    };
    app.nav.detail_scroll.set((cursor, next));
}

/// Ensure a joined narrative list ends with a full stop, so a copied group
/// reads as a finished sentence. Already-terminated (or empty) text is left
/// untouched.
fn ensure_full_stop(joined: &str) -> String {
    if joined.is_empty() || joined.ends_with('.') {
        joined.to_string()
    } else {
        format!("{joined}.")
    }
}

/// Collapse runs of whitespace to single spaces (and trim the edges), so a
/// hand-typed narrative with stray double spaces doesn't land on an invoice
/// looking ragged.
fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Uppercase the first character of `s` when it's lowercase. Digits,
/// symbols, and already-uppercase starts are left untouched, so "API design"
/// or "2026 budget call" survive intact.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_lowercase() => c.to_uppercase().collect::<String>() + chars.as_str(),
        _ => s.to_string(),
    }
}

/// Invoice-ready narrative text: collapse stray whitespace, capitalize the
/// first letter of each narrative, join with "; ", and end with a full stop
/// so the copied group reads as finished prose.
fn format_narratives_for_copy(narratives: &[String]) -> String {
    let joined = narratives
        .iter()
        .map(|n| capitalize_first(&collapse_spaces(n)))
        .collect::<Vec<_>>()
        .join("; ");
    ensure_full_stop(&joined)
}

#[cfg(test)]
mod tests {
    use super::{capitalize_first, collapse_spaces, ensure_full_stop, format_narratives_for_copy};

    #[test]
    fn appends_full_stop_when_missing() {
        assert_eq!(
            ensure_full_stop("write code; review PR"),
            "write code; review PR."
        );
    }

    #[test]
    fn keeps_existing_full_stop() {
        assert_eq!(ensure_full_stop("write code."), "write code.");
    }

    #[test]
    fn empty_stays_empty() {
        assert_eq!(ensure_full_stop(""), "");
    }

    #[test]
    fn collapse_spaces_squashes_runs_and_trims() {
        assert_eq!(collapse_spaces("  drafted   the  brief  "), "drafted the brief");
        assert_eq!(collapse_spaces("single  spaced\t\ttext"), "single spaced text");
        assert_eq!(collapse_spaces("plain"), "plain");
    }

    #[test]
    fn capitalize_first_uppercases_lowercase_start() {
        assert_eq!(capitalize_first("drafted the brief"), "Drafted the brief");
    }

    #[test]
    fn capitalize_first_leaves_uppercase_digits_symbols_alone() {
        assert_eq!(capitalize_first("API design"), "API design");
        assert_eq!(capitalize_first("2026 budget call"), "2026 budget call");
        assert_eq!(capitalize_first("(follow-up) call"), "(follow-up) call");
    }

    #[test]
    fn copy_formatting_capitalizes_each_narrative_and_collapses_spaces() {
        let narratives = vec![
            "  drafted   the  brief  ".to_string(),
            "reviewed discovery responses".to_string(),
        ];
        assert_eq!(
            format_narratives_for_copy(&narratives),
            "Drafted the brief; Reviewed discovery responses."
        );
    }

    #[test]
    fn copy_formatting_keeps_existing_full_stop() {
        let narratives = vec!["draft motion.".to_string()];
        assert_eq!(format_narratives_for_copy(&narratives), "Draft motion.");
    }
}
