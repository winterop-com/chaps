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

/// The channel this build follows, from the build script: `stable` or `dev`.
pub const BUILD_CHANNEL: &str = env!("CHAPS_BUILD_CHANNEL");

/// The tag the rolling pre-release of `main` is published under.
///
/// One tag that moves, rather than one per commit: the release it names is
/// always the newest build of `main`, so `releases/download/dev/<asset>` is a
/// URL that keeps working.
pub const DEV_TAG: &str = "dev";

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
// Channels
// --------------------------------------------------------------------------

/// Which release series a build follows.
///
/// The two are not versions of each other: a `dev` build carries the Cargo
/// version of the last tag, because it is built from `main` after that tag,
/// so comparing them as versions says nothing. What separates two dev builds
/// is the commit they came from, which is why the dev side of this module
/// compares commits and the stable side compares versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Channel {
    /// Built from a `vX.Y.Z` tag, or built anywhere else without being told
    /// otherwise: what `releases/latest` serves.
    #[default]
    Stable,
    /// The rolling build of `main`, published as the `dev` pre-release.
    Dev,
}

impl Channel {
    /// The name this channel is printed and recorded under.
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Dev => "dev",
        }
    }

    /// The channel `name` spells. Anything but `dev` is stable, so an unset or
    /// misspelled value can only ever mean the conservative answer.
    pub fn from_name(name: &str) -> Channel {
        if name.trim().eq_ignore_ascii_case("dev") {
            Channel::Dev
        } else {
            Channel::Stable
        }
    }

    /// The release tag this channel follows without an explicit `--version`,
    /// or `None` for stable, which asks `releases/latest` instead.
    pub fn tag(self) -> Option<&'static str> {
        match self {
            Channel::Stable => None,
            Channel::Dev => Some(DEV_TAG),
        }
    }
}

/// The channel this binary was built on.
pub fn channel() -> Channel {
    Channel::from_name(BUILD_CHANNEL)
}

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

/// The release asset `target` wants, e.g.
/// `chaps-universal-apple-darwin.tar.gz`.
///
/// No version in the name: the tag is already in the download URL, and one
/// name per target is what `releases/latest/download/<name>` needs to keep a
/// link working across releases. The directory inside the archive does carry
/// the version, because that is what someone sees after unpacking it.
pub fn asset_name(target: &str) -> String {
    let target = asset_target(target);
    format!("chaps-{target}.{}", archive_extension(target))
}

/// What the same archive was called up to v0.2.0, when every asset name
/// carried the tag. [`pick_asset`] falls back to it so `self update --version`
/// still reaches a release published before the rename.
pub fn legacy_asset_name(tag: &str, target: &str) -> String {
    let target = asset_target(target);
    format!("chaps-{tag}-{target}.{}", archive_extension(target))
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Release {
    /// The tag, e.g. `v0.2.0`.
    pub tag: String,
    /// Asset names attached to the release.
    pub assets: Vec<String>,
    /// Whether GitHub marks this release a pre-release. The `dev` release is
    /// one; every `vX.Y.Z` release is not.
    pub prerelease: bool,
    /// When the release was published, as GitHub's ISO-8601 string, empty
    /// when the payload did not carry one.
    pub published_at: String,
    /// The release notes, which is where a dev build records the commit it
    /// was built from. See [`release_commit`].
    pub body: String,
}

impl Release {
    /// Whether the release carries `name`.
    pub fn has_asset(&self, name: &str) -> bool {
        self.assets.iter().any(|asset| asset == name)
    }

    /// The day the release was published, `YYYY-MM-DD`, or the whole
    /// timestamp when it is not the shape GitHub documents.
    pub fn published_day(&self) -> &str {
        match self.published_at.split_once('T') {
            Some((day, _)) => day,
            None => &self.published_at,
        }
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
        #[serde(default)]
        prerelease: bool,
        #[serde(default)]
        published_at: Option<String>,
        #[serde(default)]
        body: Option<String>,
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
        prerelease: payload.prerelease,
        // `published_at` and `body` are null on a draft and on a release with
        // no notes, so both are read as an option and flattened to "".
        published_at: payload.published_at.unwrap_or_default().trim().to_string(),
        body: payload.body.unwrap_or_default(),
    })
}

/// The commit a release was built from, as its notes record it.
///
/// The dev release notes carry one line reading `built from commit <sha> on
/// <date>`, written by `scripts/release-notes.sh`, because the API does
/// not otherwise say: `target_commitish` is documented as unused once the tag
/// exists, and the `dev` tag exists from the first rolling build onwards, so
/// it goes on reporting whatever the release was first created with.
///
/// The first hexadecimal word of at least seven characters after the word
/// `commit` wins. `None` when the notes say nothing of the sort, which is the
/// case for every `vX.Y.Z` release.
pub fn release_commit(release: &Release) -> Option<String> {
    let mut words = release
        .body
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty());
    while let Some(word) = words.next() {
        if !word.eq_ignore_ascii_case("commit") {
            continue;
        }
        if let Some(candidate) = words.next()
            && is_commit_sha(candidate)
        {
            return Some(candidate.to_ascii_lowercase());
        }
    }
    None
}

/// Whether `word` could be a git object name: 7 to 40 hex digits.
fn is_commit_sha(word: &str) -> bool {
    (7..=40).contains(&word.len()) && word.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether two commits are the same one, comparing on the shorter of the two.
///
/// A build records the short revision git gave it, which is seven characters
/// on this repository today but grows with the object count, while the notes
/// carry the full forty, so neither length can be assumed.
pub fn same_commit(a: &str, b: &str) -> bool {
    if !is_commit_sha(a) || !is_commit_sha(b) {
        return false;
    }
    let shortest = a.len().min(b.len());
    a[..shortest].eq_ignore_ascii_case(&b[..shortest])
}

/// Whether `release` is a build of something other than commit `revision`.
///
/// Unknown counts as available: a build with no recorded revision, or a
/// release whose notes name no commit, cannot be shown to be the same one,
/// and re-installing a dev build that turns out to be identical costs a
/// download, while refusing one that is not would leave `self update` unable
/// to move at all. [`dev_notice_line`] takes the opposite side of the same
/// question, because an unprompted daily line has to be sure before it speaks.
pub fn dev_update_available(release: &Release, revision: &str) -> bool {
    match release_commit(release) {
        Some(commit) => !same_commit(&commit, revision),
        None => true,
    }
}

/// The asset `target` should download from `release`.
///
/// The version-less name is what a release carries; a release from v0.2.0 or
/// earlier, where every asset name held the tag, falls back to the name it
/// does have. A payload that lists no assets at all is taken at its word
/// rather than refused, because the name is derivable without it.
pub fn pick_asset(release: &Release, target: &str) -> Result<String> {
    let name = asset_name(target);
    if release.assets.is_empty() || release.has_asset(&name) {
        return Ok(name);
    }
    let legacy = legacy_asset_name(&release.tag, target);
    if release.has_asset(&legacy) {
        return Ok(legacy);
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
///
/// `releases/latest` is documented as "the most recent non-prerelease,
/// non-draft release", so the rolling `dev` pre-release is invisible here and
/// a stable build is never offered one by the daily notice. GitHub will not
/// mark a pre-release as latest either, which is the same guarantee from the
/// other end. [`Release::prerelease`] is read all the same, so the one place
/// that must never offer a pre-release can check rather than trust.
pub fn latest_release(timeout: Duration) -> Result<Release> {
    release_at(LATEST_RELEASE_URL, timeout)
}

/// One release by tag, for `--version` and for the `dev` channel.
///
/// `releases/tags/<tag>` returns a pre-release like any other release, which
/// is what makes `--version dev` work at all.
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
        Ok(file) => {
            // Windows will not delete a file whose handle is still open, so
            // the probe is closed before it is removed.
            drop(file);
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
    /// The commit that release was built from, for the `dev` channel, where
    /// the tag never changes and so says nothing on its own. Empty on the
    /// stable channel and on a state written by an older chaps.
    #[serde(default)]
    pub commit: String,
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

/// The same line for a `dev` build, which compares commits rather than
/// versions: the rolling release carries the version of the last tag, so the
/// only thing that moves between two dev builds is the commit.
///
/// Silent unless both commits are known and they differ. An unprompted line
/// that cannot tell whether there is anything new would be noise on every
/// command, which is the opposite trade-off from [`dev_update_available`],
/// where the user asked.
pub fn dev_notice_line(commit: &str, revision: &str) -> Option<String> {
    if !is_commit_sha(commit) || !is_commit_sha(revision) || same_commit(commit, revision) {
        return None;
    }
    let short = &commit[..revision.len().min(commit.len()).max(7)];
    Some(format!(
        "a newer chaps dev build is available (commit {short}, you have {revision}): \
         run `chaps self update`"
    ))
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
        assert_eq!(asset_name(LINUX), "chaps-x86_64-unknown-linux-musl.tar.gz");
        assert_eq!(
            asset_name("aarch64-unknown-linux-musl"),
            "chaps-aarch64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(asset_name(WINDOWS), "chaps-x86_64-pc-windows-msvc.zip");
        assert_eq!(
            asset_name("aarch64-pc-windows-msvc"),
            "chaps-aarch64-pc-windows-msvc.zip"
        );
        // Nothing in the name says which release it came from.
        for target in [LINUX, MAC, WINDOWS] {
            assert!(!asset_name(target).contains("v0."), "{target}");
        }
    }

    #[test]
    fn both_macs_take_the_universal_archive() {
        for target in [MAC, "x86_64-apple-darwin", "universal-apple-darwin"] {
            assert_eq!(asset_target(target), "universal-apple-darwin", "{target}");
            assert_eq!(
                asset_name(target),
                "chaps-universal-apple-darwin.tar.gz",
                "{target}"
            );
        }
        // Nothing else is rewritten.
        assert_eq!(asset_target(LINUX), LINUX);
        assert_eq!(asset_target(WINDOWS), WINDOWS);
    }

    #[test]
    fn the_legacy_name_is_the_same_archive_with_the_tag_in_it() {
        assert_eq!(
            legacy_asset_name("v0.2.0", LINUX),
            "chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz"
        );
        assert_eq!(
            legacy_asset_name("v0.2.0", MAC),
            "chaps-v0.2.0-universal-apple-darwin.tar.gz"
        );
        assert_eq!(
            legacy_asset_name("v0.2.0", WINDOWS),
            "chaps-v0.2.0-x86_64-pc-windows-msvc.zip"
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
        assert!(asset_name(TARGET).starts_with("chaps-"));
        assert!(asset_name(TARGET).contains(asset_target(TARGET)));
    }

    #[test]
    fn a_release_payload_yields_its_tag_and_assets() {
        let release = parse_release(
            r#"{
              "tag_name": "v0.2.0",
              "assets": [
                {"name": "chaps-x86_64-unknown-linux-musl.tar.gz"},
                {"name": "SHA256SUMS"}
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(release.tag, "v0.2.0");
        assert!(release.has_asset("SHA256SUMS"));
        assert!(release.has_asset("chaps-x86_64-unknown-linux-musl.tar.gz"));
        assert!(!release.has_asset("chaps-x86_64-pc-windows-msvc.zip"));
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
    fn the_asset_is_the_version_less_name_with_the_tagged_one_as_a_fallback() {
        let full = Release {
            tag: "v0.2.0".to_string(),
            assets: vec![
                "chaps-x86_64-unknown-linux-musl.tar.gz".to_string(),
                "chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz".to_string(),
                SUMS_FILE.to_string(),
            ],
            ..Default::default()
        };
        assert_eq!(
            pick_asset(&full, LINUX).unwrap(),
            "chaps-x86_64-unknown-linux-musl.tar.gz"
        );

        // A release from before the rename carries the tagged name alone.
        let legacy_only = Release {
            assets: vec!["chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz".to_string()],
            ..full.clone()
        };
        assert_eq!(
            pick_asset(&legacy_only, LINUX).unwrap(),
            "chaps-v0.2.0-x86_64-unknown-linux-musl.tar.gz"
        );

        // A payload with no asset list is taken at its word.
        let bare = Release {
            assets: Vec::new(),
            ..full.clone()
        };
        assert_eq!(
            pick_asset(&bare, LINUX).unwrap(),
            "chaps-x86_64-unknown-linux-musl.tar.gz"
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
e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  chaps-x86_64-unknown-linux-musl.tar.gz
ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad *chaps-universal-apple-darwin.tar.gz
248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1  ./chaps-x86_64-pc-windows-msvc.zip
";

    #[test]
    fn the_sums_file_is_read_in_every_spelling() {
        assert_eq!(
            sha256_for(SUMS, "chaps-x86_64-unknown-linux-musl.tar.gz").unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_for(SUMS, "chaps-universal-apple-darwin.tar.gz").unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_for(SUMS, "chaps-x86_64-pc-windows-msvc.zip").unwrap(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(sha256_for(SUMS, "chaps-aarch64-apple-darwin.tar.gz"), None);
        assert_eq!(sha256_for("", "anything"), None);
        // A line that is not a digest is not a match.
        assert_eq!(sha256_for("nothex  chaps.tar.gz", "chaps.tar.gz"), None);
    }

    #[test]
    fn verification_passes_for_the_listed_bytes_and_fails_otherwise() {
        verify(SUMS, "chaps-x86_64-unknown-linux-musl.tar.gz", b"")
            .expect("the empty digest is the one listed");
        verify(SUMS, "chaps-universal-apple-darwin.tar.gz", b"abc")
            .expect("the abc digest is the one listed");

        let err = verify(SUMS, "chaps-universal-apple-darwin.tar.gz", b"abd")
            .expect_err("a changed byte is a different digest");
        assert!(err.to_string().contains("does not match"), "{err}");

        let err = verify(SUMS, "chaps-nowhere.tar.gz", b"")
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
        #[cfg(unix)]
        set_mode(&binary, 0o755);

        replace_executable(&binary, b"the new binary").expect("the swap succeeds");

        assert_eq!(std::fs::read(&binary).unwrap(), b"the new binary");
        assert!(!staged_path(&binary).exists(), "the staged file is gone");

        // Unix renames straight over the old binary, so nothing is parked
        // beside it and the mode it had is carried across.
        #[cfg(unix)]
        {
            assert!(!backup_path(&binary).exists(), "nothing is parked");
            assert_eq!(mode_of(&binary), 0o755);
        }

        // Windows cannot rename over a running image, so the old binary is
        // moved aside and the next run is the first that can delete it.
        #[cfg(windows)]
        {
            assert_eq!(
                std::fs::read(backup_path(&binary)).unwrap(),
                b"the old binary"
            );
            clean_backup(&binary);
            assert!(!backup_path(&binary).exists(), "the next run clears it");
        }
    }

    /// Modes are a Unix thing: Windows has no 0o750 to preserve.
    #[cfg(unix)]
    #[test]
    fn replacing_preserves_a_mode_that_is_not_the_default() {
        let tmp = tempfile::tempdir().unwrap();
        let binary = tmp.path().join("chaps");
        std::fs::write(&binary, b"old").unwrap();
        set_mode(&binary, 0o750);

        replace_executable(&binary, b"new").unwrap();

        assert_eq!(std::fs::read(&binary).unwrap(), b"new");
        assert_eq!(mode_of(&binary), 0o750);
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

        // The probe it writes is cleaned up, whatever the platform thinks of
        // deleting a file that is still open.
        let left: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".chaps-write-probe"))
            .collect();
        assert!(left.is_empty(), "the probe is gone, found {left:?}");
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
            ..CheckState::default()
        };
        assert!(!is_stale(Some(&fresh), now, CHECK_INTERVAL));

        let stale = CheckState {
            checked_at_unix: now - day - 1,
            latest: "v0.2.0".to_string(),
            ..CheckState::default()
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
            ..CheckState::default()
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

    // ----------------------------------------------------------------------
    // Channels
    // ----------------------------------------------------------------------

    /// A dev release as the workflow publishes it: the `dev` tag, the notes
    /// that name the commit, and the same asset names a tag release carries.
    fn dev_release(commit: &str) -> Release {
        Release {
            tag: DEV_TAG.to_string(),
            assets: vec![
                "chaps-x86_64-unknown-linux-musl.tar.gz".to_string(),
                SUMS_FILE.to_string(),
            ],
            prerelease: true,
            published_at: "2026-09-23T17:08:44Z".to_string(),
            body: format!(
                "### Unstable\n\nThis is a rolling build of `main`, \
                 built from commit `{commit}` on 2026-09-23.\n"
            ),
        }
    }

    #[test]
    fn only_dev_names_the_dev_channel() {
        assert_eq!(Channel::from_name("dev"), Channel::Dev);
        assert_eq!(Channel::from_name(" DEV \n"), Channel::Dev);
        for other in ["stable", "", "development", "main", "nightly"] {
            assert_eq!(Channel::from_name(other), Channel::Stable, "{other}");
        }
        assert_eq!(Channel::Dev.as_str(), "dev");
        assert_eq!(Channel::Stable.as_str(), "stable");
        assert_eq!(Channel::Dev.tag(), Some(DEV_TAG));
        assert_eq!(Channel::Stable.tag(), None);
        // This build is one or the other, and a plain `cargo build` is stable.
        assert_eq!(channel(), Channel::from_name(BUILD_CHANNEL));
        assert_eq!(channel(), Channel::Stable, "an untagged build is stable");
    }

    #[test]
    fn the_dev_release_says_which_commit_it_was_built_from() {
        let release = dev_release("0b1c2d3e4f50617283940a1b2c3d4e5f60718293");
        assert_eq!(
            release_commit(&release).as_deref(),
            Some("0b1c2d3e4f50617283940a1b2c3d4e5f60718293")
        );
        assert_eq!(release.published_day(), "2026-09-23");

        // A tagged release says nothing of the sort.
        let stable = Release {
            tag: "v0.2.1".to_string(),
            body: "### Downloads\n\nNothing about a commit here.\n".to_string(),
            ..Default::default()
        };
        assert_eq!(release_commit(&stable), None);

        // The word has to be followed by something that could be a commit.
        let vague = Release {
            body: "built from commit unknown".to_string(),
            ..Default::default()
        };
        assert_eq!(release_commit(&vague), None);

        // Upper case and a short revision are both accepted, and the answer
        // is normalised.
        let shouty = Release {
            body: "Commit ABC1234 it is".to_string(),
            ..Default::default()
        };
        assert_eq!(release_commit(&shouty).as_deref(), Some("abc1234"));
    }

    /// The commit is read out of notes this repository writes, so the reader
    /// and the writer are checked against each other rather than only
    /// against a fixture: first that `scripts/release-notes.sh` still writes
    /// the line, then, where git can run, that `release_commit` reads the
    /// real script's output back as the commit this checkout is on. A
    /// packaged crate without the script has nothing to check and says
    /// nothing.
    #[test]
    fn the_notes_script_writes_the_line_the_commit_is_read_from() {
        let root = env!("CARGO_MANIFEST_DIR");
        let script = format!("{root}/scripts/release-notes.sh");
        let Ok(source) = std::fs::read_to_string(&script) else {
            return;
        };

        // `Built from commit` followed by a backtick, escaped for the heredoc
        // it sits in, and a shell expansion of the sha: `${GITHUB_SHA}`,
        // `${sha}` or `$sha`. The name has to mention the sha, so a rewrite
        // that interpolates something else fails here rather than in the
        // field.
        let expansion = source.lines().find_map(|line| {
            let rest = line.split_once("Built from commit")?.1;
            let name = rest
                .trim_start_matches([' ', '\\', '`'])
                .strip_prefix('$')?;
            let name: String = name
                .strip_prefix('{')
                .unwrap_or(name)
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            name.to_ascii_lowercase().contains("sha").then_some(name)
        });
        assert!(
            expansion.is_some(),
            "scripts/release-notes.sh no longer writes `Built from commit \
             <sha>` into the dev release notes, which is the line \
             release_commit reads"
        );

        // The writer itself, where it can run: the notes the script prints
        // for the rolling build name the commit HEAD is on, and that is what
        // comes back out. No git, or no shell to run the script with, and
        // the line above is all that was checked.
        let Some(head) = git(root, &["rev-parse", "HEAD"]) else {
            return;
        };
        let Ok(printed) = std::process::Command::new("bash")
            .arg(&script)
            .arg("dev")
            // GITHUB_SHA is what the workflow builds from; here the commit is
            // whatever this checkout has, which is what HEAD was read as.
            .env_remove("GITHUB_SHA")
            .output()
        else {
            return;
        };
        assert!(
            printed.status.success(),
            "scripts/release-notes.sh dev failed: {}",
            String::from_utf8_lossy(&printed.stderr)
        );

        let notes = Release {
            tag: DEV_TAG.to_string(),
            body: String::from_utf8_lossy(&printed.stdout).into_owned(),
            ..Default::default()
        };
        assert_eq!(
            release_commit(&notes).as_deref(),
            Some(head.to_ascii_lowercase().as_str()),
            "release_commit did not read HEAD out of the notes the script \
             printed:\n{}",
            notes.body
        );
    }

    /// One git command in this checkout, or `None` where git cannot answer,
    /// which is a test that skips rather than one that fails.
    fn git(root: &str, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    #[test]
    fn two_commits_match_on_the_shorter_of_the_two() {
        let full = "0b1c2d3e4f50617283940a1b2c3d4e5f60718293";
        assert!(same_commit(full, "0b1c2d3"));
        assert!(same_commit("0b1c2d3", full));
        assert!(same_commit("0B1C2D3", full));
        assert!(!same_commit(full, "deadbee"));
        // Too short to be anything, or not hexadecimal at all.
        assert!(!same_commit(full, "0b1c2d"));
        assert!(!same_commit(full, ""));
        assert!(!same_commit(full, "not-a-sha"));
    }

    #[test]
    fn a_dev_build_is_up_to_date_only_on_the_commit_the_release_names() {
        let commit = "0b1c2d3e4f50617283940a1b2c3d4e5f60718293";
        let release = dev_release(commit);

        assert!(!dev_update_available(&release, "0b1c2d3"));
        assert!(dev_update_available(&release, "deadbee"));
        // A build with no revision, and a release with no commit in its
        // notes, both mean "cannot tell", and an update that was asked for
        // goes ahead rather than refusing.
        assert!(dev_update_available(&release, ""));
        assert!(dev_update_available(
            &Release {
                tag: DEV_TAG.to_string(),
                ..Default::default()
            },
            "0b1c2d3"
        ));
    }

    #[test]
    fn the_dev_notice_speaks_only_when_it_knows_the_commit_moved() {
        let commit = "0b1c2d3e4f50617283940a1b2c3d4e5f60718293";
        let line = dev_notice_line(commit, "deadbee").expect("a different commit");
        assert!(line.contains("newer chaps dev build"), "{line}");
        assert!(line.contains("commit 0b1c2d3"), "{line}");
        assert!(line.contains("you have deadbee"), "{line}");
        assert!(line.contains("chaps self update"), "{line}");

        // The same build, and the two "cannot tell" cases, say nothing: this
        // line is printed without being asked for.
        assert_eq!(dev_notice_line(commit, "0b1c2d3"), None);
        assert_eq!(dev_notice_line(commit, ""), None);
        assert_eq!(dev_notice_line("", "deadbee"), None);
        assert_eq!(dev_notice_line("latest", "deadbee"), None);
    }

    /// The stable notice is fed from `releases/latest`, which GitHub
    /// documents as excluding pre-releases, so the `dev` tag can never reach
    /// it. The tag is not a version either, so even if it did, nothing would
    /// be printed.
    #[test]
    fn the_stable_notice_cannot_be_talked_into_offering_a_prerelease() {
        assert_eq!(notice_line(DEV_TAG, VERSION), None);
        assert!(!is_newer_than_current(DEV_TAG));

        let parsed = parse_release(
            r#"{"tag_name":"dev","prerelease":true,"published_at":"2026-09-23T17:08:44Z",
                "body":"built from commit abc1234 on 2026-09-23",
                "assets":[{"name":"chaps-x86_64-unknown-linux-musl.tar.gz"}]}"#,
        )
        .unwrap();
        assert!(parsed.prerelease);
        assert_eq!(parsed.tag, DEV_TAG);
        assert_eq!(release_commit(&parsed).as_deref(), Some("abc1234"));
        assert_eq!(parsed.published_day(), "2026-09-23");

        // A tagged release parses as what it is, with nulls tolerated.
        let stable =
            parse_release(r#"{"tag_name":"v0.2.1","published_at":null,"body":null}"#).unwrap();
        assert!(!stable.prerelease);
        assert!(stable.published_at.is_empty());
        assert_eq!(release_commit(&stable), None);
    }

    #[test]
    fn the_check_state_carries_the_commit_and_survives_an_older_file() {
        let tmp = tempfile::tempdir().unwrap();
        write_check(
            tmp.path(),
            &CheckState {
                checked_at_unix: 1_700_000_000,
                latest: DEV_TAG.to_string(),
                commit: "0b1c2d3".to_string(),
            },
        );
        let state = read_check(tmp.path()).expect("written a moment ago");
        assert_eq!(state.latest, DEV_TAG);
        assert_eq!(state.commit, "0b1c2d3");

        // A file written by a chaps that had no commit field still reads.
        std::fs::write(
            check_path(tmp.path()),
            r#"{"checked_at_unix":1700000000,"latest":"v0.2.1"}"#,
        )
        .unwrap();
        let old = read_check(tmp.path()).expect("an older state is still a state");
        assert_eq!(old.latest, "v0.2.1");
        assert!(old.commit.is_empty());
        assert_eq!(dev_notice_line(&old.commit, "0b1c2d3"), None);
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
        {"name": "chaps-x86_64-unknown-linux-musl.tar.gz"},
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
            "chaps-x86_64-unknown-linux-musl.tar.gz"
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

        // Build an archive shaped like a release: the asset is named after
        // the target alone, and the one directory inside it carries the tag.
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

        let asset = asset_name(TARGET);
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
        #[cfg(unix)]
        set_mode(&installed, 0o755);
        replace_executable(&installed, &std::fs::read(&found).unwrap()).expect("the swap");
        assert_eq!(std::fs::read(&installed).unwrap(), b"the new binary");
        #[cfg(unix)]
        assert_eq!(mode_of(&installed), 0o755);

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

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }
}
