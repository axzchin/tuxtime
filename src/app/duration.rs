//! Duration formatting and parsing: format seconds as hours+minutes with
//! billable tenths, parse user-supplied duration strings (minutes, decimal
//! hours, clock time, am/pm shorthand) into seconds.
//!
//! Pure functions with no `App` dependency — usable from anywhere.

use chrono::Timelike;

pub(crate) fn format_duration(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let billable = format_billable(total_secs);
    if hours > 0 {
        format!("{hours}h {minutes}m ({billable})")
    } else {
        format!("{minutes}m ({billable})")
    }
}

/// Format seconds as billable units (0.1h increments, rounded up).
/// 1 minute = 0.1h, 6 minutes = 0.1h, 30 minutes = 0.5h, etc.
#[must_use]
pub fn format_billable(total_secs: u64) -> String {
    // Round up to nearest 0.1 hour (6 minutes / 360 seconds).
    format_billable_tenths(total_secs.div_ceil(360))
}

/// Format pre-computed billable tenths. Use this when summing rounded
/// values across groups so that each project+activity rounds independently
/// (1 min × 5 matters = 0.5h, not 0.1h).
#[must_use]
pub fn format_billable_tenths(tenths: u64) -> String {
    let whole = tenths / 10;
    let frac = tenths % 10;
    if whole > 0 || frac > 0 {
        format!("{whole}.{frac}h")
    } else {
        "0.0h".to_string()
    }
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
