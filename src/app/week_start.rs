//! Week start preference (Sunday or Monday). Used for timesheet weekly view
//! and grouped list views. Persisted in the config file.

use core::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeekStart {
    Sunday,
    Monday,
}

impl WeekStart {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            WeekStart::Sunday => "sunday",
            WeekStart::Monday => "monday",
        }
    }
}

impl fmt::Display for WeekStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WeekStart {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sunday" => Ok(WeekStart::Sunday),
            "monday" => Ok(WeekStart::Monday),
            _ => Err(()),
        }
    }
}
