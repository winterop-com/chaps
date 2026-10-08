//! Releases: reading GitHub's release payload, telling dev builds apart by
//! commit, and picking the archive this build wants.

use super::{REPO, VERSION, asset_name, latest_release_url, release_url};
use crate::error::{ChapError, Result};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;

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
    /// when the payload did not carry one. For the rolling `dev` release this
    /// is the day the release was first made, not the day of the build.
    pub published_at: String,
    /// The newest `updated_at` of the assets, as GitHub's ISO-8601 string,
    /// empty when the payload lists none. Every rolling build replaces the
    /// assets, so this is the time of the build.
    pub built_at: String,
    /// The release notes, which is where a dev build records the commit it
    /// was built from. See [`release_commit`].
    pub body: String,
}

impl Release {
    /// Whether the release carries `name`.
    pub fn has_asset(&self, name: &str) -> bool {
        self.assets.iter().any(|asset| asset == name)
    }

    /// The day the release was built, `YYYY-MM-DD`, or empty when nothing
    /// says.
    ///
    /// The notes of a rolling build say `built from commit <sha> on <day>`,
    /// and that comes first; the newest asset is the fallback. The publish
    /// day is not used: the `dev` release keeps the day it was first made.
    pub fn built_day(&self) -> &str {
        match notes_day(&self.body) {
            Some(day) => day,
            None => day_of(&self.built_at),
        }
    }
}

/// The day of an ISO-8601 timestamp, or the whole string when it is not
/// that shape.
fn day_of(timestamp: &str) -> &str {
    match timestamp.split_once('T') {
        Some((day, _)) => day,
        None => timestamp,
    }
}

/// The `YYYY-MM-DD` after `commit <sha> on` in release notes.
fn notes_day(body: &str) -> Option<&str> {
    let at = body.to_ascii_lowercase().find("commit ")?;
    let rest = &body[at..];
    let on = rest.find(" on ")?;
    // The day follows the sha on the same line, not somewhere later.
    if rest[..on].contains('\n') {
        return None;
    }
    let day = rest[on + 4..].get(..10)?;
    let shaped = day.char_indices().all(|(i, c)| match i {
        4 | 7 => c == '-',
        _ => c.is_ascii_digit(),
    });
    shaped.then_some(day)
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
        #[serde(default)]
        updated_at: Option<String>,
    }

    let payload: Payload =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("invalid release JSON: {e}"))?;
    if payload.tag_name.trim().is_empty() {
        return Err(anyhow::anyhow!("the release carries no tag_name"));
    }
    // ISO-8601 timestamps in one zone sort as strings.
    let built_at = payload
        .assets
        .iter()
        .filter_map(|asset| asset.updated_at.as_deref())
        .map(str::trim)
        .max()
        .unwrap_or_default()
        .to_string();
    Ok(Release {
        tag: payload.tag_name.trim().to_string(),
        built_at,
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
pub(super) fn is_commit_sha(word: &str) -> bool {
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
/// to move at all. [`dev_notice_line`](super::dev_notice_line) takes the opposite side of the same
/// question, because an unprompted daily line has to be sure before it speaks.
pub fn dev_update_available(release: &Release, revision: &str) -> bool {
    match release_commit(release) {
        Some(commit) => !same_commit(&commit, revision),
        None => true,
    }
}

/// The asset `target` should download from `release`.
///
/// The asset name holds no version. A payload that lists no assets is taken
/// at its word rather than refused, because varde can derive the name
/// without the list. The error names the asset it looked for; the caller
/// adds the way out, which depends on how the release was picked.
pub fn pick_asset(release: &Release, target: &str) -> Result<String> {
    let name = asset_name(target);
    if release.assets.is_empty() || release.has_asset(&name) {
        return Ok(name);
    }
    Err(anyhow::anyhow!(
        "{} has no {name}, so varde cannot install it on {target}",
        release.tag
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
pub(super) fn release_at(url: &str, timeout: Duration) -> Result<Release> {
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

/// The running executable with links resolved: the file `self update`
/// replaces, not the `vg` link to it.
pub fn running_binary() -> Option<std::path::PathBuf> {
    std::env::current_exe()
        .ok()
        .map(|exe| std::fs::canonicalize(&exe).unwrap_or(exe))
}

/// How this binary was installed, for `varde self version` and `varde doctor`.
pub fn install_method(path: Option<&Path>) -> &'static str {
    install_method_of(path, super::RELEASE_BUILD)
}

/// How a binary at `path` was installed.
///
/// `release_build` is a build-time fact: the release workflow built it, so it
/// came from a release archive wherever it is now. Otherwise it is a local
/// build, and the path can tell more: `cargo install` puts it in
/// `.cargo/bin`, and a `cargo build` leaves it in a checkout's `target/`.
/// Any other path is a `local build`, because varde cannot tell more.
pub fn install_method_of(path: Option<&Path>, release_build: bool) -> &'static str {
    if release_build {
        return "release archive";
    }
    let parts: Vec<String> = path
        .map(|path| {
            path.components()
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
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
        "local build"
    }
}
