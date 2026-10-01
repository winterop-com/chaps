//! `.chaps/` — the directory that records what a deployment is meant to be.
//!
//! Three YAML files: `project.yaml` holds the project-wide settings,
//! `models.yaml` the enabled model set and `components.yaml` the enabled
//! components. They are intent; the compose files at the project root are
//! artifacts rendered from them by `chaps sync`.

use crate::components::{COMPONENTS_FILE, Components};
use crate::compose::UserSource;
use crate::error::{ChapError, Result};
use crate::registry::Channel;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Directory that marks a project, at the root of a project directory.
pub const CHAPS_DIR: &str = ".chaps";
/// Project-wide settings, inside [`CHAPS_DIR`].
pub const PROJECT_FILE: &str = "project.yaml";
/// The enabled model set, inside [`CHAPS_DIR`].
pub const MODELS_FILE: &str = "models.yaml";
/// Models added with `chaps models add`, inside [`CHAPS_DIR`].
///
/// The marketplace's answer for a model it does not list: the definition the
/// catalogue would have carried, recorded per deployment. It is a definition,
/// not an enablement - [`MODELS_FILE`] still says which models are on.
pub const MANUAL_MODELS_FILE: &str = "models-manual.yaml";
/// Base compose file: chap-core, worker, valkey, postgres.
pub const BASE_COMPOSE: &str = "compose.yml";
/// chaps-owned overrides that sit on top of [`BASE_COMPOSE`]: the API's host
/// port, and anything else this CLI decides about the base stack.
///
/// It is a separate `-f` entry rather than an `include:` because a file listed
/// in `include:` cannot override a service the main file defines.
pub const CHAPS_COMPOSE: &str = "compose.chaps.yml";
/// Umbrella file that `include:`s one overlay per enabled model.
pub const MARKETPLACE_COMPOSE: &str = "compose.marketplace.yml";
/// Environment file docker compose picks up automatically.
pub const ENV_FILE: &str = ".env";
/// The compose file a chap-core checkout carries, which `compose.yml` is
/// rendered from for a deployment built from one.
pub const CHECKOUT_COMPOSE: &str = "compose.ghcr.yml";
/// The `.env` variable the chap-core images read their tag from.
pub const CHAP_TAG_ENV_VAR: &str = "CHAP_IMAGE_TAG";
/// The `.env` variable [`CHAPS_COMPOSE`] reads the API's host port from.
pub const API_PORT_ENV_VAR: &str = "CHAP_API_PORT";
/// The `.env` variable that gives chap-core's API a path prefix, for an
/// instance served behind a reverse proxy. Never written by `init`.
pub const ROOT_PATH_ENV_VAR: &str = "CHAP_ROOT_PATH";

/// `project.yaml` schema version written by this CLI.
pub const SCHEMA_VERSION: u32 = 1;
/// Host port range model overlays are allocated from.
pub const DEFAULT_PORT_RANGE: (u16, u16) = (5001, 5999);
/// Host port chap-core's API is published on unless `--api-port` says
/// otherwise.
///
/// Not the container's 8000: that, 8080 and 9000 are the ports every other
/// development server defaults to, so chaps' services start one block away
/// from them (8700 chap-core, 8780 DHIS2, 8790 OCS).
pub const DEFAULT_API_PORT: u16 = 8700;

/// The `-f` list a project written by this CLI has.
pub fn default_compose_files() -> Vec<String> {
    compose_files_for(&Components::default())
}

/// The `-f` list a project with these components has.
///
/// The base stack and the chaps-owned override only appear when chap-core is
/// one of the components; a deployment that is only OCS has neither. Component
/// files sit between the override and the marketplace umbrella, so a component
/// can add to the base stack and still be added to by a model overlay.
pub fn compose_files_for(components: &Components) -> Vec<String> {
    let mut files = Vec::new();
    if components.chap_core.enabled {
        files.push(BASE_COMPOSE.to_string());
        files.push(CHAPS_COMPOSE.to_string());
    }
    files.extend(components.compose_files());
    files.push(MARKETPLACE_COMPOSE.to_string());
    files
}

/// [`DEFAULT_API_PORT`], for `serde(default)` on a `project.yaml` written
/// before the field existed.
fn default_api_port() -> u16 {
    DEFAULT_API_PORT
}

/// Which file decided the host port chap-core's API is published on.
///
/// Two files can, and they do not always agree: `chaps init --api-port`
/// records one in `.chaps/project.yaml` and writes the same number into
/// `.env`, but `.env` belongs to the operator afterwards and compose reads it
/// last. So the answer has to come with its source, or a report naming a port
/// the recorded state does not mention looks like a bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiPortSource {
    /// An active `CHAP_API_PORT=` line in `.env`, which is what compose reads.
    Env,
    /// `api_port` in `.chaps/project.yaml`, because `.env` sets no usable one.
    Project,
}

impl ApiPortSource {
    /// How the source reads in a sentence: which file the number came from.
    pub fn label(self) -> &'static str {
        match self {
            ApiPortSource::Env => "from .env",
            ApiPortSource::Project => "from .chaps/project.yaml",
        }
    }
}

/// How many random hex characters a generated compose project name ends in.
///
/// Three bytes: short enough to keep a container name readable, and 16 million
/// values is far more than the handful of deployments one machine ever holds.
pub const PROJECT_SUFFIX_BYTES: usize = 3;

/// The longest slug a generated compose project name starts with.
///
/// Compose puts the project name in front of every container and volume name,
/// and a name nobody can read on a `docker ps` line helps no one.
const MAX_SLUG: usize = 32;

/// What a generated name falls back to when the directory name yields no
/// usable slug at all (`~/深度`, say).
const FALLBACK_SLUG: &str = "chaps";

/// The compose project name compose itself would derive from a directory name.
///
/// Compose lowercases the name, drops every character outside `[a-z0-9_-]` and
/// trims leading `_` and `-`. This mirrors that rule exactly, because it is
/// what an existing deployment's containers and volumes are already named
/// after: recording this value changes nothing, which is the point.
///
/// `None` when nothing is left, which is a directory compose would refuse to
/// name a project after either.
pub fn normalized_project_name(dir_name: &str) -> Option<String> {
    let kept: String = dir_name
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-')
        .collect();
    let trimmed = kept.trim_start_matches(['_', '-']);
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The same, for the directory itself. `None` for a path with no file name,
/// or one whose name normalises to nothing.
pub fn derived_project_name(dir: &Path) -> Option<String> {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    normalized_project_name(&dir.file_name()?.to_string_lossy())
}

/// The readable half of a generated compose project name.
///
/// Lowercase `[a-z0-9-]` starting with a letter or a digit, which is what
/// Compose accepts and what reads as the deployment's own name on a
/// `docker ps` line. Anything else folds to a single `-`.
pub fn project_slug(dir_name: &str) -> String {
    let mut out = String::with_capacity(dir_name.len());
    for ch in dir_name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    let out: String = out.chars().take(MAX_SLUG).collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        return FALLBACK_SLUG.to_string();
    }
    out
}

/// `<slug>-<suffix>`, the compose project name a new deployment gets.
pub fn compose_project_name(dir_name: &str, suffix: &str) -> String {
    format!("{}-{suffix}", project_slug(dir_name))
}

/// A compose project name for a deployment being created in `dir`.
///
/// The directory name is only half of it: two directories both called `demo`
/// would otherwise share every named volume, so a fresh deployment gets six
/// random hex characters of its own. See [`ProjectState::compose_project`].
pub fn new_compose_project_name(dir: &Path) -> Result<String> {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(compose_project_name(
        &name,
        &crate::auth::random_hex(PROJECT_SUFFIX_BYTES)?,
    ))
}

/// File name of the cached copy of chap-core's `compose.ghcr.yml` at `tag`,
/// inside [`CHAPS_DIR`].
///
/// The tag is part of the name, so switching tags never overwrites the copy
/// the running deployment was rendered from. Anything a tag may contain that a
/// file name should not (a slash, most of all) is folded to `_`.
pub fn cached_compose_file(tag: &str) -> String {
    let mut safe = String::with_capacity(tag.len());
    for ch in tag.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
            safe.push(ch);
        } else {
            safe.push('_');
        }
    }
    format!("compose.chap-core.{safe}.yml")
}

/// The first line of every file in `.chaps/`.
///
/// One line, the same in each of them: what each file holds and which command
/// edits it is documented, and a generated file that carries the explanation
/// too is one more copy to keep in step.
const MANAGED_HEADER: &str =
    "# Managed by chaps; change it with the chaps commands, not by hand.\n";

/// Where the base `compose.yml` is rendered from.
///
/// `compose.yml` is an artifact like the overlays: [`crate::compose::sync()`]
/// re-renders it, and this says from what. Old `project.yaml` files that
/// predate the field load as [`ComposeSource::Embedded`], which is what they
/// were written from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComposeSource {
    /// The copy of chap-core's `compose.ghcr.yml` compiled into this binary.
    #[default]
    Embedded,
    /// chap-core's own `compose.ghcr.yml` at a tag, downloaded by `chaps init`
    /// or `chaps update` and kept in `.chaps/` so `sync` can replay it
    /// offline.
    Fetched {
        url: String,
        tag: String,
        /// SHA-256 of the cached copy as it was downloaded, so a later edit of
        /// it is noticed rather than silently rendered.
        sha256: String,
    },
    /// A chap-core checkout on this machine: `compose.yml` is rendered from
    /// its own `compose.ghcr.yml`, re-read on every sync, and the chap and
    /// worker images are built from it (`chaps init --source`).
    Checkout { path: String },
}

impl ComposeSource {
    /// The cached copy's file name inside [`CHAPS_DIR`], for a fetched source.
    pub fn cached_file(&self) -> Option<String> {
        match self {
            ComposeSource::Embedded | ComposeSource::Checkout { .. } => None,
            ComposeSource::Fetched { tag, .. } => Some(cached_compose_file(tag)),
        }
    }

    /// One-line description for human output.
    pub fn describe(&self) -> String {
        match self {
            ComposeSource::Embedded => "the compose.ghcr.yml built into this binary".to_string(),
            ComposeSource::Fetched { tag, .. } => format!("chap-core compose.ghcr.yml at {tag}"),
            ComposeSource::Checkout { path } => format!("the chap-core checkout at {path}"),
        }
    }
}

/// Which of chap-core's two shared secrets this deployment uses.
///
/// Booleans only, and deliberately so: the values themselves live in `.env`,
/// which is the file compose reads and the one nobody should copy around.
/// `.chaps/` records the intent, so `project.yaml` can be committed, backed up
/// and pasted into a bug report without leaking a credential. A `project.yaml`
/// written before this field existed loads as both `false`, which is what it
/// was.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthState {
    /// `CHAP_API_TOKEN` is set, so every request needs
    /// `Authorization: Bearer <token>` outside [`crate::auth::OPEN_PATHS`].
    #[serde(default)]
    pub api_token: bool,
    /// `SERVICEKIT_REGISTRATION_KEY` is set, so each model overlay carries the
    /// line that hands the key to its service.
    #[serde(default)]
    pub registration_key: bool,
}

impl AuthState {
    /// Whether anything at all is protected.
    pub fn is_on(&self) -> bool {
        self.api_token || self.registration_key
    }
}

/// The in-memory project state: `project.yaml` plus `models.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub schema_version: u32,
    /// e.g. `chaps 0.1.0`.
    pub generated_by: String,
    pub chap_image_tag: String,
    /// Where `compose.yml` is rendered from.
    #[serde(default)]
    pub chap_compose_source: ComposeSource,
    /// The compose project name: the prefix every container and every named
    /// volume of this deployment carries.
    ///
    /// Compose derives it from the directory name unless a file says
    /// otherwise, so two deployments in directories both called `demo` share
    /// `demo_chap-db` and every other volume - including one left behind by a
    /// deployment deleted long ago. `chaps init` therefore generates
    /// `<slug>-<6 hex>` and `sync` renders it as the top-level `name:` of the
    /// files it owns.
    ///
    /// Empty in a `project.yaml` written before the field existed. Those
    /// deployments keep the name their containers and volumes already carry:
    /// the first `sync` records the directory name compose was deriving
    /// anyway, without a suffix, so nothing is renamed and nothing is
    /// orphaned.
    #[serde(default)]
    pub compose_project: String,
    pub registry_url: String,
    /// Host port chap-core's API is published on. Written into `.env` as
    /// `CHAP_API_PORT` and into [`CHAPS_COMPOSE`]; a `project.yaml` from
    /// before this field loads as [`DEFAULT_API_PORT`], which is what it was.
    #[serde(default = "default_api_port")]
    pub api_port: u16,
    /// Which shared secrets `.env` sets. See [`AuthState`].
    #[serde(default)]
    pub auth: AuthState,
    /// Ordered `-f` list, relative to the project directory.
    pub compose_files: Vec<String>,
    pub port_range: (u16, u16),
    /// Files at the project root that `chaps sync` wrote last time, relative
    /// to the project directory. Only these are ever removed by a later sync.
    #[serde(default)]
    pub rendered_files: Vec<String>,
    /// Enabled models, keyed by marketplace `id`. Lives in `models.yaml`.
    #[serde(skip)]
    pub models: BTreeMap<String, EnabledModel>,
    /// Models this deployment defines itself, keyed by id. Lives in
    /// `models-manual.yaml`; a project that has added none has no such file.
    #[serde(skip)]
    pub manual: ManualModels,
    /// What the deployment is made of. Lives in `components.yaml`; a project
    /// written before that file existed loads as chap-core alone, which is
    /// what it was.
    #[serde(skip)]
    pub components: Components,
}

impl Default for ProjectState {
    fn default() -> Self {
        ProjectState {
            schema_version: SCHEMA_VERSION,
            generated_by: format!("chaps {}", env!("CARGO_PKG_VERSION")),
            chap_image_tag: "latest".to_string(),
            chap_compose_source: ComposeSource::Embedded,
            compose_project: String::new(),
            registry_url: crate::registry::DEFAULT_REGISTRY_URL.to_string(),
            api_port: DEFAULT_API_PORT,
            auth: AuthState::default(),
            compose_files: default_compose_files(),
            port_range: DEFAULT_PORT_RANGE,
            rendered_files: Vec::new(),
            models: BTreeMap::new(),
            manual: ManualModels::new(),
            components: Components::default(),
        }
    }
}

/// One enabled model, as recorded in `models.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnabledModel {
    /// Compose service name and DNS name.
    pub service_id: String,
    /// Tagless image reference.
    pub image: String,
    /// Image tag of the pinned version, e.g. `sha-fa880a1`.
    pub image_tag: String,
    pub version: String,
    /// `Some` when the pin follows a channel, `None` when it is exact.
    pub channel: Option<Channel>,
    /// Host port the service is published on, when someone asked for one.
    ///
    /// `None` - the default - means the overlay only `expose`s port 8000:
    /// chap-core reaches the model over the compose network and a human goes
    /// through chap-core's proxy. A `models.yaml` written before this was
    /// optional holds a number, which still loads as `Some`.
    #[serde(default)]
    pub host_port: Option<u16>,
    pub data_dir: String,
    /// What the container runs as: `root`, a numeric `uid:gid`, or an account
    /// name nothing could turn into numbers.
    ///
    /// Resolved from the image when the model was enabled and recorded here,
    /// which is what keeps `chaps sync` offline: the overlay is rendered from
    /// this line, not from a lookup.
    pub user: String,
    /// Where [`user`] came from, for `models info`, the browser and
    /// `chaps doctor`. A `models.yaml` written before this field existed has
    /// none and reads as [`UserSource::Table`], which is where its user did
    /// come from.
    ///
    /// [`user`]: EnabledModel::user
    #[serde(default)]
    pub user_from: UserSource,
    /// `Some("linux/amd64")` for R-INLA services.
    pub platform: Option<String>,
    /// Overlay file name, relative to the project directory.
    pub compose_file: String,
}

impl ManualModel {
    /// What `chaps models add` was given, or its equivalent: the repository
    /// when it came from one, else the pinned image reference.
    pub fn source(&self) -> String {
        match &self.repository {
            Some(repository) => repository.clone(),
            None if self.tag.starts_with('@') => format!("{}{}", self.image, self.tag),
            None => format!("{}:{}", self.image, self.tag),
        }
    }
}

/// The manual model definitions of a deployment, keyed by id.
pub type ManualModels = BTreeMap<String, ManualModel>;

/// One model added with `chaps models add`, as recorded in
/// `models-manual.yaml`.
///
/// Everything a marketplace file would have said about it, and nothing about
/// whether it is enabled: that stays in `models.yaml`, so a manually added
/// model can be disabled and enabled again like any other.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManualModel {
    /// Compose service name and DNS name. It has to match the id the service
    /// registers with chap-core under, or `chaps status` sees an unmanaged
    /// service next to a model that never arrived.
    pub service_id: String,
    pub display_name: String,
    /// The GitHub repository it was added from, when it was added from one.
    #[serde(default)]
    pub repository: Option<String>,
    /// Tagless image reference, lowercase.
    pub image: String,
    /// The pin: a `sha-<short commit>` tag, or `@sha256:...` for a digest.
    pub tag: String,
    /// The commit the tag was built from, where it is known.
    #[serde(default)]
    pub commit: Option<String>,
    /// The branch `chaps update` follows, or `None` for a pinned entry.
    #[serde(default)]
    pub follow: Option<String>,
    /// The data directory the image writes to, as `models add` resolved it.
    ///
    /// The marketplace's equivalent is [`crate::compose::overrides`], which
    /// is a table of images this CLI ships with and cannot grow an entry for
    /// a model it has never seen. Recording it here is what makes `models
    /// disable` followed by `models enable` bring the model back as it was,
    /// rather than on the chapkit defaults.
    #[serde(default)]
    pub data_dir: Option<String>,
    /// The `user:group` the container runs as, likewise.
    #[serde(default)]
    pub user: Option<String>,
    /// Whether the image is published for amd64 only, which is what the
    /// synthesised entry reports as the R-INLA runtime.
    #[serde(default)]
    pub runtime_amd64: bool,
    /// `YYYY-MM-DD`, the day it was added.
    pub added: String,
}

impl ManualModel {
    /// The marketplace entry this definition stands in for.
    ///
    /// One version, which both channels point at, so every path that resolves
    /// a model - `enable`, `sync`, `update`, the browser - reaches the
    /// recorded pin without a special case. The status is gray and the
    /// summary says where it came from, because nothing here was reviewed by
    /// the marketplace.
    pub fn to_model(&self, id: &str) -> crate::registry::Model {
        use crate::registry::model::{
            Attribution, Channels, Compatibility, Covariates, Kind, Source, Version, VersionStatus,
        };
        let origin = self
            .repository
            .clone()
            .unwrap_or_else(|| crate::compose::image_ref(&self.image, &self.tag));
        crate::registry::Model {
            schema_version: 2,
            id: id.to_string(),
            service_id: self.service_id.clone(),
            display_name: self.display_name.clone(),
            kind: Kind::Model,
            assessed_status: crate::registry::AssessedStatus::Gray,
            summary: format!("added manually from {origin}"),
            source: Source {
                repository: origin,
                image: self.image.clone(),
                runtime_image: match self.runtime_amd64 {
                    true => crate::registry::model::R_INLA_RUNTIME.to_string(),
                    false => String::new(),
                },
            },
            attribution: Attribution {
                author: String::new(),
                organization: None,
                contact: None,
                citation: None,
            },
            maintainers: Vec::new(),
            compatibility: Compatibility {
                period_types: Vec::new(),
                min_prediction_periods: 0,
                max_prediction_periods: 0,
                requires_geo: false,
            },
            covariates: Covariates {
                required: Vec::new(),
                defaults: Vec::new(),
                allow_free_additional: false,
            },
            channels: Channels {
                stable: self.tag.clone(),
                latest: self.tag.clone(),
            },
            versions: vec![Version {
                version: self.tag.clone(),
                commit: self.commit.clone().unwrap_or_default(),
                image_tag: self.tag.clone(),
                chapkit: String::new(),
                status: VersionStatus::Unstable,
                verified_by: Vec::new(),
                changelog: None,
                notes: None,
            }],
            configurations: BTreeMap::new(),
            manual: true,
        }
    }

    /// The image reference this entry pins.
    pub fn image_ref(&self) -> String {
        crate::compose::image_ref(&self.image, &self.tag)
    }
}

/// A project directory plus its parsed state.
#[derive(Debug, Clone)]
pub struct Project {
    pub dir: PathBuf,
    pub state: ProjectState,
}

impl Project {
    /// Whether `dir` itself holds a `.chaps/project.yaml`.
    pub fn exists(dir: &Path) -> bool {
        dir.join(CHAPS_DIR).join(PROJECT_FILE).is_file()
    }

    /// The project directory that contains `start`: `start` itself or the
    /// nearest ancestor holding a `.chaps/project.yaml`, like git's discovery
    /// of `.git`.
    pub fn find_root(start: &Path) -> Option<PathBuf> {
        let start = std::path::absolute(start).ok()?;
        let mut dir: &Path = &start;
        loop {
            if Project::exists(dir) {
                return Some(dir.to_path_buf());
            }
            dir = dir.parent()?;
        }
    }

    /// The marketplace registry `project.yaml` under `root` records, which
    /// `chaps init --registry-url` wrote there.
    ///
    /// Read on its own rather than through [`Project::load`], because it is
    /// asked before any command runs: a deployment whose other state files do
    /// not load still has a registry, and the command that reports the broken
    /// file needs it. `None` for a file that is missing or does not parse, and
    /// the command then goes on with the default.
    pub fn saved_registry_url(root: &Path) -> Option<String> {
        let body = std::fs::read_to_string(root.join(CHAPS_DIR).join(PROJECT_FILE)).ok()?;
        let state: serde_yaml_ng::Value = serde_yaml_ng::from_str(&body).ok()?;
        state
            .get("registry_url")?
            .as_str()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_string)
    }

    /// Load the project that contains `start`, walking up parent directories.
    ///
    /// Errors with [`ChapError::NotAProject`] naming `start` when no ancestor
    /// is a project.
    pub fn find(start: &Path) -> Result<Project> {
        match Project::find_root(start) {
            Some(root) => Project::load(&root),
            None => {
                let shown = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
                Err(ChapError::NotAProject(shown).into())
            }
        }
    }

    /// Read `dir/.chaps/project.yaml` and `dir/.chaps/models.yaml`.
    ///
    /// Errors with [`ChapError::NotAProject`] when `project.yaml` is absent;
    /// a missing `models.yaml` means no models are enabled.
    pub fn load(dir: &Path) -> Result<Project> {
        let chaps = dir.join(CHAPS_DIR);
        let project_path = chaps.join(PROJECT_FILE);
        let body = match std::fs::read_to_string(&project_path) {
            Ok(body) => body,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ChapError::NotAProject(dir.to_path_buf()).into());
            }
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context(format!("reading {}", project_path.display()))
                );
            }
        };
        let mut state: ProjectState = serde_yaml_ng::from_str(&body).map_err(|e| {
            anyhow::anyhow!("{}: invalid project.yaml: {e}", project_path.display())
        })?;

        let models_path = chaps.join(MODELS_FILE);
        state.models = match std::fs::read_to_string(&models_path) {
            Ok(body) if is_blank_yaml(&body) => BTreeMap::new(),
            Ok(body) => serde_yaml_ng::from_str(&body).map_err(|e| {
                anyhow::anyhow!("{}: invalid models.yaml: {e}", models_path.display())
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context(format!("reading {}", models_path.display()))
                );
            }
        };

        // Definitions, not enablements: a deployment that has added no model
        // of its own has no such file, which is not a state to migrate.
        let manual_path = chaps.join(MANUAL_MODELS_FILE);
        state.manual = match std::fs::read_to_string(&manual_path) {
            Ok(body) if is_blank_yaml(&body) => ManualModels::new(),
            Ok(body) => serde_yaml_ng::from_str(&body).map_err(|e| {
                anyhow::anyhow!("{}: invalid models-manual.yaml: {e}", manual_path.display())
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ManualModels::new(),
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context(format!("reading {}", manual_path.display()))
                );
            }
        };

        // Every field of `Components` defaults, so a missing or comment-only
        // file is "chap-core and nothing else" rather than an error.
        let components_path = chaps.join(COMPONENTS_FILE);
        state.components = match std::fs::read_to_string(&components_path) {
            Ok(body) if is_blank_yaml(&body) => Components::default(),
            Ok(body) => serde_yaml_ng::from_str(&body).map_err(|e| {
                anyhow::anyhow!(
                    "{}: invalid components.yaml: {e}",
                    components_path.display()
                )
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Components::default(),
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context(format!("reading {}", components_path.display()))
                );
            }
        };
        check_file_names(&state)?;
        Ok(Project {
            dir: dir.to_path_buf(),
            state,
        })
    }

    /// Write `.chaps/project.yaml` and `.chaps/models.yaml`.
    ///
    /// Each file goes to a temporary sibling first and is renamed into place,
    /// so a crash never leaves a half-written state file behind.
    pub fn save(&self) -> Result<()> {
        let chaps = self.dir.join(CHAPS_DIR);
        std::fs::create_dir_all(&chaps)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", chaps.display()))?;

        let project_body = format!("{MANAGED_HEADER}{}", serde_yaml_ng::to_string(&self.state)?);
        write_atomically(&chaps.join(PROJECT_FILE), &project_body)?;

        let models_body = format!(
            "{MANAGED_HEADER}{}",
            serde_yaml_ng::to_string(&self.state.models)?
        );
        write_atomically(&chaps.join(MODELS_FILE), &models_body)?;

        // Written only by a deployment that has one: an empty file in every
        // other project would be a file to explain, and `chaps models remove`
        // leaves the (now empty) one it emptied rather than deleting a file
        // the operator can see.
        let manual_path = chaps.join(MANUAL_MODELS_FILE);
        if !self.state.manual.is_empty() || manual_path.is_file() {
            let manual_body = format!(
                "{MANAGED_HEADER}{}",
                serde_yaml_ng::to_string(&self.state.manual)?
            );
            write_atomically(&manual_path, &manual_body)?;
        }

        let components_body = format!(
            "{MANAGED_HEADER}{}",
            serde_yaml_ng::to_string(&self.state.components)?
        );
        write_atomically(&chaps.join(COMPONENTS_FILE), &components_body)
    }

    /// The `.chaps/` directory of this project.
    pub fn chaps_dir(&self) -> PathBuf {
        self.dir.join(CHAPS_DIR)
    }

    /// Absolute path of the cached `compose.ghcr.yml` this project's
    /// `compose.yml` is rendered from, when it is not the embedded copy.
    pub fn cached_compose_path(&self) -> Option<PathBuf> {
        self.state
            .chap_compose_source
            .cached_file()
            .map(|name| self.chaps_dir().join(name))
    }

    /// The compose project name this deployment records, when it records one.
    ///
    /// `None` is a `project.yaml` written before the field existed: compose is
    /// still naming that deployment after its directory, and the next `sync`
    /// writes that same name down. See [`ProjectState::compose_project`].
    pub fn compose_project(&self) -> Option<&str> {
        let name = self.state.compose_project.trim();
        (!name.is_empty()).then_some(name)
    }

    /// The compose project name this deployment has, recorded or not: the
    /// recorded one, or the one compose derives from the directory name.
    ///
    /// Answered without asking docker, which is what makes it usable in
    /// `status` and in the volume checks.
    pub fn compose_project_name(&self) -> Option<String> {
        match self.compose_project() {
            Some(name) => Some(name.to_string()),
            None => derived_project_name(&self.dir),
        }
    }

    /// The prefix compose puts in front of every named volume of this
    /// deployment: `<project>_`.
    pub fn volume_prefix(&self) -> Option<String> {
        self.compose_project_name().map(|name| format!("{name}_"))
    }

    /// The name docker holds one of this deployment's named volumes under:
    /// the compose project name, then the name the compose file declares.
    ///
    /// Answered without asking docker, like [`Project::volume_prefix`] it is
    /// built on, so the commands that remove a volume can name it whether or
    /// not a daemon is reachable. `None` where this directory has no compose
    /// project name at all, and then there is no volume to name either.
    pub fn prefixed_volume(&self, volume: &str) -> Option<String> {
        self.volume_prefix()
            .map(|prefix| format!("{prefix}{volume}"))
    }

    /// Absolute paths of the ordered `-f` list.
    pub fn compose_file_paths(&self) -> Vec<PathBuf> {
        self.state
            .compose_files
            .iter()
            .map(|f| self.dir.join(f))
            .collect()
    }

    /// Host ports already claimed inside this deployment: the enabled models
    /// and the enabled components. Anything with no published port claims
    /// nothing.
    pub fn used_ports(&self) -> BTreeSet<u16> {
        let mut ports: BTreeSet<u16> = self
            .state
            .models
            .values()
            .filter_map(|m| m.host_port)
            .collect();
        for component in crate::components::Component::ALL {
            ports.extend(self.state.components.port_of(*component));
        }
        ports
    }

    /// Absolute path of the OCS instance config this project scaffolds.
    pub fn ocs_config_path(&self) -> PathBuf {
        self.dir
            .join(crate::components::OCS_DIR)
            .join(crate::components::OCS_CONFIG_FILE)
    }

    /// Absolute path of the OCS dataset plugin directory.
    ///
    /// Never created by this CLI: the directory existing is how a deployment
    /// says it has plugins, so creating an empty one would turn the mount on
    /// for every project.
    pub fn ocs_plugins_path(&self) -> PathBuf {
        self.dir
            .join(crate::components::OCS_DIR)
            .join(crate::components::OCS_PLUGINS_DIR)
    }

    /// The host port chap-core's API is actually published on, and which file
    /// decided that.
    ///
    /// `.env` wins, because compose reads it last and it is compose that does
    /// the publishing: an operator who wrote `CHAP_API_PORT=18000` there moved
    /// the port, whatever `.chaps/project.yaml` still records. Everything in
    /// this CLI that has to reach or reserve the API goes through here, so
    /// `status`, `doctor` and the `up` preflight all talk about the port the
    /// deployment really uses.
    ///
    /// A line that is commented out, empty or not a port is not an answer, and
    /// the recorded value stands.
    pub fn api_port_in_effect(&self) -> (u16, ApiPortSource) {
        match self.env_api_port() {
            Some(port) => (port, ApiPortSource::Env),
            None => (self.state.api_port, ApiPortSource::Project),
        }
    }

    /// [`Project::api_port_in_effect`] without the provenance.
    pub fn effective_api_port(&self) -> u16 {
        self.api_port_in_effect().0
    }

    /// A usable `CHAP_API_PORT` from this project's `.env`.
    ///
    /// Best-effort by design, like every other read of that file: a project
    /// written with `init --no-env` has none to read. Port 0 means "any free
    /// port" to the kernel and nothing at all to compose, so it is not a
    /// valid override either.
    fn env_api_port(&self) -> Option<u16> {
        let body = std::fs::read_to_string(self.dir.join(ENV_FILE)).ok()?;
        crate::dotenv::non_empty(&body, API_PORT_ENV_VAR)?
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
    }

    /// Base URL of chap-core's API on this machine: the one this deployment
    /// runs, or the one elsewhere it names.
    pub fn api_url(&self) -> String {
        match &self.state.components.chap_core_external {
            Some(external) => external.url.trim_end_matches('/').to_string(),
            None => format!("http://localhost:{}", self.effective_api_port()),
        }
    }

    /// The path prefix chap-core mounts its whole API under, from this
    /// project's `.env`, or `""` for the usual deployment that has none.
    ///
    /// `CHAP_ROOT_PATH` is never written by `init`: it is what a deployment
    /// behind a reverse proxy adds by hand, and chap-core passes it to FastAPI
    /// as `root_path`, so every route moves with it - `/docs` included. Read
    /// best-effort like every other line of that file, and normalised to a
    /// leading slash and no trailing one so it can be concatenated.
    ///
    /// Only the addresses a person is sent to use it. `chaps status` asks
    /// `/health`, which the container's own healthcheck reaches without the
    /// prefix, so nothing that probes the API is changed by this.
    pub fn root_path(&self) -> String {
        let Ok(body) = std::fs::read_to_string(self.dir.join(ENV_FILE)) else {
            return String::new();
        };
        let Some(value) = crate::dotenv::non_empty(&body, ROOT_PATH_ENV_VAR) else {
            return String::new();
        };
        let path = value
            .trim()
            .trim_end_matches('/')
            .trim_start_matches('/')
            .to_string();
        if path.is_empty() {
            String::new()
        } else {
            format!("/{path}")
        }
    }

    /// Where a person reaches chap-core's API from this machine: the origin plus
    /// whatever [`Project::root_path`] prefixes it with, without a trailing
    /// slash.
    pub fn api_base(&self) -> String {
        format!("{}{}", self.api_url(), self.root_path())
    }

    /// chap-core's read-only proxy to one model service, the way to reach a
    /// model that publishes no host port of its own.
    pub fn proxy_url(&self, service_id: &str) -> String {
        format!("{}/v2/services/{service_id}/run/", self.api_url())
    }
}

/// Whether `name` is one file directly in the deployment directory: no
/// separator, no drive, nothing that climbs out of it.
///
/// Every compose file chaps renders is such a name (`compose.yml`,
/// `compose.<service>.yml`), and `sync` writes and removes them with
/// `dir.join(name)` - which an absolute path or a `..` would take outside the
/// directory. State that arrives from elsewhere, a restored backup above all,
/// is held to this before anything joins it.
pub fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':', '\0'])
}

/// Refuse state whose compose file names are not plain file names. See
/// [`is_plain_file_name`].
fn check_file_names(state: &ProjectState) -> Result<()> {
    let way_out = "chaps only writes `compose.<service>.yml` there; correct the entry by hand, \
                   or run `chaps init --force` to rebuild the state";
    for (id, model) in &state.models {
        if !is_plain_file_name(&model.compose_file) {
            return Err(anyhow::anyhow!(
                "`{CHAPS_DIR}/{MODELS_FILE}` gives {id} the compose file `{}`, which is not a \
                 file in this deployment's directory; {way_out}",
                model.compose_file
            ));
        }
    }
    for name in state.compose_files.iter().chain(&state.rendered_files) {
        if !is_plain_file_name(name) {
            return Err(anyhow::anyhow!(
                "`{CHAPS_DIR}/{PROJECT_FILE}` lists the compose file `{name}`, which is not a \
                 file in this deployment's directory; {way_out}"
            ));
        }
    }
    Ok(())
}

/// A YAML document with nothing but comments and blank lines.
fn is_blank_yaml(body: &str) -> bool {
    body.lines()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
}

fn write_atomically(path: &Path, body: &str) -> Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, body).map_err(|e| anyhow::anyhow!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| anyhow::anyhow!("renaming {} to {}: {e}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
