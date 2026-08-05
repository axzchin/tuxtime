//! Archive autocomplete cache: project and context names from `done.txt`,
//! rebuilt when the archive changes.

#[derive(Debug, Default)]
pub struct ArchiveCache {
    /// Cached project names from the archive (done.txt). Rebuilt when the
    /// archive changes so autocomplete doesn't scan done.txt on every keystroke.
    pub projects: Vec<String>,
    /// Cached context names from the archive (done.txt).
    pub contexts: Vec<String>,
}
