use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::search::subseq_match_ci;
use crate::theme::Theme;
use crate::todo::{Task, body_after_priority};

#[derive(Clone, Copy, Default)]
pub struct RowOpts<'a> {
    pub idx_label: usize,
    pub cursor: bool,
    pub multi_mode: bool,
    pub multi_checked: bool,
    pub selected: bool,
    pub show_line_num: bool,
    pub match_term: Option<&'a str>,
    pub today: &'a str,
    /// `key:value` tokens whose key is in this list are omitted from the
    /// rendered body. Empty (the common case) means render everything,
    /// byte-for-byte as before.
    pub hidden_keys: &'a [String],
    /// Whether a positive `dur:` token renders as a compact inline badge.
    pub show_duration_inline: bool,
    /// Whether the `log:` bookkeeping token renders inline.
    pub show_log_inline: bool,
    /// True when a timer is actively running on this task.
    pub timer_running: bool,
    /// Live elapsed seconds for the running timer, if applicable.
    pub timer_elapsed: Option<u64>,
}

pub fn build_line<'a>(
    task: &'a Task,
    opts: RowOpts<'a>,
    theme: &Theme,
    max_width: u16,
) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut metadata: Vec<Span<'a>> = Vec::new();

    if opts.show_line_num {
        // The line number must stay visible on the highlighted cursor row.
        let num_color = if opts.cursor { theme.fg } else { theme.dim };
        spans.push(Span::styled(
            format!("{:>3} ", opts.idx_label + 1),
            Style::default().fg(num_color),
        ));
    }
    if opts.multi_mode {
        let mark = if opts.multi_checked { "[x] " } else { "[ ] " };
        let c = if opts.multi_checked {
            theme.accent
        } else {
            theme.dim
        };
        spans.push(Span::styled(mark, Style::default().fg(c)));
    }

    // status glyph + priority box
    let glyph = if task.done {
        "✓ "
    } else if opts.timer_running {
        "▶ "
    } else if opts.cursor {
        "▸ "
    } else {
        "  "
    };
    // Keep the glyph visible on the cursor row (a completed task's dim
    // "done" color otherwise disappears into the highlight).
    let glyph_color = if task.done && !opts.cursor {
        theme.done
    } else {
        theme.accent
    };
    let mut glyph_style = Style::default().fg(glyph_color);
    if opts.cursor {
        glyph_style = glyph_style.add_modifier(Modifier::BOLD);
    }
    spans.push(Span::styled(glyph, glyph_style));

    if task.done {
        spans.push(Span::styled("    ", Style::default().fg(theme.done)));
    } else if let Some(p) = task.priority {
        spans.push(Span::styled(
            format!("({p}) "),
            Style::default()
                .fg(theme.priority_color(p))
                .add_modifier(Modifier::BOLD),
        ));
    } else {
        spans.push(Span::raw("    "));
    }

    // body — walk &str slices instead of collecting Vec<char>. Spans borrow
    // straight from `task.raw`, so most rows allocate only for the format!()
    // calls above.
    let body_start_idx = spans.len();
    let body = body_after_priority(&task.clean_raw);
    let body_match_positions: Option<Vec<usize>> =
        opts.match_term.and_then(|n| subseq_match_ci(body, n));
    let body_start = body.as_ptr() as usize;
    let mut rest = body;
    // Whether any visible body token has been emitted yet. Drives the
    // hidden-token branch's whitespace fix-up so a skipped token never
    // leaves a leading, trailing, or doubled space. When `hidden_keys`
    // is empty the branch is never entered and output is byte-identical
    // to before.
    let mut emitted_body_token = false;
    while !rest.is_empty() {
        let ws_end = rest
            .find(|c: char| !c.is_whitespace())
            .unwrap_or(rest.len());
        let pushed_ws = ws_end > 0;
        if pushed_ws {
            spans.push(Span::raw(&rest[..ws_end]));
            rest = &rest[ws_end..];
        }
        if rest.is_empty() {
            break;
        }
        let tok_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let token = &rest[..tok_end];
        // A `dur:` with nothing worth showing (just-started `dur:0`, or a
        // malformed value) is dropped like a hidden key so it never leaves
        // a stray `dur:0` or an orphan space; a positive value renders as a
        // compact badge in `push_token_spans`. Likewise a `bill:y` (the
        // default, billable) is dropped; only `bill:n` renders a badge.
        let drop_token = is_hidden_kv(token, opts.hidden_keys, opts.show_log_inline)
            || (token.starts_with("dur:")
                && (!opts.show_duration_inline || dur_badge(task).is_none()))
            || (token.starts_with("bill:") && bill_badge(task).is_none());
        if drop_token {
            // Drop the separator we just emitted for this token...
            if pushed_ws {
                spans.pop();
            }
            rest = &rest[tok_end..];
            // ...and if nothing visible precedes it, also swallow the
            // following whitespace run so the next token doesn't inherit
            // an orphan leading space.
            if !emitted_body_token {
                let n = rest
                    .find(|c: char| !c.is_whitespace())
                    .unwrap_or(rest.len());
                rest = &rest[n..];
            }
            continue;
        }
        let token_offset = token.as_ptr() as usize - body_start;
        let context = TokenRenderContext {
            body_match_positions: body_match_positions.as_deref(),
            task,
            opts,
            theme,
        };
        push_token_spans(&mut spans, token, token_offset, &context, &mut metadata);
        emitted_body_token = true;
        rest = &rest[tok_end..];
    }
    // Right-align the live elapsed timer: reserve its width and truncate the
    // body so a long narrative can never push the timer off the right edge.
    let timer_text = if opts.timer_running {
        opts.timer_elapsed
            .map(|elapsed| format!(" [{}]", crate::app::format_clock(elapsed)))
    } else {
        None
    };
    let prefix_width: usize = spans[..body_start_idx]
        .iter()
        .map(|s| s.content.chars().count())
        .sum();
    if !metadata.is_empty() {
        metadata.insert(0, Span::raw(" "));
    }
    let metadata_width: usize = metadata.iter().map(|s| s.content.chars().count()).sum();
    let timer_width = timer_text.as_ref().map_or(0, |t| t.chars().count());
    let body_budget = usize::from(max_width)
        .saturating_sub(prefix_width)
        .saturating_sub(timer_width)
        .saturating_sub(metadata_width);
    let body_spans = spans.split_off(body_start_idx);
    let body_spans = truncate_to_width(&body_spans, body_budget);
    let body_width: usize = body_spans.iter().map(|s| s.content.chars().count()).sum();
    spans.extend(body_spans);
    // Fill the reserved gap so metadata forms a stable right-aligned column
    // across rows with different narrative lengths. This is intentionally
    // inserted only when metadata exists; ordinary rows retain their natural
    // spacing and the live timer behavior remains unchanged.
    if metadata_width > 0 {
        let used = prefix_width + body_width + metadata_width + timer_width;
        let gap = usize::from(max_width).saturating_sub(used);
        if gap > 0 {
            spans.push(Span::raw(" ".repeat(gap)));
        }
    }
    spans.extend(metadata);
    if let Some(t) = timer_text {
        spans.push(Span::styled(t, Style::default().fg(theme.accent)));
    }

    let line_style = if opts.cursor {
        Style::default().bg(theme.cursor).fg(theme.fg)
    } else if opts.selected {
        Style::default().bg(theme.selected).fg(theme.fg)
    } else {
        Style::default()
    };
    Line::from(spans).style(line_style)
}

/// Truncate a sequence of styled spans to at most `max` chars, keeping an
/// ellipsis column when anything is dropped so a clipped row reads as cut
/// rather than silently truncated. Widths are char counts, matching the rest
/// of the renderer (see `msgbox::pad_right`).
fn truncate_to_width<'a>(spans: &[Span<'a>], max: usize) -> Vec<Span<'a>> {
    let total: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if total <= max {
        return spans.to_vec();
    }
    if max == 0 {
        return Vec::new();
    }
    let budget = max - 1; // reserve the final column for the ellipsis
    let mut out: Vec<Span<'a>> = Vec::new();
    let mut used = 0usize;
    for span in spans {
        let w = span.content.chars().count();
        if w == 0 {
            continue;
        }
        if used + w <= budget {
            out.push(span.clone());
            used += w;
        } else {
            let keep = budget.saturating_sub(used);
            if keep > 0 {
                let cut: String = span.content.chars().take(keep).collect();
                out.push(Span::styled(cut, span.style));
            }
            break;
        }
    }
    out.push(Span::raw("…"));
    out
}

struct TokenRenderContext<'match_, 'task, 'opts> {
    body_match_positions: Option<&'match_ [usize]>,
    task: &'task Task,
    opts: RowOpts<'opts>,
    theme: &'task Theme,
}

fn push_token_spans<'a>(
    spans: &mut Vec<Span<'a>>,
    token: &'a str,
    token_offset_in_body: usize,
    context: &TokenRenderContext<'_, '_, 'a>,
    metadata: &mut Vec<Span<'a>>,
) {
    // `dur:` renders as a compact human badge (`2h 05m`, `45m`) instead of the
    // raw second count — lawyers log in minutes/hours, not seconds. The full
    // raw token stays available in the detail sidebar.
    if token.starts_with("dur:") {
        if let Some(badge) = dur_badge(context.task) {
            push_metadata(
                metadata,
                Span::styled(badge, dur_badge_style(context.task.done, context.theme)),
            );
        }
        return;
    }
    // `bill:n` renders as a compact `DNB` (do-not-bill) badge; billable is the
    // default so `bill:y` / absent render nothing.
    if token.starts_with("bill:") {
        if let Some(badge) = bill_badge(context.task) {
            push_metadata(
                metadata,
                Span::styled(badge, bill_badge_style(context.task.done, context.theme)),
            );
        }
        return;
    }
    if let Some(c) = sigil_token_color(token, context.task, context.theme) {
        spans.push(Span::styled(token, Style::default().fg(c)));
        return;
    }
    if let Some(rest) = token.strip_prefix("due:") {
        spans.push(Span::styled(
            token,
            due_token_style(context.task.done, rest, context.opts.today, context.theme),
        ));
        return;
    }
    // URLs are picked off before the generic key:value branch — `http:` would
    // otherwise classify as a lowercase key and steal the underline + accent
    // styling that doubles as the OSC 8 hyperlink marker (see `ui::hyperlinks`).
    if is_url_token(token) {
        spans.push(Span::styled(
            token,
            url_token_style(context.task.done, context.theme),
        ));
        return;
    }
    // generic key:value (lowercase key)
    if let Some((k, _v)) = token.split_once(':')
        && !k.is_empty()
        && k.chars()
            .next()
            .expect("invariant: !k.is_empty() guarded above")
            .is_ascii_lowercase()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        spans.push(Span::styled(token, Style::default().fg(context.theme.dim)));
        return;
    }

    // plain word — highlight each matched subsequence char inside this token.
    // Body text stays visible on the cursor row even when the task is done.
    let base_color = if context.task.done && !context.opts.cursor {
        context.theme.done
    } else {
        context.theme.fg
    };
    let base_style = apply_dim(Style::default().fg(base_color), context.task.done);
    let hl_style = Style::default()
        .fg(context.theme.bg)
        .bg(context.theme.matched)
        .add_modifier(Modifier::BOLD);

    let token_end = token_offset_in_body + token.len();
    let mut local_positions = context
        .body_match_positions
        .into_iter()
        .flatten()
        .copied()
        .filter(|&p| p >= token_offset_in_body && p < token_end)
        .map(|p| p - token_offset_in_body)
        .peekable();

    if local_positions.peek().is_none() {
        spans.push(Span::styled(token, base_style));
        return;
    }

    let mut cursor = 0usize;
    for p in local_positions {
        if cursor < p {
            spans.push(Span::styled(&token[cursor..p], base_style));
        }
        let ch = token[p..]
            .chars()
            .next()
            .expect("match offset lands on a char boundary");
        let next = p + ch.len_utf8();
        spans.push(Span::styled(&token[p..next], hl_style));
        cursor = next;
    }
    if cursor < token.len() {
        spans.push(Span::styled(&token[cursor..], base_style));
    }
}

/// The live-timer tag is never shown in a task row: it's a wall-clock
/// timestamp (noise on a task line — the status bar already shows the
/// running timer and its elapsed time). `log:` is also hidden: it's internal
/// bookkeeping (which day the accumulated time belongs to) that the timesheet
/// view owns. Both are hidden regardless of config.
const ALWAYS_HIDDEN_KEYS: [&str; 2] = ["start", "log"];

/// Compact human duration for a task's `dur:`, or `None` when there's nothing
/// worth showing (`dur:0` from a just-started timer, or a malformed value).
fn dur_badge(task: &Task) -> Option<String> {
    match task.dur {
        Some(secs) if secs > 0 => Some(crate::app::format_compact_duration(secs)),
        _ => None,
    }
}

fn push_metadata<'a>(metadata: &mut Vec<Span<'a>>, badge: Span<'a>) {
    if !metadata.is_empty() {
        metadata.push(Span::raw(" "));
    }
    metadata.push(badge);
}

fn dur_badge_style(task_done: bool, theme: &Theme) -> Style {
    let color = if task_done { theme.done } else { theme.dim };
    apply_dim(Style::default().fg(color), task_done)
}

/// Compact non-billable badge for a task's `bill:`, or `None` when the task
/// is billable (the default). `bill:n` → `DNB`; `bill:y`/absent render nothing.
fn bill_badge(task: &Task) -> Option<&'static str> {
    (task.bill.as_deref() == Some("n")).then_some("DNB")
}

fn bill_badge_style(task_done: bool, theme: &Theme) -> Style {
    let color = if task_done { theme.done } else { theme.due };
    apply_dim(Style::default().fg(color), task_done)
}

/// True when `token` is a `key:value` pair whose key (case-insensitively)
/// appears in `hidden_keys` or is always hidden (`start:`).
fn is_hidden_kv(token: &str, hidden_keys: &[String], show_log_inline: bool) -> bool {
    match token.split_once(':') {
        Some((k, v)) if !k.is_empty() && !v.is_empty() => {
            (ALWAYS_HIDDEN_KEYS.iter().any(|h| h.eq_ignore_ascii_case(k))
                && !(show_log_inline && k.eq_ignore_ascii_case("log")))
                || hidden_keys.iter().any(|h| h.eq_ignore_ascii_case(k))
        }
        _ => false,
    }
}

pub(crate) fn is_url_token(token: &str) -> bool {
    token.starts_with("http://") || token.starts_with("https://")
}

pub(crate) fn url_token_style(task_done: bool, theme: &Theme) -> Style {
    let color = if task_done { theme.done } else { theme.accent };
    let mut style = Style::default()
        .fg(color)
        .add_modifier(Modifier::UNDERLINED);
    if task_done {
        style = style.add_modifier(Modifier::DIM);
    }
    style
}

fn sigil_token_color(token: &str, task: &Task, theme: &Theme) -> Option<Color> {
    if !token.starts_with('+') && !token.starts_with('@') {
        return None;
    }
    if task.done {
        return Some(theme.done);
    }
    if token.starts_with('+') {
        Some(theme.project)
    } else {
        Some(theme.context)
    }
}

fn apply_dim(style: Style, dim: bool) -> Style {
    if dim {
        style.add_modifier(Modifier::DIM)
    } else {
        style
    }
}

#[derive(Copy, Clone)]
enum DueStatus {
    Overdue,
    Today,
    Soon,
    Later,
    None,
}

fn due_status(due: &str, today: &str) -> DueStatus {
    if due.len() != 10 || today.len() != 10 {
        return DueStatus::None;
    }
    match due.cmp(today) {
        std::cmp::Ordering::Less => DueStatus::Overdue,
        std::cmp::Ordering::Equal => DueStatus::Today,
        std::cmp::Ordering::Greater => {
            // within 2 days?
            let d = day_diff(due, today).unwrap_or(99);
            if d <= 2 {
                DueStatus::Soon
            } else {
                DueStatus::Later
            }
        }
    }
}

fn day_diff(a: &str, b: &str) -> Option<i64> {
    let to_ymd = |s: &str| -> Option<(i32, u32, u32)> {
        let y = s.get(0..4)?.parse().ok()?;
        let mo = s.get(5..7)?.parse().ok()?;
        let d = s.get(8..10)?.parse().ok()?;
        Some((y, mo, d))
    };
    let (ay, am, ad) = to_ymd(a)?;
    let (by, bm, bd) = to_ymd(b)?;
    let da = chrono::NaiveDate::from_ymd_opt(ay, am, ad)?;
    let db = chrono::NaiveDate::from_ymd_opt(by, bm, bd)?;
    Some(da.signed_duration_since(db).num_days())
}

pub(crate) fn due_token_style(task_done: bool, due: &str, today: &str, theme: &Theme) -> Style {
    let status = due_status(due, today);
    let c = if task_done {
        theme.done
    } else {
        match status {
            DueStatus::Overdue => theme.overdue,
            DueStatus::Today => theme.today,
            DueStatus::Soon => theme.due,
            DueStatus::Later | DueStatus::None => theme.dim,
        }
    };
    let mut style = Style::default().fg(c);
    if matches!(status, DueStatus::Overdue | DueStatus::Today) {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

#[must_use]
pub fn due_label(due: &str, today: &str) -> String {
    if let Some(d) = day_diff(due, today) {
        if d < 0 {
            return if d == -1 {
                "overdue 1d".into()
            } else {
                format!("overdue {}d", -d)
            };
        }
        if d == 0 {
            return "today".into();
        }
        if d == 1 {
            return "tomorrow".into();
        }
        if d < 7 {
            return format!("in {d}d");
        }
    }
    due.to_string()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::theme::MUTED;
    use crate::todo::parse_line;

    #[test]
    fn build_line_does_not_panic_on_unicode_with_match_term() {
        // Regression: the previous lowercase-find-then-byte-slice approach
        // panics here. "İ".to_lowercase() = "i" + combining dot (3 bytes vs
        // 2 in the original), so the match offset derived from the
        // lowercased string lands off a char boundary in the source token.
        let task = parse_line("İa").unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: Some("a"),
            today: "2026-05-06",
            hidden_keys: &[],
            show_duration_inline: true,
            show_log_inline: false,
            timer_running: false,
            timer_elapsed: None,
        };
        // Build must not panic; we don't assert on the rendered spans.
        let _ = build_line(&task, opts, &MUTED, 1000);
    }

    #[test]
    fn build_line_highlights_subsequence_chars() {
        // "cade" is a subsequence of "Call dentist": C(0), a(1), D(5), e(6).
        // The renderer should emit highlighted single-char spans for those
        // positions, with the unmatched chars rendered in the base style.
        let task = parse_line("Call dentist").unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: Some("cade"),
            today: "2026-05-06",
            hidden_keys: &[],
            show_duration_inline: true,
            show_log_inline: false,
            timer_running: false,
            timer_elapsed: None,
        };
        let line = build_line(&task, opts, &MUTED, 1000);
        let highlight_bg = MUTED.matched;
        let highlighted: String = line
            .spans
            .iter()
            .filter(|s| s.style.bg == Some(highlight_bg))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(highlighted, "Cade");
    }

    /// Render `raw` and return the body text (all span content joined,
    /// fixed glyph/priority prefix trimmed). Tasks here carry no priority
    /// and aren't done, so the prefix is pure leading whitespace and the
    /// "no leading body space" invariant makes `trim_start` exact.
    fn body_text(raw: &str, hidden: &[String]) -> String {
        body_text_with_visibility(raw, hidden, true, false, 1000)
    }

    fn body_text_with_visibility(
        raw: &str,
        hidden: &[String],
        show_duration_inline: bool,
        show_log_inline: bool,
        width: u16,
    ) -> String {
        let task = parse_line(raw).unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: None,
            today: "2026-05-06",
            hidden_keys: hidden,
            show_duration_inline,
            show_log_inline,
            timer_running: false,
            timer_elapsed: None,
        };
        let line = build_line(&task, opts, &MUTED, width);
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn hidden_key_in_middle_omitted() {
        let h = vec!["uid".to_string()];
        assert_eq!(
            body_text("Call dentist uid:abc-123 @phone +health", &h),
            "Call dentist @phone +health",
        );
    }

    #[test]
    fn hidden_key_at_start_omitted() {
        let h = vec!["uid".to_string()];
        assert_eq!(body_text("uid:abc-123 Call dentist", &h), "Call dentist");
    }

    #[test]
    fn hidden_key_at_end_omitted() {
        let h = vec!["uid".to_string()];
        assert_eq!(body_text("Call dentist uid:abc-123", &h), "Call dentist");
    }

    #[test]
    fn adjacent_hidden_keys_collapse_to_single_space() {
        let h = vec!["uid".to_string(), "sync".to_string()];
        assert_eq!(body_text("Call uid:a sync:b dentist", &h), "Call dentist",);
    }

    #[test]
    fn hidden_key_match_is_case_insensitive() {
        let h = vec!["uid".to_string()];
        assert_eq!(body_text("Call UID:abc done", &h), "Call done");
    }

    #[test]
    fn empty_hidden_list_renders_everything_unchanged() {
        assert_eq!(
            body_text("Call dentist uid:abc @phone +health", &[]),
            "Call dentist uid:abc @phone +health",
        );
    }

    #[test]
    fn start_token_always_hidden_from_rows() {
        // The live `start:` timestamp must never leak into a task row (it's
        // wall-clock noise; the status bar shows the running timer instead).
        // `dur:` now renders as a compact badge, not raw seconds.
        assert_eq!(
            body_text(
                "Draft motion +Smith start:2026-05-06T09:00:00 dur:3600",
                &[],
            ),
            "Draft motion +Smith 1h 00m",
        );
        assert_eq!(
            body_text("Call dentist start:2026-05-06T09:00:00 @phone", &[]),
            "Call dentist @phone",
        );
    }

    #[test]
    fn log_token_always_hidden_from_rows() {
        // `log:` is internal bookkeeping (the day accumulated time belongs to)
        // — hidden regardless of config, like `start:`.
        assert_eq!(
            body_text("Draft motion +Smith dur:3600 log:2026-08-06", &[]),
            "Draft motion +Smith 1h 00m",
        );
    }

    #[test]
    fn dur_token_renders_compact_human_badge() {
        // Raw second counts are meaningless to a lawyer at a glance; `dur:`
        // renders as a compact hours/minutes badge instead.
        assert_eq!(
            body_text("Meeting +Client dur:2700", &[]),
            "Meeting +Client 45m"
        );
        assert_eq!(
            body_text("Review +Matter dur:4020", &[]),
            "Review +Matter 1h 07m"
        );
    }

    #[test]
    fn zero_or_malformed_dur_renders_nothing() {
        // `dur:0` (a just-started timer) and unparseable values are noise.
        assert_eq!(body_text("Call +Smith dur:0", &[]), "Call +Smith");
        assert_eq!(body_text("Call +Smith dur:notanumber", &[]), "Call +Smith");
        // Mid-body `dur:0` must not leave a doubled space behind.
        assert_eq!(body_text("Call dur:0 +Smith", &[]), "Call +Smith");
    }

    #[test]
    fn bill_n_renders_dnb_badge() {
        // `bill:n` → a compact `DNB` (do-not-bill) marker, not the raw token.
        assert_eq!(
            body_text("Firm admin +Admin dur:900 bill:n", &[]),
            "Firm admin +Admin 15m DNB",
        );
    }

    #[test]
    fn billable_renders_no_badge() {
        // Billable is the default: `bill:y` and an absent tag both render
        // nothing (the raw `bill:y` token is dropped, not shown).
        assert_eq!(
            body_text("Draft +Smith dur:3600 bill:y", &[]),
            "Draft +Smith 1h 00m",
        );
        assert_eq!(
            body_text("Draft +Smith dur:3600", &[]),
            "Draft +Smith 1h 00m"
        );
    }

    #[test]
    fn inline_duration_and_log_visibility_are_configurable() {
        let raw = "Draft +Smith dur:3600 log:2026-05-06";
        assert_eq!(body_text(raw, &[]), "Draft +Smith 1h 00m");
        assert_eq!(
            body_text_with_visibility(raw, &[], false, false, 1000),
            "Draft +Smith"
        );
        assert_eq!(
            body_text_with_visibility(raw, &[], true, true, 1000),
            "Draft +Smith log:2026-05-06 1h 00m"
        );
    }

    #[test]
    fn metadata_column_stays_visible_when_body_is_truncated() {
        let raw = "A very long narrative that must be clipped +Smith dur:7200 bill:n";
        let task = parse_line(raw).unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: None,
            today: "2026-05-06",
            hidden_keys: &[],
            show_duration_inline: true,
            show_log_inline: false,
            timer_running: false,
            timer_elapsed: None,
        };
        let line = build_line(&task, opts, &MUTED, 40);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.ends_with("2h 00m DNB"),
            "metadata must be rightmost: {text:?}"
        );
        assert!(
            text.contains('…'),
            "narrative should be truncated: {text:?}"
        );
        assert!(text.chars().count() <= 40, "row exceeds width: {text:?}");
    }

    #[test]
    fn done_task_renders_duration_and_dnb_badges() {
        // Archive (done.txt) rows go through the same `build_line` path, so a
        // completed matter still shows its time and non-billable flag, with
        // the raw dur:/log:/bill:n prefixes stripped.
        let task =
            parse_line("x 2026-05-06 2026-05-01 Firm admin +Admin dur:900 bill:n log:2026-05-01")
                .unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: None,
            today: "2026-05-06",
            hidden_keys: &[],
            show_duration_inline: true,
            show_log_inline: false,
            timer_running: false,
            timer_elapsed: None,
        };
        let line = build_line(&task, opts, &MUTED, 1000);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("15m"), "duration badge missing: {text:?}");
        assert!(text.contains("DNB"), "non-billable badge missing: {text:?}");
        assert!(!text.contains("dur:"), "raw dur: leaked: {text:?}");
        assert!(!text.contains("log:"), "raw log: leaked: {text:?}");
        assert!(!text.contains("bill:"), "raw bill: leaked: {text:?}");
    }

    #[test]
    fn url_token_is_underlined_and_accented() {
        // The underline modifier is the sentinel `ui::hyperlinks::linkify`
        // looks for. If this test fails, OSC 8 hyperlinks silently stop being
        // emitted — break it intentionally only when changing the marker.
        let task = parse_line("See https://example.com for details").unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: None,
            today: "2026-05-06",
            hidden_keys: &[],
            show_duration_inline: true,
            show_log_inline: false,
            timer_running: false,
            timer_elapsed: None,
        };
        let line = build_line(&task, opts, &MUTED, 1000);
        let url_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "https://example.com")
            .expect("URL token rendered as its own span");
        assert!(
            url_span.style.add_modifier.contains(Modifier::UNDERLINED),
            "URL span must carry Modifier::UNDERLINED; got {:?}",
            url_span.style,
        );
        assert_eq!(url_span.style.fg, Some(MUTED.accent));
    }

    #[test]
    fn url_token_not_classified_as_key_value() {
        // Without the URL branch in front of the generic key:value branch,
        // `http:` would split into ("http", "//example.com") and render with
        // the dim key-value style instead of the accent + underline.
        let task = parse_line("note http://example.com").unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: None,
            today: "2026-05-06",
            hidden_keys: &[],
            show_duration_inline: true,
            show_log_inline: false,
            timer_running: false,
            timer_elapsed: None,
        };
        let line = build_line(&task, opts, &MUTED, 1000);
        let url_span = line
            .spans
            .iter()
            .find(|s| s.content.as_ref() == "http://example.com")
            .expect("URL span");
        assert_ne!(
            url_span.style.fg,
            Some(MUTED.dim),
            "URL must not pick up the dim key-value color",
        );
    }

    #[test]
    fn non_listed_key_not_hidden() {
        let h = vec!["uid".to_string()];
        // `due:` stays; only configured keys are dropped.
        assert_eq!(
            body_text("Pay rent due:2026-05-15 uid:x", &h),
            "Pay rent due:2026-05-15",
        );
    }

    #[test]
    fn truncate_to_width_keeps_short_input() {
        let spans = vec![Span::raw("hello"), Span::raw(" world")];
        let out = truncate_to_width(&spans, 20);
        let text: String = out.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "hello world");
    }

    #[test]
    fn truncate_to_width_ellipsizes_long_input() {
        let spans = vec![Span::raw("the quick brown fox")];
        let out = truncate_to_width(&spans, 10);
        let text: String = out.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "the quick…");
        assert_eq!(text.chars().count(), 10);
    }

    #[test]
    fn truncate_to_width_zero_is_empty() {
        assert!(truncate_to_width(&[Span::raw("abc")], 0).is_empty());
    }

    #[test]
    fn build_line_truncates_body_to_keep_timer_visible() {
        let task = parse_line("Draft a very long narrative that overflows the row width by a lot")
            .unwrap();
        let opts = RowOpts {
            idx_label: 0,
            cursor: false,
            multi_mode: false,
            multi_checked: false,
            selected: false,
            show_line_num: false,
            match_term: None,
            today: "2026-05-06",
            hidden_keys: &[],
            show_duration_inline: true,
            show_log_inline: false,
            timer_running: true,
            timer_elapsed: Some(65),
        };
        let line = build_line(&task, opts, &MUTED, 40);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.contains("[00:01:05]"),
            "timer must stay visible in the truncated row: {text:?}"
        );
        assert!(
            text.chars().count() <= 40,
            "row must not exceed max_width: {text:?}"
        );
        assert!(text.contains('…'), "body must be ellipsized: {text:?}");
    }
}
