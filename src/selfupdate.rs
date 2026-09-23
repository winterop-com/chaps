//! `chaps` updating `chaps`: which release asset this build wants, how to
//! verify it, how to put it in place, and the once-a-day notice that a newer
//! one exists.
//!
//! [`crate::chapcore`] is the equivalent for chap-core's images; the version
//! comparison and the SHA-256 implementation are shared with it rather than
//! written twice. Everything here is pure except [`latest_release`],
//! [`release_by_tag`] and [`download`], so the interesting parts are testable
//! without a network.

use crate::chapcore::sha256_hex;
use crate::error::{ChapError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The repository releases are published from.
pub const REPO: &str = "winterop-com/chaps";

/// Release endpoint for the newest final release.
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/winterop-com/chaps/releases/latest";

/// The triple this binary was built for, from the build script.
pub const TARGET: &str = env!("TARGET");

/// The short commit of the checkout this binary was built from, empty when
/// there was no git repository to ask.
pub const GIT_REVISION: &str = env!("GIT_REVISION");

/// The version this binary reports, without a leading `v`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `User-Agent` sent with every request, matching the registry fetch.
const USER_AGENT: &str = concat!("chaps-cli/", env!("CARGO_PKG_VERSION"));

/// The file inside the cache directory holding the last update check.
pub const CHECK_FILE: &str = "self-update-check.json";

/// How long an update check is considered fresh.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Timeout for the background check, short enough that a slow network costs a
/// command nothing worth noticing.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// Set to `1` to turn the once-a-day update notice off entirely.
pub const NO_CHECK_ENV: &str = "CHAPS_NO_UPDATE_CHECK";

/// Largest archive `self update` will read into memory.
const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

/// The name of the checksum manifest attached to every release.
pub const SUMS_FILE: &str = "SHA256SUMS";

// --------------------------------------------------------------------------
// Assets
// --------------------------------------------------------------------------

/// The target whose archive `target` should install.
///
/// macOS is the one place where a build does not take its own triple: the
/// universal archive carries both slices, it is the one that is signed and
/// notarized the same way, and it is what the install script and the download
/// table point at, so a single-architecture Mac build still updates to it.
pub fn asset_target(target: &str) -> &str {
    if target.ends_with("-apple-darwin") {
        "universal-apple-darwin"
    } else {
        target
    }
}

/// `tar.gz` everywhere but Windows, which gets `zip`.
pub fn archive_extension(target: &str) -> &str {
    if target.contains("-windows-") {
        "zip"
    } else {
        "tar.gz"
    }
}

/// The release asset `target` wants at `tag`, e.g.
/// `chaps-v0.2.0-universal-apple-darwin.tar.gz`.
pub fn asset_name(tag: &str, target: &str) -> String {
    let target = asset_target(target);
    format!("chaps-{tag}-{target}.{}", archive_extension(target))
}

/// The version-less copy of the same archive, which every release also
/// carries so a download link can be written once and keep working.
pub fn stable_asset_name(target: &str) -> String {
    let target = asset_target(target);
    format!("chaps-{target}.{}", archive_extension(target))
}

/// The name of the executable inside the archive.
pub fn binary_name(target: &str) -> &str {
    if target.contains("-windows-") {
        "chaps.exe"
    } else {
        "chaps"
    }
}

/// Where a release's assets are served from, without the file name.
pub fn download_base(tag: &str) -> String {
    format!("https://github.com/{REPO}/releases/download/{tag}")
}

// --------------------------------------------------------------------------
// Releases
// --------------------------------------------------------------------------

/// A release, trimmed to what an update needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag, e.g. `v0.2.0`.
    pub tag: String,
    /// Asset names attached to the release.
    pub assets: Vec<String>,
}

impl Release {
    /// Whether the release carries `name`.
    pub fn has_asset(&self, name: &str) -> bool {
        self.assets.iter().any(|asset| asset == name)
    }
}

/// Parse a GitHub release payload into [`Release`].
pub fn parse_release(body: &str) -> Result<Release> {
    #[derive(Debug, Default, Deserialize)]
    struct Payload {
        #[serde(default)]
        tag_name: String,
        #[serde(default)]
        assets: Vec<Asset>,
    }
    #[derive(Debug, Deserialize)]
    struct Asset {
        #[serde(default)]
        name: String,
    }

    let payload: Payload =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("invalid release JSON: {e}"))?;
    if payload.tag_name.trim().is_empty() {
        return Err(anyhow::anyhow!("the release carries no tag_name"));
    }
    Ok(Release {
        tag: payload.tag_name.trim().to_string(),
        assets: payload
            .assets
            .into_iter()
            .map(|asset| asset.name)
            .filter(|name| !name.is_empty())
            .collect(),
    })
}

/// The asset `target` should download from `release`.
///
/// The name carrying the tag is what a release is expected to hold; the
/// version-less copy, which every release also publishes so the download links
/// in the README can be written once and keep working, is the fallback. A
/// payload that lists no assets at all is taken at its word rather than
/// refused, because the name is derivable without it.
pub fn pick_asset(release: &Release, target: &str) -> Result<String> {
    let versioned = asset_name(&release.tag, target);
    if release.assets.is_empty() || release.has_asset(&versioned) {
        return Ok(versioned);
    }
    let stable = stable_asset_name(target);
    if release.has_asset(&stable) {
        return Ok(stable);
    }
    Err(anyhow::anyhow!(
        "{} has no archive for {target}; it carries {}",
        release.tag,
        release.assets.join(", ")
    ))
}

/// The release document at `url`, parsed.
fn release_at(url: &str, timeout: Duration) -> Result<Release> {
    let body = get_text(url, timeout)?;
    parse_release(&body).map_err(|e| anyhow::anyhow!("reading the release at {url}: {e}"))
}

/// The newest final release of this repository.
pub fn latest_release(timeout: Duration) -> Result<Release> {
    release_at(LATEST_RELEASE_URL, timeout)
}

/// One release by tag, for `--version`.
pub fn release_by_tag(tag: &str, timeout: Duration) -> Result<Release> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/tags/{tag}");
    release_at(&url, timeout).map_err(|e| match e.downcast_ref::<ChapError>() {
        Some(ChapError::Http { status: 404, .. }) => {
            anyhow::anyhow!("no release tagged `{tag}` in {REPO}")
        }
        _ => e,
    })
}

/// Whether `tag` is a newer release than this build.
///
/// `CARGO_PKG_VERSION` has no `v`, release tags do; [`crate::chapcore`] strips
/// it either way, so the two compare as versions rather than as strings.
pub fn is_newer_than_current(tag: &str) -> bool {
    crate::chapcore::is_newer(tag, VERSION)
}

/// Whether `path` looks like `cargo install` put it there.
pub fn install_method(path: &Path) -> &'static str {
    let parts: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let from_cargo = parts
        .windows(2)
        .any(|pair| pair[0] == ".cargo" && pair[1] == "bin");
    if from_cargo {
        "cargo install"
    } else {
        "release archive"
    }
}

// --------------------------------------------------------------------------
// Checksums
// --------------------------------------------------------------------------

/// The recorded digest of `name` in a `SHA256SUMS` body.
///
/// Both coreutils spellings are accepted: `<hex>  <name>` and the binary-mode
/// `<hex> *<name>`, with or without a `./` prefix on the name, which is what
/// `sha256sum` and `shasum -a 256` each write.
pub fn sha256_for(sums: &str, name: &str) -> Option<String> {
    for line in sums.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(2, char::is_whitespace);
        let digest = fields.next()?.trim();
        let listed = fields.next()?.trim().trim_start_matches('*');
        let listed = listed.trim_start_matches("./");
        if listed == name && digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(digest.to_ascii_lowercase());
        }
    }
    None
}

/// Check `bytes` against the digest `sums` records for `name`.
pub fn verify(sums: &str, name: &str, bytes: &[u8]) -> Result<()> {
    let expected = sha256_for(sums, name)
        .ok_or_else(|| anyhow::anyhow!("{SUMS_FILE} does not list {name}"))?;
    let actual = sha256_hex(bytes);
    if actual != expected {
        return Err(anyhow::anyhow!(
            "{name} does not match {SUMS_FILE}: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

// --------------------------------------------------------------------------
// Replacing the running binary
// --------------------------------------------------------------------------

/// Where the new binary is staged, beside the one it replaces.
///
/// A sibling rather than a temporary directory, because a rename is only
/// atomic within one filesystem and `/tmp` is routinely a different one.
pub fn staged_path(current: &Path) -> PathBuf {
    sibling(current, "new")
}

/// Where the outgoing binary is parked on Windows, which cannot rename over a
/// running executable but can rename the running executable itself.
pub fn backup_path(current: &Path) -> PathBuf {
    sibling(current, "old")
}

fn sibling(current: &Path, extension: &str) -> PathBuf {
    let stem = current
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "chaps".to_string());
    current.with_file_name(format!("{stem}.{extension}"))
}

/// Delete a leftover `chaps.old` from a previous Windows update.
///
/// Best effort and silent: the file is only still there when the previous
/// process had not exited yet, and the next run will get it. Compiled for
/// Windows alone, because nothing else ever writes a `chaps.old` and a file
/// of that name beside a Unix install belongs to whoever put it there.
#[cfg(any(windows, test))]
pub fn clean_backup(current: &Path) {
    let backup = backup_path(current);
    if backup.exists() {
        let _ = std::fs::remove_file(&backup);
    }
}

/// Whether the binary at `path` can be replaced in place.
///
/// The directory has to be writable, since the swap is a rename inside it,
/// not a write through the existing file.
pub fn is_replaceable(path: &Path) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let probe = dir.join(format!(".chaps-write-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Put `contents` at `current`, atomically as far as the platform allows.
///
/// Unix renames the staged file over the running one, which the kernel allows
/// and which leaves no window where `current` does not exist. Windows cannot
/// rename over a running image, so the running one is moved aside first and
/// the move is undone if the second rename fails.
pub fn replace_executable(current: &Path, contents: &[u8]) -> Result<()> {
    let staged = staged_path(current);
    std::fs::write(&staged, contents)
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", staged.display()))?;
    copy_mode(current, &staged);

    if cfg!(windows) {
        let backup = backup_path(current);
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(current, &backup).map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            anyhow::anyhow!("moving {} aside: {e}", current.display())
        })?;
        if let Err(e) = std::fs::rename(&staged, current) {
            // Put the old binary back rather than leave the path empty.
            let _ = std::fs::rename(&backup, current);
            let _ = std::fs::remove_file(&staged);
            return Err(anyhow::anyhow!("replacing {}: {e}", current.display()));
        }
        return Ok(());
    }

    std::fs::rename(&staged, current).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        anyhow::anyhow!("replacing {}: {e}", current.display())
    })
}

/// Give the staged file the mode the binary it replaces has, and make sure it
/// is executable whatever that mode was.
#[cfg(unix)]
fn copy_mode(current: &Path, staged: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(current)
        .map(|meta| meta.permissions().mode() & 0o7777)
        .unwrap_or(0o755);
    let _ = std::fs::set_permissions(staged, std::fs::Permissions::from_mode(mode | 0o700));
}

/// Windows has no mode bits to carry over.
#[cfg(not(unix))]
fn copy_mode(_current: &Path, _staged: &Path) {}

/// Unpack `archive` into `dir` with the system `tar`.
///
/// `tar -xf` reads both formats this project ships: gzipped tar everywhere,
/// and zip on Windows, where `tar.exe` has been bsdtar since Windows 10 1803
/// and handles a zip without a second tool.
pub fn extract(archive: &Path, dir: &Path) -> Result<()> {
    let status = std::process::Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dir)
        .status()
        .map_err(|e| anyhow::anyhow!("running tar to unpack {}: {e}", archive.display()))?;
    if !status.success() {
        return Err(anyhow::anyhow!(
            "tar could not unpack {} (exit {})",
            archive.display(),
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

/// Find the `chaps` executable somewhere under `dir`.
///
/// The archives put it in a `chaps-<tag>-<target>/` directory, but the search
/// does not depend on that: it walks what was unpacked and takes the first
/// file with the right name.
pub fn find_binary(dir: &Path, target: &str) -> Option<PathBuf> {
    let wanted = binary_name(target);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|name| name == wanted) {
                return Some(path);
            }
        }
    }
    None
}

// --------------------------------------------------------------------------
// The once-a-day check
// --------------------------------------------------------------------------

/// `self-update-check.json`: when the release feed was last asked, and what it
/// said.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckState {
    /// Seconds since the Unix epoch at the time of the check.
    pub checked_at_unix: u64,
    /// The tag the feed reported, empty when the check failed.
    #[serde(default)]
    pub latest: String,
}

/// Where the check state is kept.
pub fn check_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(CHECK_FILE)
}

/// Read the check state, or `None` when there is none to read.
///
/// A missing, truncated or hand-edited file is "no state", never an error:
/// the notice is a courtesy and must not be able to fail a command.
pub fn read_check(cache_dir: &Path) -> Option<CheckState> {
    let body = std::fs::read_to_string(check_path(cache_dir)).ok()?;
    serde_json::from_str(&body).ok()
}

/// Record a check. Failures are swallowed for the same reason.
pub fn write_check(cache_dir: &Path, state: &CheckState) {
    if std::fs::create_dir_all(cache_dir).is_err() {
        return;
    }
    if let Ok(body) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(check_path(cache_dir), format!("{body}\n"));
    }
}

/// Seconds since the Unix epoch, or 0 on a clock that predates it.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Whether the release feed should be asked again.
///
/// No state at all means yes; a timestamp in the future (a clock that moved
/// backwards since, or a copied cache) also means yes rather than never
/// again.
pub fn is_stale(state: Option<&CheckState>, now: u64, interval: Duration) -> bool {
    match state {
        None => true,
        Some(state) => {
            let age = now.saturating_sub(state.checked_at_unix);
            state.checked_at_unix > now || age >= interval.as_secs()
        }
    }
}

/// Whether the environment asked for no update checks at all.
pub fn checks_disabled() -> bool {
    disabled_by(std::env::var(NO_CHECK_ENV).ok().as_deref())
}

/// [`checks_disabled`] for an explicit value.
///
/// Exactly `1` turns the notice off. Unset, empty and anything else leave it
/// on, so `CHAPS_NO_UPDATE_CHECK=0` reads the way it looks.
fn disabled_by(value: Option<&str>) -> bool {
    value == Some("1")
}

/// The one line the notice prints, or `None` when there is nothing to say.
pub fn notice_line(latest: &str, current: &str) -> Option<String> {
    crate::chapcore::is_newer(latest, current).then(|| {
        format!("chaps {latest} is available (you have v{current}): run `chaps self update`")
    })
}

// --------------------------------------------------------------------------
// HTTP
// --------------------------------------------------------------------------

/// GET `url` as text, with one global deadline covering connect, send and
/// receive.
fn get_text(url: &str, timeout: Duration) -> Result<String> {
    let mut response = request(url, timeout)?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("reading the response body from {url}: {e}"))
}

/// GET `url` as bytes, for an archive.
pub fn download(url: &str, timeout: Duration) -> Result<Vec<u8>> {
    let mut response = request(url, timeout)?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_ARCHIVE_BYTES)
        .read_to_vec()
        .map_err(|e| anyhow::anyhow!("downloading {url}: {e}"))
}

/// Download `url` as text, for `SHA256SUMS`.
pub fn download_text(url: &str, timeout: Duration) -> Result<String> {
    get_text(url, timeout)
}

fn request(url: &str, timeout: Duration) -> Result<ureq::http::Response<ureq::Body>> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(USER_AGENT)
            .build(),
    );
    let started = std::time::Instant::now();
    let response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| {
            crate::output::verbose(&format!(
                "GET {url} -> failed in {}ms",
                started.elapsed().as_millis()
            ));
            map_error(url, e)
        })?;
    crate::output::verbose(&format!(
        "GET {url} -> {} in {}ms",
        response.status().as_u16(),
        started.elapsed().as_millis()
    ));
    Ok(response)
}

/// A non-2xx response is a [`ChapError::Http`]; everything else is a transport
/// failure that keeps the URL as context.
fn map_error(url: &str, err: ureq::Error) -> anyhow::Error {
    match err {
        ureq::Error::StatusCode(status) => ChapError::Http {
            url: url.to_string(),
            status,
        }
        .into(),
        other => anyhow::anyhow!("fetching {url}: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX: &str = "x86_64-unknown-linux-musl";
    const MAC: &str = "aarch64-apple-darwin";
    const WINDOWS: &str = "x86_64-pc-windows-msvc";

    #[test]
    fn every_target_names_its_archive() {
        assert_eq!(
            asset_name("v0.2.0", LINUX),
            "chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            asset_name("v0.2.0", "aarch64-unknown-linux-musl"),
            "chaps-v0.2.0-aarch64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            asset_name("v0.2.0", WINDOWS),
            "chaps-v0.2.0-x86_64-pc-windows-msvc.zip"
        );
        assert_eq!(
            asset_name("v0.2.0", "aarch64-pc-windows-msvc"),
            "chaps-v0.2.0-aarch64-pc-windows-msvc.zip"
        );
    }

    #[test]
    fn both_macs_take_the_universal_archive() {
        for target in [MAC, "x86_64-apple-darwin", "universal-apple-darwin"] {
            assert_eq!(asset_target(target), "universal-apple-darwin", "{target}");
            assert_eq!(
                asset_name("v0.2.0", target),
                "chaps-v0.2.0-universal-apple-darwin.tar.gz",
                "{target}"
            );
        }
        // Nothing else is rewritten.
        assert_eq!(asset_target(LINUX), LINUX);
        assert_eq!(asset_target(WINDOWS), WINDOWS);
    }

    #[test]
    fn the_stable_name_is_the_same_archive_without_the_tag() {
        assert_eq!(
            stable_asset_name(LINUX),
            "chaps-x86_64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            stable_asset_name(MAC),
            "chaps-universal-apple-darwin.tar.gz"
        );
        assert_eq!(
            stable_asset_name(WINDOWS),
            "chaps-x86_64-pc-windows-msvc.zip"
        );
    }

    #[test]
    fn the_executable_inside_gets_an_exe_on_windows() {
        assert_eq!(binary_name(LINUX), "chaps");
        assert_eq!(binary_name(MAC), "chaps");
        assert_eq!(binary_name(WINDOWS), "chaps.exe");
        assert_eq!(binary_name("aarch64-pc-windows-msvc"), "chaps.exe");
    }

    #[test]
    fn this_build_knows_its_own_target() {
        assert!(!TARGET.is_empty());
        assert_ne!(TARGET, "unknown");
        // Whatever the host is, the asset it wants exists in the release
        // matrix, which is the property that matters.
        assert!(asset_name("v0.1.0", TARGET).starts_with("chaps-v0.1.0-"));
    }

    #[test]
    fn a_release_payload_yields_its_tag_and_assets() {
        let release = parse_release(
            r#"{
              "tag_name": "v0.2.0",
              "assets": [
                {"name": "chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz"},
                {"name": "SHA256SUMS"}
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(release.tag, "v0.2.0");
        assert!(release.has_asset("SHA256SUMS"));
        assert!(release.has_asset("chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz"));
        assert!(!release.has_asset("chaps-v0.2.0-x86_64-pc-windows-msvc.zip"));
    }

    #[test]
    fn a_release_without_a_tag_is_an_error() {
        // The rate-limit body parses as JSON but carries no tag.
        let err = parse_release(r#"{"message":"API rate limit exceeded"}"#).unwrap_err();
        assert!(err.to_string().contains("tag_name"), "{err}");
        assert!(parse_release("<html>nope</html>").is_err());
        // Assets are optional; a release with none still parses.
        assert_eq!(
            parse_release(r#"{"tag_name":"v1.0.0"}"#).unwrap().assets,
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_asset_is_the_versioned_name_with_the_stable_copy_as_a_fallback() {
        let full = Release {
            tag: "v0.2.0".to_string(),
            assets: vec![
                "chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz".to_string(),
                "chaps-x86_64-unknown-linux-musl.tar.gz".to_string(),
                SUMS_FILE.to_string(),
            ],
        };
        assert_eq!(
            pick_asset(&full, LINUX).unwrap(),
            "chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz"
        );

        // Only the version-less copy is there.
        let stable_only = Release {
            assets: vec!["chaps-x86_64-unknown-linux-musl.tar.gz".to_string()],
            ..full.clone()
        };
        assert_eq!(
            pick_asset(&stable_only, LINUX).unwrap(),
            "chaps-x86_64-unknown-linux-musl.tar.gz"
        );

        // A payload with no asset list is taken at its word.
        let bare = Release {
            assets: Vec::new(),
            ..full.clone()
        };
        assert_eq!(
            pick_asset(&bare, LINUX).unwrap(),
            "chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz"
        );

        // A release that really has nothing for this target says what it has.
        let err = pick_asset(&full, WINDOWS).expect_err("no windows archive");
        assert!(err.to_string().contains("no archive for"), "{err}");
        assert!(err.to_string().contains(SUMS_FILE), "{err}");
    }

    #[test]
    fn newer_is_compared_against_this_build() {
        assert!(!is_newer_than_current(&format!("v{VERSION}")));
        assert!(!is_newer_than_current("v0.0.1"));
        assert!(is_newer_than_current("v999.0.0"));
        // A moving tag is not a point on the version line.
        assert!(!is_newer_than_current("latest"));
    }

    #[test]
    fn the_install_method_is_guessed_from_the_path() {
        let cargo: PathBuf = [r"/home/u", ".cargo", "bin", "chaps"].iter().collect();
        assert_eq!(install_method(&cargo), "cargo install");
        let cargo_win: PathBuf = [r"C:\Users\u", ".cargo", "bin", "chaps.exe"]
            .iter()
            .collect();
        assert_eq!(install_method(&cargo_win), "cargo install");
        for other in ["/usr/local/bin/chaps", "/home/u/.local/bin/chaps"] {
            assert_eq!(
                install_method(Path::new(other)),
                "release archive",
                "{other}"
            );
        }
        // `.cargo` without `bin` under it is not a cargo install.
        assert_eq!(
            install_method(Path::new("/home/u/.cargo/chaps")),
            "release archive"
        );
    }

    /// A `SHA256SUMS` in both spellings sha256sum and shasum write.
    const SUMS: &str = "\
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad *chaps-v0.2.0-universal-apple-darwin.tar.gz
248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1  ./chaps-v0.2.0-x86_64-pc-windows-msvc.zip
";

    #[test]
    fn the_sums_file_is_read_in_every_spelling() {
        assert_eq!(
            sha256_for(SUMS, "chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz").unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_for(SUMS, "chaps-v0.2.0-universal-apple-darwin.tar.gz").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_for(SUMS, "chaps-v0.2.0-x86_64-pc-windows-msvc.zip").unwrap(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha256_for(SUMS, "chaps-v0.2.0-aarch64-apple-darwin.tar.gz"),
            None
        );
        assert_eq!(sha256_for("", "anything"), None);
        // A line that is not a digest is not a match.
        assert_eq!(sha256_for("nothex  chaps.tar.gz", "chaps.tar.gz"), None);
    }

    #[test]
    fn verification_passes_for_the_listed_bytes_and_fails_otherwise() {
        verify(SUMS, "chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz", b"")
            .expect("the empty digest is the one listed");
        verify(SUMS, "chaps-v0.2.0-universal-apple-darwin.tar.gz", b"abc")
            .expect("the abc digest is the one listed");

        let err = verify(SUMS, "chaps-v0.2.0-universal-apple-darwin.tar.gz", b"abd")
            .expect_err("a changed byte is a different digest");
        assert!(err.to_string().contains("does not match"), "{err}");

        let err = verify(SUMS, "chaps-v9.9.9-nowhere.tar.gz", b"")
            .expect_err("an asset the manifest never listed");
        assert!(err.to_string().contains("does not list"), "{err}");
    }

    #[test]
    fn the_staged_and_backup_files_sit_beside_the_binary() {
        let unix = Path::new("/usr/local/bin/chaps");
        assert_eq!(staged_path(unix), Path::new("/usr/local/bin/chaps.new"));
        assert_eq!(backup_path(unix), Path::new("/usr/local/bin/chaps.old"));

        // The `.exe` is dropped, so Windows stages `chaps.new`, not
        // `chaps.exe.new`.
        let windows: PathBuf = ["bin", "chaps.exe"].iter().collect();
        assert_eq!(
            staged_path(&windows).file_name().unwrap(),
            std::ffi::OsStr::new("chaps.new")
        );
        assert_eq!(
            backup_path(&windows).file_name().unwrap(),
            std::ffi::OsStr::new("chaps.old")
        );
    }

    #[test]
    fn replacing_a_binary_swaps_the_contents_and_leaves_nothing_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join("chaps");
        std::fs::write(&binary, b"the old binary").unwrap();
        set_mode(&binary, 0o755);

        replace_executable(&binary, b"the new binary").expect("the swap succeeds");

        assert_eq!(std::fs::read(&binary).unwrap(), b"the new binary");
        assert!(!staged_path(&binary).exists(), "the staged file is gone");
        assert_eq!(mode_of(&binary), Some(0o755));
    }

    #[test]
    fn replacing_preserves_a_mode_that_is_not_the_default() {
        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join("chaps");
        std::fs::write(&binary, b"old").unwrap();
        set_mode(&binary, 0o750);

        replace_executable(&binary, b"new").unwrap();

        assert_eq!(std::fs::read(&binary).unwrap(), b"new");
        assert_eq!(mode_of(&binary), Some(0o750));
    }

    #[test]
    fn a_leftover_backup_is_cleaned_up_and_a_missing_one_is_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join("chaps");
        std::fs::write(&binary, b"current").unwrap();

        clean_backup(&binary);
        std::fs::write(backup_path(&binary), b"previous").unwrap();
        clean_backup(&binary);
        assert!(!backup_path(&binary).exists());
        assert!(binary.exists(), "the current binary is untouched");
    }

    #[test]
    fn a_writable_directory_is_replaceable_and_a_missing_one_is_not() {
        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join("chaps");
        std::fs::write(&binary, b"x").unwrap();
        assert!(is_replaceable(&binary));
        assert!(!is_replaceable(&tmp.path().join("nowhere").join("chaps")));
    }

    #[test]
    fn the_binary_is_found_wherever_the_archive_put_it() {
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp.path().join("chaps-v0.2.0-x86_64-unknown-linux-musl");
        std::fs::create_dir_all(inner.join("completions")).unwrap();
        std::fs::write(inner.join("README.md"), "readme").unwrap();
        std::fs::write(inner.join("completions/chaps.bash"), "completions").unwrap();
        std::fs::write(inner.join("chaps"), b"elf").unwrap();

        assert_eq!(
            find_binary(tmp.path(), "x86_64-unknown-linux-musl"),
            Some(inner.join("chaps"))
        );
        // A Windows archive has `chaps.exe` and nothing called `chaps`.
        assert_eq!(find_binary(tmp.path(), "x86_64-pc-windows-msvc"), None);
    }

    #[test]
    fn staleness_is_measured_against_the_interval() {
        let day = CHECK_INTERVAL.as_secs();
        let now = 1_000_000;
        assert!(is_stale(None, now, CHECK_INTERVAL), "no state at all");

        let fresh = CheckState {
            checked_at_unix: now - 60,
            latest: "v0.2.0".to_string(),
        };
        assert!(!is_stale(Some(&fresh), now, CHECK_INTERVAL));

        let stale = CheckState {
            checked_at_unix: now - day - 1,
            latest: "v0.2.0".to_string(),
        };
        assert!(is_stale(Some(&stale), now, CHECK_INTERVAL));

        // Exactly the interval counts as due.
        let due = CheckState {
            checked_at_unix: now - day,
            ..CheckState::default()
        };
        assert!(is_stale(Some(&due), now, CHECK_INTERVAL));

        // A timestamp from the future is a clock that moved, not a reason to
        // stop checking forever.
        let future = CheckState {
            checked_at_unix: now + day,
            ..CheckState::default()
        };
        assert!(is_stale(Some(&future), now, CHECK_INTERVAL));
    }

    #[test]
    fn the_check_state_round_trips_through_the_cache_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("nested").join("cache");
        assert!(read_check(&cache).is_none(), "nothing written yet");

        let state = CheckState {
            checked_at_unix: 1_700_000_000,
            latest: "v0.3.0".to_string(),
        };
        write_check(&cache, &state);
        let read = read_check(&cache).expect("what was just written");
        assert_eq!(read.checked_at_unix, 1_700_000_000);
        assert_eq!(read.latest, "v0.3.0");
        assert!(check_path(&cache).ends_with(CHECK_FILE));

        // A corrupt file is "no state", not an error.
        std::fs::write(check_path(&cache), "{not json").unwrap();
        assert!(read_check(&cache).is_none());
    }

    #[test]
    fn only_the_exact_value_turns_the_notice_off() {
        assert!(disabled_by(Some("1")));
        for value in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
            assert!(!disabled_by(value), "{value:?}");
        }
    }

    #[test]
    fn the_notice_names_both_versions_and_is_silent_when_there_is_nothing_to_say() {
        let line = notice_line("v9.9.9", "0.1.0").expect("a newer release");
        assert!(line.contains("chaps v9.9.9 is available"), "{line}");
        assert!(line.contains("you have v0.1.0"), "{line}");
        assert!(line.contains("chaps self update"), "{line}");

        assert_eq!(notice_line("v0.1.0", "0.1.0"), None);
        assert_eq!(notice_line("v0.0.9", "0.1.0"), None);
        assert_eq!(notice_line("", "0.1.0"), None);
        assert_eq!(notice_line("latest", "0.1.0"), None);
    }

    #[test]
    fn the_download_urls_name_the_repository_and_the_tag() {
        assert_eq!(
            download_base("v0.2.0"),
            "https://github.com/winterop-com/chaps/releases/download/v0.2.0"
        );
        assert!(LATEST_RELEASE_URL.contains(REPO));
    }

    /// Port 9 is the discard service, so this covers the transport-error path
    /// without a network.
    #[test]
    fn an_unreachable_host_is_an_error_not_a_hang() {
        let err = get_text("http://127.0.0.1:9/releases/latest", Duration::from_secs(2))
            .expect_err("nothing is listening on port 9");
        assert!(err.to_string().contains("127.0.0.1:9"), "{err}");
    }

    #[test]
    fn a_status_code_becomes_a_typed_http_error() {
        let err = map_error(LATEST_RELEASE_URL, ureq::Error::StatusCode(403));
        match err.downcast_ref::<ChapError>() {
            Some(ChapError::Http { url, status }) => {
                assert_eq!(url, LATEST_RELEASE_URL);
                assert_eq!(*status, 403);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    // ----------------------------------------------------------------------
    // A release served off a local socket
    //
    // The whole path an update takes - look up the release, download the
    // archive and the manifest, verify, unpack, swap - against a server in a
    // thread, so nothing here needs GitHub or the network.
    // ----------------------------------------------------------------------

    /// Serve fixed bodies by path on a loopback port, for as long as the test
    /// binary runs.
    fn serve(routes: Vec<(String, Vec<u8>)>) -> String {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let Ok(peek) = stream.try_clone() else {
                    continue;
                };
                let mut reader = BufReader::new(peek);

                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                // Drain the headers so the client is not left writing into a
                // socket nobody reads.
                loop {
                    let mut header = String::new();
                    match reader.read_line(&mut header) {
                        Ok(0) => break,
                        Ok(_) if header == "\r\n" || header == "\n" => break,
                        Ok(_) => continue,
                        Err(_) => break,
                    }
                }

                let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (status, body) = match routes.iter().find(|(route, _)| *route == path) {
                    Some((_, body)) => ("200 OK", body.clone()),
                    None => ("404 Not Found", b"no such asset".to_vec()),
                };
                // `Connection: close` keeps each request independent, which
                // is what makes a server this small enough.
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\n\
                     Content-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        base
    }

    const FEED: &str = r#"{
      "tag_name": "v9.9.9",
      "assets": [
        {"name": "chaps-v9.9.9-x86_64-unknown-linux-musl.tar.gz"},
        {"name": "SHA256SUMS"}
      ]
    }"#;

    #[test]
    fn a_release_feed_is_read_over_http() {
        let base = serve(vec![
            ("/releases/latest".to_string(), FEED.as_bytes().to_vec()),
            (
                "/releases/tags/v9.9.9".to_string(),
                FEED.as_bytes().to_vec(),
            ),
        ]);

        let release = release_at(&format!("{base}/releases/latest"), Duration::from_secs(5))
            .expect("the feed parses");
        assert_eq!(release.tag, "v9.9.9");
        assert_eq!(
            pick_asset(&release, LINUX).unwrap(),
            "chaps-v9.9.9-x86_64-unknown-linux-musl.tar.gz"
        );
        assert!(is_newer_than_current(&release.tag));

        // A tag that is not there is a 404, and a 404 is a typed error rather
        // than a parse failure on an HTML page.
        let err = release_at(
            &format!("{base}/releases/tags/v0.0.1"),
            Duration::from_secs(5),
        )
        .expect_err("no such tag");
        match err.downcast_ref::<ChapError>() {
            Some(ChapError::Http { status, .. }) => assert_eq!(*status, 404),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn an_archive_is_downloaded_verified_unpacked_and_swapped_in() {
        let tmp = tempfile::tempdir().unwrap();

        // Build an archive shaped like a release: one top-level directory
        // holding the binary, the README and the completion scripts.
        let tag = "v9.9.9";
        let name = format!("chaps-{tag}-{}", asset_target(TARGET));
        let staging = tmp.path().join("staging");
        std::fs::create_dir_all(staging.join(&name).join("completions")).unwrap();
        std::fs::write(
            staging.join(&name).join(binary_name(TARGET)),
            b"the new binary",
        )
        .unwrap();
        std::fs::write(staging.join(&name).join("README.md"), b"readme").unwrap();
        std::fs::write(
            staging.join(&name).join("completions").join("chaps.bash"),
            b"completions",
        )
        .unwrap();

        let asset = asset_name(tag, TARGET);
        let archive = tmp.path().join(&asset);
        let packed = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&staging)
            .arg(&name)
            .status()
            .expect("tar runs");
        assert!(packed.success(), "tar packed the fixture");

        let bytes = std::fs::read(&archive).unwrap();
        let sums = format!("{}  {asset}\n", sha256_hex(&bytes));
        let base = serve(vec![
            (format!("/{asset}"), bytes.clone()),
            (format!("/{SUMS_FILE}"), sums.into_bytes()),
        ]);

        // What `self update` does, in the order it does it.
        let downloaded =
            download(&format!("{base}/{asset}"), Duration::from_secs(10)).expect("the archive");
        assert_eq!(downloaded, bytes);
        let manifest = download_text(&format!("{base}/{SUMS_FILE}"), Duration::from_secs(10))
            .expect("the manifest");
        verify(&manifest, &asset, &downloaded).expect("the archive matches the manifest");

        let unpacked = tmp.path().join("unpacked");
        std::fs::create_dir_all(&unpacked).unwrap();
        let staged_archive = unpacked.join(&asset);
        std::fs::write(&staged_archive, &downloaded).unwrap();
        extract(&staged_archive, &unpacked).expect("tar unpacks it");

        let found = find_binary(&unpacked, TARGET).expect("the binary is in there");
        assert_eq!(std::fs::read(&found).unwrap(), b"the new binary");

        // And the swap, on a copy that stands in for the installed binary.
        let installed = tmp.path().join("bin").join(binary_name(TARGET));
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, b"the old binary").unwrap();
        set_mode(&installed, 0o755);
        replace_executable(&installed, &std::fs::read(&found).unwrap()).expect("the swap");
        assert_eq!(std::fs::read(&installed).unwrap(), b"the new binary");
        assert_eq!(mode_of(&installed), Some(0o755));

        // A manifest that does not agree with the bytes stops the update
        // before anything is written.
        let tampered = format!("{}  {asset}\n", sha256_hex(b"something else"));
        let err = verify(&tampered, &asset, &downloaded).expect_err("the digests differ");
        assert!(err.to_string().contains("does not match"), "{err}");
    }

    #[cfg(unix)]
    fn set_mode(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(not(unix))]
    fn set_mode(_path: &Path, _mode: u32) {}

    #[cfg(unix)]
    fn mode_of(path: &Path) -> Option<u32> {
        use std::os::unix::fs::PermissionsExt;
        Some(std::fs::metadata(path).ok()?.permissions().mode() & 0o7777)
    }

    #[cfg(not(unix))]
    fn mode_of(_path: &Path) -> Option<u32> {
        None
    }
}
