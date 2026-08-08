//! Shared wall-clock formatting helpers.
//!
//! The app resolves "today" against the local clock in several places (TUI
//! startup, midnight rollover, one-shot commands). These helpers keep the
//! format strings in one place so a drift can't silently change which day a
//! task lands on.

/// Today's date as a canonical ISO `YYYY-MM-DD` string in the local timezone.
#[must_use]
pub fn today_iso() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// The current local time as `YYYY-MM-DDTHH:MM:SS` — the `start:` tag format.
#[must_use]
pub fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn today_is_iso_date_length_and_shape() {
        let today = today_iso();
        assert_eq!(today.len(), 10);
        assert!(today.chars().nth(4) == Some('-'));
        assert!(today.chars().nth(7) == Some('-'));
    }

    #[test]
    fn now_is_full_timestamp_shape() {
        // Assert each value's shape independently rather than cross-
        // referencing two wall-clock reads — a midnight rollover between
        // the two observations would otherwise flake the test.
        let now = now_iso();
        assert_eq!(now.len(), 19);
        assert!(now[10..11].starts_with('T'));
        assert!(chrono::NaiveDateTime::parse_from_str(&now, "%Y-%m-%dT%H:%M:%S").is_ok());
        assert!(today_iso().parse::<chrono::NaiveDate>().is_ok());
    }
}
