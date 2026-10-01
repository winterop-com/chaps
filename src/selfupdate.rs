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
///
/// A function rather than a constant because `CHAPS_GITHUB_API` moves the
/// REST base for the tests, and this is one of the calls it has to move: a
/// test that asked the real GitHub would spend a request of somebody's hourly
/// quota to answer a question it already knows the answer to.
pub fn latest_release_url() -> String {
    format!("{}/repos/{REPO}/releases/latest", crate::github::api_base())
}

/// One release by its tag.
pub fn release_url(tag: &str) -> String {
    format!(
        "{}/repos/{REPO}/releases/tags/{tag}",
        crate::github::api_base()
    )
}

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
const USER_AGENT: &str = concat!("chaps/", env!("CARGO_PKG_VERSION"));

/// The file inside the cache directory holding the last update check.
pub const CHECK_FILE: &str = "self-update-check.json";

/// How long an update check is considered fresh.
pub const CHECK_INTERVAL: Duration = Duration::from_hours(24);

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
///
/// Through [`crate::github`], like every other REST call: the token where
/// this run has one, and a used-up rate limit reported as what it is. The
/// archive and its `SHA256SUMS` are *not* fetched that way - they come off
/// `github.com` rather than the API, count against no limit, and a credential
/// sent to a redirect chain ending at a signed storage URL is a credential
/// sent somewhere it was not needed.
fn release_at(url: &str, timeout: Duration) -> Result<Release> {
    let body = crate::github::get_ok(url, timeout)?;
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
    release_at(&latest_release_url(), timeout)
}

/// One release by tag, for `--version` and for the `dev` channel.
///
/// `releases/tags/<tag>` returns a pre-release like any other release, which
/// is what makes `--version dev` work at all.
pub fn release_by_tag(tag: &str, timeout: Duration) -> Result<Release> {
    let url = release_url(tag);
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

/// Whether `path` looks like `cargo install` put it there, or a `cargo build`
/// left it in a checkout's `target/`.
pub fn install_method(path: &Path) -> &'static str {
    let parts: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let from_cargo = parts
        .windows(2)
        .any(|pair| pair[0] == ".cargo" && pair[1] == "bin");
    let from_build = parts
        .windows(2)
        .any(|pair| pair[0] == "target" && matches!(pair[1].as_str(), "release" | "debug"));
    if from_cargo {
        "cargo install"
    } else if from_build {
        "cargo build"
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
mod tests;
