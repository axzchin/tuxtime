//! Rendering for the add/edit dialog and the floating metadata pickers whose
//! state lives in [`crate::app::draft_overlay_state`] (`DraftOverlay`,
//! `CalendarState`, `SlashMenuState`, …). This module only draws — it never
//! mutates the pickers' state.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::{App, DraftOverlay, Mode, Prompt, TokenKind};
use crate::theme::Theme;
use crate::todo::{SegmentKind, classify_draft};
use crate::ui::{msgbox, overlay};

mod calendar;
mod duration;
mod priority;
mod recurrence;
mod slash;

/// Syntax-highlighted draft with cursor inversion. Walks `classify_draft`
/// and emits one styled span per segment, splitting whichever segment
/// contains the cursor so its glyph stays readable with swapped fg/bg.
pub(crate) fn highlighted_draft_spans<'a>(
    draft: &'a str,
    cursor: usize,
    theme: &Theme,
) -> Vec<Span<'a>> {
    let segments = classify_draft(draft);
    let cursor = cursor.min(draft.len());
    let mut out: Vec<Span<'a>> = Vec::new();

    for (range, kind) in segments {
        let style = segment_style(kind, theme);
        if cursor >= range.start && cursor < range.end {
            let before = &draft[range.start..cursor];
            let next = next_boundary(draft, cursor);
            let cursor_char = &draft[cursor..next];
            let after = &draft[next..range.end];
            if !before.is_empty() {
                out.push(Span::styled(before, style));
            }
            // Invert: glyph fg = panel bg, glyph bg = segment colour.
            let fg = style.fg.unwrap_or(theme.fg);
            let inv = Style::default().fg(theme.panel).bg(fg);
            out.push(Span::styled(cursor_char, inv));
            if !after.is_empty() {
                out.push(Span::styled(after, style));
            }
        } else {
            out.push(Span::styled(&draft[range.start..range.end], style));
        }
    }

    if cursor == draft.len() {
        out.push(Span::styled("█", Style::default().fg(theme.fg)));
    }
    out
}

fn segment_style(kind: SegmentKind, theme: &Theme) -> Style {
    let (color, bold) = match kind {
        SegmentKind::Plain => (theme.fg, false),
        SegmentKind::Priority(p) => (theme.priority_color(p), true),
        SegmentKind::Date => (theme.dim, false),
        SegmentKind::Project => (theme.project, false),
        SegmentKind::Context => (theme.context, false),
        SegmentKind::Due => (theme.due, false),
        SegmentKind::KeyValue => (theme.dim, false),
    };
    let s = Style::default().fg(color);
    if bold {
        s.add_modifier(Modifier::BOLD)
    } else {
        s
    }
}

fn next_boundary(s: &str, i: usize) -> usize {
    let len = s.len();
    let mut j = (i + 1).min(len);
    while j < len && !s.is_char_boundary(j) {
        j += 1;
    }
    j
}

/// Render `draft` with the insertion point highlighted at byte offset `cursor`.
/// When the cursor sits past the last char, append a block glyph; otherwise the
/// character under the cursor is drawn with swapped fg/bg so it stays readable.
#[must_use]
pub fn draft_cursor_spans(draft: &str, cursor: usize, fg: Color, bg: Color) -> Vec<Span<'_>> {
    let cursor = cursor.min(draft.len());
    let before = &draft[..cursor];
    let after = &draft[cursor..];
    let mut iter = after.char_indices();
    if let Some((_, _)) = iter.next() {
        let next = iter.next().map_or(after.len(), |(i, _)| i);
        let cursor_char = &after[..next];
        let rest = &after[next..];
        vec![
            Span::styled(before, Style::default().fg(fg)),
            Span::styled(cursor_char, Style::default().fg(bg).bg(fg)),
            Span::styled(rest, Style::default().fg(fg)),
        ]
    } else {
        vec![
            Span::styled(before, Style::default().fg(fg)),
            Span::styled("█", Style::default().fg(fg)),
        ]
    }
}

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let title = if app.selection.editing().is_some() {
        " EDIT TASK "
    } else {
        " ADD TASK "
    };
    let inner = msgbox::frame_box(
        frame,
        area,
        theme.border,
        theme.panel,
        Line::from(vec![Span::styled(
            title,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
    );

    let [_p1, input_area, preview_area, _p2, hint_area, _p3] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    // Split the input row into a fixed prefix ("  › ") and a scrollable
    // content area. Without this, long drafts get clipped at the dialog's
    // right edge — including the cursor itself, so the user can't see what
    // they're typing. The prefix never scrolls; the content paragraph offsets
    // horizontally to keep the cursor onscreen.
    const PREFIX_W: u16 = 4;
    let [prefix_area, content_area] =
        Layout::horizontal([Constraint::Length(PREFIX_W), Constraint::Min(0)]).areas(input_area);

    let prefix_line = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            "› ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
    ])
    .style(Style::default().bg(theme.panel));
    frame.render_widget(
        Paragraph::new(prefix_line).style(Style::default().bg(theme.panel)),
        prefix_area,
    );

    let content_line = Line::from(highlighted_draft_spans(
        app.draft.text(),
        app.draft.cursor(),
        theme,
    ))
    .style(Style::default().bg(theme.panel));
    let cursor = app.draft.cursor().min(app.draft.text().len());
    let cursor_col = app.draft.text()[..cursor].chars().count();
    let avail = content_area.width as usize;
    // Pin the cursor to the rightmost visible column whenever it would
    // otherwise overflow. Stateless: when the cursor moves left of the
    // viewport, scroll naturally drops back to 0.
    let scroll_x = if avail == 0 {
        0
    } else {
        cursor_col.saturating_sub(avail.saturating_sub(1)) as u16
    };
    frame.render_widget(
        Paragraph::new(content_line)
            .style(Style::default().bg(theme.panel))
            .scroll((0, scroll_x)),
        content_area,
    );

    let preview = preview_line(app);
    frame.render_widget(
        Paragraph::new(preview).style(Style::default().bg(theme.panel)),
        preview_area,
    );

    let hint = hint_line(theme);
    frame.render_widget(
        Paragraph::new(hint).style(Style::default().bg(theme.panel)),
        hint_area,
    );
}

fn preview_line<'a>(app: &App) -> Line<'a> {
    let theme = app.theme();
    let parsed = match app.preview_parse() {
        Some(r) => r,
        None => return Line::raw("").style(Style::default().bg(theme.panel)),
    };
    let mut spans: Vec<Span<'a>> = vec![Span::raw("  ")];
    match parsed {
        Ok(t) => {
            spans.push(Span::styled("ok ", Style::default().fg(theme.dim)));
            if let Some(p) = t.priority {
                spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
                spans.push(Span::styled(
                    format!("pri {p} "),
                    Style::default()
                        .fg(theme.priority_color(p))
                        .add_modifier(Modifier::BOLD),
                ));
            }
            if let Some(d) = t.due {
                spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
                spans.push(Span::styled(
                    format!("due {d} "),
                    Style::default().fg(theme.due),
                ));
            }
            let np = t.projects.len();
            let nc = t.contexts.len();
            if np + nc > 0 {
                spans.push(Span::styled("· ", Style::default().fg(theme.dim)));
            }
            if np > 0 {
                spans.push(Span::styled(
                    format!("{np} +"),
                    Style::default().fg(theme.dim),
                ));
                spans.push(Span::styled(
                    if np == 1 { "project " } else { "projects " },
                    Style::default().fg(theme.project),
                ));
            }
            if nc > 0 {
                spans.push(Span::styled(
                    format!("{nc} @"),
                    Style::default().fg(theme.dim),
                ));
                spans.push(Span::styled(
                    if nc == 1 { "context" } else { "contexts" },
                    Style::default().fg(theme.context),
                ));
            }
        }
        Err(e) => {
            spans.push(Span::styled(
                "err ",
                Style::default()
                    .fg(theme.overdue)
                    .add_modifier(Modifier::BOLD),
            ));
            spans.push(Span::styled(format!("{e}"), Style::default().fg(theme.dim)));
        }
    }
    Line::from(spans).style(Style::default().bg(theme.panel))
}

/// Every prompt mode in one match, so a mode added here can't silently fall
/// through to a cleared (black) box. `PromptAddTime`, the two nudge-threshold
/// prompts and the rename prompt used to hit the `_ => return` arm, leaving
/// the caller's `Clear` as an empty rectangle on screen.
pub fn render_prompt(frame: &mut Frame, area: Rect, app: &App) {
    let theme = app.theme();
    let (sigil, label, sigil_color) = match app.nav.mode {
        Mode::Prompt(prompt) => match prompt {
            Prompt::Project => ("+", " ADD PROJECT ", theme.project),
            Prompt::Context => ("@", " TOGGLE CONTEXT ", theme.context),
            Prompt::SaveFilter => ("✦", " SAVE FILTER AS ", theme.accent),
            Prompt::AddTime => ("⏱", " ADD TIME ", theme.accent),
            Prompt::IdleNudge => ("⏱", " IDLE NUDGE (MIN) ", theme.accent),
            Prompt::LongTimerNudge => ("⏱", " LONG TIMER NUDGE (MIN) ", theme.accent),
            Prompt::RenameProject => ("+", " RENAME PROJECT ", theme.project),
            Prompt::DayBoundary => return,
        },
        _ => return,
    };
    let inner = msgbox::frame_box(
        frame,
        area,
        theme.border,
        theme.panel,
        Line::from(Span::styled(
            label,
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )),
    );

    let [_p, input_area, _p2] = Layout::vertical([
        Constraint::Length(0),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);

    let mut spans = vec![
        Span::raw("  "),
        Span::styled(
            sigil,
            Style::default()
                .fg(sigil_color)
                .add_modifier(Modifier::BOLD),
        ),
        // One space between the sigil and the draft text: the sigil glyph
        // (a wide emoji in many terminals) would otherwise sit directly on
        // the first character of the input.
        Span::raw(" "),
    ];
    spans.extend(draft_cursor_spans(
        app.draft.text(),
        app.draft.cursor(),
        theme.fg,
        theme.panel,
    ));
    let line = Line::from(spans).style(Style::default().bg(theme.panel));
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme.panel)),
        input_area,
    );
}

/// Colored example tokens illustrating the todo.txt format.
/// Used by both the empty state and the add/edit dialog so they stay in sync.
#[must_use]
pub fn format_hint_spans<'a>(theme: &Theme) -> Vec<Span<'a>> {
    use ratatui::style::Modifier;
    vec![
        Span::styled(
            "(A) ",
            Style::default()
                .fg(theme.pri_a)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Buy milk ", Style::default().fg(theme.fg)),
        Span::styled("+shop ", Style::default().fg(theme.project)),
        Span::styled("@home ", Style::default().fg(theme.context)),
        Span::styled("due:2026-05-12", Style::default().fg(theme.due)),
    ]
}

/// Floating suggestion popup anchored just below the add/edit dialog.
/// `dlg` is the dialog rect we're attached to; `screen` is the full frame
/// area, used to keep the popup on-screen when the dialog is near the bottom
/// or right edge. No-op when the popup is hidden.
pub fn render_autocomplete(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) {
    if !app.autocomplete_visible() {
        return;
    }
    let matches = app.autocomplete_matches();
    if matches.is_empty() {
        return;
    }
    let theme = app.theme();
    let kind = match app.autocomplete_target() {
        Some(t) => t.kind,
        None => return,
    };
    let (sigil, sigil_color) = match kind {
        TokenKind::Project => ('+', theme.project),
        TokenKind::Context => ('@', theme.context),
    };

    let longest = matches.iter().map(|s| s.chars().count()).max().unwrap_or(0);
    let area = overlay::autocomplete_popup_rect(dlg, screen, longest, matches.len());
    frame.render_widget(Clear, area);

    let selected = app.draft.autocomplete_index().min(matches.len() - 1);
    let lines: Vec<Line> = matches
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let is_sel = i == selected;
            let bg = if is_sel { theme.accent } else { theme.panel };
            let fg = if is_sel { theme.bg } else { theme.fg };
            Line::from(vec![
                Span::styled(
                    format!(" {}", sigil),
                    Style::default()
                        .fg(sigil_color)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("{} ", s), Style::default().fg(fg).bg(bg)),
            ])
            .style(Style::default().bg(bg))
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.panel)),
        area,
    );
}

/// Dispatch to the right per-overlay render function. Returns true when an
/// overlay rendered, so the caller can skip the regular autocomplete popup.
pub fn render_overlay(frame: &mut Frame, dlg: Rect, screen: Rect, app: &App) -> bool {
    match app.draft.overlay() {
        Some(DraftOverlay::SlashMenu(_)) => {
            slash::render_slash_menu(frame, dlg, screen, app);
            true
        }
        Some(DraftOverlay::Calendar(_)) => {
            calendar::render_calendar(frame, dlg, screen, app);
            true
        }
        Some(DraftOverlay::RecurrenceBuilder(_)) => {
            recurrence::render_recurrence_builder(frame, dlg, screen, app);
            true
        }
        Some(DraftOverlay::PriorityChooser(_)) => {
            priority::render_priority_chooser(frame, dlg, screen, app);
            true
        }
        Some(DraftOverlay::DurationPicker(_)) => {
            duration::render_duration_picker(frame, dlg, screen, app);
            true
        }
        None => false,
    }
}

fn hint_line<'a>(theme: &Theme) -> Line<'a> {
    let mut spans = vec![
        Span::raw("  "),
        Span::styled("format: ", Style::default().fg(theme.dim)),
    ];
    spans.extend(format_hint_spans(theme));
    Line::from(spans).style(Style::default().bg(theme.panel))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;

    use crate::app::{App, Mode, Prompt, Screen};
    use crate::config::Config;

    /// Pull just the rows immediately below the centered Insert dialog where
    /// the popup floats — avoids matching against the sidebar / status bar
    /// content that contains the same project / context names.
    fn popup_region_text(buf: &Buffer) -> String {
        // Mirror the dialog placement in `ui::draw`: 8 rows tall, centered.
        // The popup begins at dlg.y + dlg.height and is up to 8 rows tall.
        let rows = buf.area.height;
        let cols = buf.area.width;
        let dlg_h: u16 = 8;
        let dlg_y = (rows.saturating_sub(dlg_h)) / 2;
        let popup_top = dlg_y + dlg_h;
        let popup_bottom = (popup_top + 8).min(rows);
        let mut out = String::new();
        for y in popup_top..popup_bottom {
            for x in 0..cols {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn build_insert_app(seed: &str, draft: &str) -> App {
        let path = std::env::temp_dir().join(format!(
            "tuxtime-dialog-test-{}-{}.txt",
            std::process::id(),
            seed.len(),
        ));
        std::fs::write(&path, seed).unwrap();
        let mut app = App::new(
            path,
            seed.to_string(),
            "2026-05-06".to_string(),
            Config::default(),
        );
        app.nav.mode = Mode::Screen(Screen::Insert);
        app.draft_set_insert(draft.to_string());
        app
    }

    #[test]
    fn calendar_cells_sunday_start_places_days_without_off_by_one() {
        use crate::app::WeekStart;
        // May 2026: the 1st is a Friday (weekday index 5 in a Sunday-leading grid).
        let first = chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let grid = crate::ui::calendar_utils::calendar_cells(first, WeekStart::Sunday);

        // Leading blanks Sun..Thu, then day 1 under Friday, day 2 under Saturday.
        assert_eq!(
            grid[0],
            [None, None, None, None, None, Some(1), Some(2)],
            "first day must land under its real weekday with no phantom day 0"
        );

        // Every real day appears exactly once, in order 1..=31 (no 0, no missing 31).
        let days: Vec<u32> = grid.iter().flatten().flatten().copied().collect();
        assert_eq!(days, (1..=31).collect::<Vec<_>>());
    }

    #[test]
    fn calendar_cells_monday_start_shifts_first_column() {
        use crate::app::WeekStart;
        // May 2026, 1st = Friday. In a Monday-leading grid Friday is column 4.
        let first = chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let grid = crate::ui::calendar_utils::calendar_cells(first, WeekStart::Monday);

        assert_eq!(grid[0], [None, None, None, None, Some(1), Some(2), Some(3)]);
        let days: Vec<u32> = grid.iter().flatten().flatten().copied().collect();
        assert_eq!(days, (1..=31).collect::<Vec<_>>());
    }

    /// Pull the dialog's interior rows (between the borders) — preview lives
    /// on row 3 of the inner area in the current layout.
    fn dialog_inner_text(buf: &Buffer) -> String {
        let rows = buf.area.height;
        let cols = buf.area.width;
        let dlg_h: u16 = 9;
        let dlg_y = (rows.saturating_sub(dlg_h)) / 2;
        let mut out = String::new();
        for y in dlg_y..(dlg_y + dlg_h) {
            for x in 0..cols {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn input_row_scrolls_to_keep_cursor_visible_for_long_draft() {
        // A draft longer than the dialog's content area must scroll
        // horizontally so the tail (where the cursor sits) stays visible —
        // otherwise the user can't see what they're typing past the right
        // edge.
        let tail = "ZZSCROLLTAIL";
        let draft = format!("{}{}", "x".repeat(80), tail);
        let app = build_insert_app("plain\n", &draft);
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let buf = terminal.backend().buffer();
        // Dialog is 8 rows tall, centered in a 30-row area; input lives on
        // the second inner row (top border + 1 row padding + input).
        let dlg_y = (30u16 - 8) / 2;
        let input_y = dlg_y + 2;
        let mut row = String::new();
        for x in 0..80 {
            row.push_str(buf[(x, input_y)].symbol());
        }
        assert!(
            row.contains(tail),
            "input row should scroll so the cursor end ({tail}) stays visible:\n{row}"
        );
    }

    #[test]
    fn preview_line_shows_priority_chip() {
        let app = build_insert_app("plain\n", "(A) Buy milk");
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let text = dialog_inner_text(terminal.backend().buffer());
        assert!(text.contains("ok"), "preview should say 'ok'\n{text}");
        assert!(
            text.contains("pri A"),
            "preview should show 'pri A'\n{text}"
        );
    }

    #[test]
    fn preview_line_blank_when_draft_empty() {
        let app = build_insert_app("plain\n", "");
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let text = dialog_inner_text(terminal.backend().buffer());
        // No "ok" or "err" badge when draft is empty.
        assert!(
            !text.contains("ok "),
            "empty draft should not render preview\n{text}"
        );
        assert!(
            !text.contains("err "),
            "empty draft should not render preview\n{text}"
        );
    }

    #[test]
    fn autocomplete_popup_renders_project_matches() {
        let app = build_insert_app(
            "(A) 2026-05-01 a +work\n(A) 2026-05-01 b +health\n",
            "Foo +",
        );
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let popup = popup_region_text(terminal.backend().buffer());
        assert!(
            popup.contains("health"),
            "expected 'health' in popup\n{popup}"
        );
        assert!(popup.contains("work"), "expected 'work' in popup\n{popup}");
    }

    #[test]
    fn autocomplete_popup_hidden_when_no_token() {
        // A draft with no `+` / `@` token at the cursor should leave the
        // popup region empty even if the corpus has projects.
        let app = build_insert_app("(A) 2026-05-01 a +uniqueprojname\n", "plain text");
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let popup = popup_region_text(terminal.backend().buffer());
        assert!(
            !popup.contains("uniqueprojname"),
            "popup region should not list corpus when no active token\n{popup}"
        );
    }

    #[test]
    fn autocomplete_popup_filters_by_context_kind() {
        let app = build_insert_app("(A) 2026-05-01 a +uniqueprojname @uniquecontext\n", "Foo @");
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let popup = popup_region_text(terminal.backend().buffer());
        assert!(
            popup.contains("uniquecontext"),
            "expected context value in popup\n{popup}"
        );
        assert!(
            !popup.contains("uniqueprojname"),
            "context popup must not list projects\n{popup}"
        );
    }

    fn build_prompt_app(seed: &str, draft: &str, mode: Mode) -> App {
        let path = std::env::temp_dir().join(format!(
            "tuxtime-prompt-dialog-test-{}-{}.txt",
            std::process::id(),
            seed.len(),
        ));
        std::fs::write(&path, seed).unwrap();
        let mut app = App::new(
            path,
            seed.to_string(),
            "2026-05-06".to_string(),
            Config::default(),
        );
        app.nav.mode = mode;
        app.draft_set(draft.to_string());
        app
    }

    fn prompt_popup_region_text(buf: &Buffer) -> String {
        let rows = buf.area.height;
        let cols = buf.area.width;
        let dlg_h: u16 = 5; // PROMPT_H
        let dlg_y = (rows.saturating_sub(dlg_h)) / 2;
        let popup_top = dlg_y + dlg_h;
        let popup_bottom = (popup_top + 8).min(rows);
        let mut out = String::new();
        for y in popup_top..popup_bottom {
            for x in 0..cols {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn prompt_autocomplete_popup_renders_project_matches() {
        let app = build_prompt_app(
            "(A) 2026-05-01 a +work\n(A) 2026-05-01 b +health\n",
            "hea",
            Mode::Prompt(Prompt::Project),
        );
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let popup = prompt_popup_region_text(terminal.backend().buffer());
        assert!(
            popup.contains("health"),
            "expected 'health' in popup\n{popup}"
        );
        assert!(
            !popup.contains("work"),
            "expected 'work' not to be in popup (doesn't match 'hea')\n{popup}"
        );
    }

    #[test]
    fn prompt_autocomplete_popup_renders_context_matches() {
        let app = build_prompt_app(
            "(A) 2026-05-01 a @work\n(A) 2026-05-01 b @health\n",
            "hea",
            Mode::Prompt(Prompt::Context),
        );
        let backend = TestBackend::new(80, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &app)).unwrap();
        let popup = prompt_popup_region_text(terminal.backend().buffer());
        assert!(
            popup.contains("health"),
            "expected 'health' in popup\n{popup}"
        );
        assert!(
            !popup.contains("work"),
            "expected 'work' not to be in popup (doesn't match 'hea')\n{popup}"
        );
    }
}
