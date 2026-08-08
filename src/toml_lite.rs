//! Shared helpers for the crate's hand-rolled TOML-lite parsers.
//!
//! `config.toml`, `keybinds.toml`, and theme files are all parsed with a
//! small `key = value` subset of TOML. The bits those parsers share
//! ([`unquote`] and [`split_key_value`]) live here instead of being copied
//! into each parser.

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
}
