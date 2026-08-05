//! The six structured-extraction passes run over the scratch buffer in order:
//! leading priority, sigiled tokens, threshold, recurrence, date, then
//! project/context and priority words. Each pass consumes the byte ranges it
//! recognizes so later passes and the final body cleanup skip them.

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};

use crate::todo;

use super::scratch::Scratch;
use super::tokenizer::{parse_day_ordinal, parse_month, parse_number, parse_weekday, unit_char};
use super::types::ParsedNl;

// ---------------------------------------------------------------------------
// Pass 0: leading "(X) " priority prefix
// ---------------------------------------------------------------------------

/// Strip a leading `(X) ` priority token if the user typed canonical priority
/// syntax inside an otherwise prose buffer (e.g. `"(A) Buy milk tomorrow"`).
pub(super) fn pass_leading_priority(scratch: &mut Scratch, p: &mut ParsedNl) {
    let bytes = scratch.text.as_bytes();
    if bytes.len() >= 4
        && bytes[0] == b'('
        && bytes[1].is_ascii_uppercase()
        && bytes[2] == b')'
        && bytes[3] == b' '
    {
        p.priority = Some(bytes[1] as char);
        scratch.mark(0, 4);
    }
}

// ---------------------------------------------------------------------------
// Pass 1: sigiled tokens (+proj, @ctx)
// ---------------------------------------------------------------------------

pub(super) fn pass_sigiled(scratch: &mut Scratch, p: &mut ParsedNl) {
    let words = scratch.word_cache.clone();
    for (s, e) in words {
        if !scratch.is_live(s, e) {
            continue;
        }
        let tok = scratch.word_orig((s, e));
        if let Some(name) = tok.strip_prefix('+') {
            push_unique(&mut p.projects, name);
            scratch.mark(s, e);
        } else if let Some(name) = tok.strip_prefix('@') {
            push_unique(&mut p.contexts, name);
            scratch.mark(s, e);
        }
    }
}

pub(super) fn push_unique(out: &mut Vec<String>, name: &str) {
    if name.is_empty() || !todo::is_valid_tag_name(name) {
        return;
    }
    if !out.iter().any(|x| x == name) {
        out.push(name.to_string());
    }
}

// ---------------------------------------------------------------------------
// Pass 2: threshold ("show N (day|week|month)s? before [the] [due [date]]")
// ---------------------------------------------------------------------------

pub(super) fn pass_threshold(scratch: &mut Scratch, p: &mut ParsedNl) {
    let words = scratch.word_cache.clone();
    let mut i = 0;
    while i + 2 < words.len() {
        if !scratch.is_live(words[i].0, words[i].1) {
            i += 1;
            continue;
        }
        let Some(n) = parse_number(scratch.word_lc(words[i])) else {
            i += 1;
            continue;
        };
        let Some(unit) = unit_char(scratch.word_lc(words[i + 1])) else {
            i += 1;
            continue;
        };
        // Only d/w/m for threshold (years are not in the t: grammar).
        if !matches!(unit, 'd' | 'w' | 'm') {
            i += 1;
            continue;
        }
        if scratch.word_lc(words[i + 2]) != "before" {
            i += 1;
            continue;
        }

        // Look backward for "show [the (todo|task|item)] [me|it]" preamble.
        let mut start_word = i;
        const PREAMBLE: &[&str] = &["show", "the", "todo", "task", "item", "me", "it"];
        let mut saw_show = false;
        while start_word > 0 {
            let w = scratch.word_lc(words[start_word - 1]);
            if PREAMBLE.contains(&w) {
                if w == "show" {
                    saw_show = true;
                }
                start_word -= 1;
            } else {
                break;
            }
        }
        if !saw_show {
            start_word = i;
        }

        // Look forward through "[the] [due] [date]".
        let mut end_word = i + 3;
        const TRAILERS: &[&str] = &["the", "due", "date"];
        while end_word < words.len() {
            let w = scratch.word_lc(words[end_word]);
            if TRAILERS.contains(&w) {
                end_word += 1;
            } else {
                break;
            }
        }

        let start_byte = words[start_word].0;
        let end_byte = words[end_word - 1].1;
        scratch.mark(start_byte, end_byte);
        p.threshold = Some(format!("-{n}{unit}"));
        return;
    }
}

// ---------------------------------------------------------------------------
// Pass 3: recurrence
// ---------------------------------------------------------------------------

pub(super) fn pass_recurrence(scratch: &mut Scratch, p: &mut ParsedNl) -> Option<Weekday> {
    let words = scratch.word_cache.clone();
    for i in 0..words.len() {
        if !scratch.is_live(words[i].0, words[i].1) {
            continue;
        }
        let w = scratch.word_lc(words[i]);
        let standalone = match w {
            "daily" => Some(("+1d".to_string(), 1, None)),
            "weekly" => Some(("+1w".to_string(), 1, None)),
            "biweekly" => Some(("+2w".to_string(), 1, None)),
            "monthly" => Some(("+1m".to_string(), 1, None)),
            "yearly" | "annually" => Some(("+1y".to_string(), 1, None)),
            _ => None,
        };
        let (rec, count, wh) = if let Some(s) = standalone {
            s
        } else if w == "every" || w == "each" {
            match parse_every_phrase(scratch, &words, i) {
                Some(v) => v,
                None => continue,
            }
        } else {
            continue;
        };
        let start_byte = words[i].0;
        let end_byte = words[i + count - 1].1;
        scratch.mark(start_byte, end_byte);
        p.rec = Some(rec);
        return wh;
    }
    None
}

/// Parse `every <...>` starting at index `i`. Returns `(rec_value, word_count, weekday_hint)`
/// on success. `word_count` includes `every` itself.
pub(super) fn parse_every_phrase(
    scratch: &Scratch,
    words: &[(usize, usize)],
    i: usize,
) -> Option<(String, usize, Option<Weekday>)> {
    if i + 1 >= words.len() {
        return None;
    }
    let w1 = scratch.word_lc(words[i + 1]);

    if w1 == "weekday" {
        return Some(("+1b".to_string(), 2, None));
    }

    if w1 == "business" {
        if i + 2 < words.len() {
            let w2 = scratch.word_lc(words[i + 2]);
            if w2 == "day" || w2 == "days" {
                return Some(("+1b".to_string(), 3, None));
            }
        }
        return None;
    }

    if w1 == "other" {
        if i + 2 >= words.len() {
            return None;
        }
        let w2 = scratch.word_lc(words[i + 2]);
        if let Some(wd) = parse_weekday(w2) {
            return Some(("+2w".to_string(), 3, Some(wd)));
        }
        let unit = match w2 {
            "day" | "days" => 'd',
            "week" | "weeks" => 'w',
            "month" | "months" => 'm',
            "year" | "years" => 'y',
            _ => return None,
        };
        return Some((format!("+2{unit}"), 3, None));
    }

    if let Some(wd) = parse_weekday(w1) {
        return Some(("+1w".to_string(), 2, Some(wd)));
    }

    if let Some(n) = parse_number(w1) {
        if i + 2 >= words.len() {
            return None;
        }
        let unit = match scratch.word_lc(words[i + 2]) {
            "day" | "days" => 'd',
            "week" | "weeks" => 'w',
            "month" | "months" => 'm',
            "year" | "years" => 'y',
            _ => return None,
        };
        return Some((format!("+{n}{unit}"), 3, None));
    }

    let unit = match w1 {
        "day" => 'd',
        "week" => 'w',
        "month" => 'm',
        "year" => 'y',
        _ => return None,
    };
    Some((format!("+1{unit}"), 2, None))
}

// ---------------------------------------------------------------------------
// Pass 4: date
// ---------------------------------------------------------------------------

pub(super) fn pass_date(
    scratch: &mut Scratch,
    p: &mut ParsedNl,
    today: NaiveDate,
    weekday_hint: Option<Weekday>,
) {
    let words = scratch.word_cache.clone();
    for i in 0..words.len() {
        if !scratch.is_live(words[i].0, words[i].1) {
            continue;
        }
        if let Some((date, count)) = match_date_at(scratch, &words, i, today) {
            let start_byte = words[i].0;
            let end_byte = words[i + count - 1].1;
            scratch.mark(start_byte, end_byte);
            p.due = Some(date);
            return;
        }
    }
    if p.due.is_none()
        && let Some(wd) = weekday_hint
        && let Some(d) = next_weekday(today, wd, true)
    {
        p.due = Some(d);
    }
}

/// Try every supported date phrase starting at `words[i]`. Returns the
/// resolved date and the number of words to consume.
pub(super) fn match_date_at(
    scratch: &Scratch,
    words: &[(usize, usize)],
    i: usize,
    today: NaiveDate,
) -> Option<(NaiveDate, usize)> {
    let w = scratch.word_lc(words[i]);

    if let Ok(d) = NaiveDate::parse_from_str(w, "%Y-%m-%d") {
        return Some((d, 1));
    }

    if w == "today" || w == "tonight" {
        return Some((today, 1));
    }
    if w == "tomorrow" {
        return Some((today.checked_add_days(Days::new(1))?, 1));
    }
    if w == "yesterday" {
        return Some((today.checked_sub_days(Days::new(1))?, 1));
    }

    // Marker words that introduce a date phrase: "due April 15", "on Friday",
    // "by the 15th", "starting Friday", "before December 5". The marker is
    // consumed along with the date so it doesn't survive into the body. Any
    // "before" still standing at this point has already been ignored by the
    // threshold pass (which would have consumed "N <unit> before [trailers]").
    if matches!(w, "starting" | "on" | "due" | "by" | "before")
        && let Some((d, count)) = next_alive_match(scratch, words, i + 1, today)
    {
        return Some((d, 1 + count));
    }

    if (w == "this" || w == "next")
        && i + 1 < words.len()
        && let Some(wd) = parse_weekday(scratch.word_lc(words[i + 1]))
    {
        let strict = w == "next";
        if let Some(d) = next_weekday(today, wd, strict) {
            return Some((d, 2));
        }
    }

    if let Some(wd) = parse_weekday(w)
        && let Some(d) = next_weekday(today, wd, false)
    {
        return Some((d, 1));
    }

    // "in N <unit>s?"
    if w == "in"
        && i + 2 < words.len()
        && let Some(n) = parse_number(scratch.word_lc(words[i + 1]))
    {
        let unit = scratch.word_lc(words[i + 2]);
        if let Some(d) = advance_from(today, n, unit) {
            return Some((d, 3));
        }
    }

    // "N <unit>s? from (now|today)"
    if let Some(n) = parse_number(w)
        && i + 3 < words.len()
    {
        let unit = scratch.word_lc(words[i + 1]);
        let from = scratch.word_lc(words[i + 2]);
        let nowt = scratch.word_lc(words[i + 3]);
        if from == "from"
            && (nowt == "now" || nowt == "today")
            && let Some(d) = advance_from(today, n, unit)
        {
            return Some((d, 4));
        }
    }

    // "MONTH D[ord]?(, YYYY)?"
    if let Some(month) = parse_month(w)
        && i + 1 < words.len()
        && let Some(day) = parse_day_ordinal(scratch.word_lc(words[i + 1]))
    {
        let (year, consumed) = match try_parse_year(scratch, words, i + 2) {
            Some(y) => (y, 3),
            None => (today.year(), 2),
        };
        if let Some(d) = NaiveDate::from_ymd_opt(year, month, day) {
            let rolled = if consumed == 2 && d < today {
                NaiveDate::from_ymd_opt(year + 1, month, day).unwrap_or(d)
            } else {
                d
            };
            return Some((rolled, consumed));
        }
    }

    // "D[ord] (of)? MONTH(, YYYY)?"
    if let Some(day) = parse_day_ordinal(w) {
        let mut j = i + 1;
        if j < words.len() && scratch.word_lc(words[j]) == "of" {
            j += 1;
        }
        if j < words.len()
            && let Some(month) = parse_month(scratch.word_lc(words[j]))
        {
            let (year, year_extra) = match try_parse_year(scratch, words, j + 1) {
                Some(y) => (y, 1),
                None => (today.year(), 0),
            };
            let consumed = j - i + 1 + year_extra;
            if let Some(d) = NaiveDate::from_ymd_opt(year, month, day) {
                let rolled = if year_extra == 0 && d < today {
                    NaiveDate::from_ymd_opt(year + 1, month, day).unwrap_or(d)
                } else {
                    d
                };
                return Some((rolled, consumed));
            }
        }
    }

    // "the (Nth|first|...) (of (the|next) month)?"
    if w == "the"
        && i + 1 < words.len()
        && let Some(day) = parse_day_ordinal(scratch.word_lc(words[i + 1]))
    {
        let (date, consumed) = resolve_ordinal_month_phrase(scratch, words, i + 2, today, day);
        return Some((date, 2 + consumed));
    }

    // "(first|1st) of (the|next) month"
    if (w == "first" || w == "1st") && i + 3 < words.len() {
        let w1 = scratch.word_lc(words[i + 1]);
        let w2 = scratch.word_lc(words[i + 2]);
        let w3 = scratch.word_lc(words[i + 3]);
        if w1 == "of" && (w2 == "the" || w2 == "next") && w3 == "month" {
            let next_month = w2 == "next";
            let target = if next_month {
                today.checked_add_months(Months::new(1))?
            } else {
                today
            };
            if let Some(d) = NaiveDate::from_ymd_opt(target.year(), target.month(), 1) {
                let rolled = if !next_month && d < today {
                    today
                        .checked_add_months(Months::new(1))
                        .and_then(|n| NaiveDate::from_ymd_opt(n.year(), n.month(), 1))
                        .unwrap_or(d)
                } else {
                    d
                };
                return Some((rolled, 4));
            }
        }
    }

    None
}

/// Recurse into the date matcher at `i`, skipping over any consumed words.
/// Used by the `starting`/`on` wrappers so they can prefix a real date phrase.
pub(super) fn next_alive_match(
    scratch: &Scratch,
    words: &[(usize, usize)],
    mut i: usize,
    today: NaiveDate,
) -> Option<(NaiveDate, usize)> {
    while i < words.len() && !scratch.is_live(words[i].0, words[i].1) {
        i += 1;
    }
    if i >= words.len() {
        return None;
    }
    match_date_at(scratch, words, i, today)
}

pub(super) fn try_parse_year(scratch: &Scratch, words: &[(usize, usize)], i: usize) -> Option<i32> {
    if i >= words.len() {
        return None;
    }
    // `word_lc` already strips trailing punctuation, which handles the
    // common "April 15, 2026" shape (the comma sticks to "15,").
    let y: i32 = scratch.word_lc(words[i]).parse().ok()?;
    if (1900..=9999).contains(&y) {
        Some(y)
    } else {
        None
    }
}

pub(super) fn resolve_ordinal_month_phrase(
    scratch: &Scratch,
    words: &[(usize, usize)],
    j: usize,
    today: NaiveDate,
    day: u32,
) -> (NaiveDate, usize) {
    // After the ordinal: optional "of (the|next) month".
    let mut extra = 0;
    let mut next_month = false;
    if j < words.len() && scratch.word_lc(words[j]) == "of" {
        if j + 2 < words.len() {
            let w1 = scratch.word_lc(words[j + 1]);
            let w2 = scratch.word_lc(words[j + 2]);
            if (w1 == "the" || w1 == "next") && w2 == "month" {
                if w1 == "next" {
                    next_month = true;
                }
                extra = 3;
            }
        }
        if extra == 0 && j + 1 < words.len() && scratch.word_lc(words[j + 1]) == "month" {
            extra = 2;
        }
    }

    let target = if next_month {
        today.checked_add_months(Months::new(1)).unwrap_or(today)
    } else {
        today
    };
    let candidate = NaiveDate::from_ymd_opt(target.year(), target.month(), day);
    let resolved = match candidate {
        Some(d) if !next_month && d < today => today
            .checked_add_months(Months::new(1))
            .and_then(|n| NaiveDate::from_ymd_opt(n.year(), n.month(), day))
            .unwrap_or(d),
        Some(d) => d,
        None => today,
    };
    (resolved, extra)
}

pub(super) fn advance_from(today: NaiveDate, n: u32, unit: &str) -> Option<NaiveDate> {
    let unit_char = unit_char(unit)?;
    match unit_char {
        'd' => today.checked_add_days(Days::new(u64::from(n))),
        'w' => today.checked_add_days(Days::new(u64::from(n) * 7)),
        'm' => today.checked_add_months(Months::new(n)),
        'y' => today.checked_add_months(Months::new(n.checked_mul(12)?)),
        _ => None,
    }
}

/// Next occurrence of `target` weekday. With `strict = true`, today is
/// skipped (so "every monday" on a Monday rolls forward by 7 days).
pub(super) fn next_weekday(today: NaiveDate, target: Weekday, strict: bool) -> Option<NaiveDate> {
    let cur = today.weekday().num_days_from_monday();
    let tgt = target.num_days_from_monday();
    let mut diff = (tgt + 7 - cur) % 7;
    if diff == 0 && strict {
        diff = 7;
    }
    today.checked_add_days(Days::new(u64::from(diff)))
}

// ---------------------------------------------------------------------------
// Pass 5: project / context prose
// ---------------------------------------------------------------------------

pub(super) fn pass_project_context(scratch: &mut Scratch, p: &mut ParsedNl) {
    let words = scratch.word_cache.clone();
    let mut i = 0;
    while i < words.len() {
        if !scratch.is_live(words[i].0, words[i].1) {
            i += 1;
            continue;
        }
        let w = scratch.word_lc(words[i]);
        let is_project = w == "project" || w == "proj";
        let is_context = w == "context" || w == "ctx";
        if !is_project && !is_context {
            i += 1;
            continue;
        }
        // Find the next live word as the name.
        let mut name_idx = i + 1;
        while name_idx < words.len() && !scratch.is_live(words[name_idx].0, words[name_idx].1) {
            name_idx += 1;
        }
        if name_idx >= words.len() {
            i += 1;
            continue;
        }
        let name = scratch.word_orig(words[name_idx]).to_string();
        if !todo::is_valid_tag_name(&name) {
            i += 1;
            continue;
        }
        // Walk back over connector words ("and", "part", "of", "for", "in", "it's", "the").
        const CONNECTORS: &[&str] = &[
            "and", "or", "part", "of", "for", "in", "the", "it's", "its", "a", "an",
        ];
        let mut start_word = i;
        while start_word > 0 {
            let prev_range = words[start_word - 1];
            if !scratch.is_live(prev_range.0, prev_range.1) {
                break;
            }
            let prev = scratch.word_lc(prev_range);
            if CONNECTORS.contains(&prev) {
                start_word -= 1;
            } else {
                break;
            }
        }
        let end_byte = words[name_idx].1;
        scratch.mark(words[start_word].0, end_byte);
        if is_project {
            push_unique(&mut p.projects, &name);
        } else {
            push_unique(&mut p.contexts, &name);
        }
        i = name_idx + 1;
    }
}

// ---------------------------------------------------------------------------
// Pass 6: priority words
// ---------------------------------------------------------------------------

pub(super) fn pass_priority(scratch: &mut Scratch, p: &mut ParsedNl) {
    if p.priority.is_some() {
        return;
    }
    let words = scratch.word_cache.clone();
    for i in 0..words.len() {
        if !scratch.is_live(words[i].0, words[i].1) {
            continue;
        }
        let w = scratch.word_lc(words[i]);
        let prio = match w {
            "high" | "highest" if next_lc(scratch, &words, i + 1) == Some("priority") => {
                Some(('A', 2))
            }
            "medium" | "med" if next_lc(scratch, &words, i + 1) == Some("priority") => {
                Some(('B', 2))
            }
            "low" if next_lc(scratch, &words, i + 1) == Some("priority") => Some(('C', 2)),
            "priority" => match next_lc(scratch, &words, i + 1) {
                Some("a") => Some(('A', 2)),
                Some("b") => Some(('B', 2)),
                Some("c") => Some(('C', 2)),
                Some("high" | "highest") => Some(('A', 2)),
                Some("medium" | "med") => Some(('B', 2)),
                Some("low") => Some(('C', 2)),
                _ => None,
            },
            _ => None,
        };
        if let Some((c, count)) = prio {
            scratch.mark(words[i].0, words[i + count - 1].1);
            p.priority = Some(c);
            return;
        }
    }
}

pub(super) fn next_lc<'a>(
    scratch: &'a Scratch,
    words: &[(usize, usize)],
    i: usize,
) -> Option<&'a str> {
    words.get(i).map(|r| scratch.word_lc(*r))
}
