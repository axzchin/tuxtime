//! Shared data types for the NL parser.

use chrono::NaiveDate;

/// Structured fields extracted from a prose draft. Each `Option` field is
/// `None` when the user didn't say anything about that aspect; `Vec` fields
/// are empty for the same reason. `body` is the input with all recognized
/// phrases stripped and whitespace collapsed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedNl {
    pub body: String,
    pub due: Option<NaiveDate>,
    pub rec: Option<String>,
    pub threshold: Option<String>,
    pub projects: Vec<String>,
    pub contexts: Vec<String>,
    pub priority: Option<char>,
}
