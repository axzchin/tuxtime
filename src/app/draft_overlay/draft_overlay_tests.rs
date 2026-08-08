#![allow(clippy::unwrap_used)]
//! Unit tests for the draft overlays (slash menu, calendar, recurrence
//! builder, priority chooser, duration picker) and the `apply_kv` /
//! `apply_priority` buffer writers. Included from [`super::draft_overlay`]
//! via `mod tests;`, so `super::*` resolves to that module.

use super::*;
use crate::app::test_support::build_app;
use crate::recurrence::RecUnit;

#[test]
fn apply_kv_appends_when_absent() {
    let mut app = build_app("");
    app.draft_set("Buy milk".into());
    app.apply_kv("due", Some("2026-05-12"));
    assert_eq!(app.draft.text(), "Buy milk due:2026-05-12");
}

#[test]
fn apply_kv_replaces_when_present() {
    let mut app = build_app("");
    app.draft_set("Buy milk due:2026-05-01 +groceries".into());
    app.apply_kv("due", Some("2026-05-12"));
    assert_eq!(app.draft.text(), "Buy milk due:2026-05-12 +groceries");
}

#[test]
fn apply_kv_clear_removes_token_and_space() {
    let mut app = build_app("");
    app.draft_set("Buy milk due:2026-05-01 +groceries".into());
    app.apply_kv("due", None);
    // Leading space before `due:` is also dropped so we don't leave "  ".
    assert_eq!(app.draft.text(), "Buy milk +groceries");
}

#[test]
fn apply_kv_clear_when_absent_is_noop() {
    let mut app = build_app("");
    app.draft_set("Buy milk".into());
    app.apply_kv("due", None);
    assert_eq!(app.draft.text(), "Buy milk");
}

#[test]
fn apply_kv_appends_to_empty_buffer_without_leading_space() {
    let mut app = build_app("");
    app.apply_kv("due", Some("2026-05-12"));
    assert_eq!(app.draft.text(), "due:2026-05-12");
}

#[test]
fn apply_priority_prepends_when_absent() {
    let mut app = build_app("");
    app.draft_set("Buy milk".into());
    app.apply_priority(Some('A'));
    assert_eq!(app.draft.text(), "(A) Buy milk");
}

#[test]
fn apply_priority_replaces_when_present() {
    let mut app = build_app("");
    app.draft_set("(A) Buy milk".into());
    app.apply_priority(Some('B'));
    assert_eq!(app.draft.text(), "(B) Buy milk");
}

#[test]
fn apply_priority_clears_when_present() {
    let mut app = build_app("");
    app.draft_set("(A) Buy milk".into());
    app.apply_priority(None);
    assert_eq!(app.draft.text(), "Buy milk");
}

#[test]
fn apply_priority_clear_when_absent_is_noop() {
    let mut app = build_app("");
    app.draft_set("Buy milk".into());
    app.apply_priority(None);
    assert_eq!(app.draft.text(), "Buy milk");
}

#[test]
fn find_kv_token_range_skips_non_matching_prefix() {
    // `rec:1w` must not be picked up when looking for `re` — token has to
    // start with the exact `key:` prefix.
    assert!(find_kv_token_range("Hi rec:1w", "re").is_none());
    // But the exact key matches.
    assert!(find_kv_token_range("Hi rec:1w", "rec").is_some());
}

#[test]
fn find_kv_token_range_picks_first_only() {
    // Mirrors `todo::find_kv` — first wins. The replacement target is
    // therefore the first token.
    let r = find_kv_token_range("a due:2026-01-01 b due:2026-02-02", "due").unwrap();
    assert_eq!(&"a due:2026-01-01 b due:2026-02-02"[r], "due:2026-01-01");
}

#[test]
fn format_rec_value_emits_strict_prefix() {
    let s = RecurrenceBuilderState {
        interval: 2,
        unit: RecUnit::Month,
        strict: true,
        field: BuilderField::Interval,
        anchor: None,
    };
    assert_eq!(format_rec_value(&s), "+2m");
    let s2 = RecurrenceBuilderState {
        interval: 1,
        unit: RecUnit::Week,
        strict: false,
        field: BuilderField::Interval,
        anchor: None,
    };
    assert_eq!(format_rec_value(&s2), "1w");
}

#[test]
fn slash_menu_opens_at_bol() {
    let mut app = build_app("");
    app.nav.mode = crate::app::Mode::Insert;
    app.draft_insert_char('/');
    app.maybe_open_slash_menu();
    assert!(matches!(
        app.draft.overlay(),
        Some(DraftOverlay::SlashMenu(_))
    ));
}

#[test]
fn slash_menu_opens_after_whitespace() {
    let mut app = build_app("");
    app.draft_set("Hi ".into());
    app.draft_insert_char('/');
    app.maybe_open_slash_menu();
    assert!(matches!(
        app.draft.overlay(),
        Some(DraftOverlay::SlashMenu(_))
    ));
}

#[test]
fn slash_menu_does_not_open_mid_word() {
    // `https:/...` — the `/` follows `:` which isn't whitespace.
    let mut app = build_app("");
    app.draft_set("https:".into());
    app.draft_insert_char('/');
    app.maybe_open_slash_menu();
    assert!(app.draft.overlay().is_none());
}

#[test]
fn slash_menu_filter_narrows_entries() {
    let mut app = build_app("");
    app.draft_insert_char('/');
    app.maybe_open_slash_menu();
    // Typing `due` narrows to "Due date" only (not Duration).
    app.draft_insert_char('d');
    app.draft_insert_char('u');
    app.draft_insert_char('e');
    let matches = app.slash_matches();
    assert!(matches.iter().any(|e| e.kind == SlashKind::Due));
    assert!(matches.iter().all(|e| e.kind == SlashKind::Due));
}

#[test]
fn slash_menu_filter_dur_matches_duration() {
    let mut app = build_app("");
    app.draft_insert_char('/');
    app.maybe_open_slash_menu();
    app.draft_insert_char('d');
    app.draft_insert_char('u');
    app.draft_insert_char('r');
    let matches = app.slash_matches();
    assert!(matches.iter().any(|e| e.kind == SlashKind::Duration));
    assert!(matches.iter().all(|e| e.kind == SlashKind::Duration));
}

#[test]
fn slash_menu_revalidates_when_slash_deleted() {
    let mut app = build_app("");
    app.draft_insert_char('/');
    app.maybe_open_slash_menu();
    assert!(app.draft.overlay().is_some());
    app.draft_backspace();
    app.slash_menu_revalidate();
    assert!(app.draft.overlay().is_none());
}

#[test]
fn slash_menu_closes_when_space_typed_after_slash() {
    // Prose like "Option A / B" has a space before the `/` (which opens the
    // menu) but is not a command. Typing a space after the `/` must drop the
    // menu so Enter saves the todo instead of being swallowed by the menu.
    let mut app = build_app("");
    app.draft_set("Option A ".into());
    app.draft_insert_char('/');
    app.maybe_open_slash_menu();
    assert!(app.draft.overlay().is_some());
    app.draft_insert_char(' ');
    app.slash_menu_revalidate();
    assert!(app.draft.overlay().is_none());
    // The typed text is preserved — revalidate only closes the menu.
    assert_eq!(app.draft.text(), "Option A / ");
}

#[test]
fn slash_cancel_removes_trigger_text() {
    let mut app = build_app("");
    app.draft_set("Hi ".into());
    app.draft_insert_char('/');
    app.maybe_open_slash_menu();
    app.draft_insert_char('d');
    app.draft_insert_char('u');
    app.slash_cancel();
    assert_eq!(app.draft.text(), "Hi ");
    assert!(app.draft.overlay().is_none());
}

#[test]
fn slash_accept_due_opens_calendar() {
    let mut app = build_app("");
    app.draft_insert_char('/');
    app.maybe_open_slash_menu();
    app.slash_accept();
    // Default selection is the first entry — Due date. Calendar should open.
    assert!(matches!(
        app.draft.overlay(),
        Some(DraftOverlay::Calendar(_))
    ));
    // The `/` literal is gone.
    assert_eq!(app.draft.text(), "");
}

#[test]
fn slash_accept_proj_inserts_sigil_and_no_overlay() {
    let mut app = build_app("");
    app.draft_set("Buy milk ".into());
    app.draft_insert_char('/');
    app.maybe_open_slash_menu();
    // Filter to /proj.
    app.draft_insert_char('p');
    app.draft_insert_char('r');
    app.draft_insert_char('o');
    app.slash_accept();
    assert!(app.draft.overlay().is_none());
    assert!(app.draft.text().ends_with('+'));
}

#[test]
fn calendar_accept_writes_due_token() {
    let mut app = build_app("");
    app.draft_set("Buy milk".into());
    app.open_calendar(CalendarTarget::Due);
    // Default focus = today (2026-05-06 from test_support).
    app.calendar_accept();
    assert!(app.draft.overlay().is_none());
    assert_eq!(app.draft.text(), "Buy milk due:2026-05-06");
}

#[test]
fn calendar_clear_removes_existing_due() {
    let mut app = build_app("");
    app.draft_set("Buy milk due:2026-05-12".into());
    app.open_calendar(CalendarTarget::Due);
    app.calendar_clear();
    assert_eq!(app.draft.text(), "Buy milk");
}

#[test]
fn calendar_reopens_focused_on_existing_value() {
    let mut app = build_app("");
    app.draft_set("Buy milk due:2026-07-04".into());
    app.open_calendar(CalendarTarget::Due);
    let s = app.calendar_state().unwrap();
    assert_eq!(s.focused, NaiveDate::from_ymd_opt(2026, 7, 4).unwrap());
}

#[test]
fn recurrence_accept_writes_rec_token() {
    let mut app = build_app("");
    app.draft_set("Water plants".into());
    app.open_recurrence_builder();
    // Default = 1, Week, after-complete → "rec:1w".
    app.recurrence_accept();
    assert_eq!(app.draft.text(), "Water plants rec:1w");
}

#[test]
fn recurrence_adjust_interval_clamps_at_one() {
    let mut app = build_app("");
    app.open_recurrence_builder();
    app.recurrence_adjust(-10);
    let s = app.recurrence_state().unwrap();
    assert_eq!(s.interval, 1);
}

#[test]
fn recurrence_strict_mode_emits_plus_prefix() {
    let mut app = build_app("");
    app.draft_set("Pay rent".into());
    app.open_recurrence_builder();
    app.recurrence_focus(2); // Interval -> Mode (skipping Unit)
    app.recurrence_adjust(1); // toggle strict
    let s = app.recurrence_state().unwrap();
    assert!(s.strict);
    app.recurrence_accept();
    assert_eq!(app.draft.text(), "Pay rent rec:+1w");
}

#[test]
fn priority_accept_writes_pri_token() {
    let mut app = build_app("");
    app.draft_set("Buy milk".into());
    app.open_priority_chooser();
    // selected=0 → A.
    app.priority_accept();
    assert_eq!(app.draft.text(), "(A) Buy milk");
}

#[test]
fn priority_clear_removes_existing() {
    let mut app = build_app("");
    app.draft_set("(A) Buy milk".into());
    app.open_priority_chooser();
    app.priority_step(false); // 0 -> 3 (clear)
    app.priority_accept();
    assert_eq!(app.draft.text(), "Buy milk");
}

#[test]
fn typing_due_colon_opens_calendar_with_anchor() {
    let mut app = build_app("");
    app.draft_set("Buy milk ".into());
    for c in "due:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    let s = app.calendar_state().expect("calendar should be open");
    assert_eq!(s.target, CalendarTarget::Due);
    // Anchor points at the `d` of `due:`.
    assert_eq!(s.anchor, Some("Buy milk ".len()));
}

#[test]
fn typing_t_colon_opens_threshold_calendar() {
    let mut app = build_app("");
    app.draft_set("Pay rent ".into());
    for c in "t:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    let s = app.calendar_state().expect("calendar should be open");
    assert_eq!(s.target, CalendarTarget::Threshold);
    assert_eq!(s.anchor, Some("Pay rent ".len()));
}

#[test]
fn typing_rec_colon_opens_recurrence_builder() {
    let mut app = build_app("");
    app.draft_set("Water plants ".into());
    for c in "rec:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    let s = app.recurrence_state().expect("builder should be open");
    assert_eq!(s.anchor, Some("Water plants ".len()));
}

#[test]
fn kv_trigger_fires_at_bol() {
    let mut app = build_app("");
    for c in "due:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    let s = app.calendar_state().expect("calendar should be open");
    assert_eq!(s.anchor, Some(0));
}

#[test]
fn kv_trigger_does_not_panic_on_multibyte_prefix() {
    // Regression: `match_key_before` used byte arithmetic and then sliced
    // `text[key_start..colon_pos]` directly. If the char immediately
    // before `due`/`rec` is multi-byte, `key_start` lands mid-codepoint
    // and the slice would panic. With `text.get` it just returns `None`
    // and the overlay stays closed.
    for prefix in ["réc", "ñdue", "✓rec"] {
        let mut app = build_app("");
        app.draft_set(prefix.to_string());
        app.draft_insert_char(':');
        // Must not panic; must not open an overlay (boundary char before
        // the key is not whitespace).
        app.maybe_open_kv_overlay();
        assert!(
            app.draft.overlay().is_none(),
            "overlay must stay closed for prefix {prefix:?}",
        );
    }
}

#[test]
fn recurrence_builder_preserves_business_day_unit() {
    // Regression: opening the builder on an existing `rec:3b` used to put
    // BusinessDay outside the unit cycle, so adjusting any other field
    // silently coerced it to Week on the next +/-. Now BusinessDay is in
    // REC_UNIT_ORDER and round-trips intact.
    let mut app = build_app("");
    app.draft_set("Submit timesheet rec:3b".into());
    app.open_recurrence_builder();
    let s = app.recurrence_state().expect("builder open");
    assert_eq!(s.unit, RecUnit::BusinessDay);
    // Cycle the Mode field — the unit must not move.
    app.recurrence_focus(2); // Interval -> Mode
    app.recurrence_adjust(1); // toggle strict
    let s = app.recurrence_state().expect("builder still open");
    assert_eq!(
        s.unit,
        RecUnit::BusinessDay,
        "unit must survive a Mode toggle"
    );
    app.recurrence_accept();
    assert_eq!(app.draft.text(), "Submit timesheet rec:+3b");
}

#[test]
fn recurrence_unit_cycle_includes_business_day() {
    // Stepping through the unit cycle must reach BusinessDay.
    let mut app = build_app("");
    app.open_recurrence_builder();
    app.recurrence_focus(1); // Interval -> Unit
    let order_len = REC_UNIT_ORDER.len();
    let mut seen: Vec<RecUnit> = Vec::with_capacity(order_len);
    for _ in 0..order_len {
        let s = app.recurrence_state().expect("builder open");
        seen.push(s.unit);
        app.recurrence_adjust(1);
    }
    assert!(seen.contains(&RecUnit::BusinessDay));
}

#[test]
fn kv_trigger_does_not_fire_mid_word() {
    // `Recipe:` ends with `e:`, not a recognised key, and the boundary
    // before `due` etc. isn't whitespace either — must not pop.
    let mut app = build_app("");
    app.draft_set("Recipe".into());
    app.draft_insert_char(':');
    app.maybe_open_kv_overlay();
    assert!(app.draft.overlay().is_none());

    // `Mydue:` — `due` appears but the char before is `y`, not whitespace.
    let mut app2 = build_app("");
    app2.draft_set("Mydue".into());
    app2.draft_insert_char(':');
    app2.maybe_open_kv_overlay();
    assert!(app2.draft.overlay().is_none());

    // `let:` — single-letter `t:` test variant. Char before `t` is `e`,
    // not whitespace.
    let mut app3 = build_app("");
    app3.draft_set("let".into());
    app3.draft_insert_char(':');
    app3.maybe_open_kv_overlay();
    assert!(app3.draft.overlay().is_none());
}

#[test]
fn kv_trigger_accept_strips_literal_and_appends() {
    let mut app = build_app("");
    app.draft_set("Buy milk ".into());
    for c in "due:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    app.calendar_accept();
    // The empty `due:` we typed is stripped; the canonical token sits at
    // the end. Exactly one `due:` in the result.
    assert_eq!(app.draft.text(), "Buy milk due:2026-05-06");
    assert_eq!(app.draft.text().matches("due:").count(), 1);
}

#[test]
fn kv_trigger_accept_updates_existing_due() {
    // Re-triggering on a line that already has `due:DATE` means "change
    // the date" — the existing token gets the new value and the empty
    // literal we just typed disappears.
    let mut app = build_app("");
    app.draft_set("Buy milk due:2026-04-01 +groceries".into());
    app.draft_insert_char(' ');
    for c in "due:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    let s = app.calendar_state().expect("calendar should be open");
    // Focused date should be the existing value.
    assert_eq!(s.focused, NaiveDate::from_ymd_opt(2026, 4, 1).unwrap());
    // User picks today (the `t` shortcut) → date jumps to 2026-05-06.
    app.calendar_set_relative(0);
    app.calendar_accept();
    assert_eq!(
        app.draft.text(),
        "Buy milk due:2026-05-06 +groceries",
        "existing due: should be updated and the empty trigger removed",
    );
    assert_eq!(app.draft.text().matches("due:").count(), 1);
}

#[test]
fn kv_trigger_cancel_leaves_literal_in_buffer() {
    // Esc behaves like @/+ autocomplete: the typed `due:` stays so the
    // user can finish the date by hand if they want.
    let mut app = build_app("");
    app.draft_set("Buy milk ".into());
    for c in "due:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    app.calendar_cancel();
    assert_eq!(app.draft.text(), "Buy milk due:");
    assert!(app.draft.overlay().is_none());
}

#[test]
fn kv_trigger_rec_accept_writes_canonical_token() {
    let mut app = build_app("");
    app.draft_set("Water plants ".into());
    for c in "rec:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    // Default builder = 1 week after-complete.
    app.recurrence_accept();
    assert_eq!(app.draft.text(), "Water plants rec:1w");
    assert_eq!(app.draft.text().matches("rec:").count(), 1);
}

#[test]
fn end_to_end_flow_writes_full_task() {
    // Simulates the full add-task flow with the slash menu:
    //   type body → `/` → /due → calendar T → Enter → save.
    // The final task line must round-trip through `parse_line` with the
    // expected metadata fields populated.
    let mut app = build_app("");
    app.nav.mode = crate::app::Mode::Insert;
    for c in "Schedule team offsite".chars() {
        app.draft_insert_char(c);
    }
    app.draft_insert_char(' ');
    app.draft_insert_char('/');
    app.maybe_open_slash_menu();
    // Default selection is "Due date".
    app.slash_accept();
    assert!(matches!(
        app.draft.overlay(),
        Some(DraftOverlay::Calendar(_))
    ));
    // T = tomorrow → 2026-05-07 (today is 2026-05-06 in test_support).
    app.calendar_set_relative(1);
    app.calendar_accept();
    assert!(app.draft.overlay().is_none());
    assert_eq!(app.draft.text(), "Schedule team offsite due:2026-05-07");

    // Saving runs through `parse_line` and prepends today as creation
    // date. After save the task list grows by one with the expected fields.
    app.add_from_draft();
    let task = app.tasks().last().expect("task added");
    assert_eq!(task.due.as_deref(), Some("2026-05-07"));
    assert_eq!(task.created_date.as_deref(), Some("2026-05-06"));
    assert!(task.raw.contains("Schedule team offsite"));
}

#[test]
fn typing_date_after_trigger_syncs_calendar_focused() {
    let mut app = build_app("");
    app.draft_set("Buy milk ".into());
    for c in "due:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    // Type a full ISO date into the draft while the calendar is open.
    for c in "2026-12-25".chars() {
        app.draft_insert_char(c);
        app.calendar_sync_from_draft();
    }
    let s = app.calendar_state().expect("calendar should still be open");
    assert_eq!(s.focused, NaiveDate::from_ymd_opt(2026, 12, 25).unwrap());
}

#[test]
fn typing_date_after_trigger_accept_writes_single_token() {
    let mut app = build_app("");
    app.draft_set("Buy milk ".into());
    for c in "due:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    for c in "2026-12-25".chars() {
        app.draft_insert_char(c);
        app.calendar_sync_from_draft();
    }
    app.calendar_accept();
    assert_eq!(app.draft.text(), "Buy milk due:2026-12-25");
    assert_eq!(app.draft.text().matches("due:").count(), 1);
}

#[test]
fn backspace_past_colon_closes_calendar() {
    let mut app = build_app("");
    for c in "due:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    // Backspace once removes ':' — anchor's KEY: no longer present.
    app.draft_backspace();
    app.calendar_sync_from_draft();
    assert!(app.draft.overlay().is_none(), "calendar should close");
}

#[test]
fn partial_date_does_not_move_focused() {
    let mut app = build_app("");
    for c in "due:".chars() {
        app.draft_insert_char(c);
    }
    app.maybe_open_kv_overlay();
    let initial_focused = app.calendar_state().unwrap().focused;
    // Type an incomplete date — should not move the focused cell.
    for c in "2026-12".chars() {
        app.draft_insert_char(c);
        app.calendar_sync_from_draft();
    }
    assert_eq!(app.calendar_state().unwrap().focused, initial_focused);
}
