#![allow(clippy::unwrap_used)]
//! Unit tests for the natural-language parser. Included from `src/nl/mod.rs`
//! via `mod tests;`, so `super::*` resolves to the `nl` module (re-exports
//! [`ParsedNl`], `try_parse`, `format_as_todo_txt`, and `strip_marker`).

use super::*;

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

#[test]
fn detection_requires_leading_marker() {
    // Unmarked text is never rewritten, even when it reads like prose or
    // contains former trigger words ("daily", "project").
    assert!(strip_marker("Buy milk").is_none());
    assert!(strip_marker("(A) Buy milk").is_none());
    assert!(strip_marker("Buy milk +groceries @store").is_none());
    assert!(strip_marker("Buy milk tomorrow").is_none());
    assert!(strip_marker("Pay rent monthly").is_none());
    assert!(strip_marker("Submit timesheet every friday").is_none());
    assert!(strip_marker("daily standup").is_none());
    assert!(strip_marker("project report").is_none());
    assert!(strip_marker("Buy milk due:2026-05-10").is_none());
    assert!(strip_marker("Task rec:+1w").is_none());
    assert!(strip_marker("Hidden t:-3d").is_none());
    // A leading `>` opts the whole line into NL parsing.
    assert_eq!(
        strip_marker("> Buy milk tomorrow"),
        Some("Buy milk tomorrow")
    );
    assert_eq!(strip_marker("> daily standup"), Some("daily standup"));
}

#[test]
fn strip_marker_drops_leading_sigil_and_space() {
    assert_eq!(strip_marker("> daily standup"), Some("daily standup"));
    assert_eq!(strip_marker(">   padded"), Some("padded"));
    assert_eq!(strip_marker(">"), Some(""));
    assert_eq!(strip_marker("daily standup"), None);
    assert_eq!(strip_marker(""), None);
}

#[test]
fn parses_user_example() {
    let today = d("2026-05-11");
    let input = "Pay rent monthly on the first of the month, show the todo 3 days before the due date. It's part of project home and context bank";
    let parsed = try_parse(input, today).unwrap();
    assert_eq!(parsed.body, "Pay rent");
    assert_eq!(parsed.due, Some(d("2026-06-01")));
    assert_eq!(parsed.rec.as_deref(), Some("+1m"));
    assert_eq!(parsed.threshold.as_deref(), Some("-3d"));
    assert_eq!(parsed.projects, vec!["home".to_string()]);
    assert_eq!(parsed.contexts, vec!["bank".to_string()]);
    assert_eq!(parsed.priority, None);
}

#[test]
fn formats_user_example_canonically() {
    let today = d("2026-05-11");
    let input = "Pay rent monthly on the first of the month, show the todo 3 days before the due date. It's part of project home and context bank";
    let parsed = try_parse(input, today).unwrap();
    let out = format_as_todo_txt(&parsed);
    assert_eq!(out, "Pay rent +home @bank due:2026-06-01 rec:+1m t:-3d");
}

#[test]
fn cyrillic_body_with_parenthetical_does_not_panic() {
    // Regression: a word like "дня)" is 7 bytes (three 2-byte Cyrillic
    // chars + ")"). parse_day_ordinal sliced at byte len-2, landing inside
    // the multibyte 'я' and panicking. The whole app crashed on save.
    let today = d("2026-05-17");
    let parsed = try_parse("Приготовить ужин (на 2 дня) today", today).unwrap();
    assert_eq!(parsed.due, Some(today));
    assert_eq!(parsed.body, "Приготовить ужин (на 2 дня)");
}

#[test]
fn parses_buy_milk_tomorrow() {
    let today = d("2026-05-11");
    let parsed = try_parse("Buy milk tomorrow", today).unwrap();
    assert_eq!(parsed.body, "Buy milk");
    assert_eq!(parsed.due, Some(d("2026-05-12")));
    assert_eq!(parsed.rec, None);
    assert_eq!(parsed.threshold, None);
}

#[test]
fn parses_call_mom_every_week_starting_friday() {
    let today = d("2026-05-11"); // Monday
    let parsed = try_parse(
        "Call mom every week starting Friday for project family",
        today,
    )
    .unwrap();
    assert_eq!(parsed.body, "Call mom");
    assert_eq!(parsed.rec.as_deref(), Some("+1w"));
    assert_eq!(parsed.due, Some(d("2026-05-15"))); // next Friday
    assert_eq!(parsed.projects, vec!["family".to_string()]);
}

#[test]
fn parses_annual_review_due_april_15() {
    let today = d("2026-05-11");
    let parsed = try_parse("Annual review due April 15 +work @office", today).unwrap();
    // "Annual" stays in body: we only treat "annually" as a recurrence
    // trigger ("Annual review" reads as an adjective). "due" is consumed
    // as the date marker so it doesn't survive into the body.
    assert_eq!(parsed.body, "Annual review");
    assert_eq!(parsed.due, Some(d("2027-04-15"))); // April 15 already past this year
    assert_eq!(parsed.projects, vec!["work".to_string()]);
    assert_eq!(parsed.contexts, vec!["office".to_string()]);
    assert_eq!(parsed.rec, None);
}

#[test]
fn date_marker_words_are_consumed() {
    // "due", "by", "on", "starting", "before" preceding a date are
    // consumed alongside the date phrase — none survive in the body.
    let today = d("2026-05-11");
    for input in [
        "Pay rent due Friday",
        "Pay rent by Friday",
        "Pay rent on Friday",
        "Pay rent before Friday",
        "Pay rent starting Friday",
    ] {
        let parsed = try_parse(input, today).unwrap_or_else(|| panic!("no parse for {input:?}"));
        assert_eq!(parsed.body, "Pay rent", "input: {input:?}");
        assert!(parsed.due.is_some(), "input: {input:?}");
    }
}

#[test]
fn dangling_before_with_no_date_extracts_nothing() {
    // "before" alone (no following date) is a trigger but yields no
    // extraction — caller falls through and saves as plain prose.
    let today = d("2026-05-11");
    assert!(try_parse("Pay rent before payday", today).is_none());
}

#[test]
fn parses_every_other_friday_show_one_day_before() {
    let today = d("2026-05-11");
    let parsed = try_parse(
        "Submit timesheet every other friday show 1 day before",
        today,
    )
    .unwrap();
    assert_eq!(parsed.body, "Submit timesheet");
    assert_eq!(parsed.rec.as_deref(), Some("+2w"));
    assert_eq!(parsed.threshold.as_deref(), Some("-1d"));
    assert_eq!(parsed.due, Some(d("2026-05-15")));
}

#[test]
fn idempotent_on_canonical_form() {
    let today = d("2026-05-11");
    let parsed = try_parse(
        "Pay rent monthly on the first, show 3 days before due, project home",
        today,
    )
    .unwrap();
    let canonical = format_as_todo_txt(&parsed);
    // Canonical output never carries the `>` marker, so a second Enter on it
    // falls through to the save path instead of re-parsing.
    assert!(strip_marker(&canonical).is_none());
}

#[test]
fn first_of_the_month_rolls_forward() {
    let today = d("2026-05-11");
    let parsed = try_parse("Pay rent on the first of the month", today).unwrap();
    assert_eq!(parsed.due, Some(d("2026-06-01")));
}

#[test]
fn every_monday_on_a_monday_picks_next_week() {
    let today = d("2026-05-11"); // Monday
    let parsed = try_parse("Standup every monday", today).unwrap();
    assert_eq!(parsed.rec.as_deref(), Some("+1w"));
    assert_eq!(parsed.due, Some(d("2026-05-18")));
}

#[test]
fn daily_standup_has_rec_no_due() {
    let today = d("2026-05-11");
    let parsed = try_parse("daily standup", today).unwrap();
    assert_eq!(parsed.body, "standup");
    assert_eq!(parsed.rec.as_deref(), Some("+1d"));
    assert_eq!(parsed.due, None);
}

#[test]
fn business_day_recurrence() {
    let today = d("2026-05-11");
    let parsed = try_parse("Standup every business day", today).unwrap();
    assert_eq!(parsed.rec.as_deref(), Some("+1b"));
    assert_eq!(parsed.body, "Standup");
}

#[test]
fn empty_body_falls_back_to_todo() {
    let today = d("2026-05-11");
    let parsed = try_parse("every monday", today).unwrap();
    let out = format_as_todo_txt(&parsed);
    assert!(out.starts_with("todo "));
    assert!(out.contains("rec:+1w"));
}

#[test]
fn multiple_projects_collected() {
    let today = d("2026-05-11");
    let parsed = try_parse(
        "Plan offsite tomorrow for project home and project rentals",
        today,
    )
    .unwrap();
    assert_eq!(
        parsed.projects,
        vec!["home".to_string(), "rentals".to_string()]
    );
}

#[test]
fn invalid_project_name_left_in_body() {
    // "project two words" has "two" as the candidate name. Valid tag name
    // (no spaces in "two"), so we'd actually consume "project two" — the
    // bare word "words" remains. This is the documented behavior.
    let today = d("2026-05-11");
    let parsed = try_parse("Refactor tomorrow project two words", today).unwrap();
    assert_eq!(parsed.projects, vec!["two".to_string()]);
    assert!(parsed.body.contains("words"));
}

#[test]
fn sigiled_tokens_collected() {
    let today = d("2026-05-11");
    let parsed = try_parse("Buy milk tomorrow +groceries @store", today).unwrap();
    assert_eq!(parsed.projects, vec!["groceries".to_string()]);
    assert_eq!(parsed.contexts, vec!["store".to_string()]);
    assert_eq!(parsed.body, "Buy milk");
}

#[test]
fn priority_high_priority_maps_to_a() {
    let today = d("2026-05-11");
    let parsed = try_parse("Fix bug high priority tomorrow", today).unwrap();
    assert_eq!(parsed.priority, Some('A'));
    assert_eq!(parsed.due, Some(d("2026-05-12")));
    assert_eq!(parsed.body, "Fix bug");
}

#[test]
fn leading_priority_prefix_is_recognized() {
    // "(A) " at the head of the buffer sets priority and is stripped from
    // the body. Without this pass, the body would carry the prefix and
    // format_as_todo_txt would emit "(A) (A) Buy milk ..." if the prose
    // also mentioned priority.
    let today = d("2026-05-11");
    let parsed = try_parse("(A) Buy milk tomorrow", today).unwrap();
    assert_eq!(parsed.priority, Some('A'));
    assert_eq!(parsed.body, "Buy milk");
    assert_eq!(parsed.due, Some(d("2026-05-12")));
    assert_eq!(format_as_todo_txt(&parsed), "(A) Buy milk due:2026-05-12");
}

#[test]
fn leading_priority_does_not_double_up_with_prose() {
    // If both the prefix and a prose priority phrase are present, the
    // prefix wins and the prose pass is short-circuited so the output
    // doesn't carry two "(X) " heads.
    let today = d("2026-05-11");
    let parsed = try_parse("(B) Fix bug high priority tomorrow", today).unwrap();
    assert_eq!(parsed.priority, Some('B'));
    let out = format_as_todo_txt(&parsed);
    assert_eq!(out.matches("(B)").count(), 1);
    assert!(!out.contains("(A)"));
}

#[test]
fn try_parse_returns_none_when_nothing_extracted() {
    let today = d("2026-05-11");
    // No triggers, no extraction — try_parse returns None and the caller
    // falls through to the plain save path.
    assert!(try_parse("Hello world", today).is_none());
    // Trigger fires ("every") but the recurrence phrase is unrecognizable,
    // and no other pass finds anything — extraction is still empty.
    assert!(try_parse("every gnarbax", today).is_none());
}

#[test]
fn rec_values_are_recurrence_module_compatible() {
    // Cross-check: every emitted rec: value must round-trip through the
    // recurrence parser the rest of the app uses. Catches drift if either
    // parser's grammar changes.
    let today = d("2026-05-11");
    for input in [
        "every day standup",
        "weekly review",
        "every monday meeting",
        "every 3 weeks haircut",
        "every other friday",
        "every business day check inbox",
        "yearly taxes",
        "biweekly retro",
    ] {
        let parsed = try_parse(input, today).unwrap_or_else(|| panic!("no parse for {input:?}"));
        let rec = parsed.rec.unwrap_or_else(|| panic!("no rec for {input:?}"));
        assert!(
            crate::recurrence::parse_rec_spec(&rec).is_some(),
            "rec value {rec:?} from {input:?} failed recurrence::parse_rec_spec"
        );
    }
}

#[test]
fn threshold_values_are_threshold_module_compatible() {
    let today = d("2026-05-11");
    for input in [
        "Task due tomorrow show 3 days before due",
        "Task due tomorrow 2 weeks before due",
        "Task due tomorrow show 1 month before",
    ] {
        let parsed = try_parse(input, today).unwrap_or_else(|| panic!("no parse for {input:?}"));
        let t = parsed
            .threshold
            .unwrap_or_else(|| panic!("no threshold for {input:?}"));
        assert!(
            crate::threshold::parse_threshold(&t).is_some(),
            "t value {t:?} from {input:?} failed threshold::parse_threshold"
        );
    }
}
