//! Key resolution: map a single keystroke to an [`Action`] via built-in
//! defaults, user-configured keybinds, and chord state.
//!
//! [`resolve_normal_key`] is the main entry point — it tries keybinds first,
//! then falls back to the built-in map. [`resolve_builtin_single_key`] is
//! used when a chord expires so the leader key fires its single-key behavior.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tuxtime::action::Action;
use tuxtime::app::App;
use tuxtime::keybinds::{KeyBindings, ResolvedKey};

/// Resolve a single character key to its built-in action, ignoring keybinds
/// and chords. Used when a two-key chord expires — the leader key should still
/// trigger its built-in single-key behavior.
pub(crate) fn resolve_builtin_single_key(key: KeyEvent) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl {
        return match key.code {
            KeyCode::Char('d') => Some(Action::HalfPageDown),
            KeyCode::Char('u') => Some(Action::HalfPageUp),
            KeyCode::Char('p') => Some(Action::OpenCommandPalette),
            _ => None,
        };
    }
    Some(match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('j') | KeyCode::Down => Action::CursorDown,
        KeyCode::Char('k') | KeyCode::Up => Action::CursorUp,
        KeyCode::Char('G') => Action::CursorBottom,
        KeyCode::Char('n') => Action::BeginAdd,
        KeyCode::Char('r') => Action::Reschedule,
        KeyCode::Char('a') => Action::ToggleArchiveView,
        KeyCode::Char('l') => Action::GoList,
        KeyCode::Char('e') => Action::BeginEdit,
        KeyCode::Char('i') => Action::BeginEditInsert,
        KeyCode::Char('o') => Action::OpenNote,
        KeyCode::Char('O') => Action::CreateOrOpenNote,
        KeyCode::Char('x') => Action::ToggleComplete,
        KeyCode::Char('b') => Action::ToggleBillable,
        KeyCode::Char('p') => Action::CyclePriority,
        KeyCode::Char('c') => Action::BeginPromptContext,
        KeyCode::Char('/') => Action::BeginSearch,
        KeyCode::Char('?') => Action::OpenHelp,
        KeyCode::Char(',') => Action::OpenSettings,
        KeyCode::Char(':') => Action::OpenCommandPalette,
        KeyCode::Char('u') => Action::Undo,
        KeyCode::Char('v') => Action::ToggleVisual,
        KeyCode::Char(' ') => Action::ToggleSelected,
        KeyCode::Char('A') => Action::ArchiveCompleted,
        KeyCode::Char('f') => Action::ArmF,
        KeyCode::Char('s') => Action::OpenShare,
        KeyCode::Char('S') => Action::CycleSort,
        KeyCode::Char('+') => Action::BeginPromptProject,
        KeyCode::Char('[') => Action::ToggleLeftPane,
        KeyCode::Char(']') => Action::ToggleRightPane,
        KeyCode::Char('T') => Action::QuickInterrupt,
        KeyCode::Char('D') => Action::CycleDensity,
        KeyCode::Char('L') => Action::ToggleLineNum,
        KeyCode::Char('H') => Action::ToggleShowDone,
        KeyCode::Char('F') => Action::ToggleShowFuture,
        KeyCode::Esc => Action::EscapeStack,
        KeyCode::Char('W') => Action::ChangeWeekStart,
        KeyCode::Char('t') => Action::TimerStartStop,
        KeyCode::Char('M') => Action::ManualTimeEntry,
        KeyCode::Char('Z') => Action::OpenThemePicker,
        KeyCode::Char('V') => Action::OpenTimesheet,
        KeyCode::Char('P') => Action::OpenProjectManager,
        _ => return None,
    })
}

/// Map a single keystroke to an [`Action`]. Returns `None` when the keystroke
/// is the *first* press of a chord (e.g. `g` of `gg`) or unknown.
///
/// Tries user-configured keybinds first, then falls back to the built-in
/// mapping. Mutates the chord state because chord progress is part of
/// interpreting the key.
pub(crate) fn resolve_normal_key(app: &mut App, key: KeyEvent, keybinds: &KeyBindings) -> Option<Action> {
    match keybinds.resolve_normal(key, &mut app.chord) {
        Some(ResolvedKey::Action(action)) => return Some(action),
        Some(ResolvedKey::Pending) => return None,
        None => {}
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl {
        return match key.code {
            KeyCode::Char('d') => Some(Action::HalfPageDown),
            KeyCode::Char('u') => Some(Action::HalfPageUp),
            KeyCode::Char('p') => Some(Action::OpenCommandPalette),
            _ => None,
        };
    }
    Some(match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('j') | KeyCode::Down => Action::CursorDown,
        KeyCode::Char('k') | KeyCode::Up => Action::CursorUp,
        KeyCode::Char('G') => Action::CursorBottom,
        // First 'g' arms the chord; second 'g' fires CursorTop.
        KeyCode::Char('g') if app.chord.toggle('g') => Action::CursorTop,
        KeyCode::Char('n') => Action::BeginAdd,
        KeyCode::Char('r') => Action::Reschedule,
        KeyCode::Char('a') => Action::ToggleArchiveView,
        KeyCode::Char('l') => Action::GoList,
        KeyCode::Char('e') => Action::BeginEdit,
        KeyCode::Char('i') => Action::BeginEditInsert,
        KeyCode::Char('o') => Action::OpenNote,
        KeyCode::Char('O') => Action::CreateOrOpenNote,
        KeyCode::Char('x') => Action::ToggleComplete,
        // 'dd' chord. First press arms; second fires.
        KeyCode::Char('d') if app.chord.toggle('d') => Action::Delete,
        // 'yy' chord copies the whole line; 'yb' (after 'y' is armed) copies
        // the body only. Plain 'y' just arms the leader.
        KeyCode::Char('y') if app.chord.toggle('y') => Action::CopyLine,
        KeyCode::Char('b') if app.chord.consume('y') => Action::CopyBody,
        KeyCode::Char('b') => Action::ToggleBillable,
        KeyCode::Char('p') => {
            if app.chord.consume('f') {
                Action::PickProject
            } else {
                Action::CyclePriority
            }
        }
        KeyCode::Char('c') => {
            if app.chord.consume('f') {
                Action::PickContext
            } else {
                Action::BeginPromptContext
            }
        }
        KeyCode::Char('/') => Action::BeginSearch,
        KeyCode::Char('?') => Action::OpenHelp,
        KeyCode::Char(',') => Action::OpenSettings,
        KeyCode::Char(':') => Action::OpenCommandPalette,
        KeyCode::Char('u') => Action::Undo,
        KeyCode::Char('v') => Action::ToggleVisual,
        KeyCode::Char(' ') => Action::ToggleSelected,
        KeyCode::Char('A') => Action::ArchiveCompleted,
        // First 'f' arms the leader; a second 'f' (`ff`) opens the saved-
        // search picker.
        KeyCode::Char('f') => {
            if app.chord.consume('f') {
                Action::PickSavedFilter
            } else {
                Action::ArmF
            }
        }
        KeyCode::Char('s') => {
            if app.chord.consume('f') {
                Action::SaveCurrentFilter
            } else {
                Action::OpenShare
            }
        }
        KeyCode::Char('S') => Action::CycleSort,
        KeyCode::Char('+') => Action::BeginPromptProject,
        KeyCode::Char('[') => Action::ToggleLeftPane,
        KeyCode::Char(']') => Action::ToggleRightPane,
        KeyCode::Char('T') => Action::QuickInterrupt,
        KeyCode::Char('D') => Action::CycleDensity,
        KeyCode::Char('L') => Action::ToggleLineNum,
        KeyCode::Char('H') => Action::ToggleShowDone,
        KeyCode::Char('F') => Action::ToggleShowFuture,
        KeyCode::Esc => Action::EscapeStack,
        KeyCode::Char('W') => Action::ChangeWeekStart,
        KeyCode::Char('t') => Action::TimerStartStop,
        KeyCode::Char('M') => Action::ManualTimeEntry,
        KeyCode::Char('Z') => Action::OpenThemePicker,
        KeyCode::Char('V') => Action::OpenTimesheet,
        KeyCode::Char('P') => Action::OpenProjectManager,
        _ => return None,
    })
}
