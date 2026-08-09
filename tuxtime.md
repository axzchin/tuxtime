# tuxtime — Changelog

> Fork of `tuxedo` (v2026.7.1), a Rust ratatui-based todo.txt TUI manager.
> Transformed into a legal timekeeping workflow tool for solo practitioners.

---

## Architecture

| Component | Description |
|---|---|
| Binary | `tuxtime` (was `tuxedo`) |
| Config | `~/.config/tuxtime/config.toml` |
| Data | `todo.txt` + `done.txt` (standard todo.txt format) |
| Clipboard | `arboard` (cross-platform system clipboard) |

---

## Changes from tuxedo (HEAD~1 commit)

### 1. Rename Pass
- Package: `tuxedo` → `tuxtime` in `Cargo.toml`
- Config path: `~/.config/tuxedo/` → `~/.config/tuxtime/`
- All internal string references, terminal title, status bar labels
- 89 files changed, ~2,442 insertions

### 2. Timer System (`t` key)
- **Start**: Sets `start:<ISO-8601>` on the task line, begins counting
- **Stop**: Removes `start:`, adds elapsed to `dur:<seconds>` accumulator
- **Resume**: Re-adds `start:`, preserves existing `dur:` for accumulation
- **Switch**: Stopping one task auto-starts the next; flash shows both outcomes
- **Auto-stop on quit**: Running timer stopped, entry persisted before exit
- **Single-timer invariant**: Enforced in `Store` — only one task has `start:` at a time
- **Live indicator**: Status bar shows `▶ +project @activity HH:MM:SS body…`

### 3. Data Model Extensions (`src/todo.rs`)
- `start: Option<String>` and `dur: Option<u64>` on `Task`
- `parse_line` recognizes `start:` (ISO 8601 with seconds) and `dur:` (u64 seconds)
- `body_only()` filters both from narrative output
- Round-trip serialization preserves new fields

### 4. Timer State (`src/core/mod.rs`, `src/core/mutations.rs`)
- `TimerState { task_abs, started_at }` in `Store`
- `timer_start(abs)`, `timer_stop()`, `timer_toggle(abs)` methods
- `TimerOutcome` enum: `Started`, `Stopped`, `Switched`, `OutOfRange`, `Aborted`, `Error`
- `TimerQuitOutcome` for graceful shutdown
- Restore active timer from `start:` tag on startup

### 5. Manual Time Entry (`M` key)
- Opens insert dialog pre-populated for time entry
- Duration input: `90` (min), `1.5h` (decimal hours), `14:30` (clock time), `9am` (am/pm)
- `parse_duration_input()` converts flexible user input to seconds
- `convert_dur_in_text()` transforms `dur:VALUE` tokens on save
- Duration presets in slash menu: 6m, 15m, 30m, 1h, 1.5h, 2h

### 6. Copy Narratives (`c`/`C`/`y` in Timesheet, `C` in Normal)
- Copies semicolon-joined `body_only()` text for selected project+activity group
- Clipboard via `arboard` (cross-platform)
- Flash: "copied narrative for +project @activity"

### 7. Timesheet View (`V` key)
- Daily view: today's entries grouped by project+activity
- Weekly view (`w`/`d` toggle): last 7 days
- Each group shows: key, total time (formatted + billable), indented narratives
- Grand total at bottom
- `j`/`k` navigate groups, `c`/`C`/`y` copy selected group

### 8. Idle Nudge (configurable)
- `idle_nudge_seconds` (default 900 = 15 min): popup when no timer running
- `long_timer_nudge_seconds` (default 7200 = 2h): status bar flash when timer runs too long
- Popup: `[S]tart timer  [D]ismiss`
- Nudge timer resets on any timer activity

### 9. Config Extensions (`src/config.rs`)
- `idle_nudge_seconds`, `long_timer_nudge_seconds`
- `week_start` (sunday/monday)
- Hot-reload via `config_watcher` (`notify` crate, preserves symlinks)

### 10. New Actions & Keybindings
- `TimerStartStop` (`t`), `ManualTimeEntry` (`M`), `CopyNarratives` (`C`)
- `OpenTimesheet` (`V`), `DismissNudge`
- Command palette entries for all new actions
- Keybinding names in `src/keybinds.rs`

### 11. UI Additions
- Timer indicator in status bar
- `Mode::IdleNudge` popup rendering
- Timesheet inline view rendering
- Duration picker in insert overlay
- `format_duration()` and `format_billable()` helpers

---

## Changes from this session (pending commit)

### 12. Timesheet: Mode → View Refactor
**Why**: Timesheet is a main content view (like Archive), not a transient overlay mode.

| Before | After |
|---|---|
| `Mode::Timesheet` enum variant | `View::Timesheet` enum variant (idx 2) |
| Manual toggle logic, separate `timesheet_cursor` field | `set_view()` auto-saves/restores cursor per view |
| Rendered via `if app.mode == Mode::Timesheet` | Rendered via `match app.view()` (same as List/Archive) |
| Dismiss: `app.mode = Mode::Normal; app.timesheet_cursor = 0` | Dismiss: `app.set_view(View::List)` via action system |

**Files**: `src/app/types.rs`, `src/app/mod.rs`, `src/app/visibility.rs`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/status.rs`

### 13. Key Handling Simplification
- `Esc` and `V` in Timesheet flow through to the action system (`EscapeStack` / `OpenTimesheet`)
- Only `q` stays in `handle_timesheet_keys` as a convenience dismiss (since `q` = `Action::Quit` in the action system)
- `handle_timesheet` renamed to `handle_timesheet_keys`

### 14. Bug Fix: Invisible Copy Flash
**Problem**: `poll_config_reload` overwrote user-triggered flash messages (copy/complete/delete) before they rendered.
**Fix**: Guard with `flash_active().is_none()` — config reload notification defers to user messages.

### 15. UI Cleanup
- `FLASH_TTL`: 1400ms → 1500ms
- Status bar hint: added `y` → `"c/C/y copy"`
- Removed redundant footer hints from `render_timesheet` (duplicated status bar)
- Timesheet layout simplified from 3-row to 2-row

### 16. Test: Timesheet Copy Flash
- `timesheet_copy_flashes_key_in_message` verifies the flash includes project+activity key

### 17. Timesheet Filtering & Sorting
- **Search** (`/` key): reuses existing search bar infrastructure. Filters timesheet entries by narrative body text (case-insensitive substring). Grand total shows `(filtered)` when active.
- **Sort toggle** (`s` key): cycles `by project` → `by date` → `by duration` → `by project`.
  - `by project`: entries sorted by project+activity key, then date
  - `by date`: entries sorted by date, then project+activity — date headers render between day groups
  - `by duration`: entries sorted by total time descending
- **Grouping**: entries keyed by `(date, project+activity)` so each day has its own groups in weekly view
- `TimesheetSort` enum, `TimesheetEntry` struct in `src/app/types.rs`
- Sort label shown in timesheet title bar
- Status bar hint: `s sort  ·  / search`

**Files**: `src/app/types.rs`, `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/status.rs`

### 18. Timesheet Date Navigation
- **Anchor date** (`timesheet_date` field on `App`): determines which day/week the timesheet shows. Resets to today on entry (`V`).
- `h` / `←`: previous day
- `l` / `→`: next day
- `H`: previous week (−7 days)
- `L`: next week (+7 days)
- `t`: jump to today
- Each navigation flashes the new date, resets cursor to 0
- Weekly view computes Monday–Sunday (or Sunday–Saturday per `week_start`) week containing the anchor date
- Title bar shows anchor date; weekly view appends `(week)`
- Navigation methods: `timesheet_shift_days()`, `timesheet_goto_today()`, `timesheet_date_naive()`

**Files**: `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/status.rs`

### 19. Calendar Date Picker (`g` key)
- Pressing `g` in Timesheet opens a **centered calendar overlay** (matching the due date picker)
- `hjkl` / arrow keys navigate the calendar grid; `t` jumps to today; `T` tomorrow; `w` +1 week; `m`/`M` ±1 month
- `Enter` accepts — sets `timesheet_date`, flashes `"jumped to YYYY-MM-DD"`
- `Esc` cancels — date unchanged
- `Mode::PickTimesheetDate` with `timesheet_calendar_focus: NaiveDate` on `App`
- Navigation methods: `timesheet_calendar_move()`, `timesheet_calendar_set_relative()`, `timesheet_calendar_add_months()`, `timesheet_calendar_accept()`, `timesheet_calendar_cancel()`
- Reuses `calendar_cells`, `month_name`, `format_focused`, `calendar_footer` from `dialog.rs` (made `pub(crate)`)

**Files**: `src/app/types.rs`, `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/dialog.rs`, `src/ui/status.rs`

### 20. Calendar Typed Date Input
- While the calendar picker is open (`g`), users can **type a date directly** (digits, dashes, backspace)
- Typed characters accumulate in `timesheet_date_input: String` on `App`
- `timesheet_date_type(code: KeyCode)` method handles push/backspace and syncs calendar focus
- When the input buffer forms a valid YYYY-MM-DD date, the calendar grid focus snaps to it
- Input display line at top of calendar shows typed text with cursor; falls back to `format_focused` when empty
- Enter prefers typed input if non-empty; invalid typed dates flash error and stay in picker for retry
- `g` key clears input buffer on open
- Max 10 characters (YYYY-MM-DD is exactly 10)

**Files**: `src/app/types.rs`, `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/status.rs`

### 21. Day-of-Week Formatting
- Timesheet dates now show weekday everywhere: `"Mon 2026-08-03"` instead of bare `"2026-08-03"`
- `timesheet_date_display()` method on `App` → `%a %Y-%m-%d` format
- Applied to: title bar (daily + weekly), navigation flash messages (h/l/H/L), jumped-to flash, date headers in body
- `t` (today) key now flashes `"today (Mon 2026-08-03)"` for consistency

**Files**: `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`

### 22. Daily Subtotals in Weekly View
- In weekly (`w`) or date-sort (`s`) mode, each day's entries are followed by a subtotal line
- Format: `    ──  2h 0m (2.0h)` in `theme.dim` for subdued appearance
- `day_total: u64` accumulator tracks per-day seconds; flushes on date change and after loop
- Blank line separates the subtotal from the next day's header

**Files**: `src/ui/mod.rs`

### 23. Snapshot Test: Timesheet Weekly with Subtotals
- New `timesheet_weekly_with_daily_subtotals` snapshot in `tests/snapshots.rs`
- Builds app with 4 dur tasks on 2 days (2026-05-05 and 2026-05-06)
- Sets weekly view + date sort → date headers, subtotals, and grand total all captured
- Both `_text` and `_styled` snapshots created

**Files**: `tests/snapshots.rs`, `tests/snapshots/snapshots__timesheet_weekly_with_daily_subtotals_*.snap`

### 24. Two-Step Manual Entry (`M` key)
- Pressing `M` now opens a **choice popup** (`Mode::ManualEntryChoice`) instead of directly entering Insert
- Popup: `[N]ew blank entry  [A]dd to current task  [Esc] cancel`
- `N` → opens Insert dialog with just `"dur:"` for a fresh manual entry
- `A` → enters `PromptAddTime` mode to add time to the existing cursor task's `dur:`
- `Esc` → cancels, returns to Normal
- Replaces the old behavior where `M` always prepopulated with the current task's body
- The idle nudge's `M` key also goes through this choice flow via `apply_action(Action::ManualTimeEntry)`

**Files**: `src/app/types.rs`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/status.rs`

### 25. Add Time to Current Task (`M → A`)
- When a lawyer forgets to start the timer but can estimate how long they worked
- `M` → `A` → type duration (e.g. `30`, `1.5`, `14:30`) → `Enter`
- `add_time_to_current_from_input()` on `App`: parses duration via `parse_duration_input()`, edits the task line via `store.edit_line()`, replaces/adds `dur:` value using plain string operations
- Handles `EditOutcome` properly (Saved, Aborted, Error, Empty, OutOfRange)
- Flash: `"added 30m — Draft motion (total 2h 0m)"`
- `Mode::PromptAddTime` follows the same pattern as `PromptProject`/`PromptSaveFilter` — routes through `handle_prompt`, renders as centered prompt overlay
- Status bar label `"ADD TIME"`, hint `"type duration (e.g. 30, 1.5, 14:30) · Enter add · Esc cancel"`
- Dialog label: `⏱ ADD TIME`

**Files**: `src/app/types.rs`, `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/dialog.rs`, `src/ui/status.rs`

### 26. Removed Duplicate `[C]urrent` Option
- Removed the `[C]urrent task` option from the `M` popup (it created a duplicate task, rarely useful)
- Popup now has two clear options: `[N]ew blank entry` and `[A]dd to current task`

**Files**: `src/main.rs`, `src/ui/mod.rs`

### 27. A Key: Task Context Flash
- When `A` is pressed from the manual entry choice popup and a task is selected:
  - Flash `"add time to: {body}"` so the user sees which task will receive the time
  - Enter `PromptAddTime` directly — the common flow stays fast (M→A→type duration→Enter = 3 keystrokes)
- When no task is under cursor (e.g., after idle nudge with cursor in a weird state):
  - Flash `"no task selected — navigate and press M A"` and dismiss to Normal
  - Resets `last_timer_activity` so the idle nudge doesn't re-fire
- Addresses the idle nudge complaint: the user can see whether A will target the right task, and Esc+re-navigate if not

**Files**: `src/main.rs`

### 28. New Session from Current Task (`N` key)
- `N` (shift-n) in Normal mode creates a fresh Insert session pre-filled with the current task's body and `"dur:"`
- For tracking work across multiple days: Monday's "Draft motion" → `N` → Tuesday's entry with same body
- Uses the same `manual_time_entry` flag + `convert_dur_in_text()` pipeline as `M→N` manual entry
- `Action::BeginSessionFromCurrent` variant; keybinding name `"begin_session_from_current"` / `"new_session"`
- Guarded: flashes `"no task to start session from"` when cursor has no task

**Files**: `src/action.rs`, `src/main.rs`

### 29. Configurable Nudge Thresholds (In-TUI)
- **Idle nudge** (`idle_nudge_seconds`) and **long timer nudge** (`long_timer_nudge_seconds`) now adjustable from the Settings overlay (`,`)
- Settings overlay shows current values with `(i to change)` / `(l to change)` hints
- `i` in Settings → `PromptIdleNudge`: pre-filled with current minutes, type new value, Enter to save
- `l` in Settings → `PromptLongTimerNudge`: same flow
- Validates minutes > 0; flashes confirmation (e.g. `"idle nudge: 30 min"`)
- Returns to Settings after save/cancel so both thresholds can be adjusted in one session
- Persisted to `config.toml` via `prefs.save()` on each change
- Also accessible via command palette (`ConfigureIdleNudge` / `ConfigureLongTimerNudge`)
- Hot-reload picks up manual edits; next TUI adjustment overwrites with the new value

**Files**: `src/app/types.rs`, `src/action.rs`, `src/app/prefs.rs`, `src/app/mod.rs`, `src/main.rs`, `src/ui/settings.rs`, `src/ui/dialog.rs`, `src/ui/status.rs`

### 30. Removed Manual Entry from Idle Nudge
- Removed the `M` key from the idle nudge popup (`handle_idle_nudge`)
- Popup now shows only `[S]tart timer  [D]ismiss` (was `[S]tart timer  [M]anual entry  [D]ismiss`)
- Rationale: from within the nudge, `M → A` trapped the user — `cur_task()` returned whatever was under cursor before the nudge, with no way to navigate to a different task. Cleaner: dismiss the nudge, navigate in Normal mode, then `M → A`.
- Manual entry (`M`) is still fully available from Normal mode — only the nudge shortcut is removed

**Files**: `src/main.rs`, `src/ui/mod.rs`, `tuxtime.md`

### 31. Billable / Non-Billable Flag (`bill:n`)
- **Parsing** (`src/todo.rs`): `bill:n` / `bill:y` recognized in `parse_line`; `bill:y` normalized to absent (default billable). `bill:` filtered by `body_only()`.
- **Task struct**: `pub bill: Option<String>` — `None` = billable, `Some("n")` = non-billable.
- **Timesheet grouping**: key changed from `(String, String)` to `(String, String, bool)` so billable and DNB entries for the same project+activity on the same day form **separate groups**.
- **Rendering**: DNB groups get `(DNB)` suffix on header, dimmed `theme.dim` style on header and narratives. Selection highlight always takes priority.
- **Copy**: DNB entries get `DNB - ` prefix in `c`/`C` output and `(DNB)` suffix in flash messages.

**Files**: `src/todo.rs`, `src/app/types.rs`, `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`

### 32. `b` Key — Toggle Billable Status
- **Normal mode**: `b` calls `toggle_billable()` → adds/removes `bill:n` on task under cursor.
- **Timesheet**: `b` calls `toggle_billable_at(abs)` using exact task index from `timesheet_narrative_at()`.
- Shared method: `toggle_billable_at(abs: usize)` on `App` — takes explicit index, used from both contexts.
- Flash: `"marked as non-billable"` / `"marked as billable"`.
- Status bar: `b billable` added to Normal and Timesheet hints.

**Files**: `src/action.rs`, `src/app/mod.rs`, `src/main.rs`, `src/ui/status.rs`

### 33. 0.1h Billable Rounding
- **Per-group rounding**: `format_billable(total_secs)` rounds up to nearest 0.1h (`div_ceil(360)`) — used on group headers and copy operations.
- **Per-group tenths tracking**: `format_billable_tenths(tenths)` for aggregated values (day subtotals, footer). Each group contributes `div_ceil(360)` tenths independently; sums produce correct totals (1 min × 5 matters = 0.5h, not 0.1h).
- **Display**: group headers show `1h 7m (1.2h)`, day subtotals show `──  1h 7m (1.2h)`, footer shows `Billable: 1h 7m (1.2h)` / `DNB: 0h 30m (0.5h)` / `Total: 1h 37m (1.7h)`.
- **Copy**: `y` copies rounded 0.1h duration (e.g. `1.2h`), `c` copies narratives only, `C` copies combined `"narratives (1.2h)"`.

**Files**: `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/status.rs`

### 34. Timesheet Grand Total Footer Split
- Footer now shows three lines: `Billable: X (Y.Zh)` / `DNB: X (Y.Zh)` (only if >0) / `Total: X (Y.Zh)`.
- `(filtered)` suffix on Total line when search is active.
- Billable + DNB tracked separately alongside grand total.

**Files**: `src/ui/mod.rs`

### 35. Narrative-Level Cursor in Timesheet
- `j`/`k` navigates individual narrative lines (not group headers), via `timesheet_narrative_count()` and `timesheet_narrative_at()`.
- Rendering resolves cursor to the correct group for highlighting.
- Copy keys (`c`/`y`/`C`), `b` (toggle billable), and `Enter` (inline edit) all target the narrative under cursor.

**Files**: `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`

### 36. T Quick Interruption Timer
- When a timer is running, `T` auto-stops it and opens a blank Insert dialog with `dur:` pre-filled.
- `QuickInterrupt` action + keybinding; `interrupt_timer()` on `App`.
- **Auto-start on save**: `auto_start_on_save` flag — interruption entry saves → timer auto-starts on new task.
- `OpenThemePicker` still accessible via command palette.
- Status bar: `T interrupt` added to Normal mode hint.

**Files**: `src/action.rs`, `src/app/mod.rs`, `src/app/mutations.rs`, `src/main.rs`, `src/ui/status.rs`

### 37. Unit Test: Billable/DNB Separate Groups + Independent Rounding
- `build_timesheet_groups_separates_billable_and_dnb`: creates two tasks with same `+work @dev` (one billable, one `bill:n`).
- Asserts: two separate groups, different billable flags, correct narratives, each 60s → 0.1h, per-group tenths sum → 0.2h.

**Files**: `src/main.rs`

---

### 38. Never-Forget Audit: Capture-Gap Fixes

A batch of changes so no time is lost between the app's awareness and the
lawyer's day:

- **Idle-nudge clock resets on manual time entries** — logging time
  (`M → N`, `M → A`, day-boundary add-time) resets the nudge clock, so the
  "No timer running!" popup no longer fires moments after the user just did
  the right thing. Plain task creation doesn't reset it (creating a task
  isn't tracking time).
- **Stale-timer startup prompt** (`Mode::StaleTimer`) — when a timer was
  left running at last close (or the terminal was killed) and has exceeded
  the long-timer threshold, launch asks `[K]eep counting / [S]top & log /
  [D]iscard gap`. `D` strips `start:` without crediting the away time, so a
  zombie session (closed terminal overnight) can never silently bill hours.
- **Launch-time idle backdate** — on a fresh launch with nothing tracked
  today and no running timer, the idle clock starts already past the
  threshold, so the first tick nudges instead of granting a fresh 15-minute
  grace period after hours spent outside the app. The popup says "Nothing
  tracked yet today" (vs. the ordinary "No timer running!").
- **Idle nudge recovery actions** — the popup now offers `[S]tart timer` and
  `[M] add time`; both open a **task picker** (`Mode::PickNudgeTask`) so the
  timer never blind-starts whatever task the cursor happens to be on. The
  user consciously picks a task; `Enter` commits, `Esc` returns to the nudge.
- **Nudge alert in the terminal title + BEL** — when a nudge is active, the
  window title becomes `… ⏰ — timer check` and the bell rings once, so the
  reminder reaches the user when the terminal is unfocused (in another app).
- **End-of-day review nudge** (`Mode::ReviewNudge`, config `review_time =
  "17:00"`) — once per day after the configured time (when something is
  tracked today), asks `[V]iew timesheet / [M] add time / [s]kip`. Fires
  from Normal mode only, at most once per day.
- **Workday coverage line** — with `workday_start`/`workday_end` configured
  (`"09:00"`/`"18:00"`), the daily timesheet shows
  `Unaccounted: 6h 0m of 9h 0m` (with `— day in progress` while today's
  workday is still open), turning the timesheet into an audit surface.

**Files**: `src/app/timer.rs`, `src/app/session.rs`, `src/app/mutations.rs`,
`src/core/mod.rs`, `src/main.rs`, `src/interactive/overlays.rs`,
`src/interactive/dispatch.rs`, `src/ui/overlays.rs`, `src/ui/status.rs`,
`src/ui/nudge_picker.rs` (new), `src/ui/timesheet_render.rs`,
`src/app/timesheet.rs`, `src/config.rs`, `src/app/prefs.rs`,
`src/app/duration.rs`

---

## Keybinding Reference

| Key | Action | Context |
|---|---|---|
| `t` | Start/stop timer | Normal, Visual |
| `M` | Manual entry choice popup | Normal |
| `N` | New session from current task body | Normal |
| `N` (in popup) | New blank manual entry | ManualEntryChoice |
| `A` (in popup) | Add time to current task | ManualEntryChoice |
| `Esc` (in popup) | Cancel manual entry | ManualEntryChoice |
| `b` | Toggle billable/non-billable (`bill:n`) | Normal, Timesheet |
| `T` | Quick interruption: stop timer + blank entry | Normal (timer running) |
| `P` | Open project manager | Normal |
| `Z` | Open theme picker | Normal |
| `c` | Copy narrative text for group under cursor | Timesheet |
| `y` | Copy rounded 0.1h duration for group under cursor | Timesheet |
| `C` | Copy narrative + duration combined | Timesheet |
| `Enter` | Inline edit narrative under cursor | Timesheet |
| `b` | Toggle billable/non-billable for entry under cursor | Timesheet |
| `a` | Unarchive entry (restore from done.txt) | Timesheet |
| `w` | Weekly view | Timesheet |
| `d` | Daily view | Timesheet |
| `h`/`←` | Previous day | Timesheet |
| `l`/`→` | Next day | Timesheet |
| `H` | Previous week (−7 days) | Timesheet |
| `L` | Next week (+7 days) | Timesheet |
| `t` | Jump to today (flashes with DOW) | Timesheet |
| `g` | Open calendar picker (type date directly) | Timesheet |
| `s` | Cycle sort mode (project/date/duration) | Timesheet |
| `/` | Search/filter narratives | Timesheet |
| `j`/`k` | Navigate groups | Timesheet |
| `Esc`/`V`/`q` | Dismiss Timesheet (V toggles back to List) | Timesheet |
| `S` (in nudge) | Task picker → start timer on chosen task | IdleNudge |
| `M` (in nudge) | Task picker → add time to chosen task | IdleNudge |
| `N` (in nudge) | New blank entry | IdleNudge |
| `D`/`Esc` (in nudge) | Dismiss nudge | IdleNudge |
| `j`/`k` | Navigate task picker | PickNudgeTask |
| `Enter` | Commit picked task | PickNudgeTask |
| `Esc` | Back to idle nudge | PickNudgeTask |
| `K`/`S`/`D` | Keep counting / stop & log / discard gap | StaleTimer |
| `V` | Open timesheet (today) | ReviewNudge |
| `M` | Manual entry choice | ReviewNudge |
| `s`/`Esc` | Skip for today | ReviewNudge |

---

## Config Reference

```toml
# ~/.config/tuxtime/config.toml

# Nudge after idle (seconds, default 900 = 15 min)
idle_nudge_seconds = 900

# Nudge when timer runs too long (seconds, default 7200 = 2h)
long_timer_nudge_seconds = 7200

# End-of-day review prompt (once per day after this time). Omit to disable.
# review_time = "17:00"

# Workday bounds for the unaccounted-time coverage line in the daily
# timesheet. Omit either (or both) to hide the line.
# workday_start = "09:00"
# workday_end = "18:00"

# Week start day
week_start = "sunday"  # or "monday"
```
