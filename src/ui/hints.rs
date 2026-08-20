//! Status-bar hint text for every mode. Kept apart from the renderer so the
//! keybinding copy lives in one place instead of inline in `status::render`.
//! Most modes are a static `&str`; the few that depend on app state (manual
//! entry grammar, project counts, nudge-picker action) build a `String`.

use crate::app::{
    App, DialogInputMode, Mode, Nudge, NudgePickAction, Picker, Prompt, Screen, View,
};

/// The hint shown in the status bar's middle segment for the current mode.
pub(crate) fn mode_hint(app: &App) -> String {
    // The hint shown for modes without a dedicated line: the timesheet's long
    // key list when the timesheet view is active, the ordinary list hint
    // otherwise. Several modes (Normal, day-boundary, theme picker, stale
    // timer, manual-entry choice) deliberately fall back to this.
    let default_hint: &'static str = if matches!(app.nav.view, View::Timesheet) {
        "j/k navigate  ·  Enter edit  ·  b billable  ·  n DNB filter  ·  a archive toggle  ·  c copy text  ·  y copy time  ·  C copy both  ·  h/l ±day  ·  H/L ±week  ·  w/d view  ·  s sort  ·  f/F filter project/context  ·  / search  ·  g date  ·  t today  ·  Esc/V/q back"
    } else {
        "j/k · n new · t timer · T interrupt · x done · / search · ? help · u undo · q quit"
    };

    match app.nav.mode {
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
                "Esc/ ,/ q dismiss  ·  i idle nudge  ·  l long timer nudge  ·  I duration badge  ·  O log date".to_string()
            }
            Screen::CommandPalette => "type to filter · Enter run · Esc cancel".to_string(),
            Screen::Share => "scan the QR · any key dismisses".to_string(),
            Screen::Welcome => "c create ./todo.txt · s open sample · q quit".to_string(),
            Screen::ManageProjects => manage_projects_hint(app),
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
            Picker::NudgeTask => nudge_task_hint(app),
            Picker::Theme => default_hint.to_string(),
        },
        Mode::Nudge(nudge) => match nudge {
            Nudge::Idle => "S start timer · M add time · N new entry · D dismiss".to_string(),
            Nudge::LongTimer => "S stop timer · D dismiss".to_string(),
            Nudge::StaleTimer => default_hint.to_string(),
            Nudge::Review => "V view timesheet · M add time · S skip".to_string(),
            Nudge::ManualEntryChoice => default_hint.to_string(),
        },
    }
}

fn manage_projects_hint(app: &App) -> String {
    let all = app.all_projects();
    let archived_count = all.iter().filter(|n| app.is_project_archived(n)).count();
    let total = all.len();
    let needle = app.filter().search.to_lowercase();
    let matched = app.filtered_projects().len();
    let sort = app.project_manager.project_sort.label();
    let base = format!("j/k nav · x archive · r rename · s sort ({sort}) · / search · Esc/P back");
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

fn nudge_task_hint(app: &App) -> String {
    let commit = match app.session.nudge_picker.as_ref().map(|p| p.action) {
        Some(NudgePickAction::StartTimer) => "Enter start timer on highlighted",
        Some(NudgePickAction::AddTime) => "Enter add time on highlighted",
        None => "Enter select",
    };
    format!("{commit} · j/k navigate · / search · +/@ filter · t start · Esc back")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn hint_app() -> App {
        let path = std::env::temp_dir().join(format!(
            "tuxtime-hints-{}-{:?}.txt",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_file(&path);
        App::new(path, String::new(), "2026-05-06".into(), Config::default())
    }

    /// Every mode must produce a non-empty hint — a blank status-bar middle
    /// segment is the "you're on your own" failure mode for discoverability.
    #[test]
    fn every_mode_has_a_nonempty_hint() {
        let mut app = hint_app();
        let screens = [
            Screen::Normal,
            Screen::Insert,
            Screen::Search,
            Screen::Visual,
            Screen::Help,
            Screen::Settings,
            Screen::CommandPalette,
            Screen::Share,
            Screen::Welcome,
            Screen::ManageProjects,
        ];
        let prompts = [
            Prompt::Project,
            Prompt::Context,
            Prompt::SaveFilter,
            Prompt::AddTime,
            Prompt::IdleNudge,
            Prompt::LongTimerNudge,
            Prompt::RenameProject,
            Prompt::DayBoundary,
        ];
        let pickers = [
            Picker::Project,
            Picker::Context,
            Picker::SavedFilter,
            Picker::Theme,
            Picker::TimesheetDate,
            Picker::NudgeTask,
        ];
        let nudges = [
            Nudge::Idle,
            Nudge::LongTimer,
            Nudge::StaleTimer,
            Nudge::Review,
            Nudge::ManualEntryChoice,
        ];

        let modes = screens
            .into_iter()
            .map(Mode::Screen)
            .chain(prompts.into_iter().map(Mode::Prompt))
            .chain(pickers.into_iter().map(Mode::Picker))
            .chain(nudges.into_iter().map(Mode::Nudge));

        for mode in modes {
            app.nav.mode = mode;
            let hint = mode_hint(&app);
            assert!(
                !hint.trim().is_empty(),
                "{mode:?} must have a non-empty hint"
            );
        }
    }
}
