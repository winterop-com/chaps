//! Which account a marketplace model's service runs as, read off the image.
//!
//! A model image declares the account it runs as in `config.User`, and forcing
//! a different one is how a model ends up unable to execute its own binaries:
//! the Rwanda BYM model ships root-owned, mode 744 INLA binaries, so a
//! hardened `user: chapkit:chapkit` turned every prediction into
//! `inla.run: Permission denied`. So the image is asked, in the order the
//! answers can be trusted: the flag, the registry, the local daemon, and only
//! then the table compiled into this binary.
//!
//! What comes out is recorded on the [`crate::project::EnabledModel`], which
//! is what keeps `chaps sync` offline and deterministic: the lookup happens
//! once, when a model is enabled or moved to a new tag, and every render after
//! that reads the answer out of `.chaps/models.yaml`.

use crate::compose::overrides;
use crate::error::Result;
use crate::manual::{Endpoints, Origin, ProbeFn, PullFn, ghcr, resolve_user_with};
use serde::{Deserialize, Serialize};

/// Where the user recorded for a model came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UserSource {
    /// `--user` on the command line.
    Flag,
    /// `config.User` of the image, read from the registry.
    ImageConfig,
    /// The local docker daemon: `docker image inspect`, or `id` inside the
    /// image for an account name nothing else could resolve.
    DockerProbe,
    /// The table in [`crate::compose::overrides`], because nothing could be
    /// asked. It is also what a `models.yaml` written before any of this
    /// reads as, because that is where its user came from.
    #[default]
    Table,
}

impl UserSource {
    /// How the source reads in `models info`, the browser and `chaps update`.
    pub fn label(self) -> &'static str {
        match self {
            UserSource::Flag => "--user",
            UserSource::ImageConfig => "image config",
            UserSource::DockerProbe => "docker probe",
            UserSource::Table => "table",
        }
    }
}

/// One model whose data directory and user are to be resolved.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// Marketplace id, for the notes.
    pub id: &'a str,
    /// Tagless image reference.
    pub image: &'a str,
    /// The tag the enable pins, which is the image that will actually run.
    pub image_tag: &'a str,
    pub data_dir_flag: Option<&'a str>,
    pub user_flag: Option<&'a str>,
}

impl Request<'_> {
    /// The reference the lookups are made against.
    fn reference(&self) -> String {
        crate::compose::image_ref(self.image, self.image_tag)
    }

    /// `<owner>/<name>`, which is what ghcr scopes a pull token to.
    fn repository(&self) -> String {
        self.image
            .strip_prefix(&format!("{}/", crate::manual::source::GHCR_HOST))
            .unwrap_or(self.image)
            .to_lowercase()
    }
}

/// What the resolution decided, and what it had to guess at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub data_dir: String,
    /// The `user:` the overlay runs the service as: `root`, a numeric
    /// `uid:gid`, or the account name when nothing could turn it into numbers.
    pub user: String,
    pub user_from: UserSource,
    pub notes: Vec<String>,
}

/// How a caller of [`crate::compose::apply::apply_with`] resolves one model.
pub type ResolveFn<'a> = &'a dyn Fn(&Request) -> Resolution;

/// Reading `config.User` and `config.WorkingDir` off the registry:
/// `(repository, tag) -> config`, with `None` for a tag it does not publish.
pub type RegistryFn<'a> = &'a dyn Fn(&str, &str) -> Result<Option<ghcr::ImageConfig>>;
/// The same two fields from the local daemon: `reference -> (user,
/// working_dir)`, with `None` when the image is not on this machine.
pub type LocalFn<'a> = &'a dyn Fn(&str) -> Option<(String, String)>;

/// Resolve one model against the image it pins.
///
/// The order is the flag, the image config from ghcr, the same config from a
/// locally pulled image, and the built-in table. Nothing here fails: an answer
/// nobody could give is the table plus a note saying so, because a model that
/// cannot be enabled at all is worse than one enabled from the last thing this
/// CLI knows.
pub fn from_image(req: &Request, endpoints: &Endpoints) -> Resolution {
    from_image_with(
        req,
        endpoints,
        &|repository, tag| read_config(repository, tag, endpoints),
        &crate::docker::image_config,
        &crate::docker::pull_image,
        &crate::docker::uid_gid_in_image,
    )
}

/// [`from_image`] with every lookup injected, so the order can be tested
/// without a registry or a daemon.
pub fn from_image_with(
    req: &Request,
    endpoints: &Endpoints,
    registry: RegistryFn,
    local: LocalFn,
    pull: PullFn,
    probe: ProbeFn,
) -> Resolution {
    let reference = req.reference();
    let mut notes = Vec::new();
    let mut declared: Option<(ghcr::ImageConfig, UserSource)> = None;

    if !endpoints.offline {
        match registry(&req.repository(), req.image_tag) {
            Ok(Some(config)) => declared = Some((config, UserSource::ImageConfig)),
            Ok(None) => notes.push(format!(
                "{reference} is not published at {}, so its user comes from the \
                 local image or the built-in table",
                endpoints.ghcr_url
            )),
            Err(err) => notes.push(format!(
                "could not read the image config of {reference} from the registry \
                 ({err:#}); falling back to the local image"
            )),
        }
    }
    if declared.is_none()
        && endpoints.docker_probe
        && let Some((user, working_dir)) = local(&reference)
    {
        declared = Some((
            ghcr::ImageConfig {
                user,
                working_dir,
                amd64_only: false,
            },
            UserSource::DockerProbe,
        ));
    }

    let known = overrides::known_override(req.id);
    let data_dir = match (flag(req.data_dir_flag), &declared) {
        (Some(dir), _) => dir,
        (None, Some((config, _))) if !config.working_dir.trim().is_empty() => {
            crate::manual::data_dir_under(&config.working_dir)
        }
        _ => known
            .map(|k| k.data_dir.to_string())
            .unwrap_or_else(|| overrides::DEFAULT_DATA_DIR.to_string()),
    };

    let (user, user_from) = match (flag(req.user_flag), &declared) {
        (Some(user), _) => (user, UserSource::Flag),
        (None, Some((config, from))) => {
            // An image config with no `User` at all is an image that runs as
            // root; saying "nothing declared" here would hand it the chapkit
            // default, which is exactly the bug this module exists for.
            let user = match config.user.trim() {
                "" => "root",
                user => user,
            };
            // The name-to-numbers half is the same one `chaps models add`
            // runs, `id -u` inside the image included.
            let (user, origin) =
                resolve_user_with(None, user, &reference, endpoints, &mut notes, pull, probe);
            let from = match origin {
                Origin::Probe => UserSource::DockerProbe,
                _ => *from,
            };
            (user, from)
        }
        (None, None) => {
            notes.push(format!(
                "nothing could say what {reference} runs as, so `{}` was enabled \
                 with the user the built-in table records - re-run `chaps models \
                 enable {}` with a network, or pass `--user <uid>:<gid>`",
                req.id, req.id
            ));
            let table = from_table(req);
            (table.user, table.user_from)
        }
    };

    Resolution {
        data_dir,
        user: normalize(&user),
        user_from,
        notes,
    }
}

/// The table alone: no registry, no daemon, no notes.
///
/// What a caller that has already decided not to look anything up passes to
/// [`crate::compose::apply::apply_with`] - the unit tests, which must render
/// the same bytes on a machine with the images and on one without.
pub fn from_table(req: &Request) -> Resolution {
    let known = overrides::known_override(req.id);
    let (user, user_from) = match flag(req.user_flag) {
        Some(user) => (user, UserSource::Flag),
        None => (
            known
                .map(|k| k.user.to_string())
                .unwrap_or_else(|| overrides::DEFAULT_USER.to_string()),
            UserSource::Table,
        ),
    };
    Resolution {
        data_dir: flag(req.data_dir_flag)
            .or_else(|| known.map(|k| k.data_dir.to_string()))
            .unwrap_or_else(|| overrides::DEFAULT_DATA_DIR.to_string()),
        user: normalize(&user),
        user_from,
        notes: Vec::new(),
    }
}

/// A flag that was given and is not blank.
fn flag(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The form a resolved user is recorded and rendered in.
///
/// Root is written as `root`, because that is the one value the overlay reads
/// as "no `user:` line and no init container" and `root` says so where `0:0`
/// would have to be decoded. Everything else that resolves becomes the numeric
/// pair, so the `user:` line and the init container's `chown` are the same two
/// numbers. A name nothing could resolve is kept as it is, which is what
/// `chaps sync` warns about.
///
/// `chaps doctor` compares through here as well: `chapkit` off an image and
/// `1000:1000` out of `models.yaml` are the same answer written two ways.
pub fn normalize(user: &str) -> String {
    if overrides::is_root(user) {
        return "root".to_string();
    }
    overrides::numeric_pair(user).unwrap_or_else(|| user.trim().to_string())
}

/// The registry half, split out so its errors are notes rather than failures.
fn read_config(
    repository: &str,
    tag: &str,
    endpoints: &Endpoints,
) -> Result<Option<ghcr::ImageConfig>> {
    let client = ghcr::Client::anonymous(&endpoints.ghcr_url, repository, endpoints.timeout)?;
    client.image_config(tag.trim_start_matches('@'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE: &str = "ghcr.io/chap-models/chapkit_ewars_model";
    const TAG: &str = "sha-fa880a1";

    fn request<'a>(id: &'a str, user: Option<&'a str>) -> Request<'a> {
        Request {
            id,
            image: IMAGE,
            image_tag: TAG,
            data_dir_flag: None,
            user_flag: user,
        }
    }

    /// Endpoints that reach nothing: the registry stub decides what is known.
    fn endpoints() -> Endpoints {
        Endpoints {
            docker_probe: false,
            ..Endpoints::default()
        }
    }

    fn config(user: &str, working_dir: &str) -> ghcr::ImageConfig {
        ghcr::ImageConfig {
            user: user.to_string(),
            working_dir: working_dir.to_string(),
            amd64_only: false,
        }
    }

    /// [`from_image_with`] against a registry that answers with one config.
    fn resolve(
        req: &Request,
        endpoints: &Endpoints,
        declared: Option<ghcr::ImageConfig>,
    ) -> Resolution {
        from_image_with(
            req,
            endpoints,
            &|_, _| Ok(declared.clone()),
            &|_| None,
            &|_| true,
            &|_, _| None,
        )
    }

    #[test]
    fn an_image_that_runs_as_root_resolves_to_root() {
        for declared in ["root", "0", "0:0", ""] {
            let resolution = resolve(
                &request("chapkit_rwanda_malaria_bym_model", None),
                &endpoints(),
                Some(config(declared, "/work")),
            );
            assert_eq!(resolution.user, "root", "{declared:?}");
            assert_eq!(resolution.user_from, UserSource::ImageConfig);
            assert_eq!(resolution.data_dir, "/work/data");
            assert!(resolution.notes.is_empty(), "{:?}", resolution.notes);
        }
    }

    #[test]
    fn an_account_name_resolves_to_the_numbers_the_chown_uses() {
        let resolution = resolve(
            &request("chapkit_ewars_model", None),
            &endpoints(),
            Some(config("chapkit", "/app")),
        );
        assert_eq!(resolution.user, "1000:1000");
        assert_eq!(resolution.user_from, UserSource::ImageConfig);
        assert_eq!(resolution.data_dir, "/app/data");
        assert!(resolution.notes.is_empty(), "{:?}", resolution.notes);
    }

    #[test]
    fn a_name_nothing_knows_is_kept_with_a_note() {
        let resolution = resolve(
            &request("chapkit_ewars_model", None),
            &endpoints(),
            Some(config("app", "/app")),
        );
        assert_eq!(resolution.user, "app");
        assert_eq!(resolution.user_from, UserSource::ImageConfig);
        assert_eq!(resolution.notes.len(), 1, "{:?}", resolution.notes);
        assert!(
            resolution.notes[0].contains("--user"),
            "{:?}",
            resolution.notes
        );

        // Unless the image itself can be asked, which is the docker probe.
        let probed = from_image_with(
            &request("chapkit_ewars_model", None),
            &Endpoints::default(),
            &|_, _| Ok(Some(config("app", "/app"))),
            &|_| None,
            &|_| true,
            &|_, name| (name == "app").then_some((10001, 10001)),
        );
        assert_eq!(probed.user, "10001:10001");
        assert_eq!(probed.user_from, UserSource::DockerProbe);
    }

    #[test]
    fn the_flag_wins_over_the_image() {
        let resolution = resolve(
            &request("chapkit_ewars_model", Some("1500:1600")),
            &endpoints(),
            Some(config("root", "/app")),
        );
        assert_eq!(resolution.user, "1500:1600");
        assert_eq!(resolution.user_from, UserSource::Flag);
        // The data directory still follows the image; only the user was asked
        // for on the command line.
        assert_eq!(resolution.data_dir, "/app/data");
    }

    /// A registry that will not answer falls through to the local image, and
    /// only then to the table - with a note saying which one was used.
    #[test]
    fn the_local_image_comes_before_the_table_and_the_table_says_so() {
        let local = from_image_with(
            &request("chapkit_rwanda_malaria_bym_model", None),
            &Endpoints::default(),
            &|_, _| Err(anyhow::anyhow!("no route to host")),
            &|_| Some(("root".to_string(), "/work".to_string())),
            &|_| true,
            &|_, _| None,
        );
        assert_eq!(local.user, "root");
        assert_eq!(local.user_from, UserSource::DockerProbe);
        assert_eq!(local.data_dir, "/work/data");
        assert_eq!(local.notes.len(), 1, "the registry failure is worth saying");

        let table = from_image_with(
            &request("chapkit_ewars_model", None),
            &Endpoints {
                offline: true,
                ..Endpoints::default()
            },
            &|_, _| panic!("--offline asks no registry"),
            &|_| None,
            &|_| true,
            &|_, _| None,
        );
        assert_eq!(table.user, "1000:1000", "the table's `chapkit`, as numbers");
        assert_eq!(table.user_from, UserSource::Table);
        assert_eq!(table.data_dir, "/app/data");
        assert_eq!(table.notes.len(), 1);
        assert!(
            table.notes[0].contains("built-in table"),
            "{:?}",
            table.notes
        );
    }

    #[test]
    fn the_table_resolver_looks_nothing_up() {
        let resolution = from_table(&request("chapkit_rwanda_malaria_bym_model", None));
        assert_eq!(resolution.user, "root");
        assert_eq!(resolution.user_from, UserSource::Table);
        assert_eq!(resolution.data_dir, "/work/data");
        assert!(resolution.notes.is_empty());

        // A model the table has never heard of takes the chapkit defaults.
        let unknown = from_table(&request("something_else", None));
        assert_eq!(unknown.user, "1000:1000");
        assert_eq!(unknown.data_dir, overrides::DEFAULT_DATA_DIR);

        // And the flag still wins.
        let explicit = from_table(&request("chapkit_ewars_model", Some("0:0")));
        assert_eq!(explicit.user, "root", "0:0 is root, written the one way");
        assert_eq!(explicit.user_from, UserSource::Flag);
    }

    #[test]
    fn the_repository_is_the_image_without_the_registry_host() {
        assert_eq!(
            request("x", None).repository(),
            "chap-models/chapkit_ewars_model"
        );
        assert_eq!(
            request("x", None).reference(),
            "ghcr.io/chap-models/chapkit_ewars_model:sha-fa880a1"
        );
    }

    #[test]
    fn the_sources_read_as_labels() {
        assert_eq!(UserSource::Flag.label(), "--user");
        assert_eq!(UserSource::ImageConfig.label(), "image config");
        assert_eq!(UserSource::DockerProbe.label(), "docker probe");
        assert_eq!(UserSource::Table.label(), "table");
        assert_eq!(UserSource::default(), UserSource::Table);
    }
}
