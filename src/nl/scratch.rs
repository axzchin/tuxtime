//! [`Scratch`]: the byte-accounting buffer shared by the NL passes. Each pass
//! marks the byte ranges it has consumed; later passes skip them, and the
//! leftover text becomes the cleaned body.

use super::tokenizer::{ascii_lower, compute_words};

pub(super) struct Scratch<'a> {
    pub(super) text: &'a str,
    /// ASCII-lowercased copy of `text`. Lowercasing only ASCII letters keeps
    /// byte indices aligned between `text` and `lower`, so a range valid in
    /// one is valid in the other.
    lower: String,
    consumed: Vec<bool>,
    /// Cached word ranges over the original text. Recomputed via
    /// `live_words()` each pass — cheap since inputs are short.
    pub(super) word_cache: Vec<(usize, usize)>,
}

impl<'a> Scratch<'a> {
    pub(super) fn new(text: &'a str) -> Self {
        let lower = ascii_lower(text);
        let consumed = vec![false; text.len()];
        let word_cache = compute_words(text);
        Self {
            text,
            lower,
            consumed,
            word_cache,
        }
    }

    /// Returns `true` if every byte in `[start, end)` is unconsumed. A
    /// fully-consumed word counts as gone for subsequent passes.
    pub(super) fn is_live(&self, start: usize, end: usize) -> bool {
        (start..end).all(|i| !self.consumed.get(i).copied().unwrap_or(true))
    }

    pub(super) fn mark(&mut self, start: usize, end: usize) {
        let end = end.min(self.consumed.len());
        for slot in &mut self.consumed[start..end] {
            *slot = true;
        }
    }

    /// Lower-case slice with trailing punctuation stripped — what most pattern
    /// matchers want to compare against.
    pub(super) fn word_lc(&self, range: (usize, usize)) -> &str {
        self.lower[range.0..range.1].trim_end_matches([',', '.', ';', ':', '!', '?'])
    }

    /// Original-case slice with trailing punctuation stripped — used when the
    /// extracted value needs to round-trip (e.g. tag names).
    pub(super) fn word_orig(&self, range: (usize, usize)) -> &str {
        self.text[range.0..range.1].trim_end_matches([',', '.', ';', ':', '!', '?'])
    }

    /// Remaining body text after stripping consumed bytes and collapsing
    /// whitespace. Leading/trailing connector words ("and", "it's", …) are
    /// also dropped so the body reads cleanly.
    pub(super) fn remaining_cleaned(&self) -> String {
        let mut buf = String::new();
        let mut prev_space = true;
        for (i, c) in self.text.char_indices() {
            let is_consumed = self.consumed.get(i).copied().unwrap_or(false);
            if is_consumed || c.is_whitespace() {
                if !prev_space {
                    buf.push(' ');
                    prev_space = true;
                }
            } else {
                buf.push(c);
                prev_space = false;
            }
        }
        let mut tokens: Vec<&str> = buf.split_whitespace().collect();
        let is_connector = |t: &str| {
            let cleaned = t
                .trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | '!' | '?'))
                .to_ascii_lowercase();
            matches!(
                cleaned.as_str(),
                "and" | "or" | "but" | "it's" | "its" | "that" | "which" | ""
            )
        };
        while tokens.first().is_some_and(|t| is_connector(t)) {
            tokens.remove(0);
        }
        while tokens.last().is_some_and(|t| is_connector(t)) {
            tokens.pop();
        }
        let joined = tokens.join(" ");
        joined
            .trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':') || c.is_whitespace())
            .to_string()
    }
}
