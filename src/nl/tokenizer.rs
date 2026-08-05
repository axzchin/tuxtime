//! Pure lexical helpers: word segmentation, case folding, and the small
//! vocabulary lookups (numbers, weekdays, months, units, ordinals) shared by
//! the NL passes. No dependencies on [`super::Scratch`] or the pass pipeline.

use chrono::Weekday;

/// ASCII-only lowercasing. Non-ASCII bytes pass through untouched so byte
/// indices stay aligned between the original text and the lowered copy.
pub(super) fn ascii_lower(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii() {
                c.to_ascii_lowercase()
            } else {
                c
            }
        })
        .collect()
}

/// Byte ranges of whitespace-delimited words in `s`.
pub(super) fn compute_words(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut last_end = 0;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            if let Some(st) = start.take() {
                out.push((st, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
        last_end = i + c.len_utf8();
    }
    if let Some(st) = start {
        out.push((st, last_end));
    }
    out
}

/// Parse a number word or digit string (e.g. `"3"`, `"three"`).
pub(super) fn parse_number(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse::<u32>() {
        return Some(n);
    }
    word_number(s)
}

/// Map number words one..=ten to their numeric value.
pub(super) fn word_number(s: &str) -> Option<u32> {
    Some(match s {
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        _ => return None,
    })
}

/// Map a unit word to its single-letter todo.txt form (`day` → `d`).
pub(super) fn unit_char(s: &str) -> Option<char> {
    Some(match s {
        "day" | "days" => 'd',
        "week" | "weeks" => 'w',
        "month" | "months" => 'm',
        "year" | "years" => 'y',
        _ => return None,
    })
}

/// Map a weekday name or abbreviation to its [`Weekday`].
pub(super) fn parse_weekday(s: &str) -> Option<Weekday> {
    Some(match s {
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" | "tues" => Weekday::Tue,
        "wednesday" | "wed" => Weekday::Wed,
        "thursday" | "thu" | "thurs" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "saturday" | "sat" => Weekday::Sat,
        "sunday" | "sun" => Weekday::Sun,
        _ => return None,
    })
}

/// Map a month name or abbreviation to its 1-based month number.
pub(super) fn parse_month(s: &str) -> Option<u32> {
    Some(match s {
        "january" | "jan" => 1,
        "february" | "feb" => 2,
        "march" | "mar" => 3,
        "april" | "apr" => 4,
        "may" => 5,
        "june" | "jun" => 6,
        "july" | "jul" => 7,
        "august" | "aug" => 8,
        "september" | "sep" | "sept" => 9,
        "october" | "oct" => 10,
        "november" | "nov" => 11,
        "december" | "dec" => 12,
        _ => return None,
    })
}

/// Parse a day-of-month: digits, ordinals (`"15th"`), or words (`"fifteenth"`).
pub(super) fn parse_day_ordinal(s: &str) -> Option<u32> {
    if let Ok(n) = s.parse::<u32>() {
        if (1..=31).contains(&n) {
            return Some(n);
        }
        return None;
    }
    // "1st", "2nd", "3rd", "15th". strip_suffix matches by content, so it is
    // char-boundary safe — a word ending in a multibyte char (e.g. "дня)")
    // simply won't match an ASCII ordinal suffix instead of panicking.
    if let Some(num) = ["st", "nd", "rd", "th"]
        .iter()
        .find_map(|suf| s.strip_suffix(suf))
        && let Ok(n) = num.parse::<u32>()
        && (1..=31).contains(&n)
    {
        return Some(n);
    }
    Some(match s {
        "first" => 1,
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "fifth" => 5,
        "sixth" => 6,
        "seventh" => 7,
        "eighth" => 8,
        "ninth" => 9,
        "tenth" => 10,
        "eleventh" => 11,
        "twelfth" => 12,
        "thirteenth" => 13,
        "fourteenth" => 14,
        "fifteenth" => 15,
        "sixteenth" => 16,
        "seventeenth" => 17,
        "eighteenth" => 18,
        "nineteenth" => 19,
        "twentieth" => 20,
        "thirtieth" => 30,
        _ => return None,
    })
}
