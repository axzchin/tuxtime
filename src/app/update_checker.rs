//! Update checker state: latest release tag and background check receiver.

use std::sync::mpsc::Receiver;

#[derive(Debug)]
#[derive(Default)]
pub struct UpdateChecker {
    /// Latest known release tag, populated asynchronously by the update
    /// checker. `None` while we haven't heard back (or the check is disabled,
    /// e.g. in tests).
    pub latest_version: Option<String>,
    /// Receiver for the background update check. Drained each tick; cleared
    /// once a result has been received or the sender hung up.
    pub receiver: Option<Receiver<Option<String>>>,
}

