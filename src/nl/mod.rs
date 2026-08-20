//! Natural-language parser for the add-todo draft.
//!
//! When the user prefixes the add buffer with [`MARKER`] ("`> Pay rent
//! monthly on the first, show 3 days before due, project home`"), this
//! module extracts the structured todo.txt metadata so the caller can rewrite
//! the buffer into canonical form for the user to review. Unmarked buffers
//! are left untouched.
//!
//! Pure logic — no I/O, no app state. The crate-level wiring lives in
//! `app::actions::add_from_draft` and `inbox::canonicalize_line`.
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
//! Detection is explicit: a buffer is only parsed when it starts with the
//! [`MARKER`] sigil (`>`). Unmarked text is saved verbatim, so the parser
//! never rewrites a narrative by surprise — and the canonical output it
//! produces never carries the marker, so a second Enter on it falls through
//! to the existing save path.

use chrono::NaiveDate;

mod passes;
mod scratch;
mod tokenizer;
mod types;

pub use types::ParsedNl;

use passes::{
    pass_date, pass_leading_priority, pass_priority, pass_project_context, pass_recurrence,
    pass_sigiled, pass_threshold,
};
use scratch::Scratch;

/// The leading sigil that opts a buffer into natural-language parsing.
/// Everything that doesn't start with it is saved verbatim, so the parser
/// never rewrites a narrative by surprise.
pub const MARKER: char = '>';

/// Strip the leading [`MARKER`] (and any whitespace after it) so the parser
/// never treats the sigil as body text. Returns `None` when the buffer is
/// not marked. Callers also use this to drop the marker when the marked text
/// yields no structured extraction, so a stray `>` never lands in the saved
/// task.
#[must_use]
pub fn strip_marker(text: &str) -> Option<&str> {
    let t = text.trim_start();
    let rest = t.strip_prefix(MARKER)?;
    Some(rest.trim_start())
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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
