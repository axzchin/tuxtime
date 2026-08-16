//! Duration formatting and parsing: format seconds as hours+minutes with
//! billable rounding, parse user-supplied duration strings (minutes, decimal
//! hours, clock time, am/pm shorthand) into seconds.
//!
//! Pure functions with no `App` dependency — usable from anywhere.
//!
//! Billable rounding is configurable via `rounding_increment` (decimal hours):
//! `0.1` (six-minute units, the default), `0.25` (fifteen-minute units), or
//! `0` for no rounding (exact decimal hours shown). Rounding always rounds
//! *up* so a client can never be shorted.

use chrono::Timelike;

/// Seconds in one billable unit for the configured increment. `None` when
/// the increment is 0 — the caller shows exact decimal hours instead.
fn unit_secs(increment_hours: f64) -> Option<u64> {
    (increment_hours > 0.0).then(|| (increment_hours * 3600.0).round() as u64)
}

/// Decimal places to show for a given increment (0.1 → 1, 0.25 → 2, 1 → 0).
fn decimals_for(increment_hours: f64) -> usize {
    match format!("{increment_hours}").split_once('.') {
        Some((_, frac)) => frac.len(),
        None => 0,
    }
}

pub(crate) fn format_duration(total_secs: u64, increment_hours: f64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let billable = format_billable(total_secs, increment_hours);
    if hours > 0 {
        format!("{hours}h {minutes}m ({billable})")
    } else {
        format!("{minutes}m ({billable})")
    }
}

/// Stopwatch-style `HH:MM:SS` for a live running timer (status bar + task
/// row). Zero-padded so the readout doesn't jump width as digits change.
#[must_use]
pub fn format_clock(total_secs: u64) -> String {
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Compact human duration for narrow columns (sidebar rows): `2h 05m`, `45m`.
/// Unlike [`format_duration`] this carries no billable parenthetical — it's
/// for rows too tight to fit one.
#[must_use]
pub fn format_compact_duration(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

/// Format seconds as billable units at the configured increment, rounded up.
/// 1 minute = 0.1h, 6 minutes = 0.1h, 30 minutes = 0.5h (at 0.1); 8 minutes =
/// 0.25h (at 0.25); increment 0 shows exact decimal hours (e.g. 1.12h).
#[must_use]
pub fn format_billable(total_secs: u64, increment_hours: f64) -> String {
    format_billable_units(billable_units(total_secs, increment_hours), increment_hours)
}

/// Rounded-up billable units for a duration at the given increment. When the
/// increment is 0 the raw seconds are returned unchanged, so summing units
/// across groups reproduces the exact total (no per-group rounding applies).
#[must_use]
pub fn billable_units(total_secs: u64, increment_hours: f64) -> u64 {
    match unit_secs(increment_hours) {
        Some(unit) => total_secs.div_ceil(unit),
        None => total_secs,
    }
}

/// Format pre-computed billable units. Use this when summing rounded values
/// across groups so that each project+activity rounds independently (1 min ×
/// 5 matters = 0.5h, not 0.1h).
#[must_use]
#[allow(clippy::cast_precision_loss)] // display-only: units are tiny relative to f64 mantissa
pub fn format_billable_units(units: u64, increment_hours: f64) -> String {
    match unit_secs(increment_hours) {
        Some(_) => format!(
            "{:.decimals$}h",
            units as f64 * increment_hours,
            decimals = decimals_for(increment_hours)
        ),
        None => format!("{:.2}h", units as f64 / 3600.0),
    }
}

/// Human-readable label for a rounding increment: `0.1h`, `0.25h`, or
/// `exact` for no rounding. Used by the settings row and cycle flash.
#[must_use]
pub fn rounding_increment_label(increment_hours: f64) -> String {
    if increment_hours <= 0.0 {
        "exact".to_string()
    } else {
        format!("{increment_hours}h")
    }
}

/// Parse a `HH:MM` wall-clock string (e.g. `"17:00"`, `"09:30"`, and the
/// hand-typed `"9:00"`) into `(hour, minute)`. Returns `None` for anything
/// else — malformed config values simply disable the feature that uses them.
/// Used by the end-of-day review time and the workday coverage bounds.
#[must_use]
pub fn parse_clock(s: &str) -> Option<(u32, u32)> {
    let s = s.trim();
    let (h_part, m_part) = s.split_once(':')?;
    if h_part.is_empty() || h_part.len() > 2 || m_part.len() != 2 {
        return None;
    }
    if !h_part.bytes().all(|b| b.is_ascii_digit()) || !m_part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: u32 = h_part.parse().ok()?;
    let m: u32 = m_part.parse().ok()?;
    (h < 24 && m < 60).then_some((h, m))
}

/// Parse a user-supplied duration string into seconds. Accepts:
/// - plain minutes (no suffix): `90` → 5400s (90 min)
/// - explicit minutes: `90m` → 5400s
/// - decimal hours: `1.5` or `1.5h` → 5400s
/// - explicit seconds: `5400s` → 5400s
/// - clock time: `14:30` → duration from that time today to now
/// - am/pm shorthand: `9am`, `2pm`, `9:30am`, `2:30pm` → duration from then to now
pub(crate) fn parse_duration_input(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() {
        return 0;
    }

    // Strip unit suffix to determine the base unit.
    let (num_part, explicit_unit) = if let Some(n) = s.strip_suffix('m') {
        (n.trim(), Some('m'))
    } else if let Some(n) = s.strip_suffix('h') {
        (n.trim(), Some('h'))
    } else if let Some(n) = s.strip_suffix('s') {
        (n.trim(), Some('s'))
    } else {
        (s, None)
    };

    // Clock time with am/pm: "9am", "9:30am", "2pm", "2:30pm"
    if let Some(secs) = parse_ampm_time(s) {
        return secs;
    }

    // Clock time: "14:30" or "9:30" (no am/pm)
    if let Some((h_str, m_str)) = num_part.split_once(':') {
        if let (Ok(h), Ok(m)) = (h_str.parse::<u32>(), m_str.parse::<u32>()) {
            let now = chrono::Local::now();
            let target_secs = h * 3600 + m * 60;
            let now_secs = now.hour() * 3600 + now.minute() * 60 + now.second();
            if target_secs <= now_secs {
                return u64::from(now_secs - target_secs);
            }
            // Target is in the future — assume yesterday.
            return u64::from(now_secs + 24 * 3600 - target_secs);
        }
        return 0;
    }

    match explicit_unit {
        // Explicit seconds: "5400s"
        Some('s') => num_part.parse::<u64>().unwrap_or(0),
        // Explicit hours: "1.5h"
        Some('h') => {
            if let Ok(h) = num_part.parse::<f64>() {
                (h * 3600.0).max(0.0) as u64
            } else {
                0
            }
        }
        // Explicit minutes: "90m"
        Some('m') => num_part.parse::<u64>().map(|m| m * 60).unwrap_or(0),
        // No suffix: infer — decimal point → hours, plain integer → minutes
        None => {
            if num_part.contains('.') {
                if let Ok(h) = num_part.parse::<f64>() {
                    (h * 3600.0).max(0.0) as u64
                } else {
                    0
                }
            } else {
                // Plain integer → minutes (default for lawyers)
                num_part.parse::<u64>().map(|m| m * 60).unwrap_or(0)
            }
        }
        _ => 0,
    }
}

/// Parse am/pm clock shorthand like "9am", "2:30pm", "12p".
/// Returns the duration in seconds from that time (today, or yesterday if
/// the time is in the future) to now.
fn parse_ampm_time(s: &str) -> Option<u64> {
    let lower = s.trim().to_lowercase();
    let (time_part, is_pm) = if let Some(t) = lower.strip_suffix("am") {
        (t.trim(), false)
    } else if let Some(t) = lower.strip_suffix("pm") {
        (t.trim(), true)
    } else if let Some(t) = lower.strip_suffix('a') {
        (t.trim(), false)
    } else if let Some(t) = lower.strip_suffix('p') {
        (t.trim(), true)
    } else {
        return None;
    };

    let (hour, minute): (u32, u32) = if let Some((h, m)) = time_part.split_once(':') {
        (h.parse().ok()?, m.parse().ok()?)
    } else {
        (time_part.parse().ok()?, 0)
    };

    if hour > 12 || minute >= 60 {
        return None;
    }

    let hour_24 = match (hour, is_pm) {
        (12, false) => 0, // 12am = midnight
        (12, true) => 12, // 12pm = noon
        (h, true) if h < 12 => h + 12,
        (h, _) => h,
    };

    let now = chrono::Local::now();
    let target_secs = hour_24 * 3600 + minute * 60;
    let now_secs = now.hour() * 3600 + now.minute() * 60 + now.second();

    let diff = if target_secs <= now_secs {
        now_secs - target_secs
    } else {
        // Future — assume yesterday
        now_secs + 24 * 3600 - target_secs
    };
    Some(u64::from(diff))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billable_default_tenths_rounds_up() {
        // 6 minutes → 0.1h; 1 minute → 0.1h (round up); 30 min → 0.5h.
        assert_eq!(format_billable(360, 0.1), "0.1h");
        assert_eq!(format_billable(60, 0.1), "0.1h");
        assert_eq!(format_billable(1800, 0.1), "0.5h");
        assert_eq!(format_billable(4020, 0.1), "1.2h");
        assert_eq!(format_billable(0, 0.1), "0.0h");
    }

    #[test]
    fn billable_quarters_round_up_to_15_min() {
        // 15 min → 0.25h; 8 min → 0.25h (round up); 30 min → 0.5h;
        // 62.5 min → 5 units → 1.25h; exactly 1h → 4 units → 1.00h.
        assert_eq!(format_billable(900, 0.25), "0.25h");
        assert_eq!(format_billable(480, 0.25), "0.25h");
        assert_eq!(format_billable(1800, 0.25), "0.50h");
        assert_eq!(format_billable(3750, 0.25), "1.25h");
        assert_eq!(format_billable(3600, 0.25), "1.00h");
    }

    #[test]
    fn billable_zero_increment_shows_exact_decimal() {
        // No rounding: exact decimal hours, two decimals.
        assert_eq!(format_billable(4020, 0.0), "1.12h");
        assert_eq!(format_billable(3600, 0.0), "1.00h");
    }

    #[test]
    fn billable_units_per_group_round_independently() {
        // 1 min × 5 matters = 5 × 0.1h units = 0.5h, not the 0.1h a raw sum
        // would round to.
        let units = (0..5).map(|_| billable_units(60, 0.1)).sum::<u64>();
        assert_eq!(units, 5);
        assert_eq!(format_billable_units(units, 0.1), "0.5h");
    }

    #[test]
    fn rounding_increment_label_renders() {
        assert_eq!(rounding_increment_label(0.1), "0.1h");
        assert_eq!(rounding_increment_label(0.25), "0.25h");
        assert_eq!(rounding_increment_label(0.0), "exact");
    }

    #[test]
    fn format_duration_embeds_rounded_billable() {
        assert_eq!(format_duration(4020, 0.1), "1h 7m (1.2h)");
        assert_eq!(format_duration(4020, 0.25), "1h 7m (1.25h)");
        assert_eq!(format_duration(4020, 0.0), "1h 7m (1.12h)");
    }

    #[test]
    fn parse_clock_accepts_valid_and_rejects_garbage() {
        assert_eq!(parse_clock("17:00"), Some((17, 0)));
        assert_eq!(parse_clock("09:30"), Some((9, 30)));
        // A hand-typed single-digit hour is fine (normalized at the config
        // layer when serialized back).
        assert_eq!(parse_clock("9:00"), Some((9, 0)));
        assert_eq!(parse_clock("00:00"), Some((0, 0)));
        assert_eq!(parse_clock("23:59"), Some((23, 59)));
        // Malformed: bad separator, one-digit minutes, non-digits, out of
        // range, empty.
        assert_eq!(parse_clock("17-00"), None);
        assert_eq!(parse_clock("9:3"), None);
        assert_eq!(parse_clock("banana"), None);
        assert_eq!(parse_clock("25:00"), None);
        assert_eq!(parse_clock("17:60"), None);
        assert_eq!(parse_clock(""), None);
        assert_eq!(parse_clock("  "), None);
    }
}
