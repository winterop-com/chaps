//! Models that are not in the marketplace: resolving what `chaps models add`
//! was given into an entry a deployment can run.
//!
//! The marketplace answers three questions about a model: which image, which
//! tag, and how the container has to be run. For a repository that is not in
//! the catalogue, those answers have to come from the repository and the
//! image themselves - GitHub for the commits, ghcr for the published tags and
//! the image config, and docker only where neither can say. What comes out is
//! recorded in `.chaps/models-manual.yaml` and synthesised back into a
//! marketplace-shaped entry ([`crate::project::ManualModel::to_model`]), so
//! every command downstream treats it like any other model.

pub mod ghcr;
pub mod github;
pub mod source;

use crate::compose::overrides;
use crate::error::Result;
use crate::registry::VersionSelector;
use source::Source;
use std::time::Duration;

/// Default network timeout for the lookups `models add` makes.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// `User-Agent` sent with every request, matching the registry fetch.
const USER_AGENT: &str = concat!("chaps-cli/", env!("CARGO_PKG_VERSION"));

/// Where the lookups go, and whether they may go anywhere at all.
///
/// The two URLs are overridable so the end-to-end tests can stand a local
/// server in for GitHub and ghcr; see `docs/development.md`. Nothing on the
/// command line moves them, because an operator who wants a different
/// registry wants a different image reference.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub github_api: String,
    pub ghcr_url: String,
    /// `--offline`: the network is not to be touched.
    pub offline: bool,
    /// Whether the local docker daemon may be asked about an image at all:
    /// `docker image inspect` for a config the registry would not give up,
    /// and the `id -u` probe that pulls and runs one. Off in the tests, whose
    /// answers must not depend on what this machine has pulled.
    pub docker_probe: bool,
    pub timeout: Duration,
}

impl Default for Endpoints {
    fn default() -> Endpoints {
        Endpoints {
            github_api: github::DEFAULT_API.to_string(),
            ghcr_url: ghcr::DEFAULT_URL.to_string(),
            offline: false,
            docker_probe: true,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl Endpoints {
    /// The endpoints this run uses: the public ones, unless the test hooks
    /// say otherwise.
    pub fn from_env(offline: bool) -> Endpoints {
        Endpoints {
            github_api: std::env::var("CHAPS_GITHUB_API")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| github::DEFAULT_API.to_string()),
            ghcr_url: std::env::var("CHAPS_GHCR_URL")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| ghcr::DEFAULT_URL.to_string()),
            offline,
            docker_probe: !matches!(
                std::env::var("CHAPS_NO_DOCKER_PROBE").as_deref(),
                Ok("1") | Ok("true")
            ),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// What `chaps models add` was asked for, before anything was looked up.
#[derive(Debug, Clone, Default)]
pub struct AddRequest {
    /// The repository URL or image reference, as typed.
    pub source: String,
    pub id: Option<String>,
    pub service_id: Option<String>,
    pub display_name: Option<String>,
    pub data_dir: Option<String>,
    pub user: Option<String>,
    /// `--runtime-amd64`, for an image whose index this CLI cannot read.
    pub runtime_amd64: bool,
}

/// Where one resolved value came from, so the report can say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A flag on the command line.
    Flag,
    /// The image's own config.
    Image,
    /// A `docker run` that asked the image itself.
    Probe,
    /// The chapkit default, because nothing else said.
    Default,
}

impl Origin {
    /// How the origin reads in the report.
    pub fn label(self) -> &'static str {
        match self {
            Origin::Flag => "given on the command line",
            Origin::Image => "from the image config",
            Origin::Probe => "from a docker probe",
            Origin::Default => "the chapkit default",
        }
    }
}

/// The pull the uid probe needs first: `reference -> did it work`.
pub type PullFn<'a> = &'a dyn Fn(&str) -> bool;
/// The uid probe itself: `(reference, account name) -> (uid, gid)`.
pub type ProbeFn<'a> = &'a dyn Fn(&str, &str) -> Option<(u32, u32)>;

/// Everything `models add` needs to write the entry, and nothing written yet.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub source: Source,
    pub id: String,
    pub service_id: String,
    pub display_name: String,
    /// Tagless image reference.
    pub image: String,
    /// The pin: a `sha-` tag, or `@sha256:...` for a digest.
    pub tag: String,
    /// The commit the tag was built from, where it is known.
    pub commit: Option<String>,
    /// The branch this entry follows, or `None` for a pinned one.
    pub follow: Option<String>,
    pub data_dir: String,
    pub data_dir_from: Origin,
    pub user: String,
    pub user_from: Origin,
    pub runtime_amd64: bool,
    /// What the resolution had to guess at, in the order it came up.
    pub notes: Vec<String>,
}

impl Resolved {
    /// The image reference this entry pins.
    pub fn image_ref(&self) -> String {
        crate::compose::image_ref(&self.image, &self.tag)
    }

    /// How the version is asked for when the entry is enabled: a channel for
    /// an entry that follows a branch, an exact pin for one that does not.
    ///
    /// Both resolve to the same single version of the synthesised model; the
    /// difference is what `.chaps/models.yaml` records, and therefore what
    /// `chaps update` sees.
    pub fn selector(&self) -> VersionSelector {
        match self.follow {
            Some(_) => VersionSelector::Channel(crate::registry::Channel::Latest),
            None => VersionSelector::Exact(self.tag.clone()),
        }
    }
}

/// What an add request is called, before anything is looked up.
///
/// Separate from [`resolve`] because these are the answers a collision is
/// decided on, and asking GitHub, ghcr and docker for an entry whose id is
/// already taken would mean pulling an image to be told to pick another name.
#[derive(Debug, Clone)]
pub struct Names {
    pub source: Source,
    pub id: String,
    pub service_id: String,
    pub display_name: String,
}

/// The id, service name and display name this request would be recorded
/// under, with nothing looked up.
pub fn names(req: &AddRequest) -> Result<Names> {
    let source = Source::parse(&req.source)?;
    let id = match &req.id {
        Some(id) => id.trim().to_string(),
        None => source.default_id(),
    };
    source::check_id(&id)?;
    let service_id = match &req.service_id {
        Some(service_id) => service_id.trim().to_string(),
        None => source::default_service_id(&id),
    };
    source::check_service_id(&service_id)?;
    let display_name = match &req.display_name {
        Some(name) => name.trim().to_string(),
        None => default_display_name(&source),
    };
    Ok(Names {
        source,
        id,
        service_id,
        display_name,
    })
}

/// Resolve an add request, writing nothing.
///
/// The order is the order the answers depend on each other: the source, then
/// the tag it pins, then what the image at that tag says about how it runs.
pub fn resolve(req: &AddRequest, endpoints: &Endpoints) -> Result<Resolved> {
    let Names {
        source,
        id,
        service_id,
        display_name,
    } = names(req)?;

    let image = source.image();
    let mut notes = Vec::new();
    let (tag, commit, follow) = pin(&source, endpoints)?;
    let reference = crate::compose::image_ref(&image, &tag);

    let config = image_config(&source, &reference, endpoints, &mut notes)?;
    let data_dir = match (&req.data_dir, &config) {
        (Some(dir), _) => (dir.trim().to_string(), Origin::Flag),
        (None, Some(config)) if !config.working_dir.trim().is_empty() => {
            (data_dir_under(&config.working_dir), Origin::Image)
        }
        _ => (overrides::DEFAULT_DATA_DIR.to_string(), Origin::Default),
    };
    let (user, user_from) = resolve_user(
        req.user.as_deref(),
        config.as_ref().map(|c| c.user.as_str()).unwrap_or(""),
        &reference,
        endpoints,
        &mut notes,
    );

    Ok(Resolved {
        id,
        service_id,
        display_name,
        image,
        tag,
        commit,
        follow,
        data_dir: data_dir.0,
        data_dir_from: data_dir.1,
        user,
        user_from,
        runtime_amd64: req.runtime_amd64 || config.map(|c| c.amd64_only).unwrap_or(false),
        notes,
        source,
    })
}

/// The pin this source resolves to: the tag, the commit behind it where that
/// is known, and the branch the entry follows afterwards.
fn pin(source: &Source, endpoints: &Endpoints) -> Result<(String, Option<String>, Option<String>)> {
    match source {
        Source::Image(image) => Ok((image.tag.clone(), None, None)),
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
            )?;
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

/// The newest published build of `branch` in a repository, for `chaps
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

/// What the image says about itself: over HTTP where the registry can be
/// reached, from the local daemon where it cannot.
///
/// `None` is "nobody could say", which leaves the chapkit defaults in place
/// and puts a note in the report rather than failing: the operator can still
/// pass `--data-dir` and `--user`.
fn image_config(
    source: &Source,
    reference: &str,
    endpoints: &Endpoints,
    notes: &mut Vec<String>,
) -> Result<Option<ghcr::ImageConfig>> {
    if !endpoints.offline {
        let repository = match source {
            Source::Repo(repo) => repo.path().to_lowercase(),
            Source::Image(image) => image.repository().to_string(),
        };
        match read_config(&repository, reference, endpoints) {
            Ok(Some(config)) => return Ok(Some(config)),
            Ok(None) => notes.push(format!(
                "{reference} is not published at {}, so the data directory and user \
                 come from the local image or the chapkit defaults",
                endpoints.ghcr_url
            )),
            Err(err) => notes.push(format!(
                "could not read the image config from the registry ({err:#}); \
                 falling back to the local image"
            )),
        }
    }

    match crate::docker::image_config(reference) {
        Some((user, working_dir)) => Ok(Some(ghcr::ImageConfig {
            user,
            working_dir,
            amd64_only: false,
        })),
        None if endpoints.offline => Err(anyhow::anyhow!(
            "the amd64 variant of {reference} is not in the local image store and this \
             run is --offline; run `docker pull --platform linux/amd64 {reference}` \
             first, or drop --offline"
        )),
        None => {
            notes.push(format!(
                "nothing could say what {reference} runs as; \
                 pass `--data-dir` and `--user` if the defaults are wrong"
            ));
            Ok(None)
        }
    }
}

/// The registry half of [`image_config`], split out so its errors are notes
/// rather than failures.
fn read_config(
    repository: &str,
    reference: &str,
    endpoints: &Endpoints,
) -> Result<Option<ghcr::ImageConfig>> {
    let client = ghcr::Client::anonymous(&endpoints.ghcr_url, repository, endpoints.timeout)?;
    // The manifest is addressed by the pin alone: the client already knows
    // the repository.
    let pin = reference
        .rsplit_once('@')
        .map(|(_, digest)| digest.to_string())
        .or_else(|| {
            let last_slash = reference.rfind('/').map(|i| i + 1).unwrap_or(0);
            reference[last_slash..]
                .split_once(':')
                .map(|(_, tag)| tag.to_string())
        })
        .unwrap_or_else(|| "latest".to_string());
    client.image_config(&pin)
}

/// `/work` -> `/work/data`, which is where a chapkit service keeps its SQLite
/// file when it leaves the default `DATABASE_URL` alone.
pub(crate) fn data_dir_under(working_dir: &str) -> String {
    let base = working_dir.trim().trim_end_matches('/');
    if base.is_empty() {
        return overrides::DEFAULT_DATA_DIR.to_string();
    }
    format!("{base}/data")
}

/// The `user:` the overlay runs the service as, and where that came from.
///
/// The flag wins. After that comes whatever the image declares, which is only
/// usable if it can be turned into numbers: the overlay's init container
/// chowns the data volume from busybox, which resolves no account name of its
/// own. A name busybox and this CLI both know nothing about is what the
/// docker probe is for - `id -u` inside the image itself - and if that cannot
/// run either, the name is kept and the caller is told to pass `--user`.
pub fn resolve_user(
    flag: Option<&str>,
    declared: &str,
    reference: &str,
    endpoints: &Endpoints,
    notes: &mut Vec<String>,
) -> (String, Origin) {
    resolve_user_with(
        flag,
        declared,
        reference,
        endpoints,
        notes,
        &crate::docker::pull_image,
        &crate::docker::uid_gid_in_image,
    )
}

/// [`resolve_user`] with the docker calls injected, so the table of cases can
/// be tested without a daemon.
pub fn resolve_user_with(
    flag: Option<&str>,
    declared: &str,
    reference: &str,
    endpoints: &Endpoints,
    notes: &mut Vec<String>,
    pull: PullFn,
    probe: ProbeFn,
) -> (String, Origin) {
    if let Some(user) = flag.map(str::trim).filter(|u| !u.is_empty()) {
        return (user.to_string(), Origin::Flag);
    }
    let declared = declared.trim();
    if declared.is_empty() {
        return (overrides::DEFAULT_USER.to_string(), Origin::Default);
    }
    // Numeric already, or one of the account names the chapkit images create:
    // either way the init container can chown to it.
    if overrides::numeric_user(declared).is_some() {
        return (declared.to_string(), Origin::Image);
    }
    if endpoints.docker_probe {
        crate::output::notice(&format!(
            "asking {reference} what uid `{declared}` is (this pulls the image)"
        ));
        if pull(reference)
            && let Some((uid, gid)) = probe(reference, declared)
        {
            return (format!("{uid}:{gid}"), Origin::Probe);
        }
    }
    notes.push(format!(
        "`{declared}` has no uid this CLI knows and the image could not be asked, \
         so the volume init container will chown to {} - pass `--user <uid>:<gid>` if that is wrong",
        overrides::FALLBACK_UID_GID
    ));
    (declared.to_string(), Origin::Image)
}

/// The name a manually added model is shown under unless `--name` says
/// otherwise: the repository name, or the last segment of the image path.
fn default_display_name(source: &Source) -> String {
    match source {
        Source::Repo(repo) => repo.repo.clone(),
        Source::Image(image) => image
            .image
            .rsplit('/')
            .next()
            .unwrap_or(&image.image)
            .to_string(),
    }
}

/// GET `url`, returning the status and the body.
///
/// Modelled on [`crate::chapcore`]'s fetch - one global deadline covering
/// connect, send and receive - with one difference: a non-2xx status is a
/// value rather than an error, because a 404 from ghcr is the answer to "is
/// this tag published" and not a failure.
pub(crate) fn get(url: &str, timeout: Duration, headers: &[(&str, &str)]) -> Result<(u16, String)> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(USER_AGENT)
            .build(),
    );
    let mut request = agent.get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let started = std::time::Instant::now();
    let mut response = match request.call() {
        Ok(response) => response,
        Err(ureq::Error::StatusCode(status)) => {
            crate::output::verbose(&format!(
                "GET {url} -> {status} in {}ms",
                started.elapsed().as_millis()
            ));
            return Ok((status, String::new()));
        }
        Err(err) => {
            crate::output::verbose(&format!(
                "GET {url} -> failed in {}ms",
                started.elapsed().as_millis()
            ));
            return Err(anyhow::anyhow!("fetching {url}: {err}"));
        }
    };
    let status = response.status().as_u16();
    crate::output::verbose(&format!(
        "GET {url} -> {status} in {}ms",
        started.elapsed().as_millis()
    ));
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("reading the response body from {url}: {e}"))?;
    crate::output::debug(&format!("  {}", crate::output::trace_body(&body)));
    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoints() -> Endpoints {
        Endpoints {
            docker_probe: false,
            ..Endpoints::default()
        }
    }

    #[test]
    fn the_endpoints_default_to_the_public_ones() {
        let e = Endpoints::default();
        assert_eq!(e.github_api, "https://api.github.com");
        assert_eq!(e.ghcr_url, "https://ghcr.io");
        assert!(!e.offline && e.docker_probe);
    }

    #[test]
    fn the_newest_commit_with_a_published_build_wins() {
        let shas: Vec<String> = ["b1d6c31aaaa", "60b16a2bbbb", "81ab16bcccc"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Every build published: the head commit.
        let (sha, tag) = pick_published(&shas, &|_| Ok(true)).unwrap().unwrap();
        assert_eq!(sha, "b1d6c31aaaa");
        assert_eq!(tag, "sha-b1d6c31");

        // The head changed nothing the workflow builds, so the pin lands on
        // the newest tag there is.
        let (sha, tag) = pick_published(&shas, &|tag| Ok(tag == "sha-60b16a2"))
            .unwrap()
            .unwrap();
        assert_eq!(sha, "60b16a2bbbb");
        assert_eq!(tag, "sha-60b16a2");

        // Nothing published at all.
        assert!(pick_published(&shas, &|_| Ok(false)).unwrap().is_none());
        // And a registry that would not answer is not "nothing published".
        assert!(pick_published(&shas, &|_| Err(anyhow::anyhow!("boom"))).is_err());
    }

    #[test]
    fn the_data_directory_follows_the_working_directory() {
        assert_eq!(data_dir_under("/work"), "/work/data");
        assert_eq!(data_dir_under("/app/"), "/app/data");
        assert_eq!(data_dir_under(""), overrides::DEFAULT_DATA_DIR);
        assert_eq!(data_dir_under("/"), overrides::DEFAULT_DATA_DIR);
    }

    /// Everything `resolve_user` can be told, and what it decides.
    #[test]
    fn the_user_comes_from_the_flag_then_the_image_then_a_probe() {
        let mut notes = Vec::new();
        let never = |_: &str, _: &str| None;
        let pulled = |_: &str| true;

        // The flag wins over everything.
        assert_eq!(
            resolve_user_with(
                Some("1500:1600"),
                "app",
                "ghcr.io/x/y:t",
                &endpoints(),
                &mut notes,
                &pulled,
                &never
            ),
            ("1500:1600".to_string(), Origin::Flag)
        );
        // An image that declares no user gets the chapkit default.
        assert_eq!(
            resolve_user_with(
                None,
                "",
                "ghcr.io/x/y:t",
                &endpoints(),
                &mut notes,
                &pulled,
                &never
            ),
            (overrides::DEFAULT_USER.to_string(), Origin::Default)
        );
        // A numeric user, and a name the chapkit images create, are both
        // usable as they are.
        for declared in ["10001:10001", "chapkit", "chap:chap"] {
            assert_eq!(
                resolve_user_with(
                    None,
                    declared,
                    "ghcr.io/x/y:t",
                    &endpoints(),
                    &mut notes,
                    &pulled,
                    &never
                ),
                (declared.to_string(), Origin::Image),
                "{declared}"
            );
        }
        assert!(notes.is_empty(), "{notes:?}");

        // A name nobody knows: the image itself is asked.
        let probe = |_: &str, name: &str| (name == "app").then_some((10001, 10001));
        assert_eq!(
            resolve_user_with(
                None,
                "app",
                "ghcr.io/x/y:t",
                &Endpoints::default(),
                &mut notes,
                &pulled,
                &probe
            ),
            ("10001:10001".to_string(), Origin::Probe)
        );
        assert!(notes.is_empty(), "{notes:?}");

        // And when the probe cannot run, the name is kept and the caller is
        // told what the init container will do instead.
        assert_eq!(
            resolve_user_with(
                None,
                "app",
                "ghcr.io/x/y:t",
                &endpoints(),
                &mut notes,
                &pulled,
                &never
            ),
            ("app".to_string(), Origin::Image)
        );
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("--user"), "{}", notes[0]);
        assert!(notes[0].contains("1000:1000"), "{}", notes[0]);
    }

    #[test]
    fn a_failed_pull_is_not_a_probe() {
        let mut notes = Vec::new();
        let probe = |_: &str, _: &str| Some((10001, 10001));
        let (user, origin) = resolve_user_with(
            None,
            "app",
            "ghcr.io/x/y:t",
            &Endpoints::default(),
            &mut notes,
            &|_| false,
            &probe,
        );
        assert_eq!((user.as_str(), origin), ("app", Origin::Image));
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn an_offline_run_refuses_a_repository_and_says_what_to_pass_instead() {
        let offline = Endpoints {
            offline: true,
            ..endpoints()
        };
        let source = Source::parse("https://github.com/chap-models/chapkit_ghr_model").unwrap();
        let err = pin(&source, &offline).expect_err("--offline cannot resolve a branch");
        assert!(err.to_string().contains("--offline"), "{err}");
        assert!(
            err.to_string()
                .contains("ghcr.io/chap-models/chapkit_ghr_model"),
            "{err}"
        );

        // An image reference pins itself, with or without a network.
        let source = Source::parse("ghcr.io/chap-models/chapkit_ghr_model:sha-1eb8cf1").unwrap();
        let (tag, commit, follow) = pin(&source, &offline).unwrap();
        assert_eq!(tag, "sha-1eb8cf1");
        assert_eq!(commit, None);
        assert_eq!(follow, None);
    }

    #[test]
    fn the_selector_follows_a_branch_or_pins_the_tag() {
        let following = Resolved {
            source: Source::parse("https://github.com/chap-models/chapkit_ghr_model").unwrap(),
            id: "chapkit_ghr_model".into(),
            service_id: "chapkit-ghr-model".into(),
            display_name: "chapkit_ghr_model".into(),
            image: "ghcr.io/chap-models/chapkit_ghr_model".into(),
            tag: "sha-b1d6c31".into(),
            commit: Some("b1d6c31".repeat(5)),
            follow: Some("main".into()),
            data_dir: "/work/data".into(),
            data_dir_from: Origin::Image,
            user: "10001:10001".into(),
            user_from: Origin::Probe,
            runtime_amd64: true,
            notes: Vec::new(),
        };
        assert_eq!(
            following.selector(),
            VersionSelector::Channel(crate::registry::Channel::Latest)
        );
        assert_eq!(
            following.image_ref(),
            "ghcr.io/chap-models/chapkit_ghr_model:sha-b1d6c31"
        );

        let pinned = Resolved {
            follow: None,
            ..following
        };
        assert_eq!(
            pinned.selector(),
            VersionSelector::Exact("sha-b1d6c31".into())
        );
    }

    #[test]
    fn the_origins_read_as_sentences() {
        assert_eq!(Origin::Flag.label(), "given on the command line");
        assert_eq!(Origin::Image.label(), "from the image config");
        assert_eq!(Origin::Probe.label(), "from a docker probe");
        assert_eq!(Origin::Default.label(), "the chapkit default");
    }
}
