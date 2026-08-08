//! User-configurable normal-mode keybindings.
//!
//! Path: `${XDG_CONFIG_HOME:-$HOME/.config}/tuxtime/keybinds.toml`
//!
//! Format: a `[normal]` table whose keys are `Action` names in `snake_case` and
//! whose values are a string or array of strings, for example:
//! `open_help = "F1"` or `begin_add = ["N", "Ctrl-n"]`.

use std::fs;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;
use crate::app::Chord;
use crate::toml_lite::{parse_value_strings, split_key_value, strip_comment, table_name};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedKey {
    Action(Action),
    Pending,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct KeyBindings {
    normal: Vec<Binding>,
}

impl KeyBindings {
    #[must_use]
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    #[must_use]
    pub fn load_from(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(s) => Self::parse(&s),
            Err(_) => Self::default(),
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Self {
        let mut bindings = Self::default();
        let mut section: Option<String> = None;
        for raw_line in s.lines() {
            let line = strip_comment(raw_line);
            if let Some(name) = table_name(line) {
                section = Some(name.to_ascii_lowercase());
                continue;
            }
            if section.as_deref().is_some_and(|name| name != "normal") {
                continue;
            }
            let Some((name, value)) = split_key_value(line) else {
                continue;
            };
            let Some(action) = Action::from_keybind_name(name) else {
                continue;
            };
            // split_key_value already unquoted single values, so the second
            // unquote inside parse_value_strings is a no-op for them (array
            // values like `["F1", "Ctrl-h"]` pass through both untouched).
            for key_text in parse_value_strings(value) {
                if let Some(binding) = Binding::parse(action, &key_text) {
                    bindings.push_normal(binding);
                }
            }
        }
        bindings
    }

    /// Resolve a custom normal-mode binding. Custom bindings are checked
    /// before built-ins by the caller; `Pending` means this key was the first
    /// key of a configured two-key chord and should be consumed.
    pub fn resolve_normal(&self, key: KeyEvent, chord: &mut Chord) -> Option<ResolvedKey> {
        for binding in &self.normal {
            let Some(second) = binding.second.as_ref() else {
                continue;
            };
            let Some(leader) = binding.first.leader_char() else {
                continue;
            };
            if chord.active() == Some(leader) && second.matches(key) {
                let _ = chord.clear();
                return Some(ResolvedKey::Action(binding.action));
            }
        }
        for binding in &self.normal {
            if binding.second.is_none() && binding.first.matches(key) {
                let _ = chord.clear();
                return Some(ResolvedKey::Action(binding.action));
            }
        }
        for binding in &self.normal {
            if binding.second.is_some()
                && binding.first.matches(key)
                && let Some(leader) = binding.first.leader_char()
            {
                chord.arm(leader);
                return Some(ResolvedKey::Pending);
            }
        }
        None
    }

    #[must_use]
    pub fn path() -> Option<PathBuf> {
        let base = crate::xdg::config_home()?;
        Some(Self::path_in(&base))
    }

    #[must_use]
    pub fn path_in(xdg_base: &Path) -> PathBuf {
        xdg_base.join("tuxtime").join("keybinds.toml")
    }

    fn push_normal(&mut self, binding: Binding) {
        self.normal.retain(|existing| {
            existing.first != binding.first || existing.second != binding.second
        });
        self.normal.push(binding);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding {
    action: Action,
    first: KeyPress,
    second: Option<KeyPress>,
}

impl Binding {
    fn parse(action: Action, text: &str) -> Option<Self> {
        let keys = parse_key_sequence(text)?;
        match keys.as_slice() {
            [first] => Some(Self {
                action,
                first: first.clone(),
                second: None,
            }),
            [first, second] if first.leader_char().is_some() => Some(Self {
                action,
                first: first.clone(),
                second: Some(second.clone()),
            }),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct KeyPress {
    code: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyPress {
    fn matches(&self, key: KeyEvent) -> bool {
        let code = normalized_code(key.code, key.modifiers);
        self.code == code && self.modifiers == normalized_modifiers(code, key.modifiers)
    }

    fn leader_char(&self) -> Option<char> {
        if self.modifiers == KeyModifiers::NONE
            && let KeyCode::Char(c) = self.code
        {
            Some(c)
        } else {
            None
        }
    }
}

fn parse_key_sequence(text: &str) -> Option<Vec<KeyPress>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.split_whitespace().count() > 1 {
        let keys: Option<Vec<KeyPress>> = text.split_whitespace().map(parse_key).collect();
        return keys.filter(|keys| keys.len() <= 2);
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() == 2
        && chars.iter().all(|c| !c.is_whitespace())
        && !text.contains('-')
        && !text.contains('+')
        && parse_named_key(text).is_none()
    {
        return Some(vec![
            KeyPress {
                code: KeyCode::Char(chars[0]),
                modifiers: KeyModifiers::NONE,
            },
            KeyPress {
                code: KeyCode::Char(chars[1]),
                modifiers: KeyModifiers::NONE,
            },
        ]);
    }
    parse_key(text).map(|key| vec![key])
}

fn parse_key(text: &str) -> Option<KeyPress> {
    if let Some(code) = parse_named_key(text.trim()) {
        return Some(KeyPress {
            code,
            modifiers: normalized_modifiers(code, KeyModifiers::NONE),
        });
    }
    let mut modifiers = KeyModifiers::NONE;
    let normalized = text.trim().replace('+', "-");
    let mut parts: Vec<&str> = normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect();
    let key_name = parts.pop()?;
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "alt" | "option" | "meta" => modifiers |= KeyModifiers::ALT,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            _ => return None,
        }
    }
    let code = if let Some(named) = parse_named_key(key_name) {
        named
    } else {
        let mut chars = key_name.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => KeyCode::Char(c),
            _ => return None,
        }
    };
    let code = normalized_code(code, modifiers);
    Some(KeyPress {
        code,
        modifiers: normalized_modifiers(code, modifiers),
    })
}

fn normalized_code(code: KeyCode, modifiers: KeyModifiers) -> KeyCode {
    if modifiers.contains(KeyModifiers::CONTROL)
        && let KeyCode::Char(c) = code
    {
        KeyCode::Char(c.to_ascii_lowercase())
    } else {
        code
    }
}

fn parse_named_key(text: &str) -> Option<KeyCode> {
    let lower = text.to_ascii_lowercase();
    match lower.as_str() {
        "backspace" | "bs" => Some(KeyCode::Backspace),
        "enter" | "return" => Some(KeyCode::Enter),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "pageup" | "page-up" | "pgup" => Some(KeyCode::PageUp),
        "pagedown" | "page-down" | "pgdn" => Some(KeyCode::PageDown),
        "tab" => Some(KeyCode::Tab),
        "backtab" | "shift-tab" => Some(KeyCode::BackTab),
        "delete" | "del" => Some(KeyCode::Delete),
        "insert" | "ins" => Some(KeyCode::Insert),
        "esc" | "escape" => Some(KeyCode::Esc),
        "space" => Some(KeyCode::Char(' ')),
        _ if lower.len() > 1 && lower.starts_with('f') => {
            lower[1..].parse::<u8>().ok().and_then(|n| {
                if (1..=24).contains(&n) {
                    Some(KeyCode::F(n))
                } else {
                    None
                }
            })
        }
        _ => None,
    }
}

fn normalized_modifiers(code: KeyCode, mut modifiers: KeyModifiers) -> KeyModifiers {
    if matches!(code, KeyCode::Char(c) if c.is_ascii_uppercase()) {
        modifiers.remove(KeyModifiers::SHIFT);
    }
    modifiers
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn parses_single_keys_arrays_and_chords() {
        let bindings = KeyBindings::parse(
            r#"
            [normal]
            open_help = ["F1", "Ctrl-h"]
            quit = "ZZ"
            begin_add = "N"
            open_command_palette = "Ctrl-P"
            half_page_down = "Page-Down"
            "#,
        );
        let mut chord = Chord::default();
        assert_eq!(
            bindings.resolve_normal(KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE), &mut chord),
            Some(ResolvedKey::Action(Action::OpenHelp))
        );
        assert_eq!(
            bindings.resolve_normal(ctrl('h'), &mut chord),
            Some(ResolvedKey::Action(Action::OpenHelp))
        );
        assert_eq!(
            bindings.resolve_normal(key('Z'), &mut chord),
            Some(ResolvedKey::Pending)
        );
        assert_eq!(
            bindings.resolve_normal(key('Z'), &mut chord),
            Some(ResolvedKey::Action(Action::Quit))
        );
        assert_eq!(
            bindings.resolve_normal(key('N'), &mut chord),
            Some(ResolvedKey::Action(Action::BeginAdd))
        );
        assert_eq!(
            bindings.resolve_normal(ctrl('p'), &mut chord),
            Some(ResolvedKey::Action(Action::OpenCommandPalette))
        );
        assert_eq!(
            bindings.resolve_normal(
                KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
                &mut chord,
            ),
            Some(ResolvedKey::Action(Action::HalfPageDown))
        );
    }

    #[test]
    fn ignores_other_tables_and_unknown_actions() {
        let bindings = KeyBindings::parse(
            r#"
            [insert]
            quit = "q"
            [normal]
            not_an_action = "x"
            open_settings = ","
            "#,
        );
        let mut chord = Chord::default();
        assert_eq!(bindings.resolve_normal(key('q'), &mut chord), None);
        assert_eq!(
            bindings.resolve_normal(key(','), &mut chord),
            Some(ResolvedKey::Action(Action::OpenSettings))
        );
    }

    #[test]
    fn path_uses_tuxtime_keybinds_toml() {
        let path = KeyBindings::path_in(Path::new("/tmp/config"));
        assert!(path.ends_with("tuxtime/keybinds.toml"));
    }

    // ---- parse_key / parse_named_key / parse_key_sequence tables ----

    #[test]
    fn parses_named_keys_and_aliases() {
        for (text, expect) in [
            ("Enter", KeyCode::Enter),
            ("return", KeyCode::Enter),
            ("Backspace", KeyCode::Backspace),
            ("Tab", KeyCode::Tab),
            ("BackTab", KeyCode::BackTab),
            ("Shift-Tab", KeyCode::BackTab),
            ("PageDown", KeyCode::PageDown),
            ("PgUp", KeyCode::PageUp),
            ("F12", KeyCode::F(12)),
            ("f3", KeyCode::F(3)),
            ("Space", KeyCode::Char(' ')),
            ("Esc", KeyCode::Esc),
        ] {
            assert_eq!(parse_key(text).map(|k| k.code), Some(expect), "{text}");
        }
        // Out-of-range function keys and garbage are rejected.
        assert!(parse_named_key("F25").is_none());
        assert!(parse_named_key("F0").is_none());
        assert!(parse_key("").is_none());
    }

    #[test]
    fn parses_modifiers_and_normalizes() {
        // Ctrl- lowercases the code and sets CONTROL.
        let k = parse_key("Ctrl-H").unwrap();
        assert_eq!(k.code, KeyCode::Char('h'));
        assert_eq!(k.modifiers, KeyModifiers::CONTROL);
        // '+' is accepted as a modifier separator (vim-style).
        let k = parse_key("ctrl+h").unwrap();
        assert_eq!(k.code, KeyCode::Char('h'));
        assert_eq!(k.modifiers, KeyModifiers::CONTROL);
        // Alt+Shift on an uppercase char: SHIFT is folded away but the
        // uppercase code is preserved (non-Ctrl keys keep their case so
        // `Shift-a` stays distinct from `a`).
        let k = parse_key("Alt-Shift-A").unwrap();
        assert_eq!(k.code, KeyCode::Char('A'));
        assert_eq!(k.modifiers, KeyModifiers::ALT);
        // A modifier on a named key is kept.
        let k = parse_key("Ctrl-F5").unwrap();
        assert_eq!(k.code, KeyCode::F(5));
        assert_eq!(k.modifiers, KeyModifiers::CONTROL);
        // Unknown modifier and multi-char bare text are rejected.
        assert!(parse_key("Hyper-x").is_none());
        assert!(parse_key("ab").is_none());
    }

    #[test]
    fn parses_chords_and_sequences() {
        // Two bare chars are a chord (no modifiers, no separators).
        assert_eq!(parse_key_sequence("ZZ").unwrap().len(), 2);
        // Whitespace-separated two-key chords parse too.
        assert_eq!(parse_key_sequence("g t").unwrap().len(), 2);
        // More than two keys is rejected.
        assert!(parse_key_sequence("a b c").is_none());
        // Named / modified keys are a single key, not a chord.
        assert_eq!(parse_key_sequence("Enter").unwrap().len(), 1);
        assert_eq!(parse_key_sequence("Ctrl-P").unwrap().len(), 1);
        assert!(parse_key_sequence("").is_none());
    }

    #[test]
    fn chords_require_a_plain_char_leader() {
        // A 2-key binding is only accepted when the first key is a plain
        // char (no modifiers) — Ctrl-led chords are rejected.
        assert!(Binding::parse(Action::OpenHelp, "g t").is_some());
        assert!(Binding::parse(Action::OpenHelp, "Ctrl-a b").is_none());
        assert!(Binding::parse(Action::OpenHelp, "Ctrl-a Ctrl-b").is_none());
    }

    #[test]
    fn hash_inside_quotes_is_not_a_comment() {
        // `#` begins a comment only outside quotes; a binding whose value is
        // `"#"` must survive strip_comment and bind the '#' key.
        let bindings = KeyBindings::parse("[normal]\nopen_settings = \"#\"\n");
        let mut chord = Chord::default();
        assert_eq!(
            bindings.resolve_normal(key('#'), &mut chord),
            Some(ResolvedKey::Action(Action::OpenSettings))
        );
    }
}
