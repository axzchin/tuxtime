//! Share/server state: in-TUI capture server handle, notes directory,
//! and pending editor path for note actions.

use std::path::PathBuf;

use crate::serve::ShareInfo;

#[derive(Debug)]
pub struct ShareState {
    /// Handle to the in-TUI capture server. `None` until the first time
    /// the user presses `s` (or invokes "show capture QR" from the
    /// palette). Once bound, the entry stays for the rest of the
    /// session and the overlay just re-displays the saved QR.
    pub share: Option<ShareInfo>,
    /// Base directory used by note actions. Relative `note:<path>` tokens are
    /// resolved under this directory, and generated notes are created below it.
    pub notes_dir: PathBuf,
    /// Path queued for opening in the user's editor after the TUI temporarily
    /// restores the terminal. Set by `OpenNote` and drained by the run loop.
    pub pending_editor_path: Option<PathBuf>,
}

impl ShareState {
    #[must_use]
    pub fn new(notes_dir: PathBuf) -> Self {
        Self {
            share: None,
            notes_dir,
            pending_editor_path: None,
        }
    }
}
