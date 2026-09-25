//! chap-core itself: which release its images are tagged with, and the compose
//! file every tag publishes.
//!
//! [`crate::registry`] is the equivalent for the *model* marketplace. Here the
//! questions are narrower: the GitHub release API answers "what is the newest
//! version", and `compose.ghcr.yml` at a tag is the upstream source of truth
//! for the base stack, so a deployment can be pinned to a release and still
//! carry exactly the compose file that release shipped.

use crate::error::{ChapError, Result};
use serde::Deserialize;
use std::time::Duration;

/// GitHub repository the chap-core images and the compose file come from.
pub const REPO: &str = "dhis2-chap/chap-core";

/// Public GitHub, unless `CHAPS_GITHUB_API` points somewhere else.
pub const DEFAULT_API: &str = "https://api.github.com";

/// Public raw.githubusercontent.com, unless `CHAPS_GITHUB_RAW` points
/// somewhere else. Both are test hooks and nothing on the command line moves
/// them; see `docs/development.md`.
pub const DEFAULT_RAW: &str = "https://raw.githubusercontent.com";

/// The standalone compose file published with every chap-core tag.
pub const COMPOSE_FILE: &str = "compose.ghcr.yml";

/// The moving image tag `chaps init` resolves to the release it points at.
pub const LATEST_TAG: &str = "latest";

/// The moving tags chap-core publishes, newest build first. `latest` follows
/// the newest release; the other two follow their branches.
pub const MOVING_TAGS: &[&str] = &["dev", "master", LATEST_TAG];

/// How many releases `chaps update --list-tags` lists.
pub const LIST_LIMIT: usize = 10;

/// The media type the REST API answers best in.
const ACCEPT_JSON: &str = "application/vnd.github+json";

/// `User-Agent` sent with every request, matching the registry fetch.
const USER_AGENT: &str = concat!("chaps-cli/", env!("CARGO_PKG_VERSION"));

/// Base of the GitHub REST API this run talks to.
pub fn api_base() -> String {
    base("CHAPS_GITHUB_API", DEFAULT_API)
}

/// Base of the raw file host this run talks to.
pub fn raw_base() -> String {
    base("CHAPS_GITHUB_RAW", DEFAULT_RAW)
}

fn base(var: &str, default: &str) -> String {
    base_of(std::env::var(var).ok(), default)
}

/// [`base`] with the variable already read, so the rule is testable without
/// touching the environment of a process running tests in parallel.
fn base_of(value: Option<String>, default: &str) -> String {
    value
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// Release endpoint for the newest final release; prereleases and drafts are
/// excluded by GitHub itself, which is exactly what the `latest` image tag
/// follows. Unauthenticated requests are allowed, at 60 per hour per address.
pub fn latest_release_url() -> String {
    format!("{}/repos/{REPO}/releases/{LATEST_TAG}", api_base())
}

/// The newest `limit` releases, newest first.
pub fn releases_url(limit: usize) -> String {
    format!("{}/repos/{REPO}/releases?per_page={limit}", api_base())
}

/// One release by its tag: 200 when that release exists, 404 when it does not.
pub fn release_url(tag: &str) -> String {
    format!("{}/repos/{REPO}/releases/tags/{tag}", api_base())
}

/// The newest commit of one branch, which is what a moving tag is built from.
pub fn branch_url(branch: &str) -> String {
    format!(
        "{}/repos/{REPO}/commits?sha={branch}&per_page=1",
        api_base()
    )
}

/// Raw URL of [`COMPOSE_FILE`] at `tag`.
pub fn compose_url(tag: &str) -> String {
    format!("{}/{REPO}/{tag}/{COMPOSE_FILE}", raw_base())
}

/// The tag of the newest chap-core release, e.g. `v2.3.1`.
pub fn latest_release(timeout: Duration) -> Result<String> {
    let url = latest_release_url();
    let body = get(&url, timeout, Some(ACCEPT_JSON))?;
    parse_release(&body).map_err(|e| anyhow::anyhow!("reading the release list at {url}: {e}"))
}

/// One chap-core release: the tag it names and the day it was published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag, e.g. `v2.3.1`.
    pub tag: String,
    /// `YYYY-MM-DD`, or empty when the payload carried no date.
    pub published: String,
}

/// The newest chap-core releases, newest first.
///
/// Only the tags that are versions ([`release_version`]) come back: the
/// repository also tags other things, and a tag that is not a version is not
/// somewhere `chaps update --chap-tag` can move a deployment to as a release.
pub fn releases(limit: usize, timeout: Duration) -> Result<Vec<Release>> {
    // Asked for wider than the list that is printed: the drafts, the
    // prereleases and the tags that are not versions are dropped here, not by
    // GitHub, so a page of exactly `limit` entries could come back short.
    let url = releases_url((limit * 3).min(100));
    let body = get(&url, timeout, Some(ACCEPT_JSON))?;
    let mut list = parse_releases(&body)
        .map_err(|e| anyhow::anyhow!("reading the release list at {url}: {e}"))?;
    list.truncate(limit);
    Ok(list)
}

/// Whether chap-core has released `tag`.
///
/// `Ok(false)` is GitHub saying there is no such release; an error is this
/// run not having been able to ask, which is a different answer and the
/// caller treats it as one.
pub fn release_exists(tag: &str, timeout: Duration) -> Result<bool> {
    let url = release_url(tag);
    match get(&url, timeout, Some(ACCEPT_JSON)) {
        Ok(_) => Ok(true),
        Err(err) => match err.downcast_ref::<ChapError>() {
            Some(ChapError::Http { status: 404, .. }) => Ok(false),
            _ => Err(err),
        },
    }
}

/// The day `branch` was last committed to, `YYYY-MM-DD`.
///
/// Best effort and never fatal: it decorates one column of
/// `chaps update --list-tags`, and a branch GitHub will not talk about simply
/// has no date to print.
pub fn branch_updated(branch: &str, timeout: Duration) -> Option<String> {
    let body = get(&branch_url(branch), timeout, Some(ACCEPT_JSON)).ok()?;
    parse_commit_date(&body)
}

/// `[0].commit.committer.date` of a commit list, as a day.
pub fn parse_commit_date(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let commit = value.as_array()?.first()?.get("commit")?;
    for who in ["committer", "author"] {
        if let Some(date) = commit
            .get(who)
            .and_then(|w| w.get("date"))
            .and_then(|d| d.as_str())
            && let Some(day) = day_of(date)
        {
            return Some(day);
        }
    }
    None
}

/// The `YYYY-MM-DD` of an RFC 3339 timestamp, or `None` when it is not one.
fn day_of(timestamp: &str) -> Option<String> {
    let day = timestamp.trim().split(['T', 't', ' ']).next()?;
    let mut fields = day.split('-');
    let ok = fields
        .next()
        .is_some_and(|y| y.len() == 4 && y.chars().all(|c| c.is_ascii_digit()))
        && fields.next().is_some_and(|m| m.len() == 2)
        && fields.next().is_some_and(|d| d.len() == 2)
        && fields.next().is_none();
    ok.then(|| day.to_string())
}

/// Download chap-core's `compose.ghcr.yml` at `tag`.
///
/// The text is validated before it is returned ([`validate_compose`]), so a
/// caller that gets an `Ok` has something it can write as `compose.yml`.
pub fn fetch_compose(tag: &str, timeout: Duration) -> Result<String> {
    let url = compose_url(tag);
    let body = get(&url, timeout, None)?;
    validate_compose(&body).map_err(|e| anyhow::anyhow!("{url}: {e}"))?;
    Ok(body)
}

/// `tag_name` of a GitHub release payload.
///
/// Every other field of the (large) response is ignored: the tag is what names
/// both the images and the compose file.
pub fn parse_release(body: &str) -> Result<String> {
    #[derive(Debug, Default, Deserialize)]
    struct Release {
        #[serde(default)]
        tag_name: String,
    }
    let release: Release =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("invalid release JSON: {e}"))?;
    if release.tag_name.trim().is_empty() {
        return Err(anyhow::anyhow!("the release carries no tag_name"));
    }
    Ok(release.tag_name.trim().to_string())
}

/// The releases of a release-list payload, newest first.
///
/// Drafts and prereleases are left out - they are not what the `latest` image
/// tag follows - and so is every tag that is not a version, because a
/// deployment moved to one of those would be pinned to something `update`
/// could never compare against anything.
pub fn parse_releases(body: &str) -> Result<Vec<Release>> {
    #[derive(Debug, Default, Deserialize)]
    struct Entry {
        #[serde(default)]
        tag_name: String,
        #[serde(default)]
        published_at: String,
        #[serde(default)]
        draft: bool,
        #[serde(default)]
        prerelease: bool,
    }
    let entries: Vec<Entry> =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("invalid release JSON: {e}"))?;
    Ok(entries
        .into_iter()
        .filter(|entry| !entry.draft && !entry.prerelease)
        .filter(|entry| release_version(entry.tag_name.trim()).is_some())
        .map(|entry| Release {
            tag: entry.tag_name.trim().to_string(),
            published: day_of(&entry.published_at).unwrap_or_default(),
        })
        .collect())
}

/// What a chap-core tag is, as far as moving a deployment's pin goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagKind {
    /// `vX.Y.Z`: a point on the version line, and what `update` moves forward.
    Release,
    /// `latest`, `master`, `dev`: the same name, a different image tomorrow.
    Moving,
    /// A `sha-` build or anything else: an exact pin nothing here moves.
    Other,
}

impl TagKind {
    /// How the kind reads in a table cell.
    pub fn label(self) -> &'static str {
        match self {
            TagKind::Release => "release",
            TagKind::Moving => "moving",
            TagKind::Other => "exact",
        }
    }
}

/// Which of the three `tag` is.
pub fn tag_kind(tag: &str) -> TagKind {
    if is_moving_tag(tag) {
        return TagKind::Moving;
    }
    if release_version(tag).is_some() {
        return TagKind::Release;
    }
    TagKind::Other
}

/// Whether moving the pin from `from` to `to` goes backwards, which is the
/// direction that can run an older schema against a newer database.
///
/// Two cases, and only two: an older release than the one that is pinned, and
/// a release after a moving tag, because `dev` and `master` are ahead of
/// every release and `latest` is the newest of them. Everything else - a
/// newer release, a release to a moving tag, one moving tag to another, and
/// anything involving a `sha-` build that is not on the version line at all -
/// is not something this can call backwards, so it does not.
pub fn is_backwards(from: &str, to: &str) -> bool {
    match (tag_kind(from), tag_kind(to)) {
        (TagKind::Release, TagKind::Release) => is_newer(from, to),
        (TagKind::Moving, TagKind::Release) => true,
        _ => false,
    }
}

/// Whether `text` is usable as the base `compose.yml`.
///
/// Three things matter: it has to parse as YAML, it has to define the `chap`
/// service, and that service's image has to keep reading `CHAP_IMAGE_TAG`, or
/// the tag this CLI records in `.chaps/project.yaml` and `.env` would pin
/// nothing at all.
pub fn validate_compose(text: &str) -> Result<()> {
    let doc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(text).map_err(|e| anyhow::anyhow!("not valid YAML: {e}"))?;
    let chap = doc
        .get("services")
        .and_then(|services| services.get("chap"))
        .ok_or_else(|| anyhow::anyhow!("no `services.chap` in the document"))?;
    let image = chap
        .get("image")
        .and_then(|image| image.as_str())
        .ok_or_else(|| anyhow::anyhow!("`services.chap` has no image"))?;
    if !image.contains("${CHAP_IMAGE_TAG") {
        return Err(anyhow::anyhow!(
            "the chap image is `{image}`, which does not read CHAP_IMAGE_TAG"
        ));
    }
    Ok(())
}

/// A tag that can point at a different image tomorrow.
pub fn is_moving_tag(tag: &str) -> bool {
    matches!(
        tag,
        "latest" | "master" | "main" | "dev" | "edge" | "nightly"
    )
}

/// `(major, minor, patch)` of a release tag such as `v2.3.1`.
///
/// A leading `v` is optional and anything from the first `-` or `+` on (a
/// prerelease or build suffix) is ignored; anything that is not three numeric
/// components is not a release tag at all, which is how a moving tag or a
/// `sha-` build is told apart from a version.
pub fn release_version(tag: &str) -> Option<(u64, u64, u64)> {
    let core = tag.strip_prefix('v').unwrap_or(tag);
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let version = (next()?, next()?, next()?);
    match parts.next() {
        None => Some(version),
        Some(_) => None,
    }
}

/// Whether `candidate` is a newer release than `current`.
///
/// Both have to be release tags: a moving tag is never "older" than a release,
/// because it is not a point on the same line.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (release_version(candidate), release_version(current)) {
        (Some(new), Some(old)) => new > old,
        _ => false,
    }
}

/// SHA-256 of `bytes`, as lowercase hex.
///
/// Written out here rather than added as a dependency: it is one function, it
/// is only ever used to notice that a cached file changed since it was
/// fetched, and the six release targets (one of them a static musl build) keep
/// the dependency set they have.
pub fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut hash: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    // The padded message: the bytes, a 1 bit, zeroes up to 56 mod 64, then the
    // original length in bits as a big-endian u64.
    let mut message = Vec::with_capacity(bytes.len() + 72);
    message.extend_from_slice(bytes);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(bytes.len() as u64 * 8).to_be_bytes());

    for block in message.chunks(64) {
        let mut w = [0u32; 64];
        for (word, quad) in w.iter_mut().zip(block.chunks(4)) {
            *word = u32::from_be_bytes([quad[0], quad[1], quad[2], quad[3]]);
        }
        for i in 16..64 {
            let a = w[i - 15];
            let b = w[i - 2];
            let s0 = a.rotate_right(7) ^ a.rotate_right(18) ^ (a >> 3);
            let s1 = b.rotate_right(17) ^ b.rotate_right(19) ^ (b >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut v = hash;
        for (k, word) in K.iter().zip(w.iter()) {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(*k)
                .wrapping_add(*word);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (slot, value) in hash.iter_mut().zip(v.iter()) {
            *slot = slot.wrapping_add(*value);
        }
    }

    hash.iter().map(|word| format!("{word:08x}")).collect()
}

/// GET `url` as text, with the same timeout semantics as the registry fetch:
/// one global deadline covering connect, send and receive.
fn get(url: &str, timeout: Duration, accept: Option<&str>) -> Result<String> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(USER_AGENT)
            .build(),
    );
    let mut request = agent.get(url);
    if let Some(accept) = accept {
        request = request.header("Accept", accept);
    }
    let started = std::time::Instant::now();
    let mut response = request.call().map_err(|e| {
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
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("reading the response body from {url}: {e}"))?;
    crate::output::debug(&format!("  {}", crate::output::trace_body(&body)));
    Ok(body)
}

/// A non-2xx response is a [`ChapError::Http`] (the rate limit and a missing
/// tag both arrive that way); everything else is a transport failure that
/// keeps the URL as context.
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

    /// The shape of the release payload, trimmed to the fields that exist in
    /// the real one plus a few we ignore.
    const RELEASE: &str = r#"{
      "url": "https://api.github.com/repos/dhis2-chap/chap-core/releases/123",
      "tag_name": "v2.3.1",
      "name": "v2.3.1",
      "draft": false,
      "prerelease": false,
      "published_at": "2026-09-21T10:00:25Z",
      "assets": []
    }"#;

    #[test]
    fn the_release_payload_yields_its_tag() {
        assert_eq!(parse_release(RELEASE).unwrap(), "v2.3.1");
        assert_eq!(
            parse_release(r#"{"tag_name":" v1.0.0 "}"#).unwrap(),
            "v1.0.0"
        );
    }

    #[test]
    fn a_release_without_a_tag_is_an_error() {
        // The rate-limit body parses as JSON but carries no tag.
        let err = parse_release(r#"{"message":"API rate limit exceeded"}"#)
            .expect_err("no tag_name to read");
        assert!(err.to_string().contains("tag_name"));
        assert!(parse_release(r#"{"tag_name":""}"#).is_err());
        assert!(parse_release("<html>nope</html>").is_err());
    }

    #[test]
    fn the_urls_name_the_repository_and_the_tag() {
        assert_eq!(
            compose_url("v2.3.1"),
            "https://raw.githubusercontent.com/dhis2-chap/chap-core/v2.3.1/compose.ghcr.yml"
        );
        assert!(compose_url("master").ends_with("/master/compose.ghcr.yml"));
        assert!(latest_release_url().contains(REPO));
        assert!(latest_release_url().starts_with(DEFAULT_API));
        assert_eq!(
            release_url("v2.3.1"),
            "https://api.github.com/repos/dhis2-chap/chap-core/releases/tags/v2.3.1"
        );
        assert!(releases_url(10).ends_with("/releases?per_page=10"));
        assert!(branch_url("dev").ends_with("/commits?sha=dev&per_page=1"));
    }

    #[test]
    fn release_tags_parse_and_everything_else_does_not() {
        assert_eq!(release_version("v2.3.1"), Some((2, 3, 1)));
        assert_eq!(release_version("2.3.1"), Some((2, 3, 1)));
        assert_eq!(release_version("v10.0.0"), Some((10, 0, 0)));
        assert_eq!(release_version("v2.4.0-rc.1"), Some((2, 4, 0)));
        for bad in [
            "latest",
            "master",
            "dev",
            "sha-fa880a1",
            "v2.3",
            "v2.3.1.4",
            "",
        ] {
            assert_eq!(release_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn newer_compares_the_three_components() {
        assert!(is_newer("v2.3.1", "v2.3.0"));
        assert!(is_newer("v2.4.0", "v2.3.9"));
        assert!(is_newer("v3.0.0", "v2.99.99"));
        assert!(is_newer("v2.10.0", "v2.9.0"), "not a string comparison");
        assert!(!is_newer("v2.3.1", "v2.3.1"));
        assert!(!is_newer("v2.3.0", "v2.3.1"));
        // A moving tag is not a point on the version line.
        assert!(!is_newer("v2.3.1", "latest"));
        assert!(!is_newer("latest", "v2.3.1"));
    }

    #[test]
    fn moving_tags_are_the_ones_that_can_change_under_us() {
        for tag in ["latest", "master", "main", "dev"] {
            assert!(is_moving_tag(tag), "{tag}");
        }
        for tag in ["v2.3.1", "sha-fa880a1", "v1.0.0-rc.1"] {
            assert!(!is_moving_tag(tag), "{tag}");
        }
    }

    #[test]
    fn the_vendored_compose_file_validates() {
        let embedded =
            crate::compose::render::render_base(&crate::compose::spec::BaseSpec { upstream: None });
        validate_compose(&embedded).expect("the copy we ship is usable");
    }

    #[test]
    fn a_compose_file_that_would_lose_the_pin_is_rejected() {
        let err = validate_compose("not: [yaml").expect_err("unparseable");
        assert!(err.to_string().contains("not valid YAML"), "{err}");

        let err =
            validate_compose("services:\n  worker:\n    image: x\n").expect_err("no chap service");
        assert!(err.to_string().contains("services.chap"), "{err}");

        let err =
            validate_compose("services:\n  chap:\n    restart: always\n").expect_err("no image");
        assert!(err.to_string().contains("no image"), "{err}");

        let err = validate_compose("services:\n  chap:\n    image: ghcr.io/x/chap-core:latest\n")
            .expect_err("a hard-coded tag defeats CHAP_IMAGE_TAG");
        assert!(err.to_string().contains("CHAP_IMAGE_TAG"), "{err}");

        validate_compose("services:\n  chap:\n    image: ghcr.io/x:${CHAP_IMAGE_TAG:-latest}\n")
            .expect("the minimal acceptable document");
    }

    #[test]
    fn sha256_matches_the_published_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        // 1_000_000 bytes crosses many blocks and exercises the length field.
        assert_eq!(
            sha256_hex(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
        // And the value is stable for something the CLI actually hashes.
        assert_eq!(sha256_hex("services: {}\n".as_bytes()).len(), 64);
    }

    /// Port 9 is the discard service, so this covers the transport-error path
    /// without a network.
    #[test]
    fn an_unreachable_host_is_an_error_not_a_hang() {
        let err = get(
            "http://127.0.0.1:9/releases/latest",
            Duration::from_secs(2),
            None,
        )
        .expect_err("nothing is listening on port 9");
        assert!(err.to_string().contains("127.0.0.1:9"), "{err}");
    }

    #[test]
    fn a_status_code_becomes_a_typed_http_error() {
        let url = latest_release_url();
        let err = map_error(&url, ureq::Error::StatusCode(403));
        match err.downcast_ref::<ChapError>() {
            Some(ChapError::Http { url: at, status }) => {
                assert_eq!(*at, url);
                assert_eq!(*status, 403);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    /// The list page, trimmed to the fields that decide what is listed.
    const RELEASES: &str = r#"[
      {"tag_name":"v2.3.1","published_at":"2026-09-21T10:00:25Z","draft":false,"prerelease":false},
      {"tag_name":"v2.4.0-rc.1","published_at":"2026-09-20T10:00:25Z","draft":false,"prerelease":true},
      {"tag_name":"chap-1.1.1","published_at":"2026-08-26T18:15:28Z","draft":false,"prerelease":false},
      {"tag_name":"v2.3.0","published_at":"2026-09-11T09:20:41Z","draft":false,"prerelease":false},
      {"tag_name":"v2.2.0","draft":true,"prerelease":false}
    ]"#;

    #[test]
    fn the_release_list_keeps_the_versions_and_drops_the_rest() {
        let list = parse_releases(RELEASES).unwrap();
        assert_eq!(
            list,
            vec![
                Release {
                    tag: "v2.3.1".into(),
                    published: "2026-09-21".into()
                },
                Release {
                    tag: "v2.3.0".into(),
                    published: "2026-09-11".into()
                },
            ]
        );
        // A page with nothing usable in it is an empty list, not an error.
        assert!(parse_releases("[]").unwrap().is_empty());
        assert!(parse_releases(r#"{"message":"rate limit"}"#).is_err());
    }

    #[test]
    fn a_commit_page_yields_the_day_of_its_newest_commit() {
        let body = r#"[{"sha":"abc","commit":{"committer":{"date":"2026-09-24T08:15:00Z"}}}]"#;
        assert_eq!(parse_commit_date(body).as_deref(), Some("2026-09-24"));
        // The author's date is the fallback, and a page with neither has none.
        let author = r#"[{"commit":{"author":{"date":"2026-09-23T08:15:00Z"}}}]"#;
        assert_eq!(parse_commit_date(author).as_deref(), Some("2026-09-23"));
        assert_eq!(parse_commit_date("[]"), None);
        assert_eq!(parse_commit_date(r#"[{"commit":{}}]"#), None);
        assert_eq!(day_of("not-a-date"), None);
        assert_eq!(day_of("2026-9-1T00:00:00Z"), None);
    }

    #[test]
    fn a_tag_is_a_release_a_moving_tag_or_an_exact_pin() {
        assert_eq!(tag_kind("v2.3.1"), TagKind::Release);
        assert_eq!(tag_kind("2.3.1"), TagKind::Release);
        for moving in MOVING_TAGS {
            assert_eq!(tag_kind(moving), TagKind::Moving, "{moving}");
        }
        assert_eq!(tag_kind("sha-fa880a1"), TagKind::Other);
        assert_eq!(tag_kind(""), TagKind::Other);
        assert_eq!(TagKind::Release.label(), "release");
        assert_eq!(TagKind::Moving.label(), "moving");
        assert_eq!(TagKind::Other.label(), "exact");
    }

    #[test]
    fn only_an_older_release_counts_as_moving_backwards() {
        // Release to release, by version and not by string.
        assert!(is_backwards("v2.3.1", "v2.3.0"));
        assert!(is_backwards("v2.10.0", "v2.9.0"));
        assert!(!is_backwards("v2.3.0", "v2.3.1"));
        assert!(!is_backwards("v2.3.1", "v2.3.1"));
        // A moving tag is ahead of every release, `latest` included.
        for moving in MOVING_TAGS {
            assert!(is_backwards(moving, "v2.3.1"), "{moving}");
            assert!(!is_backwards("v2.3.1", moving), "{moving}");
        }
        assert!(!is_backwards("dev", "master"));
        // A `sha-` build is not on the line at all, in either direction.
        assert!(!is_backwards("sha-fa880a1", "v2.3.1"));
        assert!(!is_backwards("v2.3.1", "sha-fa880a1"));
        assert!(!is_backwards("dev", "sha-fa880a1"));
    }

    #[test]
    fn the_hosts_follow_the_test_hooks() {
        let hook = Some("http://127.0.0.1:18099/".to_string());
        assert_eq!(base_of(hook, DEFAULT_API), "http://127.0.0.1:18099");
        // Unset, empty and whitespace all mean the public host.
        assert_eq!(base_of(None, DEFAULT_API), DEFAULT_API);
        assert_eq!(base_of(Some(String::new()), DEFAULT_RAW), DEFAULT_RAW);
        assert_eq!(base_of(Some("  ".into()), DEFAULT_RAW), DEFAULT_RAW);
    }
}
