use std::path::{Path, PathBuf};

/// Why a line couldn't be parsed into a `Task`. Only `Empty` exists today —
/// the parser is permissive enough that almost anything else produces a
/// (possibly weird) `Task`. Kept as an enum so we can add reasons later
/// without changing every call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Empty,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ParseError::Empty => "empty",
        })
    }
}

/// Why a `+project` / `@context` mutation was rejected. `Invalid` covers
/// names that would break tokenization (whitespace, sigils, colons); `Parse`
/// would fire only if a constructed line failed to re-parse, which the
/// validators ensure cannot happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagError {
    Invalid,
    Parse(ParseError),
}

impl std::fmt::Display for TagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TagError::Invalid => f.write_str("invalid name"),
            TagError::Parse(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Task {
    pub raw: String,
    pub clean_raw: String,
    pub done: bool,
    pub done_date: Option<String>,
    pub priority: Option<char>,
    pub created_date: Option<String>,
    pub projects: Vec<String>,
    pub contexts: Vec<String>,
    pub due: Option<String>,
    /// Raw value of the `rec:` tag if present, e.g. `"+1m"` or `"3b"`. Stored
    /// as the unparsed string so a malformed value round-trips intact through
    /// `serialize` — only the spawn-on-complete code path needs to parse it.
    pub rec: Option<String>,
    /// Raw value of the `t:` (threshold) tag if present, e.g. `"2026-08-01"`
    /// or `"-3d"`. Stored unparsed for round-trip integrity; the visibility
    /// filter parses it on demand via `crate::threshold`.
    pub threshold: Option<String>,
    pub notes: Vec<String>,
    /// ISO 8601 datetime with seconds — present only while a timer is
    /// actively running on this task. `None` when no timer is active.
    pub start: Option<String>,
    /// Accumulated tracked time in integer seconds. `None` or `0` means no
    /// time has been tracked yet. Incremented when a timer is stopped.
    pub dur: Option<u64>,
    /// Billing status. `None` or `Some("y")` = billable (default).
    /// `Some("n")` = non-billable. Filtered from narrative output.
    pub bill: Option<String>,
    /// Date (`YYYY-MM-DD`) the accumulated `dur:` was last logged. Written
    /// when the timer stops and by manual time additions, so the timesheet
    /// can attribute time to the day it was actually tracked rather than the
    /// task's creation date. `None` for lines written before this tag
    /// existed or entered by hand.
    pub log: Option<String>,
}

pub fn parse_line(raw: &str) -> Result<Task, ParseError> {
    let line = raw.trim();
    if line.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut rest: &str = line;
    let mut done = false;
    let mut done_date: Option<String> = None;

    if let Some(stripped) = strip_prefix_x(rest) {
        done = true;
        rest = stripped;
        if let Some((date, after)) = take_iso_date_prefix(rest) {
            done_date = Some(date);
            rest = after;
        }
    }

    let mut priority: Option<char> = None;
    if let Some((c, after)) = take_priority_prefix(rest) {
        priority = Some(c);
        rest = after;
    }

    let mut created_date: Option<String> = None;
    if let Some((date, after)) = take_iso_date_prefix(rest) {
        created_date = Some(date);
        rest = after;
    }

    let projects = collect_tokens(rest, '+');
    let contexts = collect_tokens(rest, '@');
    let due = find_kv(rest, "due");
    let rec = find_kv(rest, "rec");
    let threshold = find_kv(rest, "t");
    let notes = find_quoted_kv(rest, "note");
    let start = find_kv(rest, "start");
    let dur = find_kv(rest, "dur").and_then(|v| v.parse::<u64>().ok());
    // Only `bill:n` is meaningful — `bill:y` is equivalent to omitting the
    // tag, so we normalize it to `None` so the field is a true "is
    // non-billable?" indicator.
    let bill = find_kv(rest, "bill").filter(|v| v == "n");
    let log = find_kv(rest, "log");
    let clean_raw = body_after_quoted_kv(line);

    Ok(Task {
        raw: line.to_string(),
        clean_raw,
        done,
        done_date,
        priority,
        created_date,
        projects,
        contexts,
        due,
        rec,
        threshold,
        notes,
        start,
        dur,
        bill,
        log,
    })
}

fn strip_prefix_x(s: &str) -> Option<&str> {
    let mut chars = s.chars();
    if chars.next()? == 'x' {
        let rest = chars.as_str();
        if rest.starts_with(' ') || rest.starts_with('\t') {
            return Some(rest.trim_start());
        }
    }
    None
}

/// True when `bytes[i..]` starts with ten ASCII bytes shaped like a date —
/// four digits, a dash, two digits, a dash, two digits. Shape only: the parser
/// additionally validates the calendar date, while the draft highlighter uses
/// the shape alone so a partially-typed or not-yet-valid date still colors.
fn is_iso_date_shape(bytes: &[u8], i: usize) -> bool {
    if bytes.len() < i + 10 {
        return false;
    }
    let d = |k: usize| bytes[i + k].is_ascii_digit();
    d(0) && d(1)
        && d(2)
        && d(3)
        && bytes[i + 4] == b'-'
        && d(5)
        && d(6)
        && bytes[i + 7] == b'-'
        && d(8)
        && d(9)
}

/// Priority character when `bytes[i..]` starts with the `(A)`..`(Z)` shape.
/// Shape only — the parser additionally requires a trailing space, while the
/// highlighter accepts a priority still being typed at end-of-input.
fn priority_shape(bytes: &[u8], i: usize) -> Option<char> {
    if bytes.len() >= i + 3
        && bytes[i] == b'('
        && bytes[i + 1].is_ascii_uppercase()
        && bytes[i + 2] == b')'
    {
        Some(bytes[i + 1] as char)
    } else {
        None
    }
}

/// Strip a leading `YYYY-MM-DD` token. Returns `(date_string, rest)` only if
/// the prefix is a *real* calendar date — `9999-99-99` and other invalid
/// month/day combos are rejected so they don't poison sort/grouping code that
/// later trusts the value.
fn take_iso_date_prefix(s: &str) -> Option<(String, &str)> {
    if !is_iso_date_shape(s.as_bytes(), 0) {
        return None;
    }
    let candidate = &s[..10];
    if chrono::NaiveDate::parse_from_str(candidate, "%Y-%m-%d").is_err() {
        return None;
    }
    if s.len() == 10 {
        return Some((candidate.to_string(), ""));
    }
    let bytes = s.as_bytes();
    if bytes[10] == b' ' || bytes[10] == b'\t' {
        return Some((candidate.to_string(), s[11..].trim_start()));
    }
    None
}

fn take_priority_prefix(s: &str) -> Option<(char, &str)> {
    let bytes = s.as_bytes();
    let c = priority_shape(bytes, 0)?;
    if bytes.len() >= 4 && (bytes[3] == b' ' || bytes[3] == b'\t') {
        return Some((c, s[4..].trim_start()));
    }
    None
}

fn collect_tokens(s: &str, sigil: char) -> Vec<String> {
    let mut out = Vec::new();
    for tok in s.split_whitespace() {
        if let Some(rest) = tok.strip_prefix(sigil)
            && !rest.is_empty()
        {
            out.push(rest.to_string());
        }
    }
    out
}

/// Find the value of `key:value` for a specific key. Returns the first hit;
/// later duplicates are ignored.
fn find_kv(s: &str, key: &str) -> Option<String> {
    for tok in s.split_whitespace() {
        if let Some((k, v)) = kv_pair(tok)
            && !v.starts_with('"')
            && k == key
        {
            return Some(v.to_string());
        }
    }
    None
}

/// Find the value of `key:"value" where value can contain spaces and is enclosed in double quotes.
/// Returns the first hit; later duplicates are ignored.
fn find_quoted_kv(s: &str, key: &str) -> Vec<String> {
    let culprit = format!(r#"{key}:""#);
    let Some(st) = s.find(&culprit) else {
        return vec![];
    };
    if st > 0 {
        let prev_char = s.as_bytes()[st - 1];
        if prev_char != b' ' && prev_char != b'\t' {
            return vec![];
        }
    }
    if !is_valid_key(key) {
        return vec![];
    }
    let v_st = st + culprit.len();
    let rest = &s[v_st..];
    let Some(end) = rest.find('"') else {
        return vec![];
    };
    rest[..end].split(". ").map(str::to_owned).collect()
}

fn is_valid_key(k: &str) -> bool {
    let mut chars = k.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Which tag a body token is: `+name` (project) or `@name` (context). `None`
/// for plain text, a bare `+`/`@` with no name, or a `key:value` token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagKind {
    Project,
    Context,
}

/// `Some(kind)` when `tok` is a `+name` / `@name` tag with a non-empty name.
fn tag_kind(tok: &str) -> Option<TagKind> {
    if let Some(name) = tok.strip_prefix('+')
        && !name.is_empty()
    {
        return Some(TagKind::Project);
    }
    if let Some(name) = tok.strip_prefix('@')
        && !name.is_empty()
    {
        return Some(TagKind::Context);
    }
    None
}

/// `Some((key, value))` when `tok` is a well-formed `key:value` token — a
/// valid key and a non-empty value. Quoted values still qualify here; callers
/// that reject quoted values add their own check.
fn kv_pair(tok: &str) -> Option<(&str, &str)> {
    let (k, v) = tok.split_once(':')?;
    (is_valid_key(k) && !v.is_empty()).then_some((k, v))
}

#[must_use]
pub fn parse_file(s: &str) -> Vec<Task> {
    s.lines().filter_map(|line| parse_line(line).ok()).collect()
}

#[must_use]
pub fn serialize(tasks: &[Task]) -> String {
    let mut out = String::new();
    for t in tasks {
        out.push_str(&t.raw);
        out.push('\n');
    }
    out
}

/// Atomically write `body` to `path`. Writes directly through a symlink when
/// `path` is one (preserving the link), otherwise writes to a unique sibling
/// temporary file (`.{stem}.tmp.{pid}.{n}`) and renames it into place. The
/// unique tmp name means concurrent writers can't clobber each other's temp
/// file, and the rename makes the target appear atomically. Missing parent
/// directories are created, so a stale path never turns into a silent error
/// — callers that previously relied on a missing dir failing `persist` will
/// see the dir appear instead. Shared by the store, archive, and config
/// persistence.
pub fn write_atomic(path: &Path, body: &str) -> std::io::Result<()> {
    if path.is_symlink() {
        // Write directly through the symlink to preserve it.
        return std::fs::write(path, body);
    }
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let stem = path
        .file_name()
        .map_or_else(|| "file".to_string(), |n| n.to_string_lossy().into_owned());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(".{stem}.tmp.{}.{}", std::process::id(), n);
    let tmp = path
        .parent()
        .map_or_else(|| PathBuf::from(&tmp_name), |p| p.join(&tmp_name));
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl Task {
    /// Mark this task complete as of `today`. No-op if already done.
    /// The serialized line follows todo.txt convention: `x DONE CREATED BODY`,
    /// where `BODY` has had any leading priority/created-date stripped. If the
    /// task carried no creation date, `today` is used so the line stays well-
    /// formed.
    pub fn mark_done(&mut self, today: &str) -> Result<(), ParseError> {
        if self.done {
            return Ok(());
        }
        let created = self
            .created_date
            .clone()
            .unwrap_or_else(|| today.to_string());
        let body = body_after_priority(&self.raw);
        let new_raw = format!("x {today} {created} {body}");
        self.replace_from_raw(&new_raw)
    }

    /// Reverse `mark_done`: drop the leading `x ` and the done-date token.
    /// Priority that was stripped at completion time is not recovered — the
    /// user can re-set it after un-archiving.
    pub fn unmark_done(&mut self) -> Result<(), ParseError> {
        if !self.done {
            return Ok(());
        }
        // `strip_prefix_x` (not a bare `"x "` prefix) so a hand-edited
        // `x\t...` completion round-trips the same way a `x ...` one does.
        let after_x = strip_prefix_x(&self.raw).unwrap_or(&self.raw);
        let body = if self.done_date.is_some() {
            // mark_done emits "x DONE_DATE CREATED BODY". Drop the 10-char
            // date plus its trailing space.
            let bytes = after_x.as_bytes();
            if bytes.len() >= 11 && (bytes[10] == b' ' || bytes[10] == b'\t') {
                after_x[11..].trim_start().to_string()
            } else {
                after_x.to_string()
            }
        } else {
            after_x.to_string()
        };
        self.replace_from_raw(&body)
    }

    /// Set or clear this task's priority. The priority byte is replaced in
    /// place at the start of the line; nothing else changes.
    pub fn set_priority(&mut self, priority: Option<char>) -> Result<(), ParseError> {
        let body = strip_priority(&self.raw);
        let new_raw = match priority {
            Some(p) => format!("({p}) {body}"),
            None => body.to_string(),
        };
        self.replace_from_raw(&new_raw)
    }

    /// Cycle priority A → B → C → none → A. Returns the new value (for the
    /// caller to flash). Behaves like `set_priority` w.r.t. the line format.
    pub fn cycle_priority(&mut self) -> Result<Option<char>, ParseError> {
        let next = match self.priority {
            None => Some('A'),
            Some('A') => Some('B'),
            Some('B') => Some('C'),
            Some(_) => None,
        };
        self.set_priority(next)?;
        Ok(next)
    }

    /// Append `+name` to the line. Returns `Ok(true)` if added, `Ok(false)`
    /// if the project was already present.
    pub fn add_project(&mut self, name: &str) -> Result<bool, TagError> {
        self.add_tag(name, '+', |t| &t.projects)
    }

    /// Append `@name` to the line. Returns `Ok(true)` if added, `Ok(false)`
    /// if the context was already present.
    pub fn add_context(&mut self, name: &str) -> Result<bool, TagError> {
        self.add_tag(name, '@', |t| &t.contexts)
    }

    /// Remove every `@name` token from the line. Returns `Ok(true)` if any
    /// was removed, `Ok(false)` if the context was absent.
    pub fn remove_context(&mut self, name: &str) -> Result<bool, TagError> {
        if !is_valid_tag_name(name) {
            return Err(TagError::Invalid);
        }
        if !self.contexts.iter().any(|c| c == name) {
            return Ok(false);
        }
        let needle = format!("@{name}");
        let new_raw = map_body_tokens(&self.raw, |tok| {
            if tok == needle.as_str() {
                None
            } else {
                Some(tok.to_string())
            }
        });
        self.replace_from_raw(&new_raw).map_err(TagError::Parse)?;
        Ok(true)
    }

    fn add_tag(
        &mut self,
        name: &str,
        sigil: char,
        existing: impl Fn(&Task) -> &Vec<String>,
    ) -> Result<bool, TagError> {
        if !is_valid_tag_name(name) {
            return Err(TagError::Invalid);
        }
        if existing(self).iter().any(|x| x == name) {
            return Ok(false);
        }
        let new_raw = format!("{} {sigil}{name}", self.raw.trim_end());
        self.replace_from_raw(&new_raw).map_err(TagError::Parse)?;
        Ok(true)
    }

    /// Re-parse `raw` and overwrite self. Only mutates on success, so a
    /// failed parse leaves the task untouched.
    fn replace_from_raw(&mut self, raw: &str) -> Result<(), ParseError> {
        *self = parse_line(raw)?;
        Ok(())
    }
}

/// True if `s` begins with a `(X) ` priority token.
#[must_use]
pub fn starts_with_priority(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 4 && b[0] == b'(' && b[1].is_ascii_uppercase() && b[2] == b')' && b[3] == b' '
}

/// True when `s` is a parseable `YYYY-MM-DD` calendar date. The length guard
/// rejects non-padded forms like `2026-5-6` that chrono would otherwise
/// accept leniently. Shared by the timesheet's `log:` validation and the
/// day-boundary prompt's effective-log-date check so their fallback
/// semantics can't drift.
#[must_use]
pub fn is_iso_date(s: &str) -> bool {
    s.len() == 10 && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// True if `s` begins with a `YYYY-MM-DD` token (followed by EOL or whitespace
/// is not required here — callers use this as a hint, not a tokenizer).
#[must_use]
pub fn starts_with_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4] == b'-'
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
        && b[7] == b'-'
        && b[8].is_ascii_digit()
        && b[9].is_ascii_digit()
}

/// Strip a leading `(X) ` priority token if present, otherwise return the
/// input unchanged.
#[must_use]
pub fn strip_priority(raw: &str) -> &str {
    let b = raw.as_bytes();
    if b.len() >= 4 && b[0] == b'(' && b[1].is_ascii_uppercase() && b[2] == b')' && b[3] == b' ' {
        return &raw[4..];
    }
    raw
}

/// A project/context name is valid if non-empty and contains no characters
/// that would break the todo.txt tokenization: whitespace splits a tag in
/// half, and `+`/`@`/`:` collide with the format's own sigils.
#[must_use]
pub fn is_valid_tag_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| !c.is_whitespace() && c != '+' && c != '@' && c != ':')
}

#[must_use]
pub fn body_after_priority(raw: &str) -> &str {
    let mut s = raw;
    if let Some(stripped) = strip_prefix_x(s) {
        s = stripped;
        if let Some((_, after)) = take_iso_date_prefix(s) {
            s = after;
        }
    }
    if let Some((_, after)) = take_priority_prefix(s) {
        s = after;
    }
    if let Some((_, after)) = take_iso_date_prefix(s) {
        s = after;
    }
    s
}

/// Rewrite a raw line by mapping its body tokens — everything after the
/// leading `x `/dates/`(P)` priority prefix — while leaving that prefix
/// untouched. `map` is called on each whitespace-delimited body token and
/// returns the replacement token, or `None` to drop it. Surviving tokens
/// rejoin with single spaces.
///
/// This is the single home for "remove/replace a token on a todo.txt line",
/// so the surgery can't drift between call sites. Doing it by hand with
/// `raw.split_whitespace()` also treats the priority and creation date as
/// removable tokens (e.g. `del 3 2026-05-06` stripping the date), and
/// reimplementing the prefix split at each site is what let `start:` stripping
/// diverge across `discard_stale_timer`, `complete_consumed_line`, and
/// `rebuild_token_line`.
#[must_use]
pub fn map_body_tokens(raw: &str, map: impl FnMut(&str) -> Option<String>) -> String {
    let body = body_after_priority(raw);
    let prefix = &raw[..raw.len() - body.len()];
    let kept = body
        .split_whitespace()
        .filter_map(map)
        .collect::<Vec<_>>()
        .join(" ");
    if prefix.is_empty() {
        kept
    } else {
        format!("{prefix}{kept}")
    }
}

/// Split a body into whitespace-delimited tokens, keeping a quoted
/// `key:"value with spaces"` token intact (the closing quote ends the token,
/// so internal spaces don't split it). Used where token surgery must not
/// corrupt a quoted `note:` value — `split_whitespace` would slice the note
/// into fragments and let a `due:`-shaped word inside it be rewritten.
pub(crate) fn split_body_tokens(body: &str) -> Vec<&str> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            // A quoted value swallows its internal whitespace: jump to the
            // closing quote so `note:"call ops"` stays one token.
            if i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1] == b'"' {
                let mut j = i + 2;
                while j < bytes.len() && bytes[j] != b'"' {
                    j += 1;
                }
                if j < bytes.len() {
                    i = j + 1;
                    break;
                }
            }
            i += 1;
        }
        out.push(&body[start..i]);
    }
    out
}

pub fn body_after_quoted_kv(raw: &str) -> String {
    let mut body = raw.to_string();
    while let Some(st) = body.find(r#":""#) {
        let before = &body[..st];
        let after = &body[st + 2..];
        let st_key = before.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        if let Some(second_aps) = after.find('"') {
            let after = after[second_aps + 1..].trim_start();
            body = format!("{}{}", &before[..st_key], after);
        } else {
            break;
        }
    }
    body.trim().to_string()
}

/// Description text only, given a line whose quoted `note:` tokens are
/// already stripped (`Task::clean_raw`). Callers that hold a `&Task` use this
/// directly so the quoted-kv scan runs once at parse time instead of on every
/// render; `body_only` computes the clean body first and delegates.
#[must_use]
pub fn body_only_from_clean(clean: &str) -> String {
    body_after_priority(clean)
        .split_whitespace()
        .filter(|tok| !is_meta_token(tok))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Description text only: strip the leading `x `, done/created dates, and
/// priority via `body_after_priority`, then drop every `+project`,
/// `@context`, and `key:value` token from what remains. Whitespace between
/// surviving words collapses to single spaces. Returns an owned `String`
/// because we're filtering tokens, not slicing a prefix.
#[must_use]
pub fn body_only(raw: &str) -> String {
    body_only_from_clean(&body_after_quoted_kv(raw))
}

fn is_meta_token(tok: &str) -> bool {
    tag_kind(tok).is_some() || kv_pair(tok).is_some()
}

/// Byte offset just past the narrative's final word — the natural place to
/// park the cursor when the edit dialog opens, so the user can append text
/// without scanning past the trailing `+project` / `@context` / `key:value`
/// metadata. Metadata conventionally trails the narrative in todo.txt, so
/// the first metadata token marks where the narrative ends. Falls back to
/// the end of the line when no narrative word can be identified
/// (metadata-only lines, malformed input).
#[must_use]
pub fn narrative_end_offset(raw: &str) -> usize {
    let mut last_word_end = raw.len();
    let mut seen_narrative = false;
    for (range, kind) in classify_draft(raw) {
        match kind {
            // The first *trailing* metadata token ends the narrative: bail
            // with the end of the last word-shaped segment seen so far.
            // Leading metadata (a `+project` before the narrative, say) is
            // skipped — it only terminates the narrative once a word has
            // appeared, otherwise metadata-only lines would truncate to 0.
            SegmentKind::Project
            | SegmentKind::Context
            | SegmentKind::Due
            | SegmentKind::KeyValue => {
                if seen_narrative {
                    return last_word_end;
                }
            }
            // Track the end of the last non-whitespace Plain run. Date and
            // priority segments are leading bookkeeping, not narrative, so
            // they deliberately don't advance the marker.
            SegmentKind::Plain if !raw[range.clone()].chars().all(char::is_whitespace) => {
                seen_narrative = true;
                last_word_end = range.end;
            }
            _ => {}
        }
    }
    last_word_end
}

/// Byte offset of the narrative's first word — right after the leading
/// `x ` / done-date / priority / creation-date tokens. Used when the edit
/// dialog's cursor preference is "narrative start". Falls back to the end of
/// the line when no narrative word exists (metadata-only lines), so both
/// cursor preferences behave identically there.
#[must_use]
pub fn narrative_start_offset(raw: &str) -> usize {
    let bytes = raw.as_bytes();
    let mut i = 0;
    // Skip the same leading bookkeeping tokens the parser recognizes: a
    // done "x " marker plus its done date, a priority, and the creation
    // date. (Not `classify_draft` — it tags the leading `x` as a Plain
    // word, which would make every done task look narrative-less.)
    if bytes.len() >= 2 && bytes[0] == b'x' && bytes[1].is_ascii_whitespace() {
        i = 2;
        if let Some(end) = match_date(bytes, i) {
            i = end;
            if i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
        }
    }
    if let Some(end) = match_priority(bytes, i) {
        i = end;
        if i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
    }
    if let Some(end) = match_date(bytes, i) {
        i = end;
        if i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
    }
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    // A line that goes straight into metadata has no narrative, so fall
    // back to the end of the line (matching `narrative_end_offset`).
    let first = raw[i..].split_whitespace().next().unwrap_or("");
    if first.is_empty() || is_meta_token(first) {
        raw.len()
    } else {
        i
    }
}

// ---------------------------------------------------------------------------
// Draft syntax classification (syntax highlighting for the add/edit dialog)
// ---------------------------------------------------------------------------

/// Classifier output: byte range + what kind of token lives there. Segments
/// cover the input contiguously and don't overlap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentKind {
    Plain,
    Priority(char),
    Date,
    Project,
    Context,
    Due,
    KeyValue,
}

/// Walk a draft and tag each byte range with what it represents in the
/// todo.txt format. Used by the dialog to syntax-highlight what the user is
/// typing. Mirrors [`parse_line`]'s grammar at the token level but doesn't
/// share code — the highlighter must keep up character-by-character even on
/// partially-typed input that the parser would reject.
pub(crate) fn classify_draft(s: &str) -> Vec<(std::ops::Range<usize>, SegmentKind)> {
    let mut out: Vec<(std::ops::Range<usize>, SegmentKind)> = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;

    // Optional leading "x " marker (done) followed by an optional done-date.
    if bytes.len() >= 2 && bytes[0] == b'x' && bytes[1].is_ascii_whitespace() {
        out.push((0..1, SegmentKind::Plain));
        out.push((1..2, SegmentKind::Plain));
        i = 2;
        if let Some(end) = match_date(bytes, i) {
            out.push((i..end, SegmentKind::Date));
            i = end;
            if i < bytes.len() && bytes[i].is_ascii_whitespace() {
                out.push((i..i + 1, SegmentKind::Plain));
                i += 1;
            }
        }
    }

    // Leading priority "(A)" through "(Z)".
    if let Some(end) = match_priority(bytes, i) {
        let pri_char = bytes[i + 1] as char;
        out.push((i..end, SegmentKind::Priority(pri_char)));
        i = end;
        if i < bytes.len() && bytes[i].is_ascii_whitespace() {
            out.push((i..i + 1, SegmentKind::Plain));
            i += 1;
        }
    }

    // Optional creation date.
    if let Some(end) = match_date(bytes, i) {
        out.push((i..end, SegmentKind::Date));
        i = end;
        if i < bytes.len() && bytes[i].is_ascii_whitespace() {
            out.push((i..i + 1, SegmentKind::Plain));
            i += 1;
        }
    }

    // Walk the rest as alternating whitespace runs and word tokens.
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push((start..i, SegmentKind::Plain));
            continue;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            // Detect a key-value pair with a quoted value, e.g. `note:"this is a note"`.
            if i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1] == b'"' {
                let init_i = i + 2;
                let mut j = init_i;
                while j < bytes.len() && bytes[j] != b'"' {
                    j += 1;
                }
                if j < bytes.len() {
                    i = j + 1;
                    break;
                }
            }
            i += 1;
        }
        let word = &s[start..i];
        out.push((start..i, classify_word(word)));
    }

    out
}

/// Byte range of a leading `(A)`..`(Z)` priority. Shares the shape predicate
/// with the parser (`take_priority_prefix`) but returns the range so the
/// highlighter can color the exact three bytes without consuming whitespace.
fn match_priority(bytes: &[u8], i: usize) -> Option<usize> {
    priority_shape(bytes, i).map(|_| i + 3)
}

/// Byte range of a leading date-shaped token. Shares the shape predicate with
/// the parser but stops at the shape — no calendar validation, so a date the
/// user is still typing (or has mistyped) still highlights as a date.
fn match_date(bytes: &[u8], i: usize) -> Option<usize> {
    is_iso_date_shape(bytes, i).then_some(i + 10)
}

fn classify_word(w: &str) -> SegmentKind {
    match tag_kind(w) {
        Some(TagKind::Project) => return SegmentKind::Project,
        Some(TagKind::Context) => return SegmentKind::Context,
        None => {}
    }
    if let Some((k, _)) = kv_pair(w) {
        if k == "due" {
            return SegmentKind::Due;
        }
        return SegmentKind::KeyValue;
    }
    SegmentKind::Plain
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_empty_returns_err() {
        assert!(matches!(parse_line(""), Err(ParseError::Empty)));
        assert!(matches!(parse_line("   "), Err(ParseError::Empty)));
        assert!(matches!(parse_line("\n"), Err(ParseError::Empty)));
    }

    #[test]
    fn parse_line_simple_input_returns_ok() {
        assert!(parse_line("Hello").is_ok());
    }

    #[test]
    fn parse_error_displays_human_message() {
        assert_eq!(format!("{}", ParseError::Empty), "empty");
    }

    #[test]
    fn parses_line_starting_with_non_ascii_after_single_byte() {
        // Regression: `take_iso_date_prefix` used byte indexing (`&s[..10]`)
        // after a byte-length check, which panicked when byte 10 fell inside
        // a multi-byte UTF-8 character. Triggered by tasks like the one
        // below, where '2' is 1 byte and the following Cyrillic chars are
        // 2 bytes each, putting byte 10 inside 'с'.
        let t = parse_line("2Написать задачи на день due:2026-05-11 rec:+1d").unwrap();
        assert_eq!(t.created_date, None);
        assert_eq!(t.due.as_deref(), Some("2026-05-11"));
        assert_eq!(t.rec.as_deref(), Some("+1d"));
    }

    #[test]
    fn rejects_invalid_calendar_dates() {
        // `9999-99-99` is well-formed lexically but not a real date —
        // earlier versions accepted it and let the bogus value flow into
        // sort/grouping code as a string. The parser now refuses.
        let t = parse_line("9999-99-99 not a date").unwrap();
        assert_eq!(t.created_date, None);
        assert!(t.raw.starts_with("9999-99-99"));
    }

    #[test]
    fn parses_priority_and_dates() {
        let t = parse_line("(A) 2026-04-28 Call dentist @phone +health due:2026-05-08").unwrap();
        assert_eq!(t.priority, Some('A'));
        assert_eq!(t.created_date.as_deref(), Some("2026-04-28"));
        assert_eq!(t.due.as_deref(), Some("2026-05-08"));
        assert_eq!(t.projects, vec!["health"]);
        assert_eq!(t.contexts, vec!["phone"]);
        assert!(!t.done);
        assert_eq!(t.rec, None);
    }

    #[test]
    fn parses_rec_tag() {
        let t = parse_line("2026-05-09 Pay rent due:2026-05-15 rec:+1m").unwrap();
        assert_eq!(t.rec.as_deref(), Some("+1m"));
        assert_eq!(t.due.as_deref(), Some("2026-05-15"));
    }

    #[test]
    fn parses_absolute_threshold_tag() {
        let t = parse_line("2026-04-01 Renew passport t:2026-08-01 +personal").unwrap();
        assert_eq!(t.threshold.as_deref(), Some("2026-08-01"));
    }

    #[test]
    fn parses_relative_threshold_tag() {
        let t = parse_line("Pay rent due:2026-06-01 t:-3d +finance").unwrap();
        assert_eq!(t.threshold.as_deref(), Some("-3d"));
        assert_eq!(t.due.as_deref(), Some("2026-06-01"));
    }

    #[test]
    fn body_only_strips_threshold_token() {
        // The "no chip" rendering choice relies on body_only filtering `t:`
        // out via is_meta_token. Asserting it here so a future change to
        // is_valid_key can't regress this without an explicit test failure.
        assert_eq!(
            body_only("2026-04-01 Renew passport t:2026-08-01 +personal"),
            "Renew passport",
        );
        assert_eq!(
            body_only("Pay rent due:2026-06-01 t:-3d +finance"),
            "Pay rent",
        );
    }

    #[test]
    fn parses_completed() {
        let t = parse_line("x 2026-05-05 2026-05-01 Submit expense report +work @laptop").unwrap();
        assert!(t.done);
        assert_eq!(t.done_date.as_deref(), Some("2026-05-05"));
        assert_eq!(t.created_date.as_deref(), Some("2026-05-01"));
        assert_eq!(t.projects, vec!["work"]);
    }

    #[test]
    fn parses_all_sample_lines() {
        let parsed = parse_file(crate::sample::TODO_RAW);
        assert_eq!(parsed.len(), 19);
        let done = parsed.iter().filter(|t| t.done).count();
        assert_eq!(done, 3);
        let with_due = parsed.iter().filter(|t| t.due.is_some()).count();
        assert_eq!(with_due, 7);
        let with_rec = parsed.iter().filter(|t| t.rec.is_some()).count();
        assert_eq!(with_rec, 1);
        let with_threshold = parsed.iter().filter(|t| t.threshold.is_some()).count();
        assert_eq!(with_threshold, 1);
    }

    #[test]
    fn body_strips_metadata() {
        let raw = "(A) 2026-05-01 Hello world";
        assert_eq!(body_after_priority(raw), "Hello world");
        let raw2 = "x 2026-05-05 2026-05-01 Hello world";
        assert_eq!(body_after_priority(raw2), "Hello world");
    }

    #[test]
    fn body_only_drops_tags_and_kv_pairs() {
        // Plain description survives unchanged.
        assert_eq!(body_only("Hello world"), "Hello world");
        // Priority + creation date prefix are stripped, +project / @context /
        // due:... are filtered out, words collapse to single spaces.
        assert_eq!(
            body_only("(A) 2026-04-28 Call dentist @phone +health due:2026-05-08"),
            "Call dentist",
        );
        // Completed lines lose `x` + done date + creation date as well.
        assert_eq!(
            body_only("x 2026-05-05 2026-05-01 Submit expense report +work @laptop"),
            "Submit expense report",
        );
        // Sigils inside a word (not at the start of a token) are not tags
        // and must be preserved.
        assert_eq!(body_only("email a+b@example.com"), "email a+b@example.com");
        // Lone sigils with no name are not valid tags either.
        assert_eq!(body_only("type @ then context"), "type @ then context");
        // Unknown key:value tokens still drop — todo.txt treats any
        // alphanumeric `key:value` as an extension, so we mirror that.
        assert_eq!(body_only("backup id:abc-123 nightly"), "backup nightly");
    }

    #[test]
    fn round_trip_preserves_raw() {
        let parsed = parse_file(crate::sample::TODO_RAW);
        let serialized = serialize(&parsed);
        let reparsed = parse_file(&serialized);
        assert_eq!(parsed.len(), reparsed.len());
        for (a, b) in parsed.iter().zip(reparsed.iter()) {
            assert_eq!(a.raw, b.raw);
        }
    }

    #[test]
    fn parses_start_tag() {
        let t =
            parse_line("Draft motion +Smith @drafting start:2026-07-31T14:30:25 dur:3600").unwrap();
        assert_eq!(t.start.as_deref(), Some("2026-07-31T14:30:25"));
        assert_eq!(t.dur, Some(3600));
    }

    #[test]
    fn parses_dur_only_no_start() {
        let t = parse_line("Draft motion +Smith @drafting dur:1800").unwrap();
        assert_eq!(t.start, None);
        assert_eq!(t.dur, Some(1800));
    }

    #[test]
    fn parses_task_without_time_tags() {
        let t = parse_line("Call dentist @phone +health due:2026-05-08").unwrap();
        assert_eq!(t.start, None);
        assert_eq!(t.dur, None);
    }

    #[test]
    fn body_only_strips_start_and_dur() {
        assert_eq!(
            body_only("Draft motion +Smith @drafting start:2026-07-31T14:30:25 dur:3600"),
            "Draft motion",
        );
    }

    #[test]
    fn invalid_dur_value_returns_none() {
        let t = parse_line("task dur:notanumber").unwrap();
        assert_eq!(t.dur, None);
    }

    #[test]
    fn unmark_done_handles_tab_separated_completion() {
        // A hand-edited `x\tDONE ...` line parses like a `x ` one; unmarking
        // must not treat the leading `x` as body text.
        let mut t = parse_line("x\t2026-05-05 2026-05-01 Hello").unwrap();
        assert!(t.done);
        t.unmark_done().unwrap();
        assert!(!t.done);
        assert!(!t.raw.starts_with('x'), "got: {}", t.raw);
        assert_eq!(t.raw, "2026-05-01 Hello");
    }

    #[test]
    fn narrative_end_offset_stops_before_trailing_metadata() {
        assert_eq!(
            narrative_end_offset("2026-05-06 Draft motion +Smith @drafting dur:3600"),
            "2026-05-06 Draft motion".len()
        );
        assert_eq!(
            narrative_end_offset("(A) 2026-04-28 Call dentist @phone +health due:2026-05-08"),
            "(A) 2026-04-28 Call dentist".len()
        );
    }

    #[test]
    fn narrative_end_offset_handles_done_tasks_and_no_metadata() {
        // Completed tasks carry a done date before the creation date.
        assert_eq!(
            narrative_end_offset("x 2026-05-05 2026-05-01 Submit expense report +work"),
            "x 2026-05-05 2026-05-01 Submit expense report".len()
        );
        // No metadata: the whole line is narrative, so the offset is the end.
        let bare = "2026-05-06 Draft motion";
        assert_eq!(narrative_end_offset(bare), bare.len());
    }

    #[test]
    fn narrative_start_offset_lands_on_first_word() {
        assert_eq!(
            narrative_start_offset("2026-05-06 Draft motion +Smith @drafting dur:3600"),
            "2026-05-06 ".len()
        );
        assert_eq!(
            narrative_start_offset("(A) 2026-04-28 Call dentist @phone +health due:2026-05-08"),
            "(A) 2026-04-28 ".len()
        );
        // Done tasks carry a done date before the creation date.
        assert_eq!(
            narrative_start_offset("x 2026-05-05 2026-05-01 Submit expense report +work"),
            "x 2026-05-05 2026-05-01 ".len()
        );
    }

    #[test]
    fn narrative_start_offset_falls_back_to_line_end() {
        // Metadata with no narrative word: park at the end, matching the
        // end-of-narrative fallback so both preferences behave identically.
        assert_eq!(
            narrative_start_offset("2026-05-06 +Smith @drafting dur:3600"),
            "2026-05-06 +Smith @drafting dur:3600".len()
        );
    }

    #[test]
    fn narrative_end_offset_skips_leading_project_before_narrative() {
        // A `+project` may precede the narrative; the cursor must still stop
        // at the end of the last narrative word, before the trailing metadata.
        assert_eq!(
            narrative_end_offset("2026-08-31 +work do stuff dur:4 log:2026-08-31"),
            "2026-08-31 +work do stuff".len()
        );
        assert_eq!(
            narrative_end_offset("+work do stuff dur:4 log:2026-08-31"),
            "+work do stuff".len()
        );
    }

    #[test]
    fn narrative_end_offset_falls_back_to_line_end() {
        // Metadata with no narrative word: park at the end, not mid-token.
        assert_eq!(
            narrative_end_offset("2026-05-06 +Smith @drafting dur:3600"),
            "2026-05-06 +Smith @drafting dur:3600".len()
        );
        // Quoted key:value tokens (e.g. note:"...") count as metadata.
        assert_eq!(
            narrative_end_offset("2026-05-06 Review note:\"call ops\" +Smith"),
            "2026-05-06 Review".len()
        );
    }

    #[test]
    fn body_only_from_clean_matches_body_only() {
        for raw in [
            "(A) 2026-04-28 Call dentist @phone +health due:2026-05-08",
            "x 2026-05-05 2026-05-01 Submit expense report +work @laptop",
            "Draft motion +Smith @drafting start:2026-07-31T14:30:25 dur:3600 log:2026-08-06",
        ] {
            let t = parse_line(raw).unwrap();
            assert_eq!(body_only_from_clean(&t.clean_raw), body_only(raw));
        }
    }

    #[test]
    fn split_body_tokens_keeps_quoted_note_intact() {
        let tokens = split_body_tokens(
            "Water plants due:2026-05-06 rec:1d note:\"call ops re: due:2026-05-20\"",
        );
        assert_eq!(
            tokens,
            vec![
                "Water",
                "plants",
                "due:2026-05-06",
                "rec:1d",
                "note:\"call ops re: due:2026-05-20\"",
            ]
        );
    }

    #[test]
    fn split_body_tokens_unterminated_quote_falls_back_to_words() {
        // No closing quote: don't swallow the rest of the line, just fall
        // back to whitespace splitting.
        let tokens = split_body_tokens("a note:\"oops b");
        assert_eq!(tokens, vec!["a", "note:\"oops", "b"]);
    }

    // ── bill: tag ──────────────────────────────────────────────────────

    #[test]
    fn parses_billable_default() {
        let t = parse_line("Draft motion +Smith @drafting dur:3600").unwrap();
        assert_eq!(t.bill, None); // absent = billable
    }

    #[test]
    fn parses_bill_y_normalizes_to_none() {
        // bill:y is equivalent to omitting the tag — normalized to None.
        let t = parse_line("Draft motion +Smith @drafting dur:3600 bill:y").unwrap();
        assert_eq!(t.bill, None);
    }

    #[test]
    fn parses_bill_n_nonbillable() {
        let t = parse_line("Firm admin +Admin @admin dur:900 bill:n").unwrap();
        assert_eq!(t.bill.as_deref(), Some("n"));
    }

    #[test]
    fn body_only_strips_bill_tag() {
        assert_eq!(
            body_only("Firm admin +Admin @admin dur:900 bill:n"),
            "Firm admin",
        );
        assert_eq!(
            body_only("Draft motion +Smith @drafting dur:3600 bill:y"),
            "Draft motion",
        );
    }

    #[test]
    fn invalid_bill_value_treated_as_billable() {
        // Unknown bill: values are silently treated as billable (None).
        let t = parse_line("task bill:xyz").unwrap();
        assert_eq!(t.bill, None);
        let t2 = parse_line("task bill:maybe").unwrap();
        assert_eq!(t2.bill, None);
    }

    #[test]
    fn bill_round_trips_through_serialize() {
        let raw = "Firm admin +Admin @admin dur:900 bill:n\n";
        let parsed = parse_file(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].bill.as_deref(), Some("n"));
        let serialized = serialize(&parsed);
        assert_eq!(serialized, raw);
    }

    // ── log: tag (day the time was tracked) ─────────────────────────────

    #[test]
    fn parses_log_tag() {
        let t = parse_line("Draft motion +Smith @drafting dur:3600 log:2026-08-06").unwrap();
        assert_eq!(t.log.as_deref(), Some("2026-08-06"));
        // Absent log: means the day isn't known (pre-log lines, hand-typed).
        let t2 = parse_line("Draft motion +Smith @drafting dur:3600").unwrap();
        assert_eq!(t2.log, None);
    }

    #[test]
    fn body_only_strips_log_tag() {
        // The narrative output must not include the log date token.
        assert_eq!(
            body_only("Draft motion +Smith @drafting dur:3600 log:2026-08-06"),
            "Draft motion",
        );
    }

    #[test]
    fn log_round_trips_through_serialize() {
        let raw = "Draft motion +Smith @drafting dur:3600 log:2026-08-06\n";
        let parsed = parse_file(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].log.as_deref(), Some("2026-08-06"));
        let serialized = serialize(&parsed);
        assert_eq!(serialized, raw);
    }

    // ── write_atomic ────────────────────────────────────────────────────

    #[test]
    fn write_atomic_writes_and_creates_parent_dirs() {
        let dir =
            std::env::temp_dir().join(format!("tuxtime-atomic-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("todo.txt");
        write_atomic(&path, "one\ntwo\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // Only meaningful where symlinks exist (not on Windows without
    // privileges); guards the symlink-preserving branch of write_atomic.
    #[cfg(unix)]
    #[test]
    fn write_atomic_preserves_symlinks() {
        let dir = std::env::temp_dir().join(format!(
            "tuxtime-atomic-symlink-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("target.txt");
        let link = dir.join("link.txt");
        std::fs::write(&target, "old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_atomic(&link, "new").unwrap();
        // The link must still be a symlink, and the target must have the new
        // content (not a replaced link file).
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── classify_draft (draft syntax highlighting) ──────────────────────

    #[test]
    fn classify_plain_text_is_all_plain() {
        let r = classify_draft("Hello world");
        assert!(r.iter().all(|(_, k)| matches!(k, SegmentKind::Plain)));
        let mut prev = 0;
        for (range, _) in &r {
            assert_eq!(range.start, prev);
            prev = range.end;
        }
        assert_eq!(prev, "Hello world".len());
    }

    #[test]
    fn classify_priority_at_start() {
        let r = classify_draft("(A) Hello");
        assert!(matches!(r[0].1, SegmentKind::Priority('A')));
        assert_eq!(r[0].0, 0..3);
    }

    #[test]
    fn classify_creation_date() {
        let r = classify_draft("2026-05-01 Hello");
        assert!(matches!(r[0].1, SegmentKind::Date));
        assert_eq!(r[0].0, 0..10);
    }

    #[test]
    fn classify_project_token() {
        let s = "Hello +work";
        let r = classify_draft(s);
        let proj = r
            .iter()
            .find(|(_, k)| matches!(k, SegmentKind::Project))
            .unwrap();
        assert_eq!(&s[proj.0.clone()], "+work");
    }

    #[test]
    fn classify_context_token() {
        let s = "Hello @home";
        let r = classify_draft(s);
        let ctx = r
            .iter()
            .find(|(_, k)| matches!(k, SegmentKind::Context))
            .unwrap();
        assert_eq!(&s[ctx.0.clone()], "@home");
    }

    #[test]
    fn classify_due_keyvalue() {
        let s = "Hello due:2026-05-15";
        let r = classify_draft(s);
        let due = r
            .iter()
            .find(|(_, k)| matches!(k, SegmentKind::Due))
            .unwrap();
        assert_eq!(&s[due.0.clone()], "due:2026-05-15");
    }

    #[test]
    fn classify_other_keyvalue() {
        let s = "Hello rec:1w";
        let r = classify_draft(s);
        let kv = r
            .iter()
            .find(|(_, k)| matches!(k, SegmentKind::KeyValue))
            .unwrap();
        assert_eq!(&s[kv.0.clone()], "rec:1w");
    }

    #[test]
    fn classify_full_line_covers_all_bytes() {
        let s = "(A) 2026-05-01 Buy milk +shop @home due:2026-05-12";
        let r = classify_draft(s);
        let mut prev = 0;
        for (range, _) in &r {
            assert_eq!(range.start, prev);
            prev = range.end;
        }
        assert_eq!(prev, s.len());
        assert!(matches!(r[0].1, SegmentKind::Priority('A')));
    }

    #[test]
    fn classify_done_marker_then_date() {
        let s = "x 2026-05-05 thing";
        let r = classify_draft(s);
        let date_seg = r
            .iter()
            .find(|(_, k)| matches!(k, SegmentKind::Date))
            .unwrap();
        assert_eq!(&s[date_seg.0.clone()], "2026-05-05");
    }

    #[test]
    fn classify_lone_sigil_stays_plain() {
        // A bare "+" or "@" with no following text shouldn't get a sigil
        // colour — it's just a character the user is mid-typing.
        let s = "Foo + bar";
        let r = classify_draft(s);
        let plus = r
            .iter()
            .find(|(range, _)| &s[range.clone()] == "+")
            .expect("lone + should appear as its own segment");
        assert!(matches!(plus.1, SegmentKind::Plain));
    }

    #[test]
    fn classifier_and_parser_share_shape_but_diverge_on_validation() {
        // The parser rejects a lexically-shaped-but-invalid calendar date
        // (`9999-99-99`) so it never poisons grouping; the highlighter must
        // still colour it as a date so the user sees what they're typing even
        // before it's a valid day. Both now share `is_iso_date_shape`, so the
        // divergence is the single validation step, not two date matchers.
        let t = parse_line("9999-99-99 not a date").unwrap();
        assert_eq!(t.created_date, None);

        let r = classify_draft("9999-99-99 Hello");
        assert!(matches!(r[0].1, SegmentKind::Date));
        assert_eq!(r[0].0, 0..10);
    }
}
