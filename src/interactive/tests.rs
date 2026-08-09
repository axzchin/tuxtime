#![allow(clippy::unwrap_used)]

use super::*;
use crate::action::Action;
use crate::app::{App, Mode, View};
use crate::config::Config;
use crate::keybinds::KeyBindings;
use chrono::NaiveDate;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}

fn resolve(app: &mut App, key: KeyEvent) -> Option<Action> {
    resolve_normal_key(app, key, &KeyBindings::default())
}

fn welcome_app(name: &str) -> (App, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "tuxtime-welcome-{name}-{}-{:?}.txt",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_file(&path);
    let mut app = App::new(
        path.clone(),
        String::new(),
        "2026-05-07".into(),
        Config::default(),
    );
    app.nav.mode = Mode::Welcome;
    (app, path)
}

#[test]
fn welcome_c_creates_cwd_file_and_enters_normal() {
    let (mut app, path) = welcome_app("c");
    assert!(!path.exists(), "precondition: file must not exist yet");
    handle_welcome(&mut app, key('c'));
    assert!(path.exists(), "`c` must create the target file");
    assert_eq!(app.nav.mode, Mode::Normal);
    assert_eq!(app.env.file_path, path, "`c` keeps the cwd target path");
    assert!(!app.nav.should_quit);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn welcome_s_opens_sample_and_enters_normal() {
    let (mut app, path) = welcome_app("s");
    handle_welcome(&mut app, key('s'));
    assert_eq!(app.nav.mode, Mode::Normal);
    assert_ne!(
        app.env.file_path, path,
        "`s` rebinds away from the cwd target"
    );
    assert!(
        app.env.file_path.ends_with("tuxtime-sample.txt"),
        "`s` opens the bundled sample, got {:?}",
        app.env.file_path
    );
    assert!(!app.tasks().is_empty(), "sample must load tasks");
    assert!(!path.exists(), "`s` must not create the cwd file");
}

#[test]
fn welcome_q_and_esc_quit_without_creating_anything() {
    let (mut app, path) = welcome_app("q");
    handle_welcome(&mut app, key('q'));
    assert!(app.nav.should_quit, "`q` must quit");
    assert!(!path.exists(), "`q` must not create a file");

    let (mut app, path) = welcome_app("esc");
    handle_welcome(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(app.nav.should_quit, "Esc must quit");
    assert!(!path.exists(), "Esc must not create a file");
}

fn build_app() -> App {
    let path = std::env::temp_dir().join(format!(
        "tuxtime-bindings-{}-{:?}.txt",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::write(&path, "a\nb\nc\n");
    App::new(
        path,
        "a\nb\nc\n".into(),
        "2026-05-07".into(),
        Config::default(),
    )
}

fn build_app_with_due() -> App {
    let path = std::env::temp_dir().join(format!(
        "tuxtime-bindings-{}-{:?}.txt",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::write(&path, "Buy milk due:2026-06-30\n");
    App::new(
        path,
        "Buy milk due:2026-06-30\n".into(),
        "2026-05-07".into(),
        Config::default(),
    )
}

#[test]
fn plain_keys_resolve_to_their_actions() {
    let mut app = build_app();
    assert_eq!(resolve(&mut app, key('q')), Some(Action::Quit));
    assert_eq!(resolve(&mut app, key('j')), Some(Action::CursorDown),);
    assert_eq!(resolve(&mut app, key('?')), Some(Action::OpenHelp));
    assert_eq!(resolve(&mut app, ctrl('d')), Some(Action::HalfPageDown),);
    assert_eq!(resolve(&mut app, key('n')), Some(Action::BeginAdd),);
    assert_eq!(resolve(&mut app, key('a')), Some(Action::ToggleArchiveView),);
    assert_eq!(resolve(&mut app, key('A')), Some(Action::ArchiveCompleted),);
    assert_eq!(resolve(&mut app, key('S')), Some(Action::CycleSort),);
}

#[test]
fn custom_keybinds_override_builtins() {
    let mut app = build_app();
    let keybinds = KeyBindings::parse("[normal]\nopen_help = \"q\"\n");
    assert_eq!(
        resolve_normal_key(&mut app, key('q'), &keybinds),
        Some(Action::OpenHelp)
    );
    assert_eq!(
        resolve_normal_key(&mut app, ctrl('d'), &keybinds),
        Some(Action::HalfPageDown),
    );
    assert_eq!(
        resolve_normal_key(&mut app, key('n'), &keybinds),
        Some(Action::BeginAdd),
    );
    assert_eq!(
        resolve_normal_key(&mut app, key('r'), &keybinds),
        Some(Action::Reschedule),
    );
    assert_eq!(
        resolve_normal_key(&mut app, key('a'), &keybinds),
        Some(Action::ToggleArchiveView),
    );
    assert_eq!(
        resolve_normal_key(&mut app, key('A'), &keybinds),
        Some(Action::ArchiveCompleted),
    );
    assert_eq!(
        resolve_normal_key(&mut app, key('S'), &keybinds),
        Some(Action::CycleSort),
    );
}

#[test]
fn capital_a_archives_only_when_completed_tasks_exist() {
    // No completed tasks → flash, no archive write.
    let mut app = build_app_with_archive("a\nb\nc\n", None);
    apply_action(&mut app, Action::ArchiveCompleted);
    assert_eq!(app.flash_active(), Some("no completed tasks to archive"));
    assert_eq!(app.tasks().len(), 3);

    // One completed task → archive_completed runs.
    let mut app = build_app_with_archive("x 2026-05-08 done one\nb\n", None);
    apply_action(&mut app, Action::ArchiveCompleted);
    assert_eq!(app.tasks().len(), 1, "completed task must be archived");
}

#[test]
fn lowercase_l_returns_to_list_from_any_view() {
    let mut app = build_app_with_archive("a\n", Some("x 2026-05-02 2026-04-02 done\n"));
    app.set_view(View::Archive);
    apply_action(&mut app, Action::GoList);
    assert_eq!(app.view(), View::List);
}

#[test]
fn lowercase_a_toggles_archive_view() {
    let mut app = build_app_with_archive("a\n", Some("x 2026-05-02 2026-04-02 done\n"));
    assert_eq!(app.view(), View::List);
    apply_action(&mut app, Action::ToggleArchiveView);
    assert_eq!(app.view(), View::Archive);
    apply_action(&mut app, Action::ToggleArchiveView);
    assert_eq!(app.view(), View::List);
}

#[test]
fn timesheet_copy_flashes_key_in_message() {
    // Build an app with a dur task for today so the timesheet is populated.
    let dir = std::env::temp_dir().join(format!(
        "tuxtime-timesheet-copy-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    let todo_path = dir.join("todo.txt");
    // Line must have today's date as creation date *and* a dur for
    // build_timesheet_groups to pick it up.
    let raw = "2026-05-07 Write code +work @dev dur:3600\n";
    std::fs::write(&todo_path, raw).expect("write todo.txt");
    let mut app = App::new(
        todo_path,
        raw.into(),
        "2026-05-07".into(),
        Config::default(),
    );
    app.set_view(View::Timesheet);

    // Verify we are indeed in Timesheet view before copying.
    assert_eq!(app.view(), View::Timesheet);
    assert!(app.flash_active().is_none(), "no flash before copy");

    handle_timesheet_keys(&mut app, key('c'));

    // Verify the flash message includes the project+activity key.
    // The status bar reads flash_active() to build its mode_label as
    // "TIMESHEET · copied narrative for +work @dev", so this assertion
    // directly verifies what the status bar will display.
    let flash = app.flash_active();
    assert!(
        flash.is_some_and(|m| m.contains("+work @dev")),
        "copy in timesheet must flash the project+activity key, got: {flash:?}"
    );
}

// ── timesheet sort ───────────────────────────────────────────────

#[test]
fn timesheet_sort_cycles_modes() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);

    assert_eq!(
        app.timesheet.sort,
        crate::app::TimesheetSort::ProjectActivity
    );
    handle_timesheet_keys(&mut app, key('s'));
    assert_eq!(app.timesheet.sort, crate::app::TimesheetSort::Date);
    assert_eq!(app.timesheet.cursor, 0, "sort change resets cursor");
    assert!(app.flash_active().is_some_and(|m| m.contains("by date")));

    handle_timesheet_keys(&mut app, key('s'));
    assert_eq!(app.timesheet.sort, crate::app::TimesheetSort::Duration);
    assert!(
        app.flash_active()
            .is_some_and(|m| m.contains("by duration"))
    );

    handle_timesheet_keys(&mut app, key('s'));
    assert_eq!(
        app.timesheet.sort,
        crate::app::TimesheetSort::ProjectActivity
    );
    assert!(app.flash_active().is_some_and(|m| m.contains("by project")));
}

#[test]
fn build_timesheet_groups_sorts_by_project() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    app.timesheet.sort = crate::app::TimesheetSort::ProjectActivity;
    let groups = app.build_timesheet_groups();
    assert!(!groups.is_empty());
    // ProjectActivity sort: keys should be in lexicographic order.
    for w in groups.windows(2) {
        assert!(w[0].key <= w[1].key, "keys must be sorted by project");
    }
}

#[test]
fn build_timesheet_groups_sorts_by_duration() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    app.timesheet.sort = crate::app::TimesheetSort::Duration;
    let groups = app.build_timesheet_groups();
    // Duration sort: descending by total_secs.
    for w in groups.windows(2) {
        assert!(w[0].total_secs >= w[1].total_secs, "durations must descend");
    }
}

#[test]
fn build_timesheet_groups_sorts_by_date() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    app.timesheet.weekly = true; // need weekly to see both dates
    app.timesheet.sort = crate::app::TimesheetSort::Date;
    let groups = app.build_timesheet_groups();
    assert!(groups.len() >= 2, "weekly must show at least 2 date groups");
    // Date sort: entries ordered by date then key.
    for w in groups.windows(2) {
        let ord = w[0]
            .date
            .cmp(&w[1].date)
            .then_with(|| w[0].key.cmp(&w[1].key));
        assert!(
            ord.is_le(),
            "date sort must be (date, key): {:?} vs {:?}",
            (&w[0].date, &w[0].key),
            (&w[1].date, &w[1].key)
        );
    }
}

#[test]
fn build_timesheet_groups_filters_by_search() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    // No search → all dur tasks appear.
    let all = app.build_timesheet_groups();
    assert!(!all.is_empty());
    // Search for a narrative substring that exists.
    app.set_search("Write".into());
    let filtered = app.build_timesheet_groups();
    assert!(!filtered.is_empty());
    // Search for something that doesn't exist.
    app.set_search("zzz_nonexistent".into());
    let none = app.build_timesheet_groups();
    assert!(none.is_empty());
    // Clear search → all back.
    app.clear_search();
    assert_eq!(app.build_timesheet_groups().len(), all.len());
}

#[test]
fn build_timesheet_groups_filters_by_project() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    let all = app.build_timesheet_groups();
    // Daily view anchored 2026-05-07: the 05-06 task is out of range, so
    // only the two 05-07 project+activity groups are present.
    assert_eq!(all.len(), 2, "fixture has 2 groups in today's daily view");

    app.set_project_filter(Some("work".into()));
    let filtered = app.build_timesheet_groups();
    assert!(!filtered.is_empty(), "+work tasks must appear");
    assert!(
        filtered.iter().all(|g| g.key.starts_with("+work")),
        "all groups must belong to +work: {:?}",
        filtered.iter().map(|g| &g.key).collect::<Vec<_>>()
    );

    // A filter matching nothing yields an empty timesheet.
    app.set_project_filter(Some("zzz_nonexistent".into()));
    assert!(app.build_timesheet_groups().is_empty());
}

#[test]
fn build_timesheet_groups_filters_by_context() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);

    app.set_context_filter(Some("research".into()));
    let filtered = app.build_timesheet_groups();
    assert!(!filtered.is_empty(), "@research tasks must appear");
    assert!(
        filtered.iter().all(|g| g.key.contains("@research")),
        "all groups must belong to @research: {:?}",
        filtered.iter().map(|g| &g.key).collect::<Vec<_>>()
    );
}

#[test]
fn timesheet_project_totals_splits_billable_and_dnb() {
    // Two projects; +work has billable and non-billable time, +legal only
    // billable. The sidebar must report each project's billable secs and
    // its non-billable secs separately.
    let raw = "2026-05-07 Billable work +work @dev dur:1800\n\
               2026-05-07 DNB work +work @dev dur:600 bill:n\n\
               2026-05-07 Legal work +legal @research dur:900\n";
    let app = timesheet_app_with_tasks(raw);
    let totals = app.timesheet_project_totals();
    let work = totals.iter().find(|(n, _, _)| n == "work").unwrap();
    let legal = totals.iter().find(|(n, _, _)| n == "legal").unwrap();
    assert_eq!(work.1, 1800, "+work billable secs");
    assert_eq!(work.2, 600, "+work non-billable secs");
    assert_eq!(legal.1, 900, "+legal billable secs");
    assert_eq!(legal.2, 0, "+legal has no non-billable time");
    // Sorted by billable descending.
    assert_eq!(totals[0].0, "work");
    assert_eq!(totals[1].0, "legal");
}

#[test]
fn timesheet_period_totals_sum_billable_and_non_billable() {
    let raw = "2026-05-07 Billable work +work @dev dur:1800\n\
               2026-05-07 DNB work +work @dev dur:600 bill:n\n\
               2026-05-07 Legal work +legal @research dur:900\n";
    let app = timesheet_app_with_tasks(raw);
    let (total, billable, non_billable) = app.timesheet_period_totals();
    assert_eq!(total, 3300);
    assert_eq!(billable, 2700);
    assert_eq!(non_billable, 600);
}

/// Timesheet `f` / `F` must open the same project/context pickers used by
/// the list view, so the timesheet can be filtered by matter without
/// leaving the view. The picker previews live on the timesheet.
#[test]
fn timesheet_f_and_shift_f_open_pickers() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    assert_eq!(app.nav.mode, Mode::Normal);

    handle_timesheet_keys(&mut app, key('f'));
    assert_eq!(app.nav.mode, Mode::PickProject);
    assert!(
        app.filter.project.is_some(),
        "picker must seed a project filter"
    );

    // Accept the picker: back to Normal, filter kept.
    app.pick_accept();
    assert_eq!(app.nav.mode, Mode::Normal);
    assert_eq!(app.view(), View::Timesheet, "must stay in timesheet view");

    handle_timesheet_keys(&mut app, key('F'));
    assert_eq!(app.nav.mode, Mode::PickContext);
    assert!(
        app.filter.context.is_some(),
        "picker must seed a context filter"
    );
    app.pick_accept();
    assert_eq!(app.nav.mode, Mode::Normal);
    assert_eq!(app.view(), View::Timesheet);
}

/// `f` in timesheet view must seed the picker from the entry under the
/// *timesheet* cursor, not the stale list cursor. The fixture's daily view
/// sorts by project+activity, so the +work @dev group ("Write code",
/// "Meeting notes") is at narrative cursor 1 — the project filter must seed
/// to "work" from there even though the list cursor points elsewhere.
#[test]
fn timesheet_f_seeds_picker_from_timesheet_cursor_entry() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    // Move the list cursor somewhere unrelated to prove seeding ignores it.
    app.nav.cursor = 3;
    app.timesheet.cursor = 1; // "Write code +work @dev" (group 2 of 2)

    handle_timesheet_keys(&mut app, key('f'));
    assert_eq!(app.nav.mode, Mode::PickProject);
    assert_eq!(
        app.filter.project.as_deref(),
        Some("work"),
        "picker must seed from the timesheet entry's project"
    );
    app.pick_accept();

    // Same for context via F: the entry is @dev.
    app.timesheet.cursor = 1;
    handle_timesheet_keys(&mut app, key('F'));
    assert_eq!(app.nav.mode, Mode::PickContext);
    assert_eq!(
        app.filter.context.as_deref(),
        Some("dev"),
        "picker must seed from the timesheet entry's context"
    );
    app.pick_accept();
}

#[test]
fn build_timesheet_groups_separates_billable_and_dnb() {
    // Same project+activity on the same day, one billable, one DNB.
    // They must form separate groups and round independently.
    let dir = std::env::temp_dir().join(format!(
        "tuxtime-ts-bill-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    let todo_path = dir.join("todo.txt");
    let raw = "2026-05-07 Billable work +work @dev dur:60\n2026-05-07 DNB work +work @dev dur:60 bill:n\n";
    std::fs::write(&todo_path, raw).expect("write todo.txt");
    let mut app = App::new(
        todo_path,
        raw.into(),
        "2026-05-07".into(),
        Config::default(),
    );
    app.set_view(View::Timesheet);
    let groups = app.build_timesheet_groups();
    // Must produce two distinct groups for the same key.
    assert_eq!(groups.len(), 2, "billable and DNB must be separate groups");
    assert_eq!(groups[0].key, "+work @dev");
    assert_eq!(groups[1].key, "+work @dev");
    // One group must be billable, the other DNB.
    assert_ne!(
        groups[0].billable, groups[1].billable,
        "groups must have different billable flags"
    );
    // Each 60-second group rounds to 0.1h independently.
    let billable_group = groups.iter().find(|g| g.billable).unwrap();
    let dnb_group = groups.iter().find(|g| !g.billable).unwrap();
    assert_eq!(billable_group.total_secs, 60);
    assert_eq!(dnb_group.total_secs, 60);
    // Verify the correct narratives landed in each group
    // (bill:n tag is stripped from body).
    assert_eq!(billable_group.narratives, vec!["Billable work"]);
    assert_eq!(dnb_group.narratives, vec!["DNB work"]);
    let billable_str = crate::app::format_billable(billable_group.total_secs, 0.1);
    let dnb_str = crate::app::format_billable(dnb_group.total_secs, 0.1);
    assert_eq!(billable_str, "0.1h");
    assert_eq!(dnb_str, "0.1h");
    // The sum of per-group units (from format_billable_units) should be 0.2h.
    let billable_units = crate::app::billable_units(billable_group.total_secs, 0.1);
    let dnb_units = crate::app::billable_units(dnb_group.total_secs, 0.1);
    assert_eq!(billable_units, 1);
    assert_eq!(dnb_units, 1);
    let total_units_str = crate::app::format_billable_units(billable_units + dnb_units, 0.1);
    assert_eq!(total_units_str, "0.2h");
}

// ── timesheet log-date attribution ────────────────────────────────

/// Regression: time must be attributed to the day it was actually logged
/// (`log:`), not the task's creation date. A task created two days ago and
/// tracked today must appear under today in the daily view.
#[test]
fn timesheet_attributes_time_to_log_date_not_creation_date() {
    let raw = "2026-05-05 Draft memo +legal @research dur:3600 log:2026-05-07\n";
    let app = timesheet_app_with_tasks(raw);
    let groups = app.build_timesheet_groups();
    assert_eq!(groups.len(), 1, "exactly one entry expected");
    assert_eq!(
        groups[0].date, "2026-05-07",
        "time must be attributed to the log date, not the creation date"
    );
    assert_eq!(groups[0].total_secs, 3600);
    assert_eq!(groups[0].narratives, vec!["Draft memo"]);
}

/// Regression: a task created today but tracked yesterday must NOT show in
/// today's daily view — it belongs to the day the time was logged.
#[test]
fn timesheet_daily_view_excludes_entries_logged_elsewhere() {
    let raw = "2026-05-07 Old work +legal @research dur:1800 log:2026-05-06\n";
    let mut app = timesheet_app_with_tasks(raw);
    assert!(
        app.build_timesheet_groups().is_empty(),
        "yesterday's work must not appear in today's daily view"
    );

    app.timesheet_shift_days(-1);
    let groups = app.build_timesheet_groups();
    assert_eq!(groups.len(), 1, "yesterday's view must show the entry");
    assert_eq!(groups[0].date, "2026-05-06");
}

/// Legacy/hand-typed lines without a `log:` tag must keep working: they fall
/// back to the creation date, preserving pre-existing behavior.
#[test]
fn timesheet_falls_back_to_creation_date_without_log() {
    let raw = "2026-05-07 Plain work +work @dev dur:900\n";
    let app = timesheet_app_with_tasks(raw);
    let groups = app.build_timesheet_groups();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].date, "2026-05-07");
}

/// A hand-typed or otherwise unparseable `log:` value must not hide the
/// entry: it falls back to the creation date like a missing tag.
#[test]
fn timesheet_falls_back_to_creation_date_when_log_invalid() {
    let raw = "2026-05-07 Bogus log +work @dev dur:600 log:garbage\n";
    let app = timesheet_app_with_tasks(raw);
    let groups = app.build_timesheet_groups();
    assert_eq!(groups.len(), 1, "invalid log must not hide the entry");
    assert_eq!(groups[0].date, "2026-05-07");
}

/// Weekly view must bucket by log date too: entries logged inside the week
/// appear, entries logged outside are excluded even if created before.
/// (Sunday-start week containing 2026-05-07 spans 2026-05-03..2026-05-09.)
#[test]
fn timesheet_weekly_view_attributes_by_log_date() {
    let raw = "2026-05-01 Early +work @dev dur:600 log:2026-05-04\n\
               2026-05-01 Late +work @dev dur:1200 log:2026-05-08\n\
               2026-05-01 Last week +legal @dev dur:300 log:2026-05-02\n";
    let mut app = timesheet_app_with_tasks(raw);
    app.timesheet.weekly = true;
    let groups = app.build_timesheet_groups();
    assert_eq!(
        groups.len(),
        2,
        "only entries logged inside the week should appear, got {groups:#?}"
    );
    let dates: Vec<&str> = groups.iter().map(|g| g.date.as_str()).collect();
    assert!(dates.contains(&"2026-05-04"), "dates: {dates:?}");
    assert!(dates.contains(&"2026-05-08"), "dates: {dates:?}");
    assert!(!dates.contains(&"2026-05-02"), "dates: {dates:?}");
}

/// Integration: stopping the timer must stamp today's log date, so a task
/// created last week (with accumulated time from earlier sessions) shows up
/// on today's timesheet the moment it's tracked again. The seeded dur also
/// keeps the test deterministic — a same-second start/stop would otherwise
/// log dur:0, which the timesheet (correctly) excludes.
#[test]
fn timer_stop_makes_entry_appear_in_todays_timesheet() {
    let raw = "2026-05-01 Long task +work @dev dur:3600\n";
    let mut app = timesheet_app_with_tasks(raw);
    // Not logged today yet: created last week, so today's view is empty.
    assert!(app.build_timesheet_groups().is_empty());

    app.toggle_timer_at(0); // start (resume)
    app.toggle_timer_at(0); // stop

    let raw_after = app.task_raw(0).unwrap_or_default();
    assert!(
        raw_after.contains("log:2026-05-07"),
        "stop must stamp today's log date, got: {raw_after}"
    );
    let groups = app.build_timesheet_groups();
    assert_eq!(groups.len(), 1, "stopped timer must create an entry");
    assert_eq!(
        groups[0].date, "2026-05-07",
        "entry must be attributed to the log date, not the creation date"
    );
    assert!(
        groups[0].total_secs >= 3600,
        "accumulated dur must be preserved, got {}",
        groups[0].total_secs
    );
}

// ── timesheet date navigation ─────────────────────────────────────

#[test]
fn timesheet_date_navigation_shifts_day() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    let orig = app.timesheet.date.clone();

    handle_timesheet_keys(&mut app, key('l')); // next day
    assert_ne!(app.timesheet.date, orig);
    assert_eq!(app.timesheet.cursor, 0, "nav resets cursor");
    assert!(
        app.flash_active()
            .is_some_and(|m| m.contains(&app.timesheet.date))
    );

    handle_timesheet_keys(&mut app, key('h')); // prev day
    assert_eq!(app.timesheet.date, orig);
}

#[test]
fn timesheet_date_navigation_shifts_week() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    let orig = app.timesheet.date.clone();

    handle_timesheet_keys(&mut app, key('L')); // next week
    assert_ne!(app.timesheet.date, orig);
    assert_eq!(app.timesheet.cursor, 0);

    handle_timesheet_keys(&mut app, key('H')); // prev week
    assert_eq!(app.timesheet.date, orig);
}

#[test]
fn timesheet_date_t_jumps_to_today() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    let today = app.today().to_string();

    // Navigate away.
    app.timesheet_shift_days(-5);
    assert_ne!(app.timesheet.date, today);

    handle_timesheet_keys(&mut app, key('t'));
    assert_eq!(app.timesheet.date, today);
    assert_eq!(app.timesheet.cursor, 0);
    assert!(app.flash_active().is_some_and(|m| m.starts_with("today (")));
}

#[test]
fn timesheet_left_right_arrows_also_navigate() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    let orig = app.timesheet.date.clone();

    handle_timesheet_keys(&mut app, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_ne!(app.timesheet.date, orig);

    handle_timesheet_keys(&mut app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(app.timesheet.date, orig);
}

// ── g-key calendar picker ─────────────────────────────────────────

#[test]
fn timesheet_g_key_opens_calendar_picker() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    // Pre-fill input to verify g clears it.
    app.timesheet.date_input = "2026-01".into();
    handle_timesheet_keys(&mut app, key('g'));
    assert_eq!(app.nav.mode, Mode::PickTimesheetDate);
    // Calendar focus is seeded from the current timesheet.date.
    assert_eq!(
        app.timesheet.calendar_focus.format("%Y-%m-%d").to_string(),
        app.timesheet.date
    );
    // Input buffer is cleared on open.
    assert!(app.timesheet.date_input.is_empty());
}

#[test]
fn pick_timesheet_date_enter_accepts() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    // Open calendar and navigate to select a date, then Enter.
    handle_timesheet_keys(&mut app, key('g'));
    assert_eq!(app.nav.mode, Mode::PickTimesheetDate);
    let orig = app.timesheet.date.clone();

    // Navigate to a different date via the calendar.
    app.timesheet.calendar_focus =
        chrono::NaiveDate::from_ymd_opt(2026, 1, 15).expect("valid date");
    handle_pick_timesheet_date(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.timesheet.date, "2026-01-15");
    assert_ne!(app.timesheet.date, orig);
    assert_eq!(app.timesheet.cursor, 0);
    assert_eq!(app.nav.mode, Mode::Normal);
    assert!(
        app.flash_active()
            .is_some_and(|m| m.contains("jumped to Thu 2026-01-15"))
    );
}

#[test]
fn pick_timesheet_date_esc_cancels() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    let orig = app.timesheet.date.clone();

    handle_timesheet_keys(&mut app, key('g'));
    // Navigate away, then cancel.
    app.timesheet.calendar_focus =
        chrono::NaiveDate::from_ymd_opt(2026, 1, 15).expect("valid date");
    handle_pick_timesheet_date(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.timesheet.date, orig, "Esc must not change date");
    assert_eq!(app.nav.mode, Mode::Normal);
}

#[test]
fn pick_timesheet_date_typing_syncs_focus_and_accepts() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    handle_timesheet_keys(&mut app, key('g'));
    assert_eq!(app.nav.mode, Mode::PickTimesheetDate);
    let orig = app.timesheet.date.clone();

    // Type a partial date — calendar focus shouldn't move yet.
    for c in "2026-".chars() {
        handle_pick_timesheet_date(
            &mut app,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        );
    }
    assert_eq!(app.timesheet.date_input, "2026-");

    // Complete the date — focus should snap.
    for c in "01-15".chars() {
        handle_pick_timesheet_date(
            &mut app,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        );
    }
    assert_eq!(app.timesheet.date_input, "2026-01-15");
    assert_eq!(
        app.timesheet.calendar_focus.format("%Y-%m-%d").to_string(),
        "2026-01-15"
    );

    // Enter accepts the typed date.
    handle_pick_timesheet_date(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.timesheet.date, "2026-01-15");
    assert_ne!(app.timesheet.date, orig);
    assert_eq!(app.nav.mode, Mode::Normal);
    assert!(
        app.flash_active()
            .is_some_and(|m| m.contains("jumped to Thu 2026-01-15"))
    );
    // Input is cleared on accept.
    assert!(app.timesheet.date_input.is_empty());
}

#[test]
fn pick_timesheet_date_invalid_typing_flashes_error() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    handle_timesheet_keys(&mut app, key('g'));
    let orig_date = app.timesheet.date.clone();

    // Type an invalid date.
    for c in "not-a-date".chars() {
        handle_pick_timesheet_date(
            &mut app,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        );
    }
    handle_pick_timesheet_date(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    // Date unchanged, mode stays in calendar, error flashed.
    assert_eq!(app.timesheet.date, orig_date);
    assert_eq!(app.nav.mode, Mode::PickTimesheetDate);
    assert!(
        app.flash_active()
            .is_some_and(|m| m.contains("invalid date"))
    );
    // Input is cleared so user can retry.
    assert!(app.timesheet.date_input.is_empty());
}

#[test]
fn pick_timesheet_date_typing_navigation_in_sync() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    handle_timesheet_keys(&mut app, key('g'));

    // Type a complete date.
    for c in "2026-12-25".chars() {
        handle_pick_timesheet_date(
            &mut app,
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
        );
    }
    let typed_focus = app.timesheet.calendar_focus;

    // Now navigate the grid.
    handle_pick_timesheet_date(&mut app, key('l'));
    assert_ne!(app.timesheet.calendar_focus, typed_focus);
    // Input buffer is still intact — typing and navigation coexist.
    assert_eq!(app.timesheet.date_input, "2026-12-25");
}

#[test]
fn pick_timesheet_date_navigation_works() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    handle_timesheet_keys(&mut app, key('g'));
    let orig = app.timesheet.calendar_focus;

    handle_pick_timesheet_date(&mut app, key('l')); // right
    assert_ne!(app.timesheet.calendar_focus, orig);
    handle_pick_timesheet_date(&mut app, key('h')); // left
    assert_eq!(app.timesheet.calendar_focus, orig);
}

// ── timesheet view/date integration ───────────────────────────────

#[test]
fn open_timesheet_resets_date_to_today() {
    let mut app = build_timesheet_app();
    let today = app.today().to_string();
    // Navigate away first.
    app.timesheet_shift_days(-10);
    assert_ne!(app.timesheet.date, today);
    // Open Timesheet via action.
    apply_action(&mut app, Action::OpenTimesheet);
    assert_eq!(app.view(), View::Timesheet);
    assert_eq!(app.timesheet.date, today, "entry resets to today");
    assert_eq!(app.timesheet.cursor, 0);
}

// ── helpers ───────────────────────────────────────────────────────

/// Build an app with dur tasks for timesheet testing.
/// Produces three groups on 2026-05-07 (two distinct project+activity
/// combos) and one on 2026-05-06, giving sort tests real data.
fn build_timesheet_app() -> App {
    let dir = std::env::temp_dir().join(format!(
        "tuxtime-ts-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    let todo_path = dir.join("todo.txt");
    let raw = "2026-05-07 Write code +work @dev dur:3600\n2026-05-07 Review PR +legal @research dur:1800\n2026-05-07 Meeting notes +work @dev dur:600\n2026-05-06 Draft memo +legal @research dur:900\n";
    std::fs::write(&todo_path, raw).expect("write todo.txt");
    App::new(
        todo_path,
        raw.into(),
        "2026-05-07".into(),
        Config::default(),
    )
}

#[test]
fn gg_chord_only_fires_on_second_press() {
    let mut app = build_app();
    // First 'g' arms the chord but produces no action.
    assert_eq!(resolve(&mut app, key('g')), None);
    // Second 'g' fires.
    assert_eq!(resolve(&mut app, key('g')), Some(Action::CursorTop));
}

#[test]
fn fp_chord_routes_to_pick_project() {
    let mut app = build_app();
    // 'f' arms the leader.
    assert_eq!(resolve(&mut app, key('f')), Some(Action::ArmF));
    apply_action(&mut app, Action::ArmF);
    // 'p' after armed 'f' picks project, not cycles priority.
    assert_eq!(resolve(&mut app, key('p')), Some(Action::PickProject));
}

#[test]
fn p_without_chord_cycles_priority() {
    let mut app = build_app();
    assert_eq!(resolve(&mut app, key('p')), Some(Action::CyclePriority),);
}

#[test]
fn unknown_key_returns_none() {
    let mut app = build_app();
    let k = KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE);
    assert_eq!(resolve(&mut app, k), None);
}

#[test]
fn yy_chord_only_fires_on_second_press() {
    let mut app = build_app();
    // First 'y' arms the chord but produces no action.
    assert_eq!(resolve(&mut app, key('y')), None);
    // Second 'y' fires the line copy.
    assert_eq!(resolve(&mut app, key('y')), Some(Action::CopyLine));
}

#[test]
fn yb_chord_routes_to_copy_body() {
    let mut app = build_app();
    // 'y' arms the leader without firing.
    assert_eq!(resolve(&mut app, key('y')), None);
    // 'b' after armed 'y' copies the body.
    assert_eq!(resolve(&mut app, key('b')), Some(Action::CopyBody));
}

#[test]
fn plain_b_toggles_billable() {
    let mut app = build_app();
    // Plain 'b' now toggles billable/non-billable.
    assert_eq!(resolve(&mut app, key('b')), Some(Action::ToggleBillable));
}

#[test]
fn cursor_actions_clamp_to_visible_range() {
    let mut app = build_app();
    // 3 visible tasks, cursor starts at 0.
    apply_action(&mut app, Action::CursorBottom);
    assert_eq!(app.nav.cursor, 2);
    apply_action(&mut app, Action::CursorDown);
    assert_eq!(app.nav.cursor, 2);
    apply_action(&mut app, Action::CursorTop);
    assert_eq!(app.nav.cursor, 0);
    apply_action(&mut app, Action::CursorUp);
    assert_eq!(app.nav.cursor, 0);
}

/// Build an isolated App rooted in a fresh temp dir, optionally seeding
/// done.txt and waiting for the startup loader to land.
fn build_app_with_archive(todo_raw: &str, done_raw: Option<&str>) -> App {
    use std::time::{Duration, Instant};
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tuxtime-bindings-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    let todo_path = dir.join("todo.txt");
    std::fs::write(&todo_path, todo_raw).expect("write todo.txt");
    if let Some(body) = done_raw {
        std::fs::write(dir.join("done.txt"), body).expect("write done.txt");
    }
    let mut app = App::new(
        todo_path,
        todo_raw.into(),
        "2026-05-06".into(),
        Config::default(),
    );
    if done_raw.is_some() {
        // Drain the startup archive loader so app.archive is populated.
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let _ = app.poll_archive();
            if !app.archive().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(!app.archive().is_empty(), "archive failed to load in time");
    }
    app
}

#[test]
fn cursor_navigation_works_in_archive() {
    let mut app = build_app_with_archive(
        "a due:2026-05-04\nb due:2026-05-06\nc due:2026-05-08\n",
        Some("x 2026-05-01 2026-04-01 first\nx 2026-05-02 2026-04-02 second\n"),
    );
    app.set_view(View::Archive);
    assert_eq!(app.nav.cursor, 0);
    apply_action(&mut app, Action::CursorDown);
    assert_eq!(app.nav.cursor, 1, "Archive view must allow CursorDown");
    apply_action(&mut app, Action::CursorTop);
    assert_eq!(app.nav.cursor, 0);
}

#[test]
fn archive_x_unarchives_task_under_cursor() {
    let mut app = build_app_with_archive("a\n", Some("x 2026-05-02 2026-04-02 done one\n"));
    app.set_view(View::Archive);
    apply_action(&mut app, Action::ToggleComplete);
    assert_eq!(app.archive().len(), 0, "task must leave the archive");
    assert!(
        app.tasks()
            .iter()
            .any(|t| t.raw.contains("done one") && !t.done),
        "un-completed entry must rejoin live tasks"
    );
}

#[test]
fn archive_dd_permanently_deletes_task_under_cursor() {
    let mut app = build_app_with_archive("a\n", Some("x 2026-05-02 2026-04-02 done one\n"));
    app.set_view(View::Archive);
    apply_action(&mut app, Action::Delete);
    assert_eq!(app.archive().len(), 0);
    assert_eq!(app.tasks().len(), 1, "todo.txt must be untouched");
}

#[test]
fn archive_e_and_p_flash_readonly() {
    let mut app = build_app_with_archive("a\n", Some("x 2026-05-02 2026-04-02 done one\n"));
    app.set_view(View::Archive);
    apply_action(&mut app, Action::BeginEdit);
    assert_eq!(app.flash_active(), Some("read-only in archive"));
    apply_action(&mut app, Action::CyclePriority);
    assert_eq!(app.flash_active(), Some("read-only in archive"));
    assert!(app.archive().tasks()[0].done);
}

#[test]
fn lowercase_r_reschedules_task_with_due_date() {
    let mut app = build_app_with_due();
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.tasks()[0].due.as_deref(), Some("2026-06-30"));
    assert_eq!(app.nav.mode, Mode::Normal);

    assert_eq!(resolve(&mut app, key('r')), Some(Action::Reschedule),);
    apply_action(&mut app, Action::Reschedule);
    assert_eq!(app.nav.mode, Mode::Insert);

    let s = app.calendar_state().expect("calendar should be open");
    assert_eq!(
        s.focused,
        NaiveDate::from_ymd_opt(2026, 6, 30).expect("there should be a date set")
    );

    app.calendar_add_months(1);
    app.calendar_accept();
    assert!(app.draft.overlay().is_none());
    app.add_from_draft();
    let task = app.tasks().last().expect("task added");
    assert_eq!(task.due.as_deref(), Some("2026-07-30"));
}

#[test]
fn lowercase_r_reschedules_task_without_due_date() {
    let mut app = build_app();
    assert_eq!(app.tasks().len(), 3);
    assert_eq!(app.tasks()[0].due.as_deref(), None);
    assert_eq!(app.nav.mode, Mode::Normal);

    assert_eq!(resolve(&mut app, key('r')), Some(Action::Reschedule),);
    apply_action(&mut app, Action::Reschedule);
    assert_eq!(app.nav.mode, Mode::Insert);

    let s = app.calendar_state().expect("calendar should be open");
    assert_eq!(
        s.focused,
        NaiveDate::from_ymd_opt(2026, 5, 7).expect("there should be a date set")
    );

    app.calendar_add_months(1);
    app.calendar_accept();
    app.add_from_draft();
    assert!(app.draft.overlay().is_none());
    let task = app.tasks().last().expect("task added");
    assert_eq!(task.due.as_deref(), Some("2026-06-07"));
}

#[test]
fn capital_w_toggles_week_start() {
    let mut app = build_app();
    assert_eq!(resolve(&mut app, key('W')), Some(Action::ChangeWeekStart));
}

#[test]
fn ctrl_emacs_keys_resolve_to_edit_actions() {
    assert_eq!(resolve_edit_key(ctrl('a')), Some(EditAction::MoveHome));
    assert_eq!(resolve_edit_key(ctrl('e')), Some(EditAction::MoveEnd));
    assert_eq!(resolve_edit_key(ctrl('b')), Some(EditAction::MoveLeft));
    assert_eq!(resolve_edit_key(ctrl('f')), Some(EditAction::MoveRight));
    assert_eq!(
        resolve_edit_key(ctrl('h')),
        Some(EditAction::DeleteBackward)
    );
    assert_eq!(resolve_edit_key(ctrl('d')), Some(EditAction::DeleteForward));
    assert_eq!(
        resolve_edit_key(ctrl('w')),
        Some(EditAction::DeleteWordBackward)
    );
    assert_eq!(resolve_edit_key(ctrl('u')), Some(EditAction::KillToStart));
    assert_eq!(resolve_edit_key(ctrl('k')), Some(EditAction::KillToEnd));
}

#[test]
fn alt_word_keys_resolve_to_word_actions() {
    assert_eq!(
        resolve_edit_key(alt('b')),
        Some(EditAction::MoveWordBackward)
    );
    assert_eq!(
        resolve_edit_key(alt('f')),
        Some(EditAction::MoveWordForward)
    );
    assert_eq!(
        resolve_edit_key(alt('d')),
        Some(EditAction::DeleteWordForward)
    );
}

#[test]
fn unmapped_ctrl_chord_is_swallowed_not_typed() {
    // The historical bug: Ctrl+H (and friends) inserted a literal letter.
    // Unmapped control chords must resolve to nothing, never an Insert.
    assert_eq!(resolve_edit_key(ctrl('g')), None);
    assert_eq!(resolve_edit_key(ctrl('z')), None);
}

#[test]
fn plain_and_shifted_chars_insert() {
    assert_eq!(resolve_edit_key(key('x')), Some(EditAction::Insert('x')));
    let shifted = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
    assert_eq!(resolve_edit_key(shifted), Some(EditAction::Insert('A')));
}

#[test]
fn altgr_char_inserts_not_swallowed() {
    // AltGr is reported as CONTROL|ALT by crossterm for printable chars on
    // international layouts. It must insert text, not fire a Ctrl chord —
    // both a letter that collides with the ctrl table ('e') and one that
    // doesn't ('€') must reach Insert.
    let altgr = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL | KeyModifiers::ALT);
    assert_eq!(resolve_edit_key(altgr('e')), Some(EditAction::Insert('e')));
    assert_eq!(resolve_edit_key(altgr('€')), Some(EditAction::Insert('€')));
}

#[test]
fn ctrl_h_deletes_instead_of_inserting_in_insert_mode() {
    // End-to-end through handle_insert: Ctrl+H must delete the char before
    // the cursor rather than typing 'h'.
    let mut app = build_app();
    app.nav.mode = Mode::Insert;
    app.draft_clear();
    app.draft_insert_char('a');
    app.draft_insert_char('b');
    handle_insert(&mut app, ctrl('h'));
    assert_eq!(app.draft.text(), "a");
}

#[test]
fn ctrl_u_clears_to_start_in_search_mode() {
    // Ctrl+U in the search box wipes back to the start and re-runs the
    // filter via the TextChanged effect.
    let mut app = build_app();
    app.nav.mode = Mode::Search;
    app.draft_clear();
    for c in "abc".chars() {
        app.draft_insert_char(c);
    }
    app.set_search("abc".into());
    handle_search(&mut app, ctrl('u'));
    assert_eq!(app.draft.text(), "");
}

// ---- timer tests ----

#[test]
fn timer_toggle_starts_on_current_task() {
    let mut app = build_app();
    assert!(!app.timer_running());
    apply_action(&mut app, Action::TimerStartStop);
    assert!(app.timer_running());
    // The first task should be the one with the running timer.
    assert!(app.is_timer_running_on(0));
}

#[test]
fn timer_toggle_stops_running_timer() {
    let mut app = build_app();
    apply_action(&mut app, Action::TimerStartStop);
    assert!(app.timer_running());
    // Second press stops it.
    apply_action(&mut app, Action::TimerStartStop);
    assert!(!app.timer_running());
}

#[test]
fn timer_toggle_switches_to_new_task() {
    let mut app = build_app();
    // Start on first task.
    apply_action(&mut app, Action::TimerStartStop);
    assert!(app.is_timer_running_on(0));
    // Move cursor to second task and toggle.
    app.nav.cursor = 1;
    apply_action(&mut app, Action::TimerStartStop);
    assert!(!app.is_timer_running_on(0));
    assert!(app.is_timer_running_on(1));
}

#[test]
fn timer_toggle_flashes_no_task_when_empty() {
    let mut app = App::new(
        std::env::temp_dir().join(format!(
            "tuxtime-empty-{}-{:?}.txt",
            std::process::id(),
            std::thread::current().id()
        )),
        String::new(),
        "2026-05-07".into(),
        Config::default(),
    );
    apply_action(&mut app, Action::TimerStartStop);
    assert!(!app.timer_running());
}

// ---- billable toggle ----

#[test]
fn toggle_billable_adds_bill_n_tag() {
    let mut app = build_app();
    let raw = app.task_raw(0).unwrap();
    assert!(!raw.contains("bill:n"));
    app.toggle_billable_at(0);
    let updated = app.task_raw(0).unwrap();
    assert!(updated.contains("bill:n"), "expected bill:n tag: {updated}");
}

#[test]
fn toggle_billable_removes_bill_n_tag() {
    let raw = "(A) Write code +work @dev bill:n\n";
    let path = std::env::temp_dir().join(format!(
        "tuxtime-bill-{}-{:?}.txt",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::write(&path, raw);
    let mut app = App::new(path, raw.into(), "2026-05-07".into(), Config::default());
    app.toggle_billable_at(0);
    let updated = app.task_raw(0).unwrap();
    assert!(
        !updated.contains("bill:n"),
        "bill:n should be removed: {updated}"
    );
}

// ---- nudge threshold config ----

#[test]
fn idle_nudge_prompt_prefills_default_minutes() {
    let mut app = build_app();
    app.nav.mode = Mode::Settings;
    // Simulate pressing 'i' in settings.
    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE),
    );
    assert_eq!(app.nav.mode, Mode::PromptIdleNudge);
    // Default idle nudge is 900s = 15 min.
    assert_eq!(app.draft.text(), "15");
}

#[test]
fn long_timer_nudge_prompt_prefills_default_minutes() {
    let mut app = build_app();
    app.nav.mode = Mode::Settings;
    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE),
    );
    assert_eq!(app.nav.mode, Mode::PromptLongTimerNudge);
    // Default long-timer nudge is 7200s = 120 min.
    assert_eq!(app.draft.text(), "120");
}

#[test]
fn idle_nudge_enter_sets_new_threshold() {
    let mut app = build_app();
    // Settings → push the nudge prompt → Enter must pop back to Settings.
    app.nav.mode = Mode::Settings;
    app.nav.push_mode(Mode::PromptIdleNudge);
    app.draft_set_insert("30".to_string());
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.nav.mode, Mode::Settings);
    assert_eq!(app.idle_nudge_seconds(), 1800); // 30 min
}

#[test]
fn idle_nudge_enter_rejects_zero_minutes() {
    let mut app = build_app();
    app.nav.mode = Mode::Settings;
    app.nav.push_mode(Mode::PromptIdleNudge);
    app.draft_set_insert("0".to_string());
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.nav.mode, Mode::Settings);
    // Value should not have changed.
    assert_eq!(app.idle_nudge_seconds(), 900);
}

#[test]
fn long_timer_nudge_enter_sets_new_threshold() {
    let mut app = build_app();
    app.nav.mode = Mode::Settings;
    app.nav.push_mode(Mode::PromptLongTimerNudge);
    app.draft_set_insert("60".to_string());
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.nav.mode, Mode::Settings);
    assert_eq!(app.long_timer_nudge_seconds(), 3600); // 60 min
}

// ---- rounding increment config ----

/// Settings 'r' cycles the billable rounding increment and persists it:
/// 0.1h (default) → 0.25h → exact → back to 0.1h.
#[test]
fn rounding_increment_r_cycles_and_persists() {
    let mut app = build_app();
    app.nav.mode = Mode::Settings;
    assert_eq!(
        crate::app::rounding_increment_label(app.prefs.rounding_increment),
        "0.1h"
    );

    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    );
    assert_eq!(
        crate::app::rounding_increment_label(app.prefs.rounding_increment),
        "0.25h",
        "first cycle → quarters"
    );
    assert_eq!(app.nav.mode, Mode::Settings, "stays in settings");

    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    );
    assert_eq!(
        crate::app::rounding_increment_label(app.prefs.rounding_increment),
        "exact",
        "second cycle → no rounding"
    );

    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    );
    assert_eq!(
        crate::app::rounding_increment_label(app.prefs.rounding_increment),
        "0.1h",
        "third cycle → back to tenths"
    );
}

/// The settings screen advertises its rows' keys as hints; pressing them
/// *inside* settings must actually apply them (not be dead text).
#[test]
fn settings_advertised_keys_apply_in_settings_mode() {
    let mut app = build_app();
    app.nav.mode = Mode::Settings;

    // Density cycles: comfortable → cozy.
    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('D'), KeyModifiers::NONE),
    );
    assert_eq!(app.prefs.density, crate::app::Density::Cozy);

    // Sort cycles to a different label.
    let before = app.sort_label();
    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('S'), KeyModifiers::NONE),
    );
    assert_ne!(app.sort_label(), before);

    // Line numbers toggle.
    let line_nums = app.prefs.layout.line_num;
    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('L'), KeyModifiers::NONE),
    );
    assert_ne!(app.prefs.layout.line_num, line_nums);

    // Show-done toggles.
    let show_done = app.prefs.show_done;
    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('H'), KeyModifiers::NONE),
    );
    assert_ne!(app.prefs.show_done, show_done);

    // Show-future toggles.
    let show_future = app.prefs.show_future;
    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('F'), KeyModifiers::NONE),
    );
    assert_ne!(app.prefs.show_future, show_future);

    // Filter/detail panes toggle.
    let left = app.prefs.layout.left;
    let right = app.prefs.layout.right;
    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE),
    );
    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
    );
    assert_ne!(app.prefs.layout.left, left);
    assert_ne!(app.prefs.layout.right, right);

    // Theme picker opens (and stays previewable).
    handle_settings(
        &mut app,
        KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::NONE),
    );
    assert_eq!(app.nav.mode, Mode::PickTheme);
}

// ---- stale-timer startup prompt ----

/// Helper: build an App with a single task whose timer started `secs` ago
/// (seeding the file on disk so the store's reconcile sees a stable state).
fn stale_timer_app(secs: i64, name: &str) -> App {
    let start = (chrono::Local::now() - chrono::Duration::seconds(secs))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    let path = std::env::temp_dir().join(format!(
        "tuxtime-stale-{name}-{}-{:?}.txt",
        std::process::id(),
        std::thread::current().id()
    ));
    let raw = format!("Draft +Smith start:{start}\n");
    let _ = std::fs::write(&path, &raw);
    let mut app = App::new(path, raw, "2026-05-07".into(), Config::default());
    app.nav.mode = Mode::StaleTimer;
    app
}

/// `S` on the stale-timer prompt stops the timer and logs the elapsed time
/// (the user asserts the whole session was real work).
#[test]
fn stale_timer_s_stops_and_logs_elapsed() {
    let mut app = stale_timer_app(7300, "s");

    handle_stale_timer(&mut app, key('S'));

    assert!(!app.timer_running(), "S must stop the timer");
    assert_eq!(app.nav.mode, Mode::Normal);
    assert!(
        app.tasks()[0].dur.unwrap_or(0) > 7200,
        "S must log the elapsed time"
    );
}

/// `d` on the stale-timer prompt discards the unrecorded gap: the timer
/// stops but no elapsed time is credited.
#[test]
fn stale_timer_d_discards_gap() {
    let mut app = stale_timer_app(7300, "d");

    handle_stale_timer(&mut app, key('d'));

    assert!(!app.timer_running(), "d must stop the timer");
    assert_eq!(app.nav.mode, Mode::Normal);
    let raw = app.task_raw(0).unwrap_or_default();
    assert!(!raw.contains("start:"), "start: must be stripped: {raw}");
    assert_eq!(app.tasks()[0].dur, None, "no time may be credited");
}

/// `k` (and Esc) on the stale-timer prompt keep the timer running.
#[test]
fn stale_timer_k_keeps_counting() {
    let mut app = stale_timer_app(7300, "k");

    handle_stale_timer(&mut app, key('k'));

    assert!(app.timer_running(), "k must keep the timer running");
    assert_eq!(app.nav.mode, Mode::Normal);
}

// ---- idle nudge recovery actions (S / M task picker) ----

/// `S` from the idle nudge opens the task picker in start-timer mode; the
/// picker is a deliberate choice, never a blind hit on the cursor's task.
#[test]
fn idle_nudge_s_opens_start_timer_picker() {
    let mut app = build_app();
    app.nav.mode = Mode::IdleNudge;

    handle_idle_nudge(&mut app, key('S'));

    assert_eq!(app.nav.mode, Mode::PickNudgeTask);
    assert_eq!(
        app.session.nudge_picker.as_ref().map(|p| p.action),
        Some(crate::app::NudgePickAction::StartTimer)
    );
}

/// `M` from the idle nudge opens the task picker in add-time mode.
#[test]
fn idle_nudge_m_opens_add_time_picker() {
    let mut app = build_app();
    app.nav.mode = Mode::IdleNudge;

    handle_idle_nudge(&mut app, key('M'));

    assert_eq!(app.nav.mode, Mode::PickNudgeTask);
    assert_eq!(
        app.session.nudge_picker.as_ref().map(|p| p.action),
        Some(crate::app::NudgePickAction::AddTime)
    );
}

/// End-to-end through the key handlers: `s` → pick the last of 3 tasks →
/// Enter starts the timer on it.
#[test]
fn pick_nudge_task_enter_starts_timer_on_chosen() {
    let mut app = build_app(); // tasks a, b, c
    app.nav.mode = Mode::IdleNudge;

    handle_idle_nudge(&mut app, key('s'));
    assert_eq!(app.nav.mode, Mode::PickNudgeTask);
    app.nudge_picker_step(true);
    app.nudge_picker_step(true); // highlight task c
    handle_pick_nudge_task(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert!(app.timer_running());
    assert!(app.is_timer_running_on(2), "timer must run on task c");
    assert_eq!(app.nav.mode, Mode::Normal);
}

/// Esc in the picker returns to the idle nudge popup.
#[test]
fn pick_nudge_task_esc_returns_to_idle_nudge() {
    let mut app = build_app();
    app.nav.mode = Mode::IdleNudge;
    handle_idle_nudge(&mut app, key('s'));
    assert_eq!(app.nav.mode, Mode::PickNudgeTask);

    handle_pick_nudge_task(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(app.nav.mode, Mode::IdleNudge);
}

// ---- sidebar toggles in every view ----

/// `[`/`]` are view-independent chrome: they must toggle the sidebars in
/// the Timesheet view too, not just the list. (Regression: the timesheet
/// key handler previously swallowed every key except Esc/V/Z//.)
#[test]
fn timesheet_brackets_toggle_sidebars() {
    let mut app = build_timesheet_app();
    app.set_view(View::Timesheet);
    let left = app.prefs.layout.left;
    let right = app.prefs.layout.right;

    dispatch(&mut app, key('['), &KeyBindings::default());
    dispatch(&mut app, key(']'), &KeyBindings::default());

    assert_ne!(
        app.prefs.layout.left, left,
        "[ must toggle the left sidebar"
    );
    assert_ne!(
        app.prefs.layout.right, right,
        "] must toggle the right sidebar"
    );
    assert_eq!(
        app.view(),
        View::Timesheet,
        "sidebar toggles must not leave the timesheet"
    );
}

/// `[`/`]` must also work inside the Project Manager view.
#[test]
fn manage_projects_brackets_toggle_sidebars() {
    let mut app = build_app();
    app.nav.mode = Mode::ManageProjects;
    let left = app.prefs.layout.left;
    let right = app.prefs.layout.right;

    handle_manage_projects(&mut app, key('['));
    handle_manage_projects(&mut app, key(']'));

    assert_ne!(app.prefs.layout.left, left);
    assert_ne!(app.prefs.layout.right, right);
    assert_eq!(
        app.nav.mode,
        Mode::ManageProjects,
        "sidebar toggles must not leave the project manager"
    );
}

/// `N` from the idle nudge opens a blank insert; Esc-cancelling it must
/// return to the nudge popup — the reminder survives an aborted recovery.
#[test]
fn idle_nudge_n_esc_returns_to_nudge() {
    let mut app = build_app();
    app.nav.mode = Mode::IdleNudge;

    handle_idle_nudge(&mut app, key('N'));
    assert_eq!(app.nav.mode, Mode::Insert);
    assert!(
        app.session.from_nudge,
        "insert must be marked as nudge-born"
    );

    // First Esc in insert sub-mode switches to normal input mode; the
    // second exits — back to the nudge, not Normal.
    handle_insert(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    handle_insert(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(
        app.nav.mode,
        Mode::IdleNudge,
        "Esc from the nudge's N insert must return to the nudge"
    );
    assert!(
        !app.session.from_nudge,
        "the nudge-born marker must be consumed"
    );
}

/// Saving the entry opened via the nudge's `N` exits to Normal (a real
/// capture happened — the reminder's job is done).
#[test]
fn idle_nudge_n_save_exits_to_normal() {
    let mut app = build_app();
    app.nav.mode = Mode::IdleNudge;

    handle_idle_nudge(&mut app, key('N'));
    assert_eq!(app.nav.mode, Mode::Insert);
    app.draft_set_insert("Buy milk".into());

    handle_insert(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(app.nav.mode, Mode::Normal, "save must exit to Normal");
    assert_eq!(app.tasks().len(), 4, "the new task must be saved");
    assert!(!app.session.from_nudge, "marker must be cleared on save");
}

/// An insert NOT born from the nudge still Esc's back to Normal (no
/// behavior change outside the nudge flow).
#[test]
fn plain_insert_esc_exits_to_normal() {
    let mut app = build_app();
    app.nav.mode = Mode::Insert;
    app.draft_clear();

    handle_insert(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    handle_insert(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(app.nav.mode, Mode::Normal);
}

/// `M` → picker → Enter (add-time prompt): Esc from that prompt must return
/// to the nudge, mirroring the N insert's cancel behavior.
#[test]
fn idle_nudge_m_add_time_esc_returns_to_nudge() {
    let mut app = build_app();
    app.nav.mode = Mode::IdleNudge;

    handle_idle_nudge(&mut app, key('M'));
    assert_eq!(app.nav.mode, Mode::PickNudgeTask);
    // Commit the picker → the add-time prompt.
    handle_pick_nudge_task(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.nav.mode, Mode::PromptAddTime);
    assert!(app.session.from_nudge);

    handle_prompt(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(
        app.nav.mode,
        Mode::IdleNudge,
        "Esc from the nudge's add-time prompt must return to the nudge"
    );
    assert!(!app.session.from_nudge);
}

/// An INVALID duration submitted at the nudge's add-time prompt must not
/// drop the reminder either: nothing was recorded, so the failed attempt
/// returns to the popup instead of Normal.
#[test]
fn idle_nudge_m_add_time_invalid_duration_returns_to_nudge() {
    let mut app = build_app();
    app.nav.mode = Mode::IdleNudge;

    handle_idle_nudge(&mut app, key('M'));
    handle_pick_nudge_task(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.nav.mode, Mode::PromptAddTime);
    assert!(app.session.from_nudge);

    app.draft_set("not-a-duration".into());
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(
        app.nav.mode,
        Mode::IdleNudge,
        "a failed add must keep the reminder alive"
    );
    assert!(
        !app.session.from_nudge,
        "flag consumed by the failed attempt"
    );
    assert!(
        app.flash_active()
            .is_some_and(|m| m.contains("invalid duration")),
        "the failure reason must be visible"
    );
    assert_eq!(app.tasks().len(), 3, "nothing may be saved");
    assert!(app.tasks()[0].raw.contains('a'), "task untouched");
}

/// A VALID duration submitted at the nudge's add-time prompt completes the
/// recovery: time lands and the flow exits to Normal.
#[test]
fn idle_nudge_m_add_time_valid_exits_to_normal() {
    let mut app = build_app();
    app.nav.mode = Mode::IdleNudge;

    handle_idle_nudge(&mut app, key('M'));
    handle_pick_nudge_task(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    app.draft_set("30".into());
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(
        app.nav.mode,
        Mode::Normal,
        "a real add exits the nudge flow"
    );
    assert!(!app.session.from_nudge);
    assert!(
        app.tasks()[0].raw.contains("dur:1800"),
        "30m must be recorded: {}",
        app.tasks()[0].raw
    );
}

/// When the nudge's add-time prompt defers to the day-boundary prompt and
/// the resolution then FAILS (the entered duration is invalid), the reminder
/// must still survive — the flag must not be dropped mid-flow.
#[test]
fn idle_nudge_m_add_time_day_boundary_invalid_returns_to_nudge() {
    let mut app = build_app_with_archive("Draft +Smith dur:7200 log:2026-05-05\n", None);
    app.nav.mode = Mode::IdleNudge;

    handle_idle_nudge(&mut app, key('M'));
    handle_pick_nudge_task(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    // Validation is deferred until the day-boundary prompt resolves.
    app.draft_set("oops".into());
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.nav.mode, Mode::PromptDayBoundary);

    handle_day_boundary(&mut app, key('c'));

    assert_eq!(
        app.nav.mode,
        Mode::IdleNudge,
        "a failed day-boundary resolution must keep the reminder"
    );
    assert!(!app.session.from_nudge);
    assert_eq!(app.tasks().len(), 1);
    assert!(!app.tasks()[0].done);
    assert!(
        app.tasks()[0].raw.contains("dur:7200"),
        "no time may be added: {}",
        app.tasks()[0].raw
    );
}

/// The day-boundary "new entry" resolution ('n') of a nudge-born add can
/// fail the same way: an invalid duration must keep the reminder AND must
/// not consume the carried task (no stale "new entry" line is left behind).
#[test]
fn idle_nudge_m_add_time_day_boundary_new_entry_invalid_returns_to_nudge() {
    let mut app = build_app_with_archive("Draft +Smith dur:7200 log:2026-05-05\n", None);
    app.nav.mode = Mode::IdleNudge;

    handle_idle_nudge(&mut app, key('M'));
    handle_pick_nudge_task(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    app.draft_set("oops".into());
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.nav.mode, Mode::PromptDayBoundary);

    handle_day_boundary(&mut app, key('n'));

    assert_eq!(
        app.nav.mode,
        Mode::IdleNudge,
        "a failed 'new entry' resolution must keep the reminder"
    );
    assert!(!app.session.from_nudge);
    assert_eq!(
        app.tasks().len(),
        1,
        "an invalid duration must not consume the task via carry-forward"
    );
    assert!(!app.tasks()[0].done);
    assert!(
        app.tasks()[0].raw.contains("dur:7200"),
        "no time may be added: {}",
        app.tasks()[0].raw
    );
}

/// Esc from the day-boundary prompt reached during a nudge recovery returns
/// to the popup and never leaks the flag into Normal.
#[test]
fn idle_nudge_m_add_time_day_boundary_esc_returns_to_nudge() {
    let mut app = build_app_with_archive("Draft +Smith dur:7200 log:2026-05-05\n", None);
    app.nav.mode = Mode::IdleNudge;

    handle_idle_nudge(&mut app, key('M'));
    handle_pick_nudge_task(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    app.draft_set("30".into());
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.nav.mode, Mode::PromptDayBoundary);

    handle_day_boundary(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(app.nav.mode, Mode::IdleNudge, "Esc keeps the reminder");
    assert!(
        !app.session.from_nudge,
        "the flag must not leak into Normal"
    );
    assert_eq!(app.tasks().len(), 1);
    assert!(!app.tasks()[0].done);
}

/// Regression guard: a failed add from a NON-nudge prompt still exits to
/// Normal — the nudge redirect must only fire for nudge-born flows.
#[test]
fn plain_add_time_invalid_exits_to_normal() {
    let mut app = build_app();
    app.nav.mode = Mode::ManualEntryChoice;

    handle_manual_entry_choice(&mut app, key('A'));
    assert_eq!(app.nav.mode, Mode::PromptAddTime);
    assert!(!app.session.from_nudge);

    app.draft_set("nope".into());
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    assert_eq!(
        app.nav.mode,
        Mode::Normal,
        "non-nudge flow keeps its Normal exit"
    );
    assert!(!app.session.from_nudge);
    assert_eq!(app.tasks().len(), 3, "nothing may be saved");
}

// ---- end-of-day review nudge ----

/// `V` on the review nudge opens the timesheet anchored on today.
#[test]
fn review_nudge_v_opens_timesheet() {
    let mut app = build_app();
    app.nav.mode = Mode::ReviewNudge;

    handle_review_nudge(&mut app, key('V'));

    assert_eq!(app.view(), View::Timesheet);
    assert_eq!(app.nav.mode, Mode::Normal);
    assert_eq!(app.timesheet.date, app.today(), "anchored on today");
}

/// `s` on the review nudge skips it (and won't re-fire today — the once-per-
/// day slot is already consumed).
#[test]
fn review_nudge_s_skips() {
    let mut app = build_app();
    app.nav.mode = Mode::ReviewNudge;

    handle_review_nudge(&mut app, key('s'));

    assert_eq!(app.nav.mode, Mode::Normal);
    assert_eq!(app.view(), View::List);
}

// ---- idle nudge safety ----

/// The idle nudge must not fire while the user is in a mode with transient
/// state (draft, prompt, overlay). Firing would run `exit_overlay_to_normal`
/// and silently discard their in-progress composition.
#[test]
fn idle_nudge_skips_modes_with_unsaved_state() {
    let mut app = build_app();
    // Make the idle threshold trivially exceeded.
    app.prefs.idle_nudge_seconds = 0;
    app.session.last_timer_activity =
        std::time::Instant::now() - std::time::Duration::from_secs(60);

    // Insert mode with a half-typed draft.
    app.nav.mode = Mode::Insert;
    app.draft_set("half typed".into());
    assert!(!app.check_nudges(), "nudge must not fire in Insert mode");
    assert_eq!(app.nav.mode, Mode::Insert, "Insert mode must be preserved");
    assert_eq!(
        app.draft.text(),
        "half typed",
        "draft must survive the tick"
    );

    // Search mode with a filter typed.
    app.nav.mode = Mode::Search;
    app.draft_set("needle".into());
    assert!(!app.check_nudges(), "nudge must not fire in Search mode");
    assert_eq!(app.nav.mode, Mode::Search);
    assert_eq!(app.draft.text(), "needle");

    // Settings overlay.
    app.nav.mode = Mode::Settings;
    assert!(!app.check_nudges(), "nudge must not fire over Settings");
    assert_eq!(app.nav.mode, Mode::Settings);

    // Command palette.
    app.nav.mode = Mode::CommandPalette;
    assert!(!app.check_nudges(), "nudge must not fire over the palette");
    assert_eq!(app.nav.mode, Mode::CommandPalette);

    // A text-input prompt.
    app.nav.mode = Mode::PromptAddTime;
    app.draft_set("30".into());
    assert!(!app.check_nudges(), "nudge must not fire over a prompt");
    assert_eq!(app.nav.mode, Mode::PromptAddTime);
}

/// From Normal mode — no draft, no modal — the idle nudge fires as designed.
#[test]
fn idle_nudge_fires_in_normal_mode() {
    let mut app = build_app();
    app.prefs.idle_nudge_seconds = 0;
    app.session.last_timer_activity =
        std::time::Instant::now() - std::time::Duration::from_secs(60);
    assert!(app.check_nudges());
    assert_eq!(app.nav.mode, Mode::IdleNudge);
}

// ---- prompt mode transitions ----

#[test]
fn idle_nudge_prompt_esc_returns_to_settings() {
    let mut app = build_app();
    app.nav.mode = Mode::Settings;
    app.nav.push_mode(Mode::PromptIdleNudge);
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.nav.mode, Mode::Settings);
    assert!(app.draft.text().is_empty());
}

#[test]
fn rename_prompt_esc_returns_to_manage_projects() {
    let mut app = build_app();
    app.nav.mode = Mode::PromptRenameProject;
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.nav.mode, Mode::ManageProjects);
    assert!(app.draft.text().is_empty());
}

// ---- timesheet 'a' archives single completed task ----

/// Helper: build an App in Timesheet view with the given raw tasks
/// (using "2026-05-07" as today). Ensures the archive loader has been
/// drained before returning.
fn timesheet_app_with_tasks(raw: &str) -> App {
    let dir = std::env::temp_dir().join(format!(
        "tuxtime-ts-archive-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create test dir");
    let todo_path = dir.join("todo.txt");
    std::fs::write(&todo_path, raw).expect("write todo.txt");
    let mut app = App::new(
        todo_path,
        raw.into(),
        "2026-05-07".into(),
        Config::default(),
    );
    app.set_view(View::Timesheet);
    app
}

#[test]
fn timesheet_a_archives_completed_task() {
    // Two tasks: one completed with dur, one active without dur.
    let raw =
        "x 2026-05-07 2026-05-07 done task +work dur:3600\n2026-05-07 active task +work dur:1800\n";
    let mut app = timesheet_app_with_tasks(raw);
    assert_eq!(app.tasks().len(), 2, "both tasks present before archive");

    // Position the timesheet cursor on the completed task (first narrative).
    app.timesheet.cursor = 0;

    handle_timesheet_keys(&mut app, key('a'));

    // The completed task must have moved to the archive.
    assert_eq!(app.tasks().len(), 1, "completed task must be archived");
    assert!(
        !app.tasks().iter().any(|t| t.raw.contains("done task")),
        "done task must not remain in live tasks"
    );
    assert!(
        app.archive()
            .tasks()
            .iter()
            .any(|t| t.raw.contains("done task")),
        "done task must be in archive"
    );
    assert!(
        app.tasks().iter().any(|t| t.raw.contains("active task")),
        "active task must stay"
    );
    assert_eq!(app.flash_active(), Some("archived"));
    // Cursor must be reclamped (now 1 task → 1 narrative → cursor 0).
    assert_eq!(app.timesheet.cursor, 0);
}

#[test]
fn timesheet_a_on_non_completed_flashes_guidance() {
    let raw = "2026-05-07 active task +work dur:3600\n";
    let mut app = timesheet_app_with_tasks(raw);
    app.timesheet.cursor = 0;

    handle_timesheet_keys(&mut app, key('a'));

    // Non-completed task must not be moved.
    assert_eq!(app.tasks().len(), 1, "non-completed task must stay");
    assert_eq!(app.flash_active(), Some("complete task first (x)"));
}

/// Regression: archive_one must invalidate the timesheet groups cache
/// immediately, not just the list/archive visible cache. Without this,
/// the timesheet would show stale data until another mutation forced a
/// recomputation. The timesheet includes both active and archived tasks
/// (it's a reporting tool), so the entry persists after archiving — but
/// its source moves from Active to Archived. The test verifies the cache
/// actually recomputes rather than returning the pre-archive snapshot.
#[test]
fn timesheet_archive_invalidates_groups_cache() {
    // Two tasks: one completed, one active. Different projects.
    let raw = "x 2026-05-07 2026-05-07 done task +alpha dur:3600\n2026-05-07 active task +beta dur:1800\n";
    let mut app = timesheet_app_with_tasks(raw);
    assert_eq!(app.tasks().len(), 2);

    // Before archiving: +alpha's task ref is Active, +beta's is Active.
    let before = app.build_timesheet_groups();
    assert_eq!(before.len(), 2, "two groups before archive");

    // Archive the completed task (index 0).
    app.archive_one(0);
    assert_eq!(app.tasks().len(), 1, "active tasks must decrease");
    assert_eq!(app.archive().tasks().len(), 1, "archive must grow");

    // After archiving: the cache must reflect the new reality. +alpha
    // still appears (now from the archive), +beta is still active.
    let after = app.build_timesheet_groups();
    assert_eq!(after.len(), 2, "both projects still in timesheet");
    // +alpha's task ref should now be Archived, not Active.
    let alpha_entry = after
        .iter()
        .find(|g| g.key.contains("+alpha"))
        .expect("+alpha must be present");
    assert!(
        alpha_entry
            .task_indices
            .iter()
            .all(|r| matches!(r, crate::app::TimesheetTaskRef::Archived(_))),
        "archived task ref must be Archived, not Active"
    );
}

/// Regression: unarchive must invalidate the timesheet groups cache
/// immediately. After unarchiving, the task moves from the archive back
/// to active, so its TimesheetTaskRef must change from Archived to Active.
#[test]
fn timesheet_unarchive_invalidates_groups_cache() {
    // One completed task with dur, one active task.
    let raw = "x 2026-05-07 2026-05-07 done task +alpha dur:3600\n2026-05-07 active task +beta dur:1800\n";
    let mut app = timesheet_app_with_tasks(raw);
    assert_eq!(app.tasks().len(), 2);

    // Archive the completed +alpha task.
    app.archive_one(0);
    assert_eq!(app.tasks().len(), 1);
    assert_eq!(app.archive().tasks().len(), 1);

    // After archiving: +alpha shows as Archived ref.
    let archived_groups = app.build_timesheet_groups();
    let alpha_entry = archived_groups
        .iter()
        .find(|g| g.key.contains("+alpha"))
        .expect("+alpha must be present");
    assert!(
        alpha_entry
            .task_indices
            .iter()
            .all(|r| matches!(r, crate::app::TimesheetTaskRef::Archived(_))),
        "archived task ref must be Archived"
    );

    // Unarchive: move the task back from archive (index 0) to active.
    app.unarchive(0);
    assert_eq!(app.tasks().len(), 2, "task must rejoin active list");
    assert_eq!(app.archive().tasks().len(), 0, "archive must be empty");

    // After unarchiving: cache must reflect the move. +alpha now Active.
    let unarchived_groups = app.build_timesheet_groups();
    let alpha_unarchived = unarchived_groups
        .iter()
        .find(|g| g.key.contains("+alpha"))
        .expect("+alpha must be present after unarchive");
    assert!(
        alpha_unarchived
            .task_indices
            .iter()
            .all(|r| matches!(r, crate::app::TimesheetTaskRef::Active(_))),
        "unarchived task ref must be Active, not Archived"
    );
}

// ---- project archive / toggle ----

#[test]
fn toggle_archive_project_adds_and_removes() {
    let mut app = build_app();
    assert!(!app.is_project_archived("testproj"));
    app.toggle_archive_project("testproj");
    assert!(app.is_project_archived("testproj"));
    app.toggle_archive_project("testproj");
    assert!(!app.is_project_archived("testproj"));
}

#[test]
fn all_projects_collects_from_active_and_archive() {
    let app = build_app_with_archive("task +alpha\n", Some("x 2026-05-01 2026-04-01 old +beta\n"));
    let projects = app.all_projects();
    assert!(projects.contains(&"alpha".to_string()));
    assert!(projects.contains(&"beta".to_string()));
}

// ---- project management key handling ----

#[test]
fn manage_projects_p_returns_to_normal() {
    let mut app = build_app();
    app.nav.mode = Mode::ManageProjects;
    handle_manage_projects(
        &mut app,
        KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE),
    );
    assert_eq!(app.nav.mode, Mode::Normal);
}

#[test]
fn manage_projects_s_cycles_sort() {
    let mut app = build_app();
    app.nav.mode = Mode::ManageProjects;
    let original = app.project_manager.project_sort;
    handle_manage_projects(
        &mut app,
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
    );
    assert_ne!(app.project_manager.project_sort, original);
}

/// Regression: `/` in the project manager pushes Search over ManageProjects
/// (replacing the old `pre_search_mode` Option field), so Esc pops straight
/// back — the search never "loses" its caller.
#[test]
fn manage_projects_search_esc_returns_to_manage_projects() {
    let mut app = build_app();
    app.nav.mode = Mode::ManageProjects;
    handle_manage_projects(&mut app, key('/'));
    assert_eq!(app.nav.mode, Mode::Search, "`/` must open search");
    assert_eq!(
        app.nav.peek_under(),
        Some(Mode::ManageProjects),
        "search must be stacked over ManageProjects"
    );
    handle_search(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.nav.mode, Mode::ManageProjects);
}

/// The command palette pushes over the current mode and pops back to it,
/// replacing the palette's old hand-rolled `prior_mode` field.
#[test]
fn palette_esc_restores_underlying_mode() {
    let mut app = build_app();
    app.nav.enter_visual();
    apply_action(&mut app, Action::OpenCommandPalette);
    assert_eq!(app.nav.mode, Mode::CommandPalette);
    assert_eq!(
        app.nav.peek_under(),
        Some(Mode::Visual),
        "palette must remember it opened over Visual"
    );
    crate::interactive::overlays::handle_command_palette(
        &mut app,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    );
    assert_eq!(app.nav.mode, Mode::Visual);
    assert!(app.nav.peek_under().is_none(), "stack must be empty");
}

/// A nudge prompt launched from the command palette pops back to the mode
/// the palette returned to — not hard-coded to Normal.
#[test]
fn palette_launched_nudge_prompt_pops_back_to_caller() {
    let mut app = build_app();
    // Palette opened over Settings; Enter pops the palette back to Settings
    // *before* running the chosen action (mirroring handle_command_palette).
    app.nav.mode = Mode::Settings;
    app.nav.push_mode(Mode::CommandPalette);
    app.nav.pop_mode();
    apply_action(&mut app, Action::ConfigureIdleNudge);
    assert_eq!(app.nav.mode, Mode::PromptIdleNudge);
    handle_prompt(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(
        app.nav.mode,
        Mode::Settings,
        "nudge prompt must return to the mode the palette returned to"
    );
}
