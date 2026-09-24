//! The two GitHub questions `chaps models add` asks: which branch a
//! repository publishes from, and which commits are on it.
//!
//! Unauthenticated reads, like [`crate::chapcore`]: 60 requests an hour per
//! address, and two of them per `models add`.

use crate::error::Result;
use crate::manual::source::Repo;
use serde::Deserialize;
use std::time::Duration;

/// Public GitHub, unless `CHAPS_GITHUB_API` points somewhere else.
pub const DEFAULT_API: &str = "https://api.github.com";

/// The media type the REST API answers best in.
const ACCEPT: &str = "application/vnd.github+json";

/// How many commits are asked for when looking for a published build.
///
/// The publish workflow tags every build of the default branch, so the answer
/// is almost always the first commit; the rest are there for a branch whose
/// newest commits changed nothing the workflow builds.
pub const COMMIT_PAGE: u32 = 30;

/// The repository's default branch, e.g. `main`.
pub fn default_branch(api: &str, repo: &Repo, timeout: Duration) -> Result<String> {
    let url = format!("{}/repos/{}", api.trim_end_matches('/'), repo.path());
    let body = get(&url, timeout)?;
    parse_default_branch(&body).map_err(|e| anyhow::anyhow!("reading {url}: {e}"))
}

/// The newest commits on `branch`, newest first, as full SHAs.
pub fn commits(
    api: &str,
    repo: &Repo,
    branch: &str,
    per_page: u32,
    timeout: Duration,
) -> Result<Vec<String>> {
    let url = format!(
        "{}/repos/{}/commits?sha={branch}&per_page={per_page}",
        api.trim_end_matches('/'),
        repo.path()
    );
    let body = get(&url, timeout)?;
    parse_commits(&body).map_err(|e| anyhow::anyhow!("reading {url}: {e}"))
}

/// `default_branch` out of a repository payload; every other field is ignored.
pub fn parse_default_branch(body: &str) -> Result<String> {
    #[derive(Debug, Default, Deserialize)]
    struct RepoPayload {
        #[serde(default)]
        default_branch: String,
    }
    let payload: RepoPayload =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("invalid repository JSON: {e}"))?;
    let branch = payload.default_branch.trim();
    if branch.is_empty() {
        return Err(anyhow::anyhow!("the repository names no default_branch"));
    }
    Ok(branch.to_string())
}

/// The `sha` of every entry of a commit listing, in the order GitHub gave
/// them, which is newest first.
pub fn parse_commits(body: &str) -> Result<Vec<String>> {
    #[derive(Debug, Deserialize)]
    struct Commit {
        #[serde(default)]
        sha: String,
    }
    let commits: Vec<Commit> =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("invalid commit JSON: {e}"))?;
    let shas: Vec<String> = commits
        .into_iter()
        .map(|c| c.sha.trim().to_string())
        .filter(|sha| !sha.is_empty())
        .collect();
    if shas.is_empty() {
        return Err(anyhow::anyhow!("the branch has no commits"));
    }
    Ok(shas)
}

/// The image tag the publish workflow gives a commit: `sha-` plus the short
/// SHA GitHub Actions' metadata action writes, which is seven characters.
pub fn sha_tag(sha: &str) -> String {
    let short: String = sha.chars().take(7).collect();
    format!("sha-{short}")
}

/// GET `url` as text, failing on anything but a 2xx.
fn get(url: &str, timeout: Duration) -> Result<String> {
    let (status, body) = super::get(url, timeout, &[("Accept", ACCEPT)])?;
    if !(200..300).contains(&status) {
        return Err(crate::error::ChapError::Http {
            url: url.to_string(),
            status,
        }
        .into());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repository payload, trimmed to the fields that exist in the real
    /// one plus a few that are ignored.
    const REPO: &str = r#"{
      "id": 1042,
      "name": "chapkit_ghr_model",
      "full_name": "chap-models/chapkit_ghr_model",
      "private": false,
      "default_branch": "main",
      "visibility": "public"
    }"#;

    const COMMITS: &str = r#"[
      {"sha": "b1d6c31f4b2f0d8a0f4c6e2a5e9d3c7b8a1f0e2d", "commit": {"message": "docs"}},
      {"sha": "60b16a2c1e9f4d3a2b8c7d6e5f4a3b2c1d0e9f8a", "commit": {"message": "fix"}}
    ]"#;

    #[test]
    fn the_repository_payload_yields_its_default_branch() {
        assert_eq!(parse_default_branch(REPO).unwrap(), "main");
        assert_eq!(
            parse_default_branch(r#"{"default_branch":" master "}"#).unwrap(),
            "master"
        );
    }

    #[test]
    fn a_repository_without_a_branch_is_an_error() {
        // The rate-limit body parses as JSON and names no branch.
        let err = parse_default_branch(r#"{"message":"API rate limit exceeded"}"#)
            .expect_err("no default_branch");
        assert!(err.to_string().contains("default_branch"), "{err}");
        assert!(parse_default_branch("<html>nope</html>").is_err());
    }

    #[test]
    fn the_commit_listing_yields_its_shas_newest_first() {
        let shas = parse_commits(COMMITS).unwrap();
        assert_eq!(shas.len(), 2);
        assert!(shas[0].starts_with("b1d6c31"));
        assert_eq!(sha_tag(&shas[0]), "sha-b1d6c31");
        assert_eq!(sha_tag(&shas[1]), "sha-60b16a2");
    }

    #[test]
    fn an_empty_or_unreadable_commit_listing_is_an_error() {
        assert!(parse_commits("[]").is_err());
        assert!(parse_commits(r#"{"message":"Not Found"}"#).is_err());
    }

    #[test]
    fn a_short_sha_is_the_first_seven_characters() {
        assert_eq!(sha_tag("b1d6c31f4b2f"), "sha-b1d6c31");
        assert_eq!(sha_tag("abc"), "sha-abc");
    }
}
