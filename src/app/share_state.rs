//! Share/server state: in-TUI capture server handle, notes directory,
//! and pending editor path for note actions. Owns the bind/start flow so
//! the composition-root module stays free of share-domain logic.

use std::path::PathBuf;

use super::App;
use crate::config::Config;
use crate::serve::{self, ShareInfo};

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

impl App {
    /// Idempotent: bind the capture server on first call, then store
    /// the [`ShareInfo`] so subsequent calls just re-show the overlay.
    ///
    /// First-call behavior: if the user has a persisted token + port in
    /// config, reuse them so phone bookmarks survive across sessions.
    /// Otherwise, generate a fresh token, let the OS pick a port, and
    /// write both back to the config. If the persisted port is taken
    /// (another tuxtime instance on the same machine, say), fall back to
    /// an OS-assigned port and rewrite the config so the next session
    /// starts fresh.
    pub fn ensure_share_started(&mut self) -> Result<&ShareInfo, String> {
        // Two-step to dodge stable's lack of Polonius: do the bind work
        // first (which needs `&mut self`), then the unconditional final
        // borrow returns the now-present `ShareInfo`.
        if self.share_state.share.is_none() {
            let info = self.bind_share()?;
            self.share_state.share = Some(info);
        }
        Ok(self
            .share_state
            .share
            .as_ref()
            .expect("share is Some after the bind branch"))
    }

    fn bind_share(&mut self) -> Result<ShareInfo, String> {
        let cfg = Config::load();
        let token = match cfg.share_token {
            Some(t) => t,
            None => serve::net::generate_token().map_err(|e| format!("token: {e}"))?,
        };
        let requested_port = cfg.share_port.unwrap_or(0);
        let info = match serve::start(self.env.file_path.clone(), token.clone(), requested_port) {
            Ok(info) => info,
            Err(_) if requested_port != 0 => {
                // Configured port is taken — fall back to OS-assigned.
                serve::start(self.env.file_path.clone(), token.clone(), 0)
                    .map_err(|e| format!("bind: {e}"))?
            }
            Err(e) => return Err(format!("bind: {e}")),
        };
        // Persist token + port back to config so phone bookmarks survive.
        // Config::update re-reads the file under an advisory lock, so prefs
        // the user toggled since this App was constructed (or a concurrent
        // save from another instance) survive instead of being clobbered by
        // a whole-file rewrite from a stale read.
        if let Err(e) = Config::update(|to_save| {
            to_save.share_token = Some(info.token.clone());
            to_save.share_port = Some(info.port);
        }) {
            self.flash(format!("share config save failed: {e}"));
        }
        Ok(info)
    }
}
