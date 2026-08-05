# Project Management Spec

## Overview

Add project-level management to tuxtime: archive/hide projects from autocomplete
and picker, and rename projects across all tasks in both `todo.txt` and `done.txt`.

## Storage

**File**: `~/.config/tuxtime/archived-projects.txt`
One project name per line (the bare name, no `+` prefix).

```
old-client
completed-matter-2025
```

- Read on startup alongside config.
- Written atomically on every archive/unarchive change.
- If the file doesn't exist, treat as empty (no archived projects).
- If a project is renamed and it was archived, update the archived list too.

## Entry Point: `P` Project Management View

The dedicated project management view (press `P` in Normal mode) is the
canonical entry point for project-level operations. It lists all projects
across active and archived tasks, supports search filtering (`/`), and
provides:

| Key | Action |
|-----|--------|
| `j`/`k` | Navigate the project list |
| `x` | **Toggle archive** — if the project is currently in the archive list, remove it (unarchive). If it isn't, add it (archive). Flash the action taken. |
| `r` | **Rename** — enter `Mode::PromptRenameProject`. The prompt is pre-filled with the current project name (bare name, no `+` prefix). User types the new name and presses Enter. Renames every `+oldname` token across all active and archived tasks. |
| `s` | Cycle sort mode (by name, archived-last, archived-first) |
| `/` | Search/filter projects |
| `Esc`/`q`/`P` | Dismiss, returning to Normal mode |

The `fp` project picker does not support archive/rename operations — those are
only available in the dedicated `P` view.

## Visibility Rules

After a project is archived:

1. **Project picker** (`fp`): The archived project no longer appears in the list.
2. **Autocomplete popup**: When typing `+...` in Insert mode or `PromptProject`, archived projects are excluded from suggestions.
3. **Project filter sidebar**: Archived projects are hidden from the project list (same as picker).
4. **Tasks themselves are unaffected**: The `+project` token remains on every task. Only the UI lists filter it out.

Unarchiving restores the project to all three lists.

## Rename Project — Full Find-and-Replace

When the user presses `r` in the project picker and confirms with a new name:

### Core: `Store::rename_project(old: &str, new: &str) -> RenameOutcome`

```rust
pub enum RenameOutcome {
    Renamed { old: String, new: String, active_count: usize, archived_count: usize },
    NoTasks,             // no task has this project
    InvalidName,         // new name is empty or contains whitespace
    Aborted(Reconcile),  // external edit detected
    Error(StoreError),
}
```

Algorithm:
1. Reconcile against disk.
2. Validate `new` name (non-empty, no whitespace — same rules as `add_project`).
3. For each active task: parse `projects` field; if it contains `old`, rebuild the raw line replacing `+old` with `+new` (only the exact token, not substrings), re-parse, and write back via `rewrite_raw`.
4. For each archived task (in `done.txt`): same treatment, rebuilding `archive.tasks` in place.
5. Persist both `todo.txt` and `done.txt`.
6. Push one undo-history entry for the batch.
7. If `old` is in the archived-projects list, replace it with `new`.

Token-boundary matching: replace only the exact `+oldname` token. Use `t.projects.iter().position(|p| p == old)` to find it, then reconstruct the raw line by splitting on whitespace and substituting only that occurrence. This avoids false matches like `+proj` matching `+project`.

### App layer: `App::rename_project(old: &str, new: &str)`

Wraps `Store::rename_project`, flashes the result, refreshes visible cache and autocomplete cache. Also updates `cached_archive_projects` and `archived_projects` in memory if the renamed project appears in either.

## Archive/Unarchive Project

### App layer: `App::toggle_archive_project(name: &str)`

```rust
pub fn toggle_archive_project(&mut self, name: &str) {
    if let Some(pos) = self.archived_projects.iter().position(|p| p == name) {
        self.archived_projects.remove(pos);
        self.flash(format!("unarchived +{name}"));
    } else {
        self.archived_projects.push(name.to_string());
        self.flash(format!("archived +{name}"));
    }
    self.save_archived_projects();
}
```

### `save_archived_projects()` / `load_archived_projects()`

- Load: `std::fs::read_to_string` the archived-projects file, split by newlines, trim, filter empty.
- Save: atomic write (same pattern as config saving). One name per line, trailing newline.

### On startup

In `App::from_store`, call `load_archived_projects()`. If the file doesn't exist, start with an empty list.

## Affected Files

| File | Changes |
|------|---------|
| `src/app/types.rs` | Add `Mode::PromptRenameProject`, `RenameOutcome` enum |
| `src/app/mod.rs` | Add `archived_projects: Vec<String>` field, `load_archived_projects`, `save_archived_projects`, `toggle_archive_project`, `rename_project` methods. Initialize and load in `from_store`. |
| `src/core/mutations.rs` | Add `Store::rename_project(old, new)` |
| `src/core/outcome.rs` | Add `RenameOutcome` enum |
| `src/app/picker.rs` | Filter archived projects from `unique_values` output. Add `pick_rename`, `pick_toggle_archive` methods. |
| `src/app/autocomplete.rs` | Exclude archived projects from autocomplete suggestions (in the `projects` source). |
| `src/app/visibility.rs` | Exclude archived projects from the sidebar project list. |
| `src/main.rs` | In `handle_pick`, handle `r` and `x` for `PickProject` mode. Handle `PromptRenameProject` in `handle_prompt`. |
| `src/ui/mod.rs` | Render `PromptRenameProject` prompt overlay. |
| `src/ui/filters.rs` | Exclude archived projects from the project filter sidebar list. |

## UI Mock: `PromptRenameProject`

```
┌─ Rename Project ──────────────────────────┐
│                                            │
│  +old-client  →  +new-client█              │
│                                            │
│           [Enter] confirm  [Esc] cancel    │
└────────────────────────────────────────────┘
```

Pre-filled with the current project name. The `+` prefix is maintained
automatically — the user types only the name part after `+`.

## Edge Cases

- **Rename to existing name**: Allowed — the two projects merge.
- **Rename with no matches**: `RenameOutcome::NoTasks`, flash "no tasks with project +oldname".
- **Rename while file changed externally**: `RenameOutcome::Aborted`, same reconcile pattern as other mutations.
- **Archive already-archived project**: Unarchives it (toggle).
- **Archived-projects file deleted mid-session**: Next save recreates it. Load on missing = empty list.
- **Empty archived-projects file**: Treated as no archived projects. An empty file is valid.
