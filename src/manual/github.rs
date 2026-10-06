//! The two GitHub questions `varde models add` asks: which branch a
//! repository publishes from, and which commits are on it.
//!
//! Reads, like [`crate::chapcore`]'s, and through the same door
//! ([`crate::github`]): 60 requests an hour per address without a token and
//! 5000 with one, two of them per `models add` and three per `registry pin`
//! line of `varde doctor`.

use crate::error::Result;
use crate::manual::source::Repo;
use serde::Deserialize;
use std::time::Duration;

/// Public GitHub, unless `VARDE_GITHUB_API` points somewhere else.
pub const DEFAULT_API: &str = crate::github::DEFAULT_API;

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
///
/// Every request here is a REST call, so all of them go through
/// [`crate::github`]: the same headers, the token where this run has one, and
/// a rate limit that has run out reported as what it is rather than as a bare
/// `HTTP 403`.
fn get(url: &str, timeout: Duration) -> Result<String> {
    crate::github::get_ok(url, timeout)
}

#[cfg(test)]
mod tests;
