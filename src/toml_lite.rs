//! Shared helpers for the crate's hand-rolled TOML-lite parsers.
//!
//! `config.toml`, `keybinds.toml`, and theme files are all parsed with a
//! small `key = value` subset of TOML. The primitives those parsers share
//! live here instead of being copied into each parser:
//!
//! - [`unquote`] — strip a surrounding pair of quotes
//! - [`split_key_value`] — trim + skip blank/`#` lines + split on the first `=`
//! - [`strip_comment`] — quote-aware inline comment stripping (keybinds only)
//! - [`table_name`] — `[section]` header matching (keybinds only)
//! - [`parse_value_strings`] — single-string vs `["a", "b"]` array values
//!
//! Comment handling is deliberately split: `config.toml` and theme files
//! only skip *whole-line* `#` comments ([`split_key_value`]), so `k = v # note`
//! keeps the note in the value. `keybinds.toml` strips *inline* comments
//! quote-aware ([`strip_comment`]) because a binding like `"#"` is data.

/// Strip one pair of surrounding double quotes, if present. Leaves the input
/// untouched when it isn't fully wrapped in quotes (including a lone `"`).
#[must_use]
pub(crate) fn unquote(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Split one `key = value` line into its trimmed key and unquoted, trimmed
/// value. Skips blank lines and whole-line `#` comments (the subset of TOML
/// comment handling all three parsers share). Returns `None` for lines that
/// aren't key/value pairs.
#[must_use]
pub(crate) fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (k, v) = line.split_once('=')?;
    Some((k.trim(), unquote(v.trim())))
}

/// Strip an inline `#` comment from a line, honoring double-quoted strings: a
/// `#` inside quotes is data, not a comment. Used by the keybinds parser,
/// whose values can legitimately contain `#` (e.g. `open_settings = "#"`).
/// The other parsers don't strip inline comments at all — they skip only
/// whole-line comments inside [`split_key_value`] — so `k = v # note` keeps
/// the note in the value there.
#[must_use]
pub(crate) fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quote => escaped = true,
            '"' => in_quote = !in_quote,
            '#' if !in_quote => return &line[..idx],
            _ => {}
        }
    }
    line
}

/// Match a `[table]` header, returning its trimmed section name.
#[must_use]
pub(crate) fn table_name(line: &str) -> Option<&str> {
    line.trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Parse a value into its constituent strings: either a single (optionally
/// quoted) string or a `["a", "b"]` array of quoted strings. An empty value
/// yields nothing.
#[must_use]
pub(crate) fn parse_value_strings(value: &str) -> Vec<String> {
    let value = value.trim();
    if value.is_empty() {
        return Vec::new();
    }
    if let Some(inner) = value.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return parse_array_strings(inner);
    }
    vec![unquote(value).to_string()]
}

/// Parse the inside of a `["a", "b"]` array into its quoted strings, honoring
/// `\"` escapes.
fn parse_array_strings(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = inner.char_indices().peekable();
    while let Some((_, ch)) = chars.peek().copied() {
        if ch.is_whitespace() || ch == ',' {
            let _ = chars.next();
            continue;
        }
        if ch != '"' {
            break;
        }
        let start = chars.next().map(|(idx, _)| idx + 1);
        let Some(start) = start else {
            break;
        };
        let mut escaped = false;
        let mut end = None;
        for (idx, c) in chars.by_ref() {
            if escaped {
                escaped = false;
                continue;
            }
            if c == '\\' {
                escaped = true;
                continue;
            }
            if c == '"' {
                end = Some(idx);
                break;
            }
        }
        let Some(end) = end else {
            break;
        };
        out.push(inner[start..end].replace("\\\"", "\""));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_matching_outer_quotes() {
        assert_eq!(unquote("\"Muted Slate\""), "Muted Slate");
        assert_eq!(unquote("\"\""), "");
    }

    #[test]
    fn leaves_unquoted_and_partially_quoted_input() {
        assert_eq!(unquote("plain"), "plain");
        assert_eq!(unquote(""), "");
        // A lone opening quote is not a wrapping pair.
        assert_eq!(unquote("\"x"), "\"x");
        assert_eq!(unquote("x\""), "x\"");
    }

    #[test]
    fn splits_key_value_with_trimming_and_unquoting() {
        assert_eq!(
            split_key_value("theme = \"Muted Slate\""),
            Some(("theme", "Muted Slate"))
        );
        assert_eq!(
            split_key_value("density=compact"),
            Some(("density", "compact"))
        );
        assert_eq!(split_key_value("  sort =  due  "), Some(("sort", "due")));
    }

    #[test]
    fn skips_blanks_comments_and_non_keyvalue_lines() {
        assert_eq!(split_key_value(""), None);
        assert_eq!(split_key_value("   "), None);
        assert_eq!(split_key_value("# a comment"), None);
        assert_eq!(split_key_value("no-equals-here"), None);
    }

    // Whole-line `#` comments are skipped, but a `#` inside a value is
    // kept verbatim — config/theme parsing is deliberately not
    // comment-aware like keybinds' quote-aware `strip_comment`. Pinned
    // so a future "improvement" can't silently change that boundary.
    #[test]
    fn keeps_hash_inside_value_not_whole_line_comments() {
        assert_eq!(split_key_value("k = v # note"), Some(("k", "v # note")));
        assert_eq!(split_key_value("  # indented comment"), None);
    }

    #[test]
    fn strips_inline_comments_quote_aware() {
        assert_eq!(
            strip_comment("open_help = \"F1\" # a note"),
            "open_help = \"F1\" "
        );
        assert_eq!(
            strip_comment("open_settings = \"#\""),
            "open_settings = \"#\""
        );
        assert_eq!(strip_comment("  # whole-line"), "  ");
        assert_eq!(strip_comment("no comment here"), "no comment here");
    }

    #[test]
    fn matches_table_headers() {
        assert_eq!(table_name("[normal]"), Some("normal"));
        assert_eq!(table_name("  [ insert ]  "), Some("insert"));
        assert_eq!(table_name("[]"), None);
        assert_eq!(table_name("normal"), None);
    }

    #[test]
    fn parses_single_and_array_values() {
        assert_eq!(parse_value_strings("\"F1\""), vec!["F1".to_string()]);
        assert_eq!(
            parse_value_strings("[\"F1\", \"Ctrl-h\"]"),
            vec!["F1".to_string(), "Ctrl-h".to_string()]
        );
        assert_eq!(parse_value_strings(""), Vec::<String>::new());
        assert_eq!(
            parse_value_strings("[\"a\\\"b\"]"),
            vec!["a\"b".to_string()]
        );
    }
}
