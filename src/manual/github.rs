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

/// One entry of a commit listing: the commit, and the day it was made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The full SHA.
    pub sha: String,
    /// The committer date as `YYYY-MM-DD`, empty when the listing carried
    /// none. A date nobody can print is not worth failing a lookup over.
    pub day: String,
}

/// The newest commits on `branch`, newest first, with their days.
pub fn commit_log(
    api: &str,
    repo: &Repo,
    branch: &str,
    per_page: u32,
    timeout: Duration,
) -> Result<Vec<Commit>> {
    let url = format!(
        "{}/repos/{}/commits?sha={branch}&per_page={per_page}",
        api.trim_end_matches('/'),
        repo.path()
    );
    let body = get(&url, timeout)?;
    parse_commit_log(&body).map_err(|e| anyhow::anyhow!("reading {url}: {e}"))
}

/// The newest commits on `branch`, newest first, as full SHAs.
pub fn commits(
    api: &str,
    repo: &Repo,
    branch: &str,
    per_page: u32,
    timeout: Duration,
) -> Result<Vec<String>> {
    Ok(commit_log(api, repo, branch, per_page, timeout)?
        .into_iter()
        .map(|commit| commit.sha)
        .collect())
}

/// The day one commit was made, `YYYY-MM-DD`.
///
/// One request, and only ever made for a commit the branch listing did not
/// carry: a pin from a branch or a tag of its own has no day until it is
/// asked for by name.
pub fn commit_day(api: &str, repo: &Repo, sha: &str, timeout: Duration) -> Result<String> {
    let url = format!(
        "{}/repos/{}/commits/{sha}",
        api.trim_end_matches('/'),
        repo.path()
    );
    let body = get(&url, timeout)?;
    parse_commit_day(&body).ok_or_else(|| anyhow::anyhow!("{url} names no commit date"))
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

/// One entry of a commit listing, as GitHub writes it. Every other field of
/// the (large) payload is ignored.
#[derive(Debug, Default, Deserialize)]
struct Entry {
    #[serde(default)]
    sha: String,
    #[serde(default)]
    commit: CommitField,
}

#[derive(Debug, Default, Deserialize)]
struct CommitField {
    #[serde(default)]
    committer: Signature,
    #[serde(default)]
    author: Signature,
}

#[derive(Debug, Default, Deserialize)]
struct Signature {
    #[serde(default)]
    date: String,
}

impl Entry {
    /// The day this entry was committed: the committer's, or the author's
    /// where a rewritten commit carries only that.
    fn day(&self) -> String {
        crate::chapcore::day_of(&self.commit.committer.date)
            .or_else(|| crate::chapcore::day_of(&self.commit.author.date))
            .unwrap_or_default()
    }
}

/// Every entry of a commit listing, in the order GitHub gave them, which is
/// newest first.
pub fn parse_commit_log(body: &str) -> Result<Vec<Commit>> {
    let entries: Vec<Entry> =
        serde_json::from_str(body).map_err(|e| anyhow::anyhow!("invalid commit JSON: {e}"))?;
    let commits: Vec<Commit> = entries
        .into_iter()
        .filter(|entry| !entry.sha.trim().is_empty())
        .map(|entry| Commit {
            day: entry.day(),
            sha: entry.sha.trim().to_string(),
        })
        .collect();
    if commits.is_empty() {
        return Err(anyhow::anyhow!("the branch has no commits"));
    }
    Ok(commits)
}

/// The day of a single commit payload, `YYYY-MM-DD`.
pub fn parse_commit_day(body: &str) -> Option<String> {
    let entry: Entry = serde_json::from_str(body).ok()?;
    let day = entry.day();
    (!day.is_empty()).then_some(day)
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
        let log = parse_commit_log(COMMITS).unwrap();
        assert_eq!(log.len(), 2);
        assert!(log[0].sha.starts_with("b1d6c31"));
        assert_eq!(sha_tag(&log[0].sha), "sha-b1d6c31");
        assert_eq!(sha_tag(&log[1].sha), "sha-60b16a2");
    }

    #[test]
    fn an_empty_or_unreadable_commit_listing_is_an_error() {
        assert!(parse_commit_log("[]").is_err());
        assert!(parse_commit_log(r#"{"message":"Not Found"}"#).is_err());
    }

    /// The listing the pin checks read: every commit with the day it was
    /// made, and an entry with neither date left with none.
    #[test]
    fn the_commit_listing_carries_the_day_of_every_commit() {
        const DATED: &str = r#"[
          {"sha": "a6a05ef1111111111111111111111111111111ff",
           "commit": {"committer": {"date": "2026-09-23T08:15:00Z"},
                      "author": {"date": "2026-09-20T08:15:00Z"}}},
          {"sha": "70c07a92222222222222222222222222222222ff",
           "commit": {"author": {"date": "2026-09-08T11:00:00Z"}}},
          {"sha": "00000003333333333333333333333333333333ff", "commit": {"message": "x"}}
        ]"#;
        let log = parse_commit_log(DATED).unwrap();
        assert_eq!(log.len(), 3);
        // The committer date wins over the author's, which is the day the
        // build that carries the commit was made.
        assert_eq!(log[0].day, "2026-09-23");
        // A rewritten commit with only an author date still has a day.
        assert_eq!(log[1].day, "2026-09-08");
        assert_eq!(log[2].day, "", "no date is no day, and not a failure");
        assert_eq!(sha_tag(&log[1].sha), "sha-70c07a9");

        // A listing with no dates at all is still a listing.
        assert!(
            parse_commit_log(COMMITS)
                .unwrap()
                .iter()
                .all(|c| c.day.is_empty())
        );
        assert!(parse_commit_log("[]").is_err());
    }

    #[test]
    fn one_commit_payload_yields_its_day() {
        assert_eq!(
            parse_commit_day(
                r#"{"sha":"70c07a9","commit":{"committer":{"date":"2026-09-08T11:00:00Z"}}}"#
            )
            .as_deref(),
            Some("2026-09-08")
        );
        // A commit GitHub will not talk about has no day to print.
        assert_eq!(parse_commit_day(r#"{"message":"Not Found"}"#), None);
        assert_eq!(parse_commit_day("<html>"), None);
    }

    #[test]
    fn a_short_sha_is_the_first_seven_characters() {
        assert_eq!(sha_tag("b1d6c31f4b2f"), "sha-b1d6c31");
        assert_eq!(sha_tag("abc"), "sha-abc");
    }
}
