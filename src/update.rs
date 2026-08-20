//! Update-availability check and `tuxtime update` subcommand.
//!
//! The update command previews the operation, asks for confirmation by default,
//! and supports `--yes` for explicit automation and `--dry-run` for inspection.
//! Package-manager installs delegate to their owning package manager. Downloaded
//! binaries use checksum-verified release assets and an atomic replacement.
//!
//! The background checker invoked at TUI startup consults a cache under
//! `$XDG_CACHE_HOME/tuxtime/latest_version.json`. If the cache is missing or
//! older than 24h, it shells out to `curl` to read the latest GitHub release.
//! All checker failures are silent — a stale or missing cache simply means no
//! status-bar hint.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How tuxtime appears to have been installed, judged by the path of the
/// currently-running executable. Used to recommend the right upgrade command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    Homebrew,
    Cargo,
    Binary,
    Unknown,
}

/// Options for the update command.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateOptions {
    /// Apply the update without reading confirmation from stdin.
    pub yes: bool,
    /// Show the operation without downloading or changing anything.
    pub dry_run: bool,
}

/// Parse arguments after the `update` subcommand.
pub fn parse_options(args: &[String]) -> Result<UpdateOptions, String> {
    let mut options = UpdateOptions::default();
    for arg in args {
        match arg.as_str() {
            "--yes" | "-y" => options.yes = true,
            "--dry-run" => options.dry_run = true,
            other => return Err(format!("unknown update option: {other}")),
        }
    }
    Ok(options)
}

/// Run `tuxtime update`.
///
/// The default mode is deliberately interactive: package managers are invoked
/// only after the user confirms, and a downloaded binary is never replaced
/// until its release checksum and `--version` check succeed. Use `--yes` for
/// automation and `--dry-run` to inspect the operation without side effects.
pub fn run(args: &[String]) -> io::Result<()> {
    let options = parse_options(args).map_err(invalid_input)?;
    let exe = std::env::current_exe().ok();
    let kind = exe.as_deref().map_or(InstallKind::Unknown, detect_kind);
    let current = env!("CARGO_PKG_VERSION");

    println!("tuxtime {current}");
    if let Some(path) = &exe {
        println!("installed at: {}", path.display());
    }
    println!("install method: {}", install_kind_name(kind));
    println!();

    if options.dry_run {
        println!("Dry run — no changes will be made.");
        print_plan(kind, exe.as_deref());
        return Ok(());
    }

    print_plan(kind, exe.as_deref());
    validate_update_target(kind)?;
    if !options.yes && !confirm("\nUpdate now? [y/N] ") {
        println!("Update cancelled.");
        return Ok(());
    }

    match kind {
        InstallKind::Homebrew => run_homebrew_update(),
        InstallKind::Cargo => run_external_command(
            "cargo",
            &[
                "install",
                "--git",
                "https://github.com/axzchin/tuxtime",
                "--force",
            ],
            "cargo install --git https://github.com/axzchin/tuxtime --force",
        ),
        InstallKind::Binary => update_binary(exe.as_deref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "running executable path is unavailable",
            )
        })?),
        InstallKind::Unknown => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "could not detect the install method; use Homebrew, Cargo, or the release page",
        )),
    }
}

fn invalid_input(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn validate_update_target(kind: InstallKind) -> io::Result<()> {
    match kind {
        InstallKind::Unknown => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "could not detect the install method; use Homebrew, Cargo, or the release page",
        )),
        InstallKind::Binary
            if release_target(std::env::consts::OS, std::env::consts::ARCH).is_none() =>
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "no self-update artifact for {}/{}; use the release page",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
            ))
        }
        InstallKind::Homebrew | InstallKind::Cargo | InstallKind::Binary => Ok(()),
    }
}

fn confirm(prompt: &str) -> bool {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    match io::stdin().read_line(&mut answer) {
        Ok(n) if n > 0 => matches!(answer.trim(), "y" | "Y" | "yes" | "YES"),
        _ => false,
    }
}

fn install_kind_name(kind: InstallKind) -> &'static str {
    match kind {
        InstallKind::Homebrew => "Homebrew",
        InstallKind::Cargo => "Cargo",
        InstallKind::Binary => "downloaded binary",
        InstallKind::Unknown => "unknown",
    }
}

fn print_plan(kind: InstallKind, exe: Option<&Path>) {
    match kind {
        InstallKind::Homebrew => {
            println!("Will run: brew update && brew upgrade axzchin/tap/tuxtime");
        }
        InstallKind::Cargo => {
            println!("Will run: cargo install --git https://github.com/axzchin/tuxtime --force");
        }
        InstallKind::Binary => {
            let target = release_target(std::env::consts::OS, std::env::consts::ARCH)
                .unwrap_or("unsupported platform");
            println!("Will download and checksum-verify the latest {target} release");
            if let Some(path) = exe {
                println!("and atomically replace: {}", path.display());
            }
        }
        InstallKind::Unknown => {
            println!("Could not detect the install method. Choose one of:");
            println!("  brew upgrade axzchin/tap/tuxtime");
            println!("  cargo install --git https://github.com/axzchin/tuxtime --force");
            println!("  https://github.com/axzchin/tuxtime/releases/latest");
        }
    }
}

fn run_homebrew_update() -> io::Result<()> {
    println!("Running: brew update && brew upgrade axzchin/tap/tuxtime");
    let update = run_command("brew", &["update"])?;
    if !update.success() {
        return Err(io::Error::other(format!(
            "brew update failed with {update}"
        )));
    }
    let upgrade = run_command("brew", &["upgrade", "axzchin/tap/tuxtime"])?;
    if !upgrade.success() {
        return Err(io::Error::other(format!(
            "brew upgrade failed with {upgrade}"
        )));
    }
    println!("Update complete.");
    Ok(())
}

fn run_external_command(program: &str, args: &[&str], display: &str) -> io::Result<()> {
    println!("Running: {display}");
    let status = run_command(program, args)?;
    if status.success() {
        println!("Update complete.");
        Ok(())
    } else {
        Err(io::Error::other(format!("{display} failed with {status}")))
    }
}

fn run_command(program: &str, args: &[&str]) -> io::Result<ExitStatus> {
    Command::new(program).args(args).status().map_err(|e| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("failed to run {program}: {e}"),
        )
    })
}

const DOWNLOAD_TIMEOUT_SECS: u64 = 30;
const RELEASE_DOWNLOAD_BASE: &str = "https://github.com/axzchin/tuxtime/releases/download";

/// Download, verify, validate, and atomically install the latest binary.
fn update_binary(exe: &Path) -> io::Result<()> {
    let target = release_target(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "no self-update artifact for {}/{}; use the release page",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        )
    })?;
    let tag = fetch_latest_body()
        .and_then(|body| parse_tag_from_release_json(&body))
        .filter(|tag| valid_release_tag(tag))
        .ok_or_else(|| io::Error::other("could not determine the latest release tag"))?;
    if !is_newer(&tag, env!("CARGO_PKG_VERSION")) {
        println!("Already up to date ({tag}).");
        return Ok(());
    }
    let asset = format!("tuxtime-{tag}-{target}.tar.gz");
    let base = format!("{RELEASE_DOWNLOAD_BASE}/{tag}/{asset}");
    let temp = TempDir::new("tuxtime-update")?;
    let archive = temp.path().join(&asset);
    let checksum = temp.path().join(format!("{asset}.sha256"));

    println!("Downloading {tag} for {target}...");
    download_to(&base, &archive)?;
    download_to(&format!("{base}.sha256"), &checksum)?;

    let checksum_text = fs::read_to_string(&checksum)?;
    let expected = first_token(&checksum_text)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "checksum file is empty"))?;
    let actual = sha256_file(&archive)?;
    if expected != actual {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("checksum mismatch: expected {expected}, got {actual}"),
        ));
    }

    let status = Command::new("tar")
        .args(["xzf"])
        .arg(&archive)
        .arg("-C")
        .arg(temp.path())
        .status()
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, format!("failed to run tar: {e}")))?;
    if !status.success() {
        return Err(io::Error::other(format!(
            "tar extraction failed with {status}"
        )));
    }

    let downloaded = temp
        .path()
        .join(format!("tuxtime-{tag}-{target}"))
        .join("tuxtime");
    if !downloaded.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release archive did not contain tuxtime",
        ));
    }
    let version_status = Command::new(&downloaded)
        .arg("--version")
        .status()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("downloaded binary failed to run: {e}"),
            )
        })?;
    if !version_status.success() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "downloaded binary failed its --version check",
        ));
    }

    let parent = exe.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "running executable has no parent directory",
        )
    })?;
    let staged = parent.join(format!(".tuxtime-update-{}", std::process::id()));
    let _ = fs::remove_file(&staged);
    fs::copy(&downloaded, &staged)?;
    set_executable(&staged)?;
    let staged_status = Command::new(&staged)
        .arg("--version")
        .status()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("staged binary failed: {e}"),
            )
        })?;
    if !staged_status.success() {
        let _ = fs::remove_file(&staged);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "staged binary failed its --version check",
        ));
    }
    fs::rename(&staged, exe).map_err(|e| {
        let _ = fs::remove_file(&staged);
        io::Error::new(
            e.kind(),
            format!("could not replace {}: {e}", exe.display()),
        )
    })?;
    println!("Updated tuxtime to {tag}.");
    Ok(())
}

fn release_target(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Some("aarch64-unknown-linux-gnu"),
        ("macos", "x86_64") => Some("x86_64-apple-darwin"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        _ => None,
    }
}

fn valid_release_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.starts_with('v')
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn download_to(url: &str, destination: &Path) -> io::Result<()> {
    let status = Command::new("curl")
        .args(["-fsSL", "-m", &DOWNLOAD_TIMEOUT_SECS.to_string(), "-o"])
        .arg(destination)
        .arg(url)
        .status()
        .map_err(|e| io::Error::new(io::ErrorKind::NotFound, format!("failed to run curl: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("download failed with {status}")))
    }
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let output = match Command::new("sha256sum").arg(path).output() {
        Ok(output) if output.status.success() => output,
        Ok(_) | Err(_) => Command::new("shasum")
            .args(["-a", "256"])
            .arg(path)
            .output()
            .map_err(|e| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no SHA-256 tool found: {e}"),
                )
            })?,
    };
    if !output.status.success() {
        return Err(io::Error::other("SHA-256 calculation failed"));
    }
    String::from_utf8(output.stdout)
        .ok()
        .and_then(|line| first_token(&line).map(str::to_owned))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid SHA-256 output"))
}

fn first_token(text: &str) -> Option<&str> {
    text.split_whitespace().next()
}

#[cfg(unix)]
fn set_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> io::Result<Self> {
        let path =
            std::env::temp_dir().join(format!("{prefix}-{}-{}", std::process::id(), now_epoch()));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Classify an executable path into an [`InstallKind`]. Exposed for tests.
#[must_use]
pub fn detect_kind(exe: &Path) -> InstallKind {
    let s = exe.to_string_lossy();
    if s.contains("/Cellar/")
        || s.starts_with("/opt/homebrew/")
        || s.starts_with("/usr/local/Homebrew/")
        || s.contains("/homebrew/Cellar/")
        || s.contains("/linuxbrew/")
    {
        return InstallKind::Homebrew;
    }
    if s.contains("/.cargo/bin/") || s.contains("\\.cargo\\bin\\") {
        return InstallKind::Cargo;
    }
    // A bare /usr/local/bin/tuxtime could be either a Homebrew shim (older
    // macOS) or a manual download. Without more signal, treat it as a binary.
    if !s.is_empty() {
        return InstallKind::Binary;
    }
    InstallKind::Unknown
}

/// The update guidance for a downloaded-binary install. `os` is injected
/// rather than read directly so this decision remains unit-testable.
#[must_use]
pub fn binary_update_instructions(os: &str) -> &'static str {
    if matches!(os, "linux" | "macos") {
        "tuxtime update --yes"
    } else {
        "https://github.com/axzchin/tuxtime/releases/latest"
    }
}

/// Spawn the background update check. Returns a receiver that yields exactly
/// one message — `Some(tag)` if a cached or freshly-fetched tag is available,
/// otherwise `None`. The receiver is dropped when the thread exits, so a
/// disconnect on `try_recv` means "give up, nothing's coming".
#[must_use]
pub fn spawn_check() -> Receiver<Option<String>> {
    let (tx, rx) = mpsc::sync_channel::<Option<String>>(1);
    thread::spawn(move || {
        let result = check_for_update();
        let _ = tx.send(result);
    });
    rx
}

const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// How long to honor a cached *failure* (empty `tag`) before trying the
/// network again. Short enough to recover within an hour, long enough to
/// stop hammering GitHub once anonymous API calls have been rate-limited
/// (which is what burned us once during testing).
const NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const CURL_TIMEOUT_SECS: u64 = 5;
const RELEASE_URL: &str = "https://api.github.com/repos/axzchin/tuxtime/releases/latest";

fn check_for_update() -> Option<String> {
    let cache_path = cache_path();
    let now = now_epoch();
    // Honor the cache for both success and failure. A non-empty tag within
    // CACHE_TTL is a success entry; an empty tag within NEGATIVE_CACHE_TTL
    // is a "we just tried and it failed, don't pummel GitHub again" marker.
    if let Some(p) = &cache_path
        && let Some((ts, tag)) = read_cache(p)
    {
        let age = now.saturating_sub(ts);
        if !tag.is_empty() && age < CACHE_TTL.as_secs() {
            return Some(tag);
        }
        if tag.is_empty() && age < NEGATIVE_CACHE_TTL.as_secs() {
            return None;
        }
    }
    // Cache is stale, missing, or expired-negative — try the network. Cache
    // either outcome so we don't retry on every launch when offline or
    // rate-limited.
    let tag = fetch_latest_body().and_then(|b| parse_tag_from_release_json(&b));
    if let Some(p) = &cache_path {
        let _ = write_cache(p, now, tag.as_deref().unwrap_or(""));
    }
    tag
}

fn fetch_latest_body() -> Option<String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "-m",
            &CURL_TIMEOUT_SECS.to_string(),
            "-H",
            "Accept: application/vnd.github+json",
            "-A",
            concat!("tuxtime/", env!("CARGO_PKG_VERSION")),
            RELEASE_URL,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// Pull the `tag_name` value out of a GitHub release JSON payload. Doesn't
/// pull in a JSON parser — release payloads are well-formed enough that a
/// targeted string scan suffices, and a malformed payload simply returns
/// `None`. Exposed for unit testing.
#[must_use]
pub fn parse_tag_from_release_json(body: &str) -> Option<String> {
    let key = "\"tag_name\"";
    let i = body.find(key)?;
    let rest = &body[i + key.len()..];
    let colon = rest.find(':')?;
    let after_colon = &rest[colon + 1..];
    let q = after_colon.find('"')?;
    let after_q = &after_colon[q + 1..];
    let end = after_q.find('"')?;
    let tag = &after_q[..end];
    if tag.is_empty() {
        return None;
    }
    Some(tag.to_string())
}

/// True when `latest` is a strictly newer version than `current`, comparing
/// each dot-separated segment numerically. A leading `v` (e.g. `v2026.5.5`)
/// is stripped on both sides. Non-numeric segments fall back to lexicographic
/// comparison of that segment, so a future suffix like `2026.5.5-rc1` won't
/// crash — it just compares the strings.
#[must_use]
pub fn is_newer(latest: &str, current: &str) -> bool {
    let l = latest.trim_start_matches('v');
    let c = current.trim_start_matches('v');
    let mut li = l.split('.');
    let mut ci = c.split('.');
    loop {
        match (li.next(), ci.next()) {
            (None, None) => return false,
            (Some(a), None) => return a.parse::<u64>().is_ok_and(|n| n > 0) || !a.is_empty(),
            (None, Some(_)) => return false,
            (Some(a), Some(b)) => match (a.parse::<u64>(), b.parse::<u64>()) {
                (Ok(x), Ok(y)) if x != y => return x > y,
                (Ok(_), Ok(_)) => {}
                _ => match a.cmp(b) {
                    std::cmp::Ordering::Greater => return true,
                    std::cmp::Ordering::Less => return false,
                    std::cmp::Ordering::Equal => {}
                },
            },
        }
    }
}

fn cache_path() -> Option<PathBuf> {
    let base = xdg_cache_home()?;
    Some(base.join("tuxtime").join("latest_version.json"))
}

fn xdg_cache_home() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("XDG_CACHE_HOME")
        && !v.is_empty()
    {
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            return Some(p);
        }
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".cache"))
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read a cache file. Returns `(epoch_secs, tag)` on success. Format is a
/// tiny JSON-ish payload: `{"checked_at": <int>, "tag": "<str>"}`. An empty
/// `tag` is a sentinel for a cached failure (see [`NEGATIVE_CACHE_TTL`]);
/// callers distinguish success from failure on the returned string.
fn read_cache(path: &Path) -> Option<(u64, String)> {
    let body = std::fs::read_to_string(path).ok()?;
    let ts = scan_int(&body, "\"checked_at\"")?;
    let tag = scan_str(&body, "\"tag\"").unwrap_or_default();
    Some((ts, tag))
}

fn write_cache(path: &Path, now: u64, tag: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = format!("{{\"checked_at\": {now}, \"tag\": \"{tag}\"}}\n");
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn scan_int(body: &str, key: &str) -> Option<u64> {
    let i = body.find(key)?;
    let rest = &body[i + key.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    after[..end].parse().ok()
}

fn scan_str(body: &str, key: &str) -> Option<String> {
    let i = body.find(key)?;
    let rest = &body[i + key.len()..];
    let colon = rest.find(':')?;
    let after_colon = &rest[colon + 1..];
    let q = after_colon.find('"')?;
    let after_q = &after_colon[q + 1..];
    let end = after_q.find('"')?;
    Some(after_q[..end].to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detect_kind_homebrew_paths() {
        assert_eq!(
            detect_kind(&PathBuf::from("/opt/homebrew/bin/tuxtime")),
            InstallKind::Homebrew
        );
        assert_eq!(
            detect_kind(&PathBuf::from(
                "/opt/homebrew/Cellar/tuxtime/2026.5.3/bin/tuxtime"
            )),
            InstallKind::Homebrew
        );
        assert_eq!(
            detect_kind(&PathBuf::from(
                "/usr/local/Cellar/tuxtime/2026.5.3/bin/tuxtime"
            )),
            InstallKind::Homebrew
        );
        assert_eq!(
            detect_kind(&PathBuf::from(
                "/home/linuxbrew/.linuxbrew/Cellar/tuxtime/2026.5.3/bin/tuxtime"
            )),
            InstallKind::Homebrew
        );
    }

    #[test]
    fn detect_kind_cargo_path() {
        assert_eq!(
            detect_kind(&PathBuf::from("/home/m/.cargo/bin/tuxtime")),
            InstallKind::Cargo
        );
    }

    #[test]
    fn detect_kind_falls_back_to_binary() {
        assert_eq!(
            detect_kind(&PathBuf::from("/usr/local/bin/tuxtime")),
            InstallKind::Binary
        );
        assert_eq!(
            detect_kind(&PathBuf::from("/tmp/tuxtime")),
            InstallKind::Binary
        );
    }

    #[test]
    fn binary_update_instructions_points_supported_oses_at_self_update() {
        for os in ["linux", "macos"] {
            assert_eq!(binary_update_instructions(os), "tuxtime update --yes");
        }
    }

    #[test]
    fn binary_update_instructions_points_windows_at_release_page() {
        for os in ["windows", "freebsd"] {
            let hint = binary_update_instructions(os);
            assert!(hint.contains("releases/latest"), "{os}: {hint}");
        }
    }

    #[test]
    fn parse_options_supports_automation_and_preview() {
        let args = vec!["--yes".to_string(), "--dry-run".to_string()];
        assert_eq!(
            parse_options(&args).unwrap(),
            UpdateOptions {
                yes: true,
                dry_run: true
            }
        );
        assert!(parse_options(&["--nope".to_string()]).is_err());
    }

    #[test]
    fn release_target_matches_release_workflow() {
        assert_eq!(
            release_target("linux", "x86_64"),
            Some("x86_64-unknown-linux-gnu")
        );
        assert_eq!(
            release_target("linux", "aarch64"),
            Some("aarch64-unknown-linux-gnu")
        );
        assert_eq!(
            release_target("macos", "aarch64"),
            Some("aarch64-apple-darwin")
        );
        assert_eq!(release_target("windows", "x86_64"), None);
    }

    #[test]
    fn release_tags_are_path_safe() {
        assert!(valid_release_tag("v2026.7.2"));
        assert!(valid_release_tag("v2026.7.2-rc1"));
        assert!(!valid_release_tag("2026.7.2"));
        assert!(!valid_release_tag("v../../evil"));
    }

    #[test]
    fn is_newer_handles_calver_segments() {
        // Same version
        assert!(!is_newer("2026.5.3", "2026.5.3"));
        // Patch bump
        assert!(is_newer("2026.5.4", "2026.5.3"));
        assert!(!is_newer("2026.5.3", "2026.5.4"));
        // Crucially: numeric (not lex) compare on patches >= 10
        assert!(is_newer("2026.5.10", "2026.5.9"));
        assert!(!is_newer("2026.5.9", "2026.5.10"));
        // Month rollover
        assert!(is_newer("2026.10.1", "2026.9.5"));
        // Year rollover
        assert!(is_newer("2027.1.1", "2026.12.31"));
    }

    #[test]
    fn is_newer_strips_v_prefix() {
        assert!(is_newer("v2026.5.4", "2026.5.3"));
        assert!(is_newer("2026.5.4", "v2026.5.3"));
        assert!(!is_newer("v2026.5.3", "v2026.5.3"));
    }

    #[test]
    fn is_newer_handles_segment_count_mismatch() {
        // "2026.5" vs "2026.5.0" — equal in spirit, but with a non-zero suffix
        // the longer one is newer.
        assert!(is_newer("2026.5.1", "2026.5"));
        assert!(!is_newer("2026.5", "2026.5.1"));
    }

    #[test]
    fn parse_tag_extracts_first_tag_name() {
        let body = r#"{"url":"x","tag_name":"v2026.5.5","name":"2026.5.5"}"#;
        assert_eq!(
            parse_tag_from_release_json(body).as_deref(),
            Some("v2026.5.5")
        );
    }

    #[test]
    fn parse_tag_with_whitespace_and_extra_keys() {
        let body = r#"
        {
          "url": "x",
          "tag_name" : "2026.5.10" ,
          "draft": false
        }
        "#;
        assert_eq!(
            parse_tag_from_release_json(body).as_deref(),
            Some("2026.5.10")
        );
    }

    #[test]
    fn parse_tag_returns_none_on_missing_field() {
        let body = r#"{"name":"hi"}"#;
        assert!(parse_tag_from_release_json(body).is_none());
    }

    #[test]
    fn cache_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "tuxtime-update-cache-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("latest_version.json");
        write_cache(&path, 1_700_000_000, "2026.5.5").unwrap();
        let (ts, tag) = read_cache(&path).unwrap();
        assert_eq!(ts, 1_700_000_000);
        assert_eq!(tag, "2026.5.5");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_cache_none_on_missing_file() {
        assert!(read_cache(&PathBuf::from("/tmp/does-not-exist-xyzzy")).is_none());
    }

    #[test]
    fn cache_round_trip_empty_tag_is_negative_marker() {
        let dir = std::env::temp_dir().join(format!(
            "tuxtime-update-neg-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("latest_version.json");
        // A failed check writes an empty tag.
        write_cache(&path, 1_700_000_000, "").unwrap();
        let (ts, tag) = read_cache(&path).unwrap();
        assert_eq!(ts, 1_700_000_000);
        assert_eq!(tag, "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
