//! Shared advisory file-lock helper.
//!
//! tuxtime guards short read-modify-write sections (config saves, inbox
//! drain/append) with exclusive advisory locks so concurrent writers — a
//! second tuxtime instance, or the capture server's POST handler vs. the TUI
//! drain — can't interleave and lose each other's updates. The lock is taken
//! on a persistent *sidecar* file (e.g. `todo.txt.tuxtime-lock`), never the
//! data file itself, because the data files are rewritten via atomic
//! tmp-then-rename: locking the data file would lock an inode that the next
//! rename discards.

use std::fs;
use std::io;
use std::path::Path;

/// Open-or-create `path` and take an exclusive advisory lock on it. The
/// returned handle holds the lock for its lifetime — drop it (or the process
/// exits/crashes) to release. Cross-platform via `std::fs::File::lock`
/// (`flock` on Unix, `LockFileEx` on Windows).
pub fn acquire(path: &Path) -> io::Result<fs::File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)?;
    file.lock()?;
    Ok(file)
}
