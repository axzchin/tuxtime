# Lawyer UX Improvements — Specification

> Based on interviews with a solo practitioner lawyer using tuxtime alongside
> a billing system. Captures pain points, workflow gaps, and proposed solutions.

---

## 1. Interview Summary

### User Profile
- Solo practitioner lawyer
- Uses tuxtime for time tracking + a separate billing system for invoicing
- Billing system uses **decimal hours** (0.1h = 6 min increments)
- All time billable by default; non-billable entries flagged explicitly

### Current Workflow
1. Create tasks in tuxtime (often rushed — just enough detail to start working)
2. Start/stop timer (`t`) as they work through tasks
3. At end of day, open Timesheet (`V`), review entries, copy narratives
4. Paste narratives into billing system, **manually type billable time**
5. Non-billable entries get `DNB - ` prefix before pasting

### Pain Points (ranked)
| # | Pain | Severity |
|---|---|---|
| 1 | **Timer management** — forgetting to start, forgetting to stop, interruptions | High |
| 2 | **Narrative editing** — task body is rushed, not client-ready; needs polishing at end of day | High |
| 3 | **Copy workflow** — narratives and duration copied separately; no 0.1h rounding on copy | Medium |
| 4 | **Billable/non-billable** — no way to flag entries; manual DNB prefix each time | Medium |

---

## 2. Proposed Features

### 2.1 Inline Narrative Editing in Timesheet (`Enter` key)

**Problem**: Narratives captured during the day are rushed — they need polishing
before they're client-facing. Currently the user must exit Timesheet, find the
task in the list, edit it there, then return to Timesheet.

**Solution**: Press `Enter` on any narrative line in the Timesheet to open a
focused inline edit dialog, pre-filled with that task's body text. Edit, save,
and the timesheet re-renders with the updated text — all without leaving the
Timesheet view.

**Flow**:
1. Timesheet view open, `j`/`k` navigates entries
2. Press `Enter` on an entry's narrative line
3. Opens a small edit popup pre-filled with the full task body (raw line)
4. User edits the narrative text (task body), adds detail, formalizes tone
5. `Enter` saves → task line updated in todo.txt → timesheet re-renders
6. `Esc` cancels → no change

**Key design decisions**:
- Edit popup shows the **full task line** (priority, dates, projects, contexts, body) so the user can edit any part — not just the narrative body
- Reuses existing insert/edit infrastructure (same edit dialog as `e`/`i` in List view)
- Only available on entries with a backing task line (not on subtotal/grand total lines)
- The narrative line index must map back to the original task for editing

**Implementation notes**:
- Each `TimesheetEntry` needs to carry the task index (or a reference) for lines that back to real tasks
- Need to distinguish "group header" lines (project+activity + total) from "narrative" lines for the Enter key
- On save, `rebuild_timesheet_groups()` must re-run to refresh the view
- Status bar hint update: add `Enter edit narrative`

**Files**: `src/app/types.rs`, `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/status.rs`

---

### 2.2 Billable / Non-Billable Flag (`bill:n`)

**Problem**: Some entries are non-billable (admin, CPD, firm management) but
there's no way to flag them. The user manually types `DNB - ` before pasting.

**Proposed solution**: An inline `bill:n` metadata tag on task lines.
Everything is **billable by default**. `bill:n` marks an entry as non-billable.

**Data model**:
```
# Billable (default — no tag needed):
(A) 2026-07-31 Draft motion +Smith @drafting dur:3600

# Non-billable:
2026-07-31 Firm admin +Admin @admin dur:900 bill:n
```

**Parsing** (extends `parse_line` in `src/todo.rs`):
- Recognize `bill:` as a known key:value pair
- Valid values: `y` (billable — default), `n` (non-billable)
- `bill:` is filtered by `body_only()` — does NOT appear in narratives
- `bill:y` is equivalent to omitting the tag (default)
- Round-trip serialization preserves the tag

**Task struct addition**:
```rust
pub bill: Option<String>,  // None = billable (default), Some("n") = non-billable
```

**Timesheet behavior**:
- **Grouping key changes** from `(date, project+activity)` to `(date, project+activity, billable?)`. Billable and DNB entries for the same project+activity on the same day form **separate groups**. DNB groups get a `(DNB)` suffix on the group header:
  ```
  +Admin @admin  —  1h 0m (1.0h)
    • Firm administration
  +Admin @admin (DNB)  —  0h 30m
    • Organised CPD records
  ```
- Copy narratives (`c`/`C`/`y`): non-billable entries get `DNB - ` prefix:
  ```
  DNB - Firm administration; Organised CPD records
  ```
- Billable entries copy normally — no prefix needed
- Each group copies independently — there's no mixing of billable and DNB in one copy operation

**Timesheet totals** (split subtotals at grand total only):
- The grand total footer shows three lines:
  ```
    Billable: 6h 30m (6.5h)
    DNB:      2h 0m
    Total:    8h 30m
  ```
- Billable line shows raw time + rounded 0.1h value in parens (ready to copy into billing)
- DNB line shows raw time only (non-billable doesn't go to billing, so no rounding needed)
- Total line shows combined raw time for a complete picture of the day
- When there are no non-billable entries, only the Total line appears (no DNB line)
- When there are no billable entries, the Billable line shows `0h 0m (0.0h)`
- Filtered search: when search is active, subtotals reflect only matching entries; `(filtered)` suffix on Total line
- **Per-day subtotals** (the `──` lines in weekly/date-sort view): stay as a single combined line — no billable/DNB split. Rationale: weekly view is for scanning/overview; the billing split matters at review time, which happens at the grand total level. Per-day split would add 21+ lines of clutter for information that's rarely needed at that granularity.

**Toggle key**: `b` key in Timesheet and in Normal mode on selected task. Toggles `bill:n` tag. Flash: `"marked as non-billable"` / `"marked as billable"`. In Timesheet, toggling immediately re-renders with updated subtotals — the toggled entry may move to a different group (billable ↔ DNB) on re-render.

### 2.2.1 Narrative-Level Cursor in Timesheet

**Decision**: The timesheet cursor operates at the **narrative level**, not the
group level. `j`/`k` moves between individual narrative lines; group headers,
date headers, and subtotal lines are non-interactive visual elements that the
cursor skips over.

**Why**: The `b` (toggle billable), `Enter` (edit narrative), and copy keys
all need to target a specific entry, not a whole group. Making the cursor
narrative-level gives every key a precise target without ambiguity.

**Cursor behavior**:
- `j` / `k` / `Down` / `Up`: move to next/previous narrative line, skipping
  group headers, date headers, day-subtotal lines, and the grand total footer
- Only the active narrative line gets the selection highlight (theme.selection
  background). Group headers are NOT highlighted even when their narratives
  are selected
- When the cursor is on a narrative, copy keys (`c`/`C`/`y`) copy the
  **entire group** that contains that narrative — the user doesn't need to
  navigate to a group header to copy

**`b` key at narrative level**:
- Toggles `bill:n` on the specific entry under the cursor
- On re-render, the entry may move groups:
  - Billable → DNB: the entry leaves its billable group, forms (or joins)
    a `(DNB)` group for the same project+activity
  - DNB → Billable: reverse — leaves the DNB group, joins the billable group
- If the toggled entry was the last one in its group, the group disappears;
  cursor snaps to the nearest remaining narrative
- Flash: `"marked as non-billable"` or `"marked as billable"`

**`Enter` key at narrative level**:
- Opens inline edit dialog pre-filled with the full task line of the entry
  under cursor (see §2.1)

**`b` key in Normal mode** (List/Archive view):
- Toggles `bill:n` on the task under cursor, same as in Timesheet
- Flash: same `"marked as non-billable"` / `"marked as billable"`

**Implementation notes**:
- The cursor tracks a flat index into the ordered list of all narrative lines
  (not group indices)
- `TimesheetEntry` needs to carry the task index (abs position in `tasks`)
  for each narrative so `b` and `Enter` can find the right task to mutate
- On group rebuild (after edit/toggle/filter change), cursor should try to
  stay on the same task; if that task is no longer in view, snap to the
  nearest remaining narrative
- When the timesheet is empty, cursor is 0 (no-op — handled by the existing
  empty-state UI)

**Config option** (deferred): `default_billable = true` in config.toml — lets firms that are non-billable-by-default flip the default.

**Files**: `src/todo.rs`, `src/app/types.rs`, `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/status.rs`

---

### 2.3 0.1h Rounding on Copy

**Problem**: The billing system uses 0.1h increments (6 minutes). Raw `dur:`
values track in seconds — 7 minutes = 420 seconds = 0.1167h. The user needs
this rounded UP to 0.2h before pasting into their billing system.

**Solution**: Round durations up to the nearest 0.1h (6 min = 360 seconds) at
copy time. Raw tracking stays in seconds — rounding only applies to the
formatted output from copy operations.

**Rounding function**:
```rust
/// Round seconds up to the nearest 0.1 hour (360 seconds).
/// 0 → 0.0, 1-360 → 0.1, 361-720 → 0.2, etc.
fn round_up_01h(seconds: u64) -> f64 {
    let units = (seconds as f64 / 360.0).ceil();
    (units * 0.1 * 100.0).round() / 100.0  // round to 2 decimal places
}
```

**Copy output format** (two-step copy):

**Step 1 — Copy narrative** (`c` key):
```
Drafted motion for summary judgment; Reviewed discovery responses; Prepared exhibit list
```
(No change from current behavior — narratives joined by `; `)

**Step 2 — Copy duration** (`y` key, or adjust existing):
```
1.5
```
(Rounded up to nearest 0.1h in decimal. For non-billable: still shows the duration.)

**Combined copy** (`C` key, Shift-c):
```
1.5h — Drafted motion for summary judgment; Reviewed discovery responses; Prepared exhibit list
```
(Each entry with its own duration. Or a single total for the group.)

**Status bar hints**: Update to show `c copy narrative · y copy duration · C copy both`

**Display**: Timesheet view should show raw time (HH:MM or formatted) plus
the rounded 0.1h value in parentheses:
```
+Smith @drafting  —  1h 7m (1.2h)
  • Drafted motion for summary judgment
  • Reviewed discovery responses
```

**Key design decision**: Rounding is configurable but defaults to 0.1h.
The `config.toml` accepts `rounding_increment` as a decimal hour value:
```toml
# Round durations up to this increment on copy (decimal hours).
# 0.1 = 6 min (standard US legal billing), 0.25 = 15 min.
# Omit or set to 0 for no rounding.
rounding_increment = 0.1
```

**Files**: `src/app/mod.rs`, `src/main.rs`, `src/ui/mod.rs`, `src/ui/status.rs`, `src/config.rs`

---

### 2.4 Quick Timers for Interruptions

**Problem**: When interrupted (phone call, colleague stops by), the user needs
to stop their current timer and quickly start logging the interruption.
Currently this is: stop timer (`t`) → navigate to interruption task → start
timer (`t`) — 3+ keystrokes.

**Solution**: When a timer is running, pressing `T` (Shift-t) opens a
**quick-timer popup** that lets the user pick or create an interruption task
without losing the current timer context.

**Flow**:
1. Timer running on "Draft motion +Smith @drafting"
2. Colleague stops by — user presses `T`
3. Popup: "Interrupt timer?" with options:
   - `[n]ew quick task` — opens a mini prompt for a description (e.g. "call with Jim"), creates a new task, starts timer on it
   - `[s]elect task` — shows a quick picker of recent/visible tasks, select one, start timer on it
   - `[Esc] cancel` — go back to current timer
4. Current timer is **automatically stopped** (entry created) before the new one starts
5. Flash: "stopped Draft motion (1h 30m); started call with Jim"

**Alternative (simpler)**: Don't add a popup. Just make `T` auto-stop the
current timer and immediately start a new blank entry for the interruption:
- `T` → stops current timer → opens mini insert with `"dur:"` pre-filled
  and a blank body → user types "call with Jim dur:" → Enter saves → starts
  timer on the new task

This is simpler to implement and less modal. The user just describes what the
interruption was, then continues. No popup, no picker.

**Decision**: Start with the simpler version (`T` = stop + blank quick entry).
If picker/selections are needed later, add them.

**Keybinding**: `T` (Shift-t) in Normal mode when a timer is running.
When no timer is running, `T` is the same as `t` (or flash a hint).

**Files**: `src/action.rs`, `src/main.rs`, `src/app/mod.rs`

---

### 2.5 Narrative Review Mode (Timesheet + Inline Edit)

**Problem**: At end of day, the user needs to review all entries — polish
narratives, verify durations, ensure nothing is missing. Currently this means
navigating between Timesheet (see entries) and List (edit entries).

**Solution**: Enhance the Timesheet view to serve as the review interface.
Combined with inline editing (#2.1), the Timesheet becomes the one-stop review
screen.

**Review workflow**:
1. Press `V` → Timesheet (daily view by default)
2. Scan entries — see project, activity, duration, narrative for each
3. Spot a rushed narrative → `Enter` → edit → save → back in timesheet
4. Spot wrong duration → `M → A` from task, or use `y` to copy the rounded duration for billing system
5. Toggle billable/non-billable → `b` on entry
6. When satisfied → `c`/`C`/`y` to copy → paste into billing system

**Enhancements to existing Timesheet**:
- `Enter` on narrative → inline edit (#2.1)
- `b` on entry → toggle billable/non-billable (#2.2)
- Durations show rounded value in parens: `1h 7m (1.2h)` (#2.3)
- Non-billable entries visually distinguished (dimmed or `(DNB)` tag) (#2.2)
- Footer shows split subtotals: Billable / DNB / Total (#2.2)
- `y` copies rounded duration, `c` copies narrative, `C` copies both with duration (#2.3)

---

### 2.6 Suggested (Not Prioritized — for Future Discussion)

These came up during the interview but are deferred:

| Idea | Notes |
|---|---|
| **Pause/resume timer** | User questioned whether it's different from stop/start. Deferred until clearer use case emerges. |
| **Auto-stop on inactivity** | Stop timer after N minutes of keyboard inactivity. Could complement the long-timer nudge. |
| **Weekly/monthly total report** | Export timesheet totals to clipboard or a file. |
| **Client portal / share** | Extend the existing `s` (share) server to show a timesheet view. |
| **LEDES export** | User doesn't need it now but it's the legal industry standard. |
| **Billing system API integration** | Direct push to practice management software (Clio, MyCase, etc.) rather than clipboard copy-paste. |

---

## 3. Data Model Changes

### 3.1 New Metadata Tags

| Tag | Values | Default | Description |
|---|---|---|---|
| `bill:` | `y` (billable), `n` (non-billable) | absent = billable | Entry billing status |

### 3.2 Updated `Task` Struct (`src/todo.rs`)

```rust
pub struct Task {
    // ... existing fields ...
    pub bill: Option<String>,  // None = billable (default), Some("n") = non-billable
}
```

### 3.3 Updated `TimesheetEntry` Struct (`src/app/types.rs`)

```rust
pub struct TimesheetEntry {
    pub date: String,
    pub key: String,          // "+project @activity" (no DNB suffix — that's a display concern)
    pub total_secs: u64,
    pub narratives: Vec<String>,
    pub billable: bool,       // true = billable, false = DNB
}
```

Groups are keyed by `(date, project+activity, billable)` instead of the
original `(date, project+activity)`. This ensures billable and DNB entries
for the same matter on the same day are always separate groups.

### 3.4 Parsing Rules

- `bill:y` and `bill:n` recognized in `parse_line`
- `bill:y` is equivalent to omitting the tag (no-op)
- `bill:n` marks as non-billable
- `bill:` is filtered by `body_only()` and `is_meta_token()`
- Round-trip serialization preserves `bill:` tags

---

## 4. Config Changes

### 4.1 New `config.toml` Keys

```toml
# Round durations up to this increment on copy (decimal hours).
# 0.1 = 6 min increments (standard US legal billing)
# 0.25 = 15 min increments
# 0 = no rounding (raw seconds converted to decimal)
rounding_increment = 0.1
```

### 4.2 Config Struct Additions

```rust
pub rounding_increment: Option<f64>,  // default 0.1
```

---

## 5. Implementation Order

Suggested order — each builds on the previous:

| # | Feature | Effort | Depends on |
|---|---|---|---|
| 1 | `bill:n` tag parsing + data model | Small | — |
| 2 | `b` key to toggle billable status | Small | #1 |
| 3 | DNB prefix in copy output | Small | #1, #2 |
| 4 | Timesheet inline narrative edit (`Enter`) | Medium | — |
| 5 | 0.1h rounding + `y` duration copy | Medium | — |
| 6 | `T` quick interruption timer | Small | — |

---

## 6. Keybinding Changes

| Key | Action | Context |
|---|---|---|
| `Enter` | Edit narrative (opens inline edit popup) | Timesheet (on narrative line) |
| `b` | Toggle billable/non-billable | Normal, Timesheet |
| `y` | Copy rounded duration (0.1h) | Timesheet |
| `c` | Copy narrative (unchanged behavior) | Timesheet |
| `C` | Copy narrative + duration combined | Timesheet |
| `T` | Quick interruption: stop current, start blank entry | Normal (timer running) |

---

## 7. Test Plan

### Unit Tests
- `bill:n` parse/serialize round-trip
- `body_only()` excludes `bill:` tag
- `b` toggle: adds `bill:n`, removes it, flashes correctly
- Copy: non-billable prefix `DNB - ` applied
- Copy: billable (default) has no prefix
- `round_up_01h()`: 0→0.0, 1→0.1, 360→0.1, 361→0.2, 720→0.2
- Timesheet entry carries task index for Enter-to-edit mapping

### Snapshot Tests
- Timesheet with `bill:n` entry — verify DNB visual indicator
- Timesheet with rounded duration display: `1h 7m (1.2h)`
- Timesheet footer with split subtotals: Billable / DNB / Total
- Timesheet with only billable entries — DNB line absent, only Total shown
- Timesheet with only non-billable entries — Billable line shows `0h 0m (0.0h)`
- Inline edit popup in Timesheet
- Timesheet review mode (all enhancements active)
