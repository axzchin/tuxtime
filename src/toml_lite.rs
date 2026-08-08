//! Shared helpers for the crate's hand-rolled TOML-lite parsers.
//!
//! `config.toml`, `keybinds.toml`, and theme files are all parsed with a
//! small `key = value` subset of TOML. The bits those parsers share (so far
//! just `unquote`) live here instead of being copied into each parser.

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
}
