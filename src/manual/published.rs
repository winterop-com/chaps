//! Published builds of a repository: which `sha-` tag a branch resolves to,
//! and the commit log a pin is measured against.

use super::source::{self, Source};
use super::{Endpoints, ghcr, github};
use crate::error::Result;

/// The pin this source resolves to: the tag, the commit behind it where that
/// is known, and the branch the entry follows afterwards.
pub(super) fn pin(
    source: &Source,
    endpoints: &Endpoints,
) -> Result<(String, Option<String>, Option<String>)> {
    match source {
        Source::Image(image) | Source::Local(image) => Ok((image.tag.clone(), None, None)),
        Source::Repo(repo) => {
            if endpoints.offline {
                return Err(anyhow::anyhow!(
                    "resolving {} needs GitHub and ghcr, and this run is --offline; \
                     add the image reference instead (ghcr.io/{}:<tag>)",
                    repo.url(),
                    repo.path().to_lowercase()
                ));
            }
            let branch = github::default_branch(&endpoints.github_api, repo, endpoints.timeout)?;
            let shas = github::commits(
                &endpoints.github_api,
                repo,
                &branch,
                github::COMMIT_PAGE,
                endpoints.timeout,
            )?;
            let client = ghcr::Client::anonymous(
                &endpoints.ghcr_url,
                &repo.path().to_lowercase(),
                endpoints.timeout,
            )
            .map_err(|err| no_public_image(err, &repo.url(), &repo.image()))?;
            match pick_published(&shas, &|tag| client.tag_exists(tag))? {
                Some((sha, tag)) => Ok((tag, Some(sha), Some(branch))),
                None => Err(anyhow::anyhow!(
                    "none of the newest {} commits on {branch} has a published `sha-` build at {}; \
                     wait for the publish workflow, or add an image reference with the tag you want",
                    shas.len(),
                    repo.image()
                )),
            }
        }
    }
}

/// A refused anonymous pull token, said as what it means for `models add`:
/// ghcr answers 401 or 403 alike for a private package and for one that was
/// never published, and either way there is no image to run from that URL.
/// Anything else is passed through as it came.
fn no_public_image(err: anyhow::Error, url: &str, image: &str) -> anyhow::Error {
    match err.downcast_ref::<crate::error::ChapError>() {
        Some(crate::error::ChapError::Http { status, .. }) if matches!(status, 401 | 403) => {
            anyhow::anyhow!(
                "{url} publishes no public image at {image} (ghcr refused an anonymous pull, \
                 HTTP {status}); build one in the checkout with `docker build --platform \
                 linux/amd64 -t NAME:dev .` and run `varde models add NAME:dev`"
            )
        }
        _ => err,
    }
}

/// The newest commit of `shas` that has a published image tag, as
/// `(commit, tag)`.
///
/// `exists` is asked in order, newest first, and the first hit wins: a branch
/// whose head changed nothing the publish workflow builds still resolves, to
/// the newest build there is.
pub fn pick_published(
    shas: &[String],
    exists: &dyn Fn(&str) -> Result<bool>,
) -> Result<Option<(String, String)>> {
    for sha in shas {
        let tag = github::sha_tag(sha);
        if exists(&tag)? {
            return Ok(Some((sha.clone(), tag)));
        }
    }
    Ok(None)
}

/// The newest published build of `branch` in a repository, for `varde
/// update`: `(commit, tag)`, or `None` when the branch has none.
pub fn newest_published(
    repository: &str,
    branch: &str,
    endpoints: &Endpoints,
) -> Result<Option<(String, String)>> {
    if endpoints.offline {
        return Err(anyhow::anyhow!(
            "checking {repository} needs the network, and this run is --offline"
        ));
    }
    let Source::Repo(repo) = Source::parse(repository)? else {
        return Err(anyhow::anyhow!(
            "{repository} is not a GitHub repository URL"
        ));
    };
    let shas = github::commits(
        &endpoints.github_api,
        &repo,
        branch,
        github::COMMIT_PAGE,
        endpoints.timeout,
    )?;
    let client = ghcr::Client::anonymous(
        &endpoints.ghcr_url,
        &repo.path().to_lowercase(),
        endpoints.timeout,
    )?;
    pick_published(&shas, &|tag| client.tag_exists(tag))
}

/// The commit log of a branch, and the newest of those commits that has a
/// published image.
///
/// What a pin is measured against: the tag says whether it is the newest
/// build, and the log says where a pin that is not sits against it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchBuilds {
    /// The newest commits of the branch, newest first.
    pub log: Vec<github::Commit>,
    /// The newest of them with a published image, as `(commit, tag)`, or
    /// `None` when none of the ones that were asked about has one.
    pub newest: Option<(String, String)>,
}

/// The default branch of a repository URL, e.g. `main`.
pub fn default_branch_of(repository: &str, endpoints: &Endpoints) -> Result<String> {
    let repo = repo_of(repository, endpoints)?;
    github::default_branch(&endpoints.github_api, &repo, endpoints.timeout)
}

/// The newest `per_page` commits of `branch`, and the newest of the first
/// `probes` of them that ghcr has an image for.
///
/// `probes` bounds the registry half: the publish workflow builds every push,
/// so the newest commit is normally the first and only tag asked about, and a
/// branch that has published nothing recently must not turn one doctor line
/// into a page of registry round trips.
pub fn branch_builds(
    repository: &str,
    branch: &str,
    per_page: u32,
    probes: usize,
    endpoints: &Endpoints,
) -> Result<BranchBuilds> {
    let repo = repo_of(repository, endpoints)?;
    let log = github::commit_log(
        &endpoints.github_api,
        &repo,
        branch,
        per_page,
        endpoints.timeout,
    )?;
    let client = ghcr::Client::anonymous(
        &endpoints.ghcr_url,
        &repo.path().to_lowercase(),
        endpoints.timeout,
    )?;
    let asked: Vec<String> = log
        .iter()
        .take(probes)
        .map(|commit| commit.sha.clone())
        .collect();
    let newest = pick_published(&asked, &|tag| client.tag_exists(tag))?;
    Ok(BranchBuilds { log, newest })
}

/// The day one commit of a repository was made, `YYYY-MM-DD`.
pub fn commit_day(repository: &str, sha: &str, endpoints: &Endpoints) -> Result<String> {
    let repo = repo_of(repository, endpoints)?;
    github::commit_day(&endpoints.github_api, &repo, sha, endpoints.timeout)
}

/// The `<owner>/<repo>` a lookup is made against, with the two things that
/// rule one out: `--offline`, and a source that names no repository.
fn repo_of(repository: &str, endpoints: &Endpoints) -> Result<source::Repo> {
    if endpoints.offline {
        return Err(anyhow::anyhow!(
            "reading {repository} needs the network, and this run is --offline"
        ));
    }
    match Source::parse(repository)? {
        Source::Repo(repo) => Ok(repo),
        Source::Image(_) | Source::Local(_) => Err(anyhow::anyhow!(
            "{repository} is not a GitHub repository URL"
        )),
    }
}
