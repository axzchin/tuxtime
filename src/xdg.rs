//! XDG Base Directory resolution shared between `config`, `keybinds`, and
//! `theme` loaders, plus the generic "load a config file or default" helper
//! those loaders all use.

use std::fs;
use std::path::{Path, PathBuf};

/// Resolve the XDG base config directory. Per the XDG Base Directory Spec,
/// `XDG_CONFIG_HOME` MUST be an absolute path; relative values are ignored.
/// We warn once so users debugging path resolution can see why their relative
/// override didn't take effect.
#[must_use]
pub fn config_home() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("XDG_CONFIG_HOME")
        && !v.is_empty()
    {
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return Some(p);
        }
        eprintln!(
            "tuxtime: ignoring non-absolute XDG_CONFIG_HOME={:?} (per XDG spec)",
            p.display()
        );
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config"))
}

/// Join `rel` under an explicit XDG-style base directory (no env lookup).
/// The test-facing sibling of [`tuxtime_path`]: loaders expose a `path_in`
/// variant so tests can point at a temp dir without mutating process env.
#[must_use]
pub fn tuxtime_path_in(xdg_base: &Path, rel: &str) -> PathBuf {
    xdg_base.join("tuxtime").join(rel)
}

/// Resolve a path under `${XDG_CONFIG_HOME:-$HOME/.config}/tuxtime`. Returns
/// `None` only when neither `XDG_CONFIG_HOME` nor HOME is set. Used by every
/// tuxtime config file (`config.toml`, `keybinds.toml`, `themes/`).
#[must_use]
pub fn tuxtime_path(rel: &str) -> Option<PathBuf> {
    Some(tuxtime_path_in(&config_home()?, rel))
}

/// Read `path` and parse it; any read failure (missing file, permission,
/// non-UTF-8) falls back to `T::default()`. This is the load-from-disk half
/// shared by every config loader — a missing config is indistinguishable
/// from defaults by design.
#[must_use]
pub fn load_config_file<T: Default>(path: &Path, parse: fn(&str) -> T) -> T {
    match fs::read_to_string(path) {
        Ok(s) => parse(&s),
        Err(_) => T::default(),
    }
}
