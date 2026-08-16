//! Billable / non-billable flagging: toggling the `bill:n` tag on a task.
//! `bill:n` marks an entry as non-billable (everything is billable by
//! default). Used from the list view and from the timesheet's narrative
//! cursor, where the cursor tracks groups rather than raw task indices.

use super::App;
use crate::core::outcome::EditOutcome;

impl App {
    /// Toggle the `bill:n` tag on the current task. Adds `bill:n` (marking
    /// non-billable) or removes it (marking billable). Flashes the new status.
    pub fn toggle_billable(&mut self) {
        let Some(abs) = self.cur_task_index_in_tasks() else {
            self.flash("no task selected");
            return;
        };
        self.toggle_billable_at(abs);
    }

    /// Toggle `bill:n` on the task at `abs` (absolute index into tasks).
    /// Used from Timesheet view where the cursor tracks groups, not tasks.
    pub fn toggle_billable_at(&mut self, abs: usize) {
        let raw = self.store.task_raw(abs).unwrap_or_default();
        let (updated, became_nonbillable) = if raw.contains(" bill:n") || raw.ends_with(" bill:n") {
            // Remove `bill:n` — strip the token and collapse whitespace.
            let cleaned = crate::todo::map_body_tokens(&raw, |tok| {
                if tok == "bill:n" {
                    None
                } else {
                    Some(tok.to_string())
                }
            });
            (cleaned, false)
        } else {
            (format!("{raw} bill:n"), true)
        };
        match self.store.edit_line(abs, &updated) {
            EditOutcome::Saved { abs } => {
                if became_nonbillable {
                    self.flash("marked as non-billable");
                } else {
                    self.flash("marked as billable");
                }
                self.after_mutation(abs);
            }
            EditOutcome::Aborted(r) => self.handle_reconcile_abort(r),
            EditOutcome::Error(e) => self.flash(format!("edit failed: {e}")),
            EditOutcome::Empty | EditOutcome::OutOfRange | EditOutcome::TermNotFound => {}
        }
    }
}
