# tuxtime: Legal Timekeeping TUI — Specification

> Fork of `tuxedo` (v2026.7.1), a Rust ratatui-based todo.txt TUI manager.
> Transformed into a complete legal timekeeping workflow tool for solo practitioners.

---

## 1. Vision & Workflow

**tuxtime** captures the full lawyer workflow:

1. **Capture tasks** — jot down work items using existing todo.txt format (priorities, due dates, projects, activity codes)
2. **Work through tasks** — start a timer on the selected task; the timer counts seconds
3. **Stop the timer** — stopping auto-creates a time entry with the task body as the narrative
4. **Edit narratives** — later, refine narratives on completed time entries
5. **Copy & export** — copy narratives (semicolon-joined) for a project+activity/day to system clipboard; view daily/weekly timesheet summaries

---

## 2. Architecture & Design Decisions

### 2.1 Naming
- Binary/package renamed from `tuxedo` to `tuxtime`
- Config path: `~/.config/tuxtime/config.toml`
- `tuxedo` references in code, docs, and config paths updated throughout

### 2.2 Timer Model
| Aspect | Decision |
|--------|----------|
| Concurrency | **Single timer only** — one active timer at a time |
| State ownership | **Part of `Store`** — persistent, survives redraws and view switches |
| On quit | **Auto-stop** — timer stops, entry is created, app exits |
| Resume | **Continue most recent entry** on the same task — adds to accumulated `dur` |

### 2.3 Keep Everything
All existing `tuxedo` features are preserved:
- Priorities `(A)`–`(C)` and cycling
- Creation dates, due dates (`due:`), recurrence (`rec:`), thresholds (`t:`)
- Archive (`done.txt`), undo/redo, inbox capture server (`serve`), share QR
- Natural language parsing, command palette, themes, density, sort orders
- CLI commands (`add`, `done`, `list`, etc.) — **timer actions are TUI-only in MVP**

### 2.4 Semantics of `+` and `@`
- `+project` — remains the project tag. The user maps projects to clients *outside* the app
- `@activity` — repurposed as an **activity code** (e.g., `@research`, `@drafting`, `@court`, `@meeting`, `@review`)
- Both are parsed inline from task lines as before; no separate client/matter config needed

### 2.5 Out of Scope (MVP)
- Billing/invoicing/hourly-rate calculations
- Multi-user or firm support
- Desktop notifications (visual-only nudges)
- CSV/PDF/LEDES export (TUI timesheet view + clipboard copy only)
- Client metadata (`clients.toml`)

---

## 3. Data Model

### 3.1 Time Entry Format (Inline in todo.txt)

A task accumulates time through new metadata tags on the task line:

```
start:<ISO-8601-with-seconds>  — when the timer most recently started (present = timer running)
dur:<seconds>                  — accumulated tracked time in integer seconds
```

**Example — timer running:**
```
(A) 2026-07-31 Draft motion for summary judgment +Smith @drafting start:2026-07-31T14:30:25 dur:0
```

**Example — timer stopped (entry complete):**
```
(A) 2026-07-31 Draft motion for summary judgment +Smith @drafting dur:3600
```
(Narrative = "Draft motion for summary judgment" — the task body with metadata stripped)

**Example — resumed then stopped again:**
```
(A) 2026-07-31 Draft motion for summary judgment +Smith @drafting dur:5400
```
(`dur` accumulated from two sessions: 3600 + 1800 seconds)

### 3.2 Parsing Rules
- `start:` value is a full ISO 8601 datetime with seconds: `YYYY-MM-DDTHH:MM:SS`
- `start:` is **only present while a timer is actively running** on that task
- `dur:` is a non-negative integer (seconds). Absent dur means 0 (no time tracked yet)
- When timer is stopped: `start:` is removed; elapsed seconds added to `dur:`
- Only **one task** can have a `start:` tag at a time (single-timer invariant)
- The `parse_line` function in `todo.rs` is extended to recognize `start:` and `dur:` as known key:value pairs

### 3.3 Narrative = Task Body
- When "copy narratives" is invoked, the narrative is the task's **body text** (`body_only()` — priority, dates, `+project`, `@context`, `key:value` tags all stripped)
- If the user wants to expand a narrative, they edit the task body (via `e` or `i`) or add a `note:"..."` for supplementary detail
- The copy feature copies **body text only**, not notes
- **No duplication** — the task description IS the narrative in the common case

### 3.4 Manual Time Entries
- Created via a new `ManualEntry` mode/popup
- User enters: duration (in minutes, or decimal hours like `1.5`, or start time like `14:30`), optionally a description override
- Produces a task line with `dur:<seconds>` but no `start:` (already done)
- Default narrative from the description field, or blank if none provided

---

## 4. New Features (Detailed)

### 4.1 Timer Start/Stop

**Keybinding:** `t` on the selected task in Normal/Visual mode

**Start flow:**
1. User presses `t` on a task
2. If a timer is already running on another task → stop that timer first (auto-create entry), flash "stopped <task>; starting <new task>"
3. Set `start:<now>` on the selected task's raw line
4. Re-parse the line. `dur:` is preserved (for resuming)
5. Flash status: "▶ <project> @<activity> — <body snippet>"

**Stop flow:**
1. User presses `t` on the task that has the running timer
2. Calculate elapsed = `now - start_time`
3. Add elapsed to existing `dur:` (or create `dur:<elapsed>` if none)
4. Remove `start:` tag
5. Flash: "■ <project> @<activity> — <elapsed_formatted> (<total_formatted>)"
6. Time entry is now "complete" and available for copy/export

**Resume flow:**
1. User presses `t` on a task that has `dur:>0` but no `start:`
2. Sets `start:<now>` on the line (keeps existing `dur:`)
3. On next stop, elapsed is added to the existing `dur:`

### 4.2 Idle Nudge (No Timer Running)

**Trigger:** No timer has been running for `idle_nudge_seconds` (configurable, default 900 = 15 min)

**Behavior:**
1. Status bar flashes with a distinct color/style
2. A small popup appears: **"No timer running for 15m. [S]tart timer / [M]anual entry / [D]ismiss"**
3. `S` — starts a timer on the currently selected task (same as pressing `t`)
4. `M` — opens the manual entry dialog
5. `D` / `Esc` — dismisses the popup
6. Once any timer is started (or manual entry created), the nudge timer resets

**Configuration in config.toml:**
```toml
idle_nudge_seconds = 900
```

### 4.3 Long-Timer Nudge (Timer Running Too Long)

**Trigger:** A single timer has been running for `long_timer_nudge_seconds` (configurable, default 7200 = 2 hours)

**Behavior:**
1. Status bar flashes to remind the user they might have forgotten to stop
2. No popup — just the visual indicator
3. Resets when the timer is stopped

**Configuration in config.toml:**
```toml
long_timer_nudge_seconds = 7200
```

### 4.4 Manual Time Entry

**Keybinding:** `M` (Shift+m) in Normal mode, or accessible via command palette, or from the idle nudge popup

**Flow:**
1. Opens a small popup/dialog
2. Fields:
   - **Duration** (required): accepts `90` (minutes), `1.5` (hours), or `14:30` (clock time → calculates duration from now)
   - **Description** (optional): defaults to the current task's body if a task is selected, otherwise blank
   - **Project** (optional, pre-filled from selected task)
   - **Activity** (optional, pre-filled from selected task)
3. On confirm (Enter): creates a new task line in todo.txt with `dur:<seconds>` and the description as body
4. On cancel (Esc): no entry created

### 4.5 Copy Narratives

**Keybinding:** `C` (Shift+c) in Normal mode, or via command palette

**Flow:**
1. If a project filter is active, copies for that project. Otherwise, prompts or defaults to project of selected task
2. If an activity filter is active, copies for that activity. Otherwise, copies across all activities for the project
3. Collects all time entries for **today** matching the project (+activity) filter
4. Extracts each entry's body text (via `body_only()`)
5. Joins them with `; ` (semicolon-space)
6. Copies to system clipboard
7. Flash: "copied N narratives for +project @activity"

**Example output (clipboard):**
```
Drafted motion for summary judgment; Reviewed discovery responses; Prepared exhibit list
```

### 4.6 Timesheet Summary View

**Access:** Via command palette action "Timesheet" or a dedicated keybinding (`V`? `ts`?)

**View:** A new modal/overlay or a new `View::Timesheet` variant

**Content (Daily view — default):**
- Shows today's date
- For each project+activity combination with time today:
  - Project name, activity name
  - Total time (formatted as `Xh Ym`)
  - List of narratives (one per line, indented)
- Footer: grand total for the day

**Content (Weekly view — toggle):**
- Same as daily but spanning the last 7 days
- Grouped by day, then by project+activity within each day
- Daily subtotals + weekly grand total

**Navigation:**
- Switch between daily/weekly with `w`/`d` keys
- `Esc` or `q` to dismiss, returning to List view

### 4.7 Timer Indicator in Status Bar

**When timer is running:**
```
▶ +Smith @drafting  00:12:34  Draft motion for summary judgment...
```
(Elapsed time as HH:MM:SS, updating live every second, task body truncated)

**When no timer:**
```
■  idle  ·  3 tasks shown  ·  priority  ·  comfortable
```
(No timer information shown)

---

## 5. Config Changes

### 5.1 New `config.toml` Keys

```toml
# Nudge after this many seconds with no timer running (default 900 = 15 min)
idle_nudge_seconds = 900

# Nudge after a single timer runs this long (default 7200 = 2 hours)
long_timer_nudge_seconds = 7200
```

### 5.2 Updated `Config` Struct

Add fields:
```rust
pub idle_nudge_seconds: Option<u64>,
pub long_timer_nudge_seconds: Option<u64>,
```

Parse from `idle_nudge_seconds` and `long_timer_nudge_seconds` keys. Serialize back on save.

---

## 6. Implementation Targets (Module-by-Module)

### 6.1 `src/todo.rs` — Parser Extensions
- Add `start:` and `dur:` parsing to `parse_line`
- `start:` value: validate ISO 8601 with seconds
- `dur:` value: parse as `u64`
- Add `start: Option<String>` and `dur: Option<u64>` fields to `Task`
- Extend `is_meta_token` and `body_only` to filter `start:` and `dur:` (they should NOT appear in body/narrative output)
- Ensure round-trip serialization handles new fields

### 6.2 `src/core/mod.rs` — `Store` Extensions
- Add `active_timer: Option<TimerState>` to `Store`
  ```rust
  struct TimerState {
      task_abs: usize,       // index into self.tasks
      started_at: Instant,   // wall-clock instant for live elapsed display
  }
  ```
- Add `timer_start(abs)`, `timer_stop()`, `timer_toggle(abs)` methods
- `timer_stop` calculates elapsed, updates the task's `dur`, removes `start:`
- On `Store` construction, scan tasks for one with `start:` → restore `active_timer`
- `active_task_raw_with_timer` or similar: gets the running timer's task raw line for status bar

### 6.3 `src/core/mutations.rs` — Timer Mutations
- `start_timer(&mut self, abs: usize)` — sets `start:<now>`, updates `active_timer`
- `stop_timer(&mut self) -> TimerStopOutcome` — removes `start:`, adds to `dur:`, returns elapsed + total
- `add_manual_entry(...)` — creates a task line with `dur:` and description
- On **quit**: `stop_timer_on_quit(&mut self)` — auto-stops and persists

### 6.4 `src/action.rs` — New Actions
```rust
TimerStartStop,     // 't' — toggle timer on selected task
ManualTimeEntry,    // 'M' — open manual entry dialog
CopyNarratives,     // 'C' — copy today's narratives for project+activity
OpenTimesheet,      // command palette / keybinding
DismissNudge,       // dismiss the idle-nudge popup
```

### 6.5 `src/app/mod.rs` — App State
- `Mode::ManualEntry` — new mode for the manual entry popup
- `Mode::Timesheet` — new mode for the timesheet overlay
- `Mode::IdleNudge` — new mode for the idle nudge popup
- `nudge_timer: Instant` — tracks time since last timer activity
- Timer elapsed display logic (tick every second)
- `recompute_visible` must handle tasks with `start:` (they're still visible)

### 6.6 `src/main.rs` — Event Loop Changes
- **Timer tick:** Currently polls at 250ms. To display live elapsed seconds in the status bar, we may need a more frequent tick when a timer is running (1s). Or compute elapsed from `Instant::now()` on each frame.
- **Idle nudge check:** On each event poll tick, check if no timer running and idle time exceeded threshold → enter `Mode::IdleNudge`
- **Long timer check:** On each tick, check if running timer exceeded threshold → set a flag for status bar rendering
- **Quit handling:** On `Action::Quit`, call `stop_timer_on_quit` before setting `should_quit`
- **New key handlers:** `handle_timesheet`, `handle_manual_entry`, `handle_idle_nudge`

### 6.7 `src/ui/` — Rendering Extensions

**`ui/status.rs`:**
- Render live timer indicator when timer is running: `▶ +project @activity  HH:MM:SS  body...`
- Render nudge indicator (flashing/highlighted background)
- Idle state: normal rendering with subtle "idle" indicator after threshold

**`ui/mod.rs`:**
- Route `Mode::ManualEntry`, `Mode::Timesheet`, `Mode::IdleNudge` to their renderers

**New files:**
- `ui/timesheet.rs` — timesheet summary view rendering
- `ui/manual_entry.rs` — manual time entry dialog rendering
- `ui/idle_nudge.rs` — idle nudge popup rendering

### 6.8 `src/config.rs` — Config Extensions
- Parse `idle_nudge_seconds` and `long_timer_nudge_seconds`
- Serialize to `config.toml`

### 6.9 `src/app/prefs.rs` — Prefs
- Expose nudge thresholds from `Config` into `Prefs` so the app can read them without touching config directly each tick

### 6.10 `src/cmd/mod.rs` — CLI (Minimal Changes)
- Update `--help` output to reflect new name `tuxtime`
- CLI commands for timer operations are deferred (TUI-only for MVP)

### 6.11 `src/keybinds.rs` — Keybinding Names
- Add `timer_start_stop`, `manual_time_entry`, `copy_narratives`, `open_timesheet` to recognized names

### 6.12 `src/clipboard.rs` — Already Exists
- Reuse existing `clipboard::copy()` for the copy-narratives feature

### 6.13 Rename Pass
- Package name in `Cargo.toml`: `tuxedo` → `tuxtime`
- Config path: `tuxedo` → `tuxtime` in `xdg.rs`/`config.rs`
- Terminal title and status references
- Docs, examples, README, flake.nix
- All internal references to "tuxedo" in strings (flashes, usage text, etc.)

---

## 7. Keybinding Summary

| Key | Action | Mode |
|-----|--------|------|
| `t` | Start/stop timer on selected task | Normal, Visual |
| `M` | Manual time entry dialog | Normal |
| `C` | Copy narratives (today, project+activity) | Normal |
| `V` (or command palette) | Open timesheet view | Normal |
| `S` (in nudge popup) | Start timer | IdleNudge |
| `M` (in nudge popup) | Manual entry | IdleNudge |
| `D` / `Esc` (in nudge popup) | Dismiss nudge | IdleNudge |
| `Esc` / `q` | Dismiss timesheet/manual-entry | Timesheet, ManualEntry |
| `w` | Weekly view (in timesheet) | Timesheet |
| `d` | Daily view (in timesheet) | Timesheet |

All existing keybindings preserved.

---

## 8. Open Questions / Deferred Decisions

1. **Should the shell command `tuxtime start "Draft motion" +Smith @drafting` work from CLI?** Deferred. Timer actions are TUI-only in MVP.
2. **Should there be a `timesheet export` CLI command?** Deferred. Timesheet is TUI view only in MVP.
3. **Should `done` tasks with time entries be included in timesheet/copy?** Yes — time tracked is time tracked, even if the task is complete.
4. **What happens to `start:` if the user manually edits the line?** Standard re-parse handles it. If they remove `start:`, the timer is effectively stopped (no elapsed added to dur). This is acceptable — the user is explicitly editing the raw line.
5. **Should timer survive `undo`?** Yes. Undo restores the previous task state including `start:`/`dur:` values.

---

## 9. Test Plan

### Unit Tests
- Parser: `start:` and `dur:` parse correctly; invalid values rejected; round-trip serialization
- Timer: start/stop elapsed accumulation; resume adds correctly; single-timer invariant enforced
- Nudge: idle threshold triggers; long-timer threshold triggers; dismiss resets
- Copy narratives: correct body extraction; semicolon join; project+activity filter; date filtering
- Manual entry: duration parsing (minutes, hours, clock time); task line generation
- Config: new keys parse/serialize round-trip

### Snapshot Tests
- Existing snapshots updated for new metadata tokens (body_only changes)
- New snapshots: timesheet view, manual entry dialog, idle nudge popup, status bar with active timer

### Integration Tests
- Full flow: create task → start timer → stop → verify dur → resume → stop → verify accumulated dur
- Quit with running timer → verify entry created, no `start:` remains
