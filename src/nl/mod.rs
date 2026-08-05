//! Natural-language parser for the add-todo draft.
//!
//! When the user types prose into the add buffer ("Pay rent monthly on the
//! first, show 3 days before due, project home"), this module extracts the
//! structured todo.txt metadata so the caller can rewrite the buffer into
//! canonical form for the user to review.
//!
//! Pure logic — no I/O, no app state. The crate-level wiring lives in
//! `app::mutations::add_from_draft` and `inbox::canonicalize_line`.
//!
//! # Architecture
//!
//! The parser is split into small focused modules:
//! - [`tokenizer`] — pure lexical helpers (word segmentation, number/weekday/
//!   month/unit/ordinal lookups).
//! - [`scratch`] — [`Scratch`], the byte-accounting buffer passes consume.
//! - [`types`] — [`ParsedNl`], the extraction result.
//! - [`passes`] — the six ordered extraction passes (priority, sigils,
//!   threshold, recurrence, date, project/context) plus date resolution.
//!
//! Detection (`looks_like_natural_language`) is intentionally conservative:
//! it returns `false` whenever the buffer already contains a `due:` / `rec:`
//! / `t:` token, which gives the rewrite pipeline trivial idempotency — a
//! second Enter on the canonical output falls through to the existing save
//! path.

use chrono::NaiveDate;

mod passes;
mod scratch;
mod tokenizer;
mod types;

pub use types::ParsedNl;

use passes::{pass_date, pass_leading_priority, pass_priority, pass_project_context,
             pass_recurrence, pass_sigiled, pass_threshold};
use scratch::Scratch;

/// Cheap heuristic gating the full parse. Returns `true` when the buffer
/// looks like prose worth interpreting. Two rules:
///
/// 1. The buffer must not already contain a `due:` / `rec:` / `t:` token —
///    that means the user (or a previous NL rewrite) already produced
///    canonical form, so leave it alone.
/// 2. The buffer must contain at least one trigger word (date words,
///    weekdays, months, recurrence vocabulary, `before`, `project`,
///    `context`, …).
#[must_use]
pub fn looks_like_natural_language(text: &str) -> bool {
    if has_kv_token(text, "due") || has_kv_token(text, "rec") || has_kv_token(text, "t") {
        return false;
    }
    contains_trigger(text)
}

/// Main entry point. `today` resolves relative dates ("tomorrow", "the first
/// of the month"). Returns `None` when the parser couldn't extract anything
/// structured — the caller then falls through to the plain save path.
#[must_use]
pub fn try_parse(text: &str, today: NaiveDate) -> Option<ParsedNl> {
    let mut scratch = Scratch::new(text);
    let mut parsed = ParsedNl::default();

    pass_leading_priority(&mut scratch, &mut parsed);
    pass_sigiled(&mut scratch, &mut parsed);
    pass_threshold(&mut scratch, &mut parsed);
    let weekday_hint = pass_recurrence(&mut scratch, &mut parsed);
    pass_date(&mut scratch, &mut parsed, today, weekday_hint);
    pass_project_context(&mut scratch, &mut parsed);
    pass_priority(&mut scratch, &mut parsed);

    parsed.body = scratch.remaining_cleaned();

    let extracted = parsed.due.is_some()
        || parsed.rec.is_some()
        || parsed.threshold.is_some()
        || !parsed.projects.is_empty()
        || !parsed.contexts.is_empty()
        || parsed.priority.is_some();
    if extracted { Some(parsed) } else { None }
}

/// Serialize a parsed result back to a canonical todo.txt line. Token order
/// is fixed: `(P) body +proj… @ctx… due:… rec:… t:…`. An empty body falls
/// back to `"todo"` so the result is always a well-formed task — the caller
/// is expected to flash a hint so the user knows to fix the body.
#[must_use]
pub fn format_as_todo_txt(p: &ParsedNl) -> String {
    let mut out = String::new();
    if let Some(prio) = p.priority {
        out.push('(');
        out.push(prio);
        out.push(')');
        out.push(' ');
    }
    let body = p.body.trim();
    if body.is_empty() {
        out.push_str("todo");
    } else {
        out.push_str(body);
    }
    for proj in &p.projects {
        out.push_str(" +");
        out.push_str(proj);
    }
    for ctx in &p.contexts {
        out.push_str(" @");
        out.push_str(ctx);
    }
    if let Some(d) = p.due {
        out.push_str(" due:");
        out.push_str(&d.format("%Y-%m-%d").to_string());
    }
    if let Some(r) = &p.rec {
        out.push_str(" rec:");
        out.push_str(r);
    }
    if let Some(t) = &p.threshold {
        out.push_str(" t:");
        out.push_str(t);
    }
    out
}

// ---------------------------------------------------------------------------
// Trigger detection
// ---------------------------------------------------------------------------

fn has_kv_token(text: &str, key: &str) -> bool {
    for tok in text.split_whitespace() {
        if let Some((k, v)) = tok.split_once(':')
            && k == key
            && !v.is_empty()
        {
            return true;
        }
    }
    false
}

fn contains_trigger(text: &str) -> bool {
    let lower = tokenizer::ascii_lower(text);
    let words: Vec<&str> = lower
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | '!' | '?')))
        .collect();

    const SINGLE_TRIGGERS: &[&str] = &[
        // date words
        "today",
        "tonight",
        "tomorrow",
        "yesterday",
        // weekdays
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "mon",
        "tue",
        "tues",
        "wed",
        "thu",
        "thurs",
        "fri",
        "sat",
        "sun",
        // recurrence
        "every",
        "each",
        "daily",
        "weekly",
        "biweekly",
        "monthly",
        "yearly",
        "annually",
        // prose markers
        "project",
        "proj",
        "context",
        "ctx",
        "priority",
        "before",
        "starting",
        "due",
        "by",
    ];

    for w in &words {
        if SINGLE_TRIGGERS.contains(w) {
            return true;
        }
        if tokenizer::parse_month(w).is_some() {
            return true;
        }
    }

    // Multi-word: "in N (day|week|month|year)s?"
    for i in 0..words.len() {
        if words[i] == "in" && i + 2 < words.len() {
            let n = words[i + 1]
                .parse::<u32>()
                .ok()
                .or_else(|| tokenizer::word_number(words[i + 1]));
            let unit = tokenizer::unit_char(words[i + 2]);
            if n.is_some() && unit.is_some() {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
