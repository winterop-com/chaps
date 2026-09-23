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

/// Release endpoint for the newest final release; prereleases and drafts are
/// excluded by GitHub itself, which is exactly what the `latest` image tag
/// follows. Unauthenticated requests are allowed, at 60 per hour per address.
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/dhis2-chap/chap-core/releases/latest";

/// The standalone compose file published with every chap-core tag.
pub const COMPOSE_FILE: &str = "compose.ghcr.yml";

/// `User-Agent` sent with every request, matching the registry fetch.
const USER_AGENT: &str = concat!("chaps-cli/", env!("CARGO_PKG_VERSION"));

/// Raw URL of [`COMPOSE_FILE`] at `tag`.
pub fn compose_url(tag: &str) -> String {
    format!("https://raw.githubusercontent.com/{REPO}/{tag}/{COMPOSE_FILE}")
}

/// The tag of the newest chap-core release, e.g. `v2.3.1`.
pub fn latest_release(timeout: Duration) -> Result<String> {
    let body = get(
        LATEST_RELEASE_URL,
        timeout,
        Some("application/vnd.github+json"),
    )?;
    parse_release(&body)
        .map_err(|e| anyhow::anyhow!("reading the release list at {LATEST_RELEASE_URL}: {e}"))
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
    let mut response = request.call().map_err(|e| map_error(url, e))?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("reading the response body from {url}: {e}"))
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
        assert!(LATEST_RELEASE_URL.contains(REPO));
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
        let embedded = crate::compose::render::render_base(&crate::compose::spec::BaseSpec {
            cli_version: "0.1.0".to_string(),
            upstream: None,
        });
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
        let err = map_error(LATEST_RELEASE_URL, ureq::Error::StatusCode(403));
        match err.downcast_ref::<ChapError>() {
            Some(ChapError::Http { url, status }) => {
                assert_eq!(url, LATEST_RELEASE_URL);
                assert_eq!(*status, 403);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }
}
