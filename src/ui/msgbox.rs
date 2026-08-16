//! Presentation helpers shared by every floating box: the framed panel
//! (`frame_box`), the small prompt message boxes built on it
//! (`render_message_box`), plus the word-wrapping and padding primitives they
//! all use. One frame implementation keeps every overlay — prompt messages,
//! help, welcome — visually consistent.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::theme::Theme;

/// Render the shared framed panel used by every floating overlay: `ALL`
/// borders in `border_color` over `background`, with `title`. Returns the
/// inner content rect. Message-box prompts pass `theme.accent` borders so they
/// draw the eye; informational overlays (help, welcome, palette, theme
/// picker, empty state) pass `theme.border` (and `theme.bg` for the body-level
/// empty state, `theme.panel` for true floating panels).
pub(crate) fn frame_box(
    frame: &mut Frame,
    area: Rect,
    border_color: Color,
    background: Color,
    title: Line<'_>,
) -> Rect {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color).bg(background))
        .title(title)
        .style(Style::default().bg(background));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

/// Right-pad `s` to `width` graphemes with spaces (no-op when already at or
/// over the width).
#[must_use]
pub(crate) fn pad_right(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        let mut out = String::with_capacity(s.len() + (width - len));
        out.push_str(s);
        for _ in len..width {
            out.push(' ');
        }
        out
    }
}

/// Wrap `s` to roughly `width` graphemes, returning each output line as a
/// vector of borrowed words. Borrowing avoids the per-frame `String` alloc
/// that a `Vec<String>` form would force on every render.
pub(crate) fn wrap_words(s: &str, width: usize) -> Vec<Vec<&str>> {
    let mut out: Vec<Vec<&str>> = Vec::new();
    let mut acc: Vec<&str> = Vec::new();
    let mut acc_len = 0;
    for word in s.split_whitespace() {
        let wlen = word.chars().count();
        let extra = usize::from(!acc.is_empty());
        if acc_len + wlen + extra > width && !acc.is_empty() {
            out.push(std::mem::take(&mut acc));
            acc_len = 0;
        }
        if !acc.is_empty() {
            acc_len += 1;
        }
        acc.push(word);
        acc_len += wlen;
    }
    if !acc.is_empty() {
        out.push(acc);
    }
    out
}

/// Number of lines `message` occupies when word-wrapped to `width` graphemes.
/// Callers use it to size the box before rendering so wrapped text always
/// fits.
#[must_use]
pub(crate) fn wrapped_line_count(message: &str, width: usize) -> usize {
    wrap_words(message, width).len().max(1)
}

/// Draw a titled, bordered message box containing `message` (word-wrapped to
/// the box's inner width), a blank row, and a dim hint `footer` line. Lines
/// are centered, matching the inline prompt dialogs' presentation. The caller
/// clears the area first and picks the box rect.
pub(crate) fn render_message_box(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    title: &str,
    message: &str,
    footer: &str,
) {
    let inner = frame_box(frame, area, theme.accent, theme.panel, Line::from(title));

    // Wrap to the inner width: callers size the box from the same width
    // (`box_w - 2` borders), so the line count they computed matches what
    // actually renders.
    let wrap_w = (inner.width as usize).max(16);
    let mut lines: Vec<Line> = Vec::new();
    for chunk in wrap_words(message, wrap_w) {
        lines.push(Line::from(Span::styled(
            chunk.join(" "),
            Style::default()
                .fg(theme.fg)
                .bg(theme.panel)
                .add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::raw(""));
    lines.push(footer_line(theme, footer));
    frame.render_widget(Paragraph::new(lines).centered(), inner);
}

/// Render a footer string with every `[Key]` token bolded and the rest dim.
/// This is the single place message-box key legends get their styling, so the
/// `[S]tart timer` / `[M] add time` / `[Esc] cancel` family renders
/// consistently across every prompt instead of drifting per call site.
fn footer_line<'a>(theme: &Theme, footer: &'a str) -> Line<'a> {
    let dim = Style::default().fg(theme.dim).bg(theme.panel);
    let key = Style::default()
        .fg(theme.dim)
        .bg(theme.panel)
        .add_modifier(Modifier::BOLD);
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut rest = footer;
    while let Some(idx) = rest.find('[') {
        if idx > 0 {
            spans.push(Span::styled(&rest[..idx], dim));
        }
        let after = &rest[idx..];
        let end = after.find(']').map_or(after.len(), |i| idx + i + 1);
        spans.push(Span::styled(&rest[idx..end], key));
        rest = &rest[end..];
    }
    if !rest.is_empty() {
        spans.push(Span::styled(rest, dim));
    }
    Line::from(spans).style(Style::default().bg(theme.panel))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn wrap_words_splits_on_word_boundaries() {
        let out = wrap_words("one two three four five", 10);
        assert_eq!(
            out.iter().map(|c| c.join(" ")).collect::<Vec<_>>(),
            vec!["one two", "three four", "five"]
        );
    }

    #[test]
    fn wrap_words_single_word_smaller_than_width_stays_one_line() {
        let out = wrap_words("hello", 100);
        assert_eq!(out, vec![vec!["hello"]]);
    }

    #[test]
    fn wrap_words_long_word_alone_overflows_rather_than_splitting() {
        // Word-level wrapping: a single word wider than the box is kept whole
        // (todo.txt task lines are practically bounded, so a hard character
        // break adds complexity without a real caller).
        let out = wrap_words("abcdefghijklmnopqrstuvwxyz", 8);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].join(" "), "abcdefghijklmnopqrstuvwxyz");
    }

    #[test]
    fn wrap_words_empty_input_yields_no_lines() {
        assert!(wrap_words("", 20).is_empty());
        assert_eq!(wrapped_line_count("", 20), 1);
    }

    #[test]
    fn pad_right_pads_short_and_passes_long() {
        assert_eq!(pad_right("x", 4), "x   ");
        assert_eq!(pad_right("long", 4), "long");
        assert_eq!(pad_right("", 2), "  ");
        // Grapheme-aware: multibyte characters count once.
        assert_eq!(pad_right("→", 3), "→  ");
    }

    #[test]
    fn footer_line_bolds_keys_and_dims_the_rest() {
        let theme = &crate::theme::MUTED;
        let line = footer_line(theme, "[S]tart timer  [M] add time  [Esc] cancel");
        // Key tokens are bolded; the action words and separators are not.
        let bold: Vec<&str> = line
            .spans
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::BOLD))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(bold, vec!["[S]", "[M]", "[Esc]"]);
        let plain: String = line
            .spans
            .iter()
            .filter(|s| !s.style.add_modifier.contains(Modifier::BOLD))
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(
            plain, "tart timer   add time   cancel",
            "action words stay unbolded"
        );
    }

    #[test]
    fn footer_line_handles_string_without_keys() {
        let theme = &crate::theme::MUTED;
        let line = footer_line(theme, "no keys here");
        assert!(
            line.spans
                .iter()
                .all(|s| !s.style.add_modifier.contains(Modifier::BOLD))
        );
    }
}
