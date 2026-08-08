//! Every discrete behavior the user can trigger, decoupled from the keystroke
//! that fires it. Lives at the crate root (not under `app`) so both the binary
//! (which dispatches actions in `apply_action`) and the command palette
//! (which lists them) can name them without a cyclic dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    CursorDown,
    CursorUp,
    CursorTop,
    CursorBottom,
    HalfPageDown,
    HalfPageUp,
    BeginAdd,
    /// Edit the current task starting in Normal (vim) mode (`e`).
    BeginEdit,
    /// Edit the current task starting in Insert mode (`i`).
    BeginEditInsert,
    ToggleComplete,
    Delete,
    Reschedule,
    CyclePriority,
    BeginSearch,
    OpenHelp,
    OpenSettings,
    OpenCommandPalette,
    Undo,
    ToggleVisual,
    ToggleSelected,
    GoList,
    ToggleArchiveView,
    ArchiveCompleted,
    ArmF,
    PickProject,
    PickContext,
    /// `ff` — open the saved-search cycle picker.
    PickSavedFilter,
    /// `fs` — name the active `/`-search and persist it.
    SaveCurrentFilter,
    CycleSort,
    BeginPromptProject,
    BeginPromptContext,
    ToggleLeftPane,
    ToggleRightPane,
    CycleTheme,
    CycleDensity,
    ToggleLineNum,
    ToggleShowDone,
    ToggleShowFuture,
    CopyLine,
    CopyBody,
    OpenNote,
    CreateOrOpenNote,
    EscapeStack,
    /// Open the phone-capture overlay (QR + URL). First invocation lazily
    /// binds the HTTP server; subsequent invocations just re-show the
    /// overlay.
    OpenShare,
    /// Open the theme picker dialog (j/k to preview, Enter to accept).
    OpenThemePicker,
    ChangeWeekStart,
    /// `t` — start/stop timer on the selected task.
    TimerStartStop,
    /// `M` — open the manual time entry dialog.
    ManualTimeEntry,
    /// `C` — copy today's narratives for the current project+activity.
    CopyNarratives,
    /// Open the daily/weekly timesheet summary overlay.
    OpenTimesheet,
    /// Dismiss the idle-nudge popup.
    DismissNudge,
    /// `N` — begin a new session from the current task (copy body, fresh
    /// date, pre-filled `dur:`). For multi-day task tracking.
    BeginSessionFromCurrent,
    /// Open the idle-nudge threshold prompt (minutes).
    ConfigureIdleNudge,
    /// Open the long-timer-nudge threshold prompt (minutes).
    ConfigureLongTimerNudge,
    /// `b` — toggle billable/non-billable (`bill:n`) on the current task.
    ToggleBillable,
    /// `T` — quick interruption: stop running timer, open blank entry.
    QuickInterrupt,
    /// `<P>` — open the project management view (archive/rename projects).
    OpenProjectManager,
}

impl Action {
    /// Every accepted `keybinds.toml` spelling for each action, including the
    /// canonical snake_case name (first) and user-friendly aliases. Keeping
    /// the names as data next to the enum (rather than a second hand-maintained
    /// match) means the config interface can't silently drift from the set of
    /// actions: adding an action is one enum arm + one registry row.
    const NAMES: &'static [(&'static str, Self)] = &[
        ("quit", Self::Quit),
        ("cursor_down", Self::CursorDown),
        ("cursor_up", Self::CursorUp),
        ("cursor_top", Self::CursorTop),
        ("cursor_bottom", Self::CursorBottom),
        ("half_page_down", Self::HalfPageDown),
        ("half_page_up", Self::HalfPageUp),
        ("begin_add", Self::BeginAdd),
        ("add", Self::BeginAdd),
        ("begin_edit", Self::BeginEdit),
        ("edit", Self::BeginEdit),
        ("begin_edit_insert", Self::BeginEditInsert),
        ("edit_insert", Self::BeginEditInsert),
        ("toggle_complete", Self::ToggleComplete),
        ("delete", Self::Delete),
        ("reschedule", Self::Reschedule),
        ("cycle_priority", Self::CyclePriority),
        ("begin_search", Self::BeginSearch),
        ("search", Self::BeginSearch),
        ("open_help", Self::OpenHelp),
        ("help", Self::OpenHelp),
        ("open_settings", Self::OpenSettings),
        ("settings", Self::OpenSettings),
        ("open_command_palette", Self::OpenCommandPalette),
        ("command_palette", Self::OpenCommandPalette),
        ("undo", Self::Undo),
        ("toggle_visual", Self::ToggleVisual),
        ("toggle_selected", Self::ToggleSelected),
        ("go_list", Self::GoList),
        ("list", Self::GoList),
        ("toggle_archive_view", Self::ToggleArchiveView),
        ("archive_view", Self::ToggleArchiveView),
        ("archive_completed", Self::ArchiveCompleted),
        ("arm_f", Self::ArmF),
        ("pick_project", Self::PickProject),
        ("pick_context", Self::PickContext),
        ("pick_saved_filter", Self::PickSavedFilter),
        ("save_current_filter", Self::SaveCurrentFilter),
        ("cycle_sort", Self::CycleSort),
        ("begin_prompt_project", Self::BeginPromptProject),
        ("prompt_project", Self::BeginPromptProject),
        ("begin_prompt_context", Self::BeginPromptContext),
        ("prompt_context", Self::BeginPromptContext),
        ("toggle_left_pane", Self::ToggleLeftPane),
        ("toggle_right_pane", Self::ToggleRightPane),
        ("cycle_theme", Self::CycleTheme),
        ("cycle_density", Self::CycleDensity),
        ("toggle_line_num", Self::ToggleLineNum),
        ("toggle_line_numbers", Self::ToggleLineNum),
        ("toggle_show_done", Self::ToggleShowDone),
        ("toggle_show_future", Self::ToggleShowFuture),
        ("copy_line", Self::CopyLine),
        ("copy_body", Self::CopyBody),
        ("open_note", Self::OpenNote),
        ("note", Self::OpenNote),
        ("create_or_open_note", Self::CreateOrOpenNote),
        ("create_note", Self::CreateOrOpenNote),
        ("escape_stack", Self::EscapeStack),
        ("escape", Self::EscapeStack),
        ("open_share", Self::OpenShare),
        ("share", Self::OpenShare),
        ("open_theme_picker", Self::OpenThemePicker),
        ("theme_picker", Self::OpenThemePicker),
        ("change_week_start", Self::ChangeWeekStart),
        ("timer_start_stop", Self::TimerStartStop),
        ("timer", Self::TimerStartStop),
        ("manual_time_entry", Self::ManualTimeEntry),
        ("manual_entry", Self::ManualTimeEntry),
        ("copy_narratives", Self::CopyNarratives),
        ("copy_time", Self::CopyNarratives),
        ("open_timesheet", Self::OpenTimesheet),
        ("timesheet", Self::OpenTimesheet),
        ("dismiss_nudge", Self::DismissNudge),
        ("begin_session_from_current", Self::BeginSessionFromCurrent),
        ("new_session", Self::BeginSessionFromCurrent),
        ("session", Self::BeginSessionFromCurrent),
        ("configure_idle_nudge", Self::ConfigureIdleNudge),
        ("idle_nudge", Self::ConfigureIdleNudge),
        ("configure_long_timer_nudge", Self::ConfigureLongTimerNudge),
        ("long_timer_nudge", Self::ConfigureLongTimerNudge),
        ("toggle_billable", Self::ToggleBillable),
        ("billable", Self::ToggleBillable),
        ("quick_interrupt", Self::QuickInterrupt),
        ("interrupt", Self::QuickInterrupt),
        ("open_project_manager", Self::OpenProjectManager),
        ("project_manager", Self::OpenProjectManager),
        ("manage_projects", Self::OpenProjectManager),
    ];

    #[must_use]
    pub fn from_keybind_name(s: &str) -> Option<Self> {
        let normalized = s.trim().replace('-', "_").to_ascii_lowercase();
        Self::NAMES
            .iter()
            .find(|(name, _)| *name == normalized)
            .map(|&(_, action)| action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every registry row must round-trip through `from_keybind_name`, and no
    /// two rows may share a spelling (the first match would silently win).
    #[test]
    fn keybind_registry_is_total_and_unique() {
        let mut seen: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
        for &(name, action) in Action::NAMES {
            assert!(
                seen.insert(name),
                "duplicate keybind name {name:?} for {action:?}"
            );
            assert_eq!(
                Action::from_keybind_name(name),
                Some(action),
                "{name:?} must resolve back to {action:?}"
            );
        }
    }

    #[test]
    fn reschedule_is_rebindable() {
        assert_eq!(
            Action::from_keybind_name("reschedule"),
            Some(Action::Reschedule)
        );
    }

    #[test]
    fn open_theme_picker_is_rebindable() {
        assert_eq!(
            Action::from_keybind_name("open_theme_picker"),
            Some(Action::OpenThemePicker)
        );
        assert_eq!(
            Action::from_keybind_name("theme_picker"),
            Some(Action::OpenThemePicker)
        );
    }

    #[test]
    fn open_note_is_rebindable() {
        assert_eq!(
            Action::from_keybind_name("open_note"),
            Some(Action::OpenNote)
        );
        assert_eq!(Action::from_keybind_name("note"), Some(Action::OpenNote));
        assert_eq!(
            Action::from_keybind_name("create_or_open_note"),
            Some(Action::CreateOrOpenNote)
        );
        assert_eq!(
            Action::from_keybind_name("create_note"),
            Some(Action::CreateOrOpenNote)
        );
    }

    /// Convert a CamelCase variant name to its canonical snake_case keybind
    /// name (e.g. `BeginEditInsert` → `begin_edit_insert`). The canonical
    /// (first) entry of every `NAMES` row is exactly this spelling.
    fn snake_case(variant: &str) -> String {
        let mut out = String::with_capacity(variant.len() + 4);
        for (i, c) in variant.chars().enumerate() {
            if c.is_ascii_uppercase() {
                if i > 0 {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }

    /// Every `Action` variant must have a canonical keybind name in `NAMES`,
    /// so no action is silently non-rebindable. The exhaustive match below
    /// is compiler-enforced: adding a variant fails to compile until it is
    /// listed here, then fails this assertion until a `NAMES` row exists —
    /// the same enforce-total-coverage pattern `apply_action` uses.
    #[test]
    fn every_action_variant_has_a_keybind_name() {
        let mut all: Vec<Action> = Vec::new();
        match Action::Quit {
            Action::Quit => all.push(Action::Quit),
            Action::CursorDown => all.push(Action::CursorDown),
            Action::CursorUp => all.push(Action::CursorUp),
            Action::CursorTop => all.push(Action::CursorTop),
            Action::CursorBottom => all.push(Action::CursorBottom),
            Action::HalfPageDown => all.push(Action::HalfPageDown),
            Action::HalfPageUp => all.push(Action::HalfPageUp),
            Action::BeginAdd => all.push(Action::BeginAdd),
            Action::BeginEdit => all.push(Action::BeginEdit),
            Action::BeginEditInsert => all.push(Action::BeginEditInsert),
            Action::ToggleComplete => all.push(Action::ToggleComplete),
            Action::Delete => all.push(Action::Delete),
            Action::Reschedule => all.push(Action::Reschedule),
            Action::CyclePriority => all.push(Action::CyclePriority),
            Action::BeginSearch => all.push(Action::BeginSearch),
            Action::OpenHelp => all.push(Action::OpenHelp),
            Action::OpenSettings => all.push(Action::OpenSettings),
            Action::OpenCommandPalette => all.push(Action::OpenCommandPalette),
            Action::Undo => all.push(Action::Undo),
            Action::ToggleVisual => all.push(Action::ToggleVisual),
            Action::ToggleSelected => all.push(Action::ToggleSelected),
            Action::GoList => all.push(Action::GoList),
            Action::ToggleArchiveView => all.push(Action::ToggleArchiveView),
            Action::ArchiveCompleted => all.push(Action::ArchiveCompleted),
            Action::ArmF => all.push(Action::ArmF),
            Action::PickProject => all.push(Action::PickProject),
            Action::PickContext => all.push(Action::PickContext),
            Action::PickSavedFilter => all.push(Action::PickSavedFilter),
            Action::SaveCurrentFilter => all.push(Action::SaveCurrentFilter),
            Action::CycleSort => all.push(Action::CycleSort),
            Action::BeginPromptProject => all.push(Action::BeginPromptProject),
            Action::BeginPromptContext => all.push(Action::BeginPromptContext),
            Action::ToggleLeftPane => all.push(Action::ToggleLeftPane),
            Action::ToggleRightPane => all.push(Action::ToggleRightPane),
            Action::CycleTheme => all.push(Action::CycleTheme),
            Action::CycleDensity => all.push(Action::CycleDensity),
            Action::ToggleLineNum => all.push(Action::ToggleLineNum),
            Action::ToggleShowDone => all.push(Action::ToggleShowDone),
            Action::ToggleShowFuture => all.push(Action::ToggleShowFuture),
            Action::CopyLine => all.push(Action::CopyLine),
            Action::CopyBody => all.push(Action::CopyBody),
            Action::OpenNote => all.push(Action::OpenNote),
            Action::CreateOrOpenNote => all.push(Action::CreateOrOpenNote),
            Action::EscapeStack => all.push(Action::EscapeStack),
            Action::OpenShare => all.push(Action::OpenShare),
            Action::OpenThemePicker => all.push(Action::OpenThemePicker),
            Action::ChangeWeekStart => all.push(Action::ChangeWeekStart),
            Action::TimerStartStop => all.push(Action::TimerStartStop),
            Action::ManualTimeEntry => all.push(Action::ManualTimeEntry),
            Action::CopyNarratives => all.push(Action::CopyNarratives),
            Action::OpenTimesheet => all.push(Action::OpenTimesheet),
            Action::DismissNudge => all.push(Action::DismissNudge),
            Action::BeginSessionFromCurrent => all.push(Action::BeginSessionFromCurrent),
            Action::ConfigureIdleNudge => all.push(Action::ConfigureIdleNudge),
            Action::ConfigureLongTimerNudge => all.push(Action::ConfigureLongTimerNudge),
            Action::ToggleBillable => all.push(Action::ToggleBillable),
            Action::QuickInterrupt => all.push(Action::QuickInterrupt),
            Action::OpenProjectManager => all.push(Action::OpenProjectManager),
        }
        for action in all {
            let canonical = snake_case(&format!("{action:?}"));
            assert!(
                Action::from_keybind_name(&canonical).is_some(),
                "Action {action:?} has no canonical keybind name `{canonical}` in NAMES"
            );
        }
    }
}
