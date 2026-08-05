//! Insert-mode key handlers: draft text editing (vim/readline keys),
//! slash menu, calendar, recurrence builder, priority chooser, and
//! duration picker overlays.
//!
//! Extracted from `main.rs` to keep the TUI event loop focused on mode
//! dispatch. The public entry point is [`handle_insert`]; overlay-specific
//! handlers are `pub(crate)` for testing.
//!
//! [`DraftEffect`] and [`EditAction`] model the result of each keystroke on
//! the draft buffer — callers like search and the command palette reuse
//! [`apply_to_draft`] to share the same text-editing behavior.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tuxtime::app::{AddOutcome, App, DialogInputMode, OverlayKind};
use crate::handle_autocomplete_keys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DraftEffect {
    Unhandled,
    CursorMoved,
    TextChanged,
}

/// A single text-editing operation on the draft buffer. Covers the standard
/// keys (insert/backspace/delete/arrows/Home/End) plus the readline/emacs set
/// (Ctrl+A/E/B/F/H/D/W/U/K, Alt+B/F/D). Modeling the keystroke as an action
/// keeps the insert/search/prompt/command-palette contexts in sync — they all
/// route through the same resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditAction {
    Insert(char),
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    KillToStart,
    KillToEnd,
    MoveLeft,
    MoveRight,
    MoveHome,
    MoveEnd,
    MoveWordForward,
    MoveWordBackward,
}

impl EditAction {
    fn apply(self, app: &mut App) -> DraftEffect {
        match self {
            EditAction::Insert(c) => {
                app.draft_insert_char(c);
                DraftEffect::TextChanged
            }
            EditAction::DeleteBackward => {
                app.draft_backspace();
                DraftEffect::TextChanged
            }
            EditAction::DeleteForward => {
                app.draft_delete_forward();
                DraftEffect::TextChanged
            }
            EditAction::DeleteWordBackward => {
                app.draft_delete_word_backward();
                DraftEffect::TextChanged
            }
            EditAction::DeleteWordForward => {
                app.draft_delete_word_forward();
                DraftEffect::TextChanged
            }
            EditAction::KillToStart => {
                app.draft_kill_to_start();
                DraftEffect::TextChanged
            }
            EditAction::KillToEnd => {
                app.draft_kill_to_end();
                DraftEffect::TextChanged
            }
            EditAction::MoveLeft => {
                app.draft_left();
                DraftEffect::CursorMoved
            }
            EditAction::MoveRight => {
                app.draft_right();
                DraftEffect::CursorMoved
            }
            EditAction::MoveHome => {
                app.draft_home();
                DraftEffect::CursorMoved
            }
            EditAction::MoveEnd => {
                app.draft_end();
                DraftEffect::CursorMoved
            }
            EditAction::MoveWordForward => {
                app.draft_word_forward();
                DraftEffect::CursorMoved
            }
            EditAction::MoveWordBackward => {
                app.draft_word_backward();
                DraftEffect::CursorMoved
            }
        }
    }
}

/// Map a single keystroke to an `EditAction`, or `None` when the key isn't a
/// text-editing key. A *single* Control or Alt chord is matched first and
/// never falls through to the plain `Char(c)` insert arm, so an unmapped chord
/// (e.g. Ctrl+G) is swallowed rather than typed as a literal control letter —
/// this is what fixes Ctrl+H inserting an 'h' instead of deleting. Ctrl+N/Ctrl+P
/// are deliberately left unmapped: upstream handlers reserve them for popup
/// and list navigation.
///
/// CONTROL **and** ALT together is AltGr, which crossterm reports for printable
/// characters on international layouts (e.g. AltGr+E → `€`). That is text, not a
/// chord, so the chord arms are gated on exactly one modifier being held and
/// AltGr falls through to the `Char(c)` insert arm.

pub(crate) fn resolve_edit_key(key: KeyEvent) -> Option<EditAction> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    if ctrl && !alt {
        return match key.code {
            KeyCode::Char('a') => Some(EditAction::MoveHome),
            KeyCode::Char('e') => Some(EditAction::MoveEnd),
            KeyCode::Char('b') => Some(EditAction::MoveLeft),
            KeyCode::Char('f') => Some(EditAction::MoveRight),
            KeyCode::Char('h') => Some(EditAction::DeleteBackward),
            KeyCode::Char('d') => Some(EditAction::DeleteForward),
            KeyCode::Char('w') => Some(EditAction::DeleteWordBackward),
            KeyCode::Char('u') => Some(EditAction::KillToStart),
            KeyCode::Char('k') => Some(EditAction::KillToEnd),
            // Ctrl+Backspace as delete-word is a common modern expectation;
            // terminals that report it this way get it for free.
            KeyCode::Backspace => Some(EditAction::DeleteWordBackward),
            _ => None,
        };
    }
    if alt && !ctrl {
        return match key.code {
            KeyCode::Char('b') => Some(EditAction::MoveWordBackward),
            KeyCode::Char('f') => Some(EditAction::MoveWordForward),
            KeyCode::Char('d') => Some(EditAction::DeleteWordForward),
            // M-DEL is readline's backward-kill-word.
            KeyCode::Backspace => Some(EditAction::DeleteWordBackward),
            _ => None,
        };
    }
    match key.code {
        KeyCode::Backspace => Some(EditAction::DeleteBackward),
        KeyCode::Delete => Some(EditAction::DeleteForward),
        KeyCode::Left => Some(EditAction::MoveLeft),
        KeyCode::Right => Some(EditAction::MoveRight),
        KeyCode::Home => Some(EditAction::MoveHome),
        KeyCode::End => Some(EditAction::MoveEnd),
        KeyCode::Char(c) => Some(EditAction::Insert(c)),
        _ => None,
    }
}

/// Apply a standard text-editing key to the draft. Thin wrapper over
/// `resolve_edit_key` + `EditAction::apply`, returning `Unhandled` for keys
/// that aren't text editing so callers can layer their own handling.
pub(crate) fn apply_to_draft(app: &mut App, key: KeyEvent) -> DraftEffect {
    match resolve_edit_key(key) {
        Some(action) => action.apply(app),
        None => DraftEffect::Unhandled,
    }
}

// ---- shared save helpers -----------------------------------------------

/// Save the current draft (new task via [`App::add_from_draft`] or edit via
/// [`App::save_edit`]) and return the outcome. Shared by [`handle_insert_normal`]
/// and [`handle_insert`] to eliminate the duplicated save-on-Enter logic.
fn commit_draft(app: &mut App) -> AddOutcome {
    if app.selection.editing().is_some() {
        app.session.manual_time_entry = false;
        app.save_edit();
        AddOutcome::Saved
    } else {
        app.add_from_draft()
    }
}

/// Transition out of Insert mode back to Normal, clearing transient state.
fn exit_insert(app: &mut App) {
    app.nav.enter_normal();
    app.draft_clear();
    app.selection.exit_edit();
    app.session.manual_time_entry = false;
}

pub(crate) fn handle_insert_normal(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Enter if !ctrl => {
            let outcome = commit_draft(app);
            if !matches!(outcome, AddOutcome::Parsed) {
                exit_insert(app);
            }
        }
        KeyCode::Enter if ctrl => {
            let outcome = commit_draft(app);
            if outcome == AddOutcome::Saved {
                exit_insert(app);
                app.toggle_timer();
            }
        }
        KeyCode::Esc => {
            exit_insert(app);
        }
        KeyCode::Char('h') | KeyCode::Left => app.draft_left(),
        KeyCode::Char('l') | KeyCode::Right => app.draft_right(),
        KeyCode::Char('w') if app.chord.consume('d') => app.draft_delete_word_forward(),
        KeyCode::Char('w') if app.chord.consume('c') => {
            app.draft_delete_word_forward();
            app.draft.set_input_mode(DialogInputMode::Insert);
        }
        KeyCode::Char('w') => app.draft_word_forward(),
        KeyCode::Char('b') => app.draft_word_backward(),
        KeyCode::Char('e') => app.draft_word_end(),
        KeyCode::Char('d') => app.chord.arm('d'),
        KeyCode::Char('c') => app.chord.arm('c'),
        KeyCode::Char('x') => app.draft_delete_forward(),
        KeyCode::Char('i') => app.draft.set_input_mode(DialogInputMode::Insert),
        KeyCode::Char('a') => {
            app.draft_right();
            app.draft.set_input_mode(DialogInputMode::Insert);
        }
        KeyCode::Char('A') => {
            app.draft_end();
            app.draft.set_input_mode(DialogInputMode::Insert);
        }
        _ => {}
    }
}

pub(crate) fn handle_insert(app: &mut App, key: KeyEvent) {
    if app.draft.input_mode() == DialogInputMode::Normal {
        handle_insert_normal(app, key);
        return;
    }

    // Metadata-picker overlays take precedence. Non-slash overlays fully
    // consume keys until accepted or cancelled; the slash menu intercepts
    // only its navigation keys and lets text editing flow through so the
    // filter text in the buffer keeps growing as the user types.
    let overlay = app.draft.overlay().map(tuxtime::app::DraftOverlay::kind);
    match overlay {
        Some(OverlayKind::Calendar) => {
            handle_insert_calendar(app, key);
            return;
        }
        Some(OverlayKind::RecurrenceBuilder) => {
            handle_insert_rec_builder(app, key);
            return;
        }
        Some(OverlayKind::PriorityChooser) => {
            handle_insert_priority(app, key);
            return;
        }
        Some(OverlayKind::DurationPicker) => {
            handle_insert_duration(app, key);
            return;
        }
        Some(OverlayKind::SlashMenu) => {
            if handle_insert_slash_menu(app, key) {
                return;
            }
            // Fall through — let the key flow into the editor so filter chars
            // can be typed/erased. We re-check the overlay invariants after.
            apply_to_draft(app, key);
            // Backspacing past the `/` closes the menu; typing more chars
            // just narrows the filter.
            app.slash_menu_revalidate();
            return;
        }
        None => {}
    }

    // Autocomplete bindings take precedence — only when the popup is visible.
    // Tab accepts; Enter falls through to save so the popup never swallows the
    // submit keystroke (e.g. when the typed token already matches an existing
    // project/context). Esc with the popup open dismisses the popup but leaves
    // Insert mode intact; a second Esc enters Normal mode (handled below).
    if app.autocomplete_visible() {
        match key.code {
            // Tab accepts; Enter accepts (regular, not Ctrl+Enter). Ctrl+Enter
            // falls through so it can save+start instead of being swallowed.
            KeyCode::Tab => {
                app.autocomplete_accept();
                app.draft.suppress_autocomplete();
                return;
            }
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.autocomplete_accept();
                app.draft.suppress_autocomplete();
                return;
            }
            _ => {
                if handle_autocomplete_keys(app, key) {
                    return;
                }
            }
        }
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            app.draft.set_input_mode(DialogInputMode::Normal);
        }
        KeyCode::Enter if !ctrl => {
            let outcome = commit_draft(app);
            // `Parsed` means the NL parser rewrote the draft into canonical
            // todo.txt and is asking the user to confirm — stay in Insert so
            // they can review/edit before a second Enter saves.
            if !matches!(outcome, AddOutcome::Parsed) {
                exit_insert(app);
            }
        }
        KeyCode::Enter if ctrl => {
            let outcome = commit_draft(app);
            if outcome == AddOutcome::Saved {
                exit_insert(app);
                app.toggle_timer();
            }
        }
        _ => {
            let before = app.draft.text().len();
            let effect = apply_to_draft(app, key);
            // `/` opens the slash menu; `:` after a recognised key
            // (`due` / `t` / `rec`) opens the matching picker directly. Both
            // detections run post-insert so they inspect what actually
            // landed in the buffer.
            if effect == DraftEffect::TextChanged && app.draft.text().len() > before {
                match key.code {
                    KeyCode::Char('/') => app.maybe_open_slash_menu(),
                    KeyCode::Char(':') => app.maybe_open_kv_overlay(),
                    _ => {}
                }
            }
        }
    }
}

/// Slash-menu key handler. Returns `true` when the key was consumed by the
/// menu (navigation, accept, dismiss); `false` when the key should fall
/// through to text editing so filter chars are typed into the buffer.
pub(crate) fn handle_insert_slash_menu(app: &mut App, key: KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Up => {
            app.slash_step(false);
            true
        }
        KeyCode::Down => {
            app.slash_step(true);
            true
        }
        KeyCode::Char('n') if ctrl => {
            app.slash_step(true);
            true
        }
        KeyCode::Char('p') if ctrl => {
            app.slash_step(false);
            true
        }
        KeyCode::Tab | KeyCode::Enter => {
            app.slash_accept();
            true
        }
        KeyCode::Esc => {
            app.slash_cancel();
            true
        }
        _ => false,
    }
}

pub(crate) fn handle_insert_calendar(app: &mut App, key: KeyEvent) {
    // In auto-trigger mode (anchor set): digit, dash, and backspace are
    // forwarded to the draft buffer so the user can type the date directly.
    // The calendar grid tracks the typed date as it becomes valid.
    if app.calendar_state().is_some_and(|s| s.anchor.is_some()) {
        let is_date_char = matches!(key.code, KeyCode::Char(c) if c.is_ascii_digit() || c == '-');
        if is_date_char || matches!(key.code, KeyCode::Backspace) {
            apply_to_draft(app, key);
            app.calendar_sync_from_draft();
            return;
        }
    }
    match key.code {
        KeyCode::Char('h') | KeyCode::Left => app.calendar_move(-1, 0),
        KeyCode::Char('l') | KeyCode::Right => app.calendar_move(1, 0),
        KeyCode::Char('k') | KeyCode::Up => app.calendar_move(0, -1),
        KeyCode::Char('j') | KeyCode::Down => app.calendar_move(0, 1),
        KeyCode::Char('t') => app.calendar_set_relative(0),
        KeyCode::Char('T') => app.calendar_set_relative(1),
        KeyCode::Char('w') => app.calendar_set_relative(7),
        KeyCode::Char('m') => app.calendar_add_months(1),
        KeyCode::Char('M') => app.calendar_add_months(-1),
        KeyCode::Char('x') => app.calendar_clear(),
        KeyCode::Enter => app.calendar_accept(),
        KeyCode::Esc => app.calendar_cancel(),
        _ => {}
    }
}

pub(crate) fn handle_insert_rec_builder(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('h') | KeyCode::Left => app.recurrence_focus(-1),
        KeyCode::Char('l') | KeyCode::Right => app.recurrence_focus(1),
        KeyCode::Char('j') | KeyCode::Down => app.recurrence_focus(1),
        KeyCode::Char('k') | KeyCode::Up => app.recurrence_focus(-1),
        // `=` is the unshifted `+` on US keyboards — accept both so users
        // don't have to chord Shift to bump the interval.
        KeyCode::Char('+') | KeyCode::Char('=') => app.recurrence_adjust(1),
        KeyCode::Char('-') | KeyCode::Char('_') => app.recurrence_adjust(-1),
        KeyCode::Enter => app.recurrence_accept(),
        KeyCode::Esc => app.recurrence_cancel(),
        _ => {}
    }
}

pub(crate) fn handle_insert_priority(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.priority_step(true),
        KeyCode::Char('k') | KeyCode::Up => app.priority_step(false),
        KeyCode::Enter => app.priority_accept(),
        KeyCode::Esc => app.priority_cancel(),
        _ => {}
    }
}

pub(crate) fn handle_insert_duration(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.duration_step(true),
        KeyCode::Char('k') | KeyCode::Up => app.duration_step(false),
        KeyCode::Enter => app.duration_accept(),
        KeyCode::Esc => app.duration_cancel(),
        _ => {}
    }
}
