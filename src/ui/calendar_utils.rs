//! Calendar utility functions: grid generation, month names, focused date
//! formatting, and keyboard shortcut footer. Shared between the insert-dialog
//! calendar overlay ([`super::dialog`]) and the timesheet date picker
//! ([`super::timesheet_render`]).

use chrono::{Datelike, NaiveDate, Weekday};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::app::WeekStart;
use crate::theme::Theme;

/// Number of days in `month` (1-12) for `year`.
pub(crate) fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = NaiveDate::from_ymd_opt(ny, nm, 1);
    let first_this = NaiveDate::from_ymd_opt(year, month, 1);
    match (first_next, first_this) {
        (Some(n), Some(t)) => (n - t).num_days() as u32,
        _ => 30,
    }
}

/// Build the calendar grid for `first_of_month`'s month as rows of 7 columns.
/// Each cell is `Some(day)` for a real day-of-month or `None` for a blank
/// padding cell.
pub(crate) fn calendar_cells(
    first_of_month: NaiveDate,
    week_start: WeekStart,
) -> Vec<[Option<u32>; 7]> {
    let lead = i64::from(match week_start {
        WeekStart::Sunday => first_of_month.weekday().num_days_from_sunday(),
        WeekStart::Monday => first_of_month.weekday().num_days_from_monday(),
    });
    let days = i64::from(days_in_month(first_of_month.year(), first_of_month.month()));

    let mut weeks = Vec::new();
    let mut start = -lead;
    while start < days {
        let mut row = [None; 7];
        for (col, cell) in row.iter_mut().enumerate() {
            let idx = start + col as i64;
            if (0..days).contains(&idx) {
                *cell = Some((idx + 1) as u32);
            }
        }
        weeks.push(row);
        start += 7;
    }
    weeks
}

/// Full month name (e.g. `"January"`) for month 1-12.
pub(crate) fn month_name(m: u32) -> &'static str {
    match m {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "?",
    }
}

/// Human-readable date label for the focused cell (e.g. `"Mon 2026-08-03"`).
pub(crate) fn format_focused(d: NaiveDate) -> String {
    let dow = match d.weekday() {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    };
    format!(
        "{dow} {year}-{month:02}-{day:02}",
        year = d.year(),
        month = d.month(),
        day = d.day()
    )
}

/// Keyboard shortcut legend shown below the calendar overlay.
pub(crate) fn calendar_footer<'a>(theme: &Theme) -> Line<'a> {
    let chip = |k: &'static str, label: &'static str| -> Vec<Span<'a>> {
        vec![
            Span::styled(
                k,
                Style::default().fg(theme.dim).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {label} "), Style::default().fg(theme.dim)),
        ]
    };
    let mut spans = vec![Span::raw("  ")];
    spans.extend(chip("t", "today"));
    spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
    spans.extend(chip("T", "tmw"));
    spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
    spans.extend(chip("w", "+1w"));
    spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
    spans.extend(chip("m", "+1mo"));
    spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
    spans.extend(chip("x", "clear"));
    Line::from(spans).style(Style::default().bg(theme.panel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_cells_spacing_for_known_month() {
        // May 2026 starts on a Friday (weekday 5).
        let first = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let weeks = calendar_cells(first, WeekStart::Sunday);
        // With Sunday start, the first row should have 5 blank cells then day 1.
        assert_eq!(weeks[0][0], None);
        assert_eq!(weeks[0][1], None);
        assert_eq!(weeks[0][2], None);
        assert_eq!(weeks[0][3], None);
        assert_eq!(weeks[0][4], None);
        assert_eq!(weeks[0][5], Some(1));
        assert_eq!(weeks[0][6], Some(2));
    }

    #[test]
    fn calendar_cells_monday_start_for_known_month() {
        // May 2026 starts on a Friday (ISO weekday 5 = Friday, Mon=1).
        let first = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let weeks = calendar_cells(first, WeekStart::Monday);
        // With Monday start, the first row has 4 blank cells then day 1.
        assert_eq!(weeks[0][0], None);
        assert_eq!(weeks[0][1], None);
        assert_eq!(weeks[0][2], None);
        assert_eq!(weeks[0][3], None);
        assert_eq!(weeks[0][4], Some(1));
        assert_eq!(weeks[0][5], Some(2));
        assert_eq!(weeks[0][6], Some(3));
    }

    #[test]
    fn month_name_january() {
        assert_eq!(month_name(1), "January");
        assert_eq!(month_name(12), "December");
        assert_eq!(month_name(13), "?");
    }

    #[test]
    fn days_in_month_standard() {
        assert_eq!(days_in_month(2026, 1), 31);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29); // leap year
        assert_eq!(days_in_month(2026, 4), 30);
    }
}
