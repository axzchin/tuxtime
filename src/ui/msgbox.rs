//! Presentation helpers for the small floating message boxes used by the
//! inline prompt dialogs (idle nudge, manual-entry choice, day boundary).
//! Word-wrapping and the framed box are shared so a long message wraps to the
//! box width instead of being clipped at the edge.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::theme::Theme;

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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.accent).bg(theme.panel))
        .title(title)
        .style(Style::default().bg(theme.panel));
    let inner = block.inner(area);
    frame.render_widget(block, area);

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
    lines.push(Line::from(Span::styled(
        footer,
        Style::default().fg(theme.dim).bg(theme.panel),
    )));
    frame.render_widget(Paragraph::new(lines).centered(), inner);
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
}
