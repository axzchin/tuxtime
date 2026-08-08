//! Archive autocomplete cache: project and context names from `done.txt`,
//! rebuilt when the archive changes.

use crate::core::Archive;

#[derive(Debug, Default)]
pub struct ArchiveCache {
    /// Cached project names from the archive (done.txt). Rebuilt when the
    /// archive changes so autocomplete doesn't scan done.txt on every keystroke.
    pub projects: Vec<String>,
    /// Cached context names from the archive (done.txt).
    pub contexts: Vec<String>,
}

impl ArchiveCache {
    /// Scan the archive for unique project and context names so autocomplete
    /// can offer them without re-scanning done.txt on every keystroke. Call
    /// whenever the archive may have changed (startup, archive mutations,
    /// external `done.txt` edits picked up by polling).
    pub fn rebuild(&mut self, archive: &Archive) {
        let mut projs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut ctxs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for t in archive.tasks() {
            for p in &t.projects {
                projs.insert(p.clone());
            }
            for c in &t.contexts {
                ctxs.insert(c.clone());
            }
        }
        self.projects = projs.into_iter().collect();
        self.contexts = ctxs.into_iter().collect();
    }
}
