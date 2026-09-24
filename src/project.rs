//! `.chaps/` — the directory that records what a deployment is meant to be.
//!
//! Three YAML files: `project.yaml` holds the project-wide settings,
//! `models.yaml` the enabled model set and `components.yaml` the enabled
//! components. They are intent; the compose files at the project root are
//! artifacts rendered from them by `chaps sync`.

use crate::components::{COMPONENTS_FILE, Components};
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
/// The `.env` variable the chap-core images read their tag from.
pub const CHAP_TAG_ENV_VAR: &str = "CHAP_IMAGE_TAG";
/// The `.env` variable [`CHAPS_COMPOSE`] reads the API's host port from.
pub const API_PORT_ENV_VAR: &str = "CHAP_API_PORT";

/// `project.yaml` schema version written by this CLI.
pub const SCHEMA_VERSION: u32 = 1;
/// Host port range model overlays are allocated from.
pub const DEFAULT_PORT_RANGE: (u16, u16) = (5001, 5999);
/// Host port chap-core's API is published on unless `--api-port` says
/// otherwise.
pub const DEFAULT_API_PORT: u16 = 8000;

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

const PROJECT_HEADER: &str = "\
# .chaps/project.yaml - managed by chaps. Written by `chaps init`; `rendered_files` is
# updated by `chaps sync` (which `chaps up` runs first). Compose files at the project
# root are rendered from this directory; edit here, then run `chaps sync`.
";

const MODELS_HEADER: &str = "\
# .chaps/models.yaml - managed by chaps. The enabled model set, edited by
# `chaps models enable|disable`, `chaps ui` and `chaps update`.
# `chaps sync` renders one compose.<service_id>.yml per entry plus compose.marketplace.yml.
";

const COMPONENTS_HEADER: &str = "\
# .chaps/components.yaml - managed by chaps. What this deployment is made of, edited by
# `chaps init --with|--without` and `chaps components enable|disable`.
# chap-core is on unless it was turned off; `chaps sync` renders one compose file per
# enabled component. A file that does not mention a component leaves it at its default.
";

/// Where the base `compose.yml` is rendered from.
///
/// `compose.yml` is an artifact like the overlays: [`crate::compose::sync`]
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
}

impl ComposeSource {
    /// The cached copy's file name inside [`CHAPS_DIR`], for a fetched source.
    pub fn cached_file(&self) -> Option<String> {
        match self {
            ComposeSource::Embedded => None,
            ComposeSource::Fetched { tag, .. } => Some(cached_compose_file(tag)),
        }
    }

    /// One-line description for human output.
    pub fn describe(&self) -> String {
        match self {
            ComposeSource::Embedded => "the compose.ghcr.yml built into this binary".to_string(),
            ComposeSource::Fetched { tag, .. } => format!("chap-core compose.ghcr.yml at {tag}"),
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
    /// e.g. `chaps-cli 0.1.0`.
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
            generated_by: format!("chaps-cli {}", env!("CARGO_PKG_VERSION")),
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
    /// `user:group` the container runs as.
    pub user: String,
    /// `Some("linux/amd64")` for R-INLA services.
    pub platform: Option<String>,
    /// Overlay file name, relative to the project directory.
    pub compose_file: String,
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

        let project_body = format!("{PROJECT_HEADER}{}", serde_yaml_ng::to_string(&self.state)?);
        write_atomically(&chaps.join(PROJECT_FILE), &project_body)?;

        let models_body = format!(
            "{MODELS_HEADER}{}",
            serde_yaml_ng::to_string(&self.state.models)?
        );
        write_atomically(&chaps.join(MODELS_FILE), &models_body)?;

        let components_body = format!(
            "{COMPONENTS_HEADER}{}",
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

    /// Base URL of chap-core's API on this machine.
    pub fn api_url(&self) -> String {
        format!("http://localhost:{}", self.effective_api_port())
    }

    /// chap-core's read-only proxy to one model service, the way to reach a
    /// model that publishes no host port of its own.
    pub fn proxy_url(&self, service_id: &str) -> String {
        format!("{}/v2/services/{service_id}/run/", self.api_url())
    }
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
mod tests {
    use super::*;

    fn enabled(port: Option<u16>) -> EnabledModel {
        EnabledModel {
            service_id: "chapkit-ewars-model".into(),
            image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
            image_tag: "sha-fa880a1".into(),
            version: "1.0.0".into(),
            channel: Some(Channel::Stable),
            host_port: port,
            data_dir: "/app/data".into(),
            user: "chapkit:chapkit".into(),
            platform: Some("linux/amd64".into()),
            compose_file: "compose.chapkit-ewars-model.yml".into(),
        }
    }

    #[test]
    fn save_then_load_round_trips_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let state = ProjectState {
            chap_image_tag: "v1.2.3".into(),
            rendered_files: vec!["compose.marketplace.yml".into()],
            models: BTreeMap::from([("chapkit_ewars_model".to_string(), enabled(Some(5001)))]),
            ..ProjectState::default()
        };
        let project = Project {
            dir: dir.path().to_path_buf(),
            state,
        };
        project.save().unwrap();

        let chaps = dir.path().join(CHAPS_DIR);
        let project_body = std::fs::read_to_string(chaps.join(PROJECT_FILE)).unwrap();
        assert!(project_body.starts_with("# .chaps/project.yaml - managed by chaps"));
        assert!(project_body.contains("\nschema_version: 1\n"));
        assert!(project_body.contains("chap_image_tag: v1.2.3"));
        assert!(project_body.contains("\napi_port: 8000\n"));
        assert!(
            !project_body.contains("models:"),
            "models live in their own file"
        );
        let models_body = std::fs::read_to_string(chaps.join(MODELS_FILE)).unwrap();
        assert!(models_body.starts_with("# .chaps/models.yaml - managed by chaps"));
        assert!(models_body.contains("\nchapkit_ewars_model:\n"));
        assert!(models_body.contains("host_port: 5001"));
        assert!(models_body.contains("channel: stable"));
        assert!(
            std::fs::read_dir(&chaps).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")),
            "temp files are renamed away"
        );

        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.dir, dir.path());
        assert_eq!(loaded.state.chap_image_tag, "v1.2.3");
        assert_eq!(loaded.state.port_range, DEFAULT_PORT_RANGE);
        assert_eq!(loaded.state.api_port, DEFAULT_API_PORT);
        assert_eq!(loaded.state.rendered_files, vec!["compose.marketplace.yml"]);
        assert_eq!(
            loaded.state.models["chapkit_ewars_model"],
            enabled(Some(5001))
        );
    }

    #[test]
    fn a_model_with_no_published_port_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState {
                models: BTreeMap::from([("chapkit_ewars_model".to_string(), enabled(None))]),
                ..ProjectState::default()
            },
        };
        project.save().unwrap();
        let body = std::fs::read_to_string(dir.path().join(CHAPS_DIR).join(MODELS_FILE)).unwrap();
        assert!(body.contains("host_port: null"), "{body}");

        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.state.models["chapkit_ewars_model"].host_port, None);
        assert!(loaded.used_ports().is_empty(), "nothing is published");
    }

    #[test]
    fn a_models_file_written_before_the_port_was_optional_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let chaps = dir.path().join(CHAPS_DIR);
        std::fs::create_dir_all(&chaps).unwrap();
        std::fs::write(
            chaps.join(PROJECT_FILE),
            "schema_version: 1\n\
             generated_by: chaps-cli 0.1.0\n\
             chap_image_tag: latest\n\
             registry_url: https://example.test/registry.yaml\n\
             compose_files:\n\
             - compose.yml\n\
             - compose.marketplace.yml\n\
             port_range:\n\
             - 5001\n\
             - 5999\n",
        )
        .unwrap();
        std::fs::write(
            chaps.join(MODELS_FILE),
            "chapkit_ewars_model:\n\
             \x20 service_id: chapkit-ewars-model\n\
             \x20 image: ghcr.io/chap-models/chapkit_ewars_model\n\
             \x20 image_tag: sha-fa880a1\n\
             \x20 version: 1.0.0\n\
             \x20 channel: stable\n\
             \x20 host_port: 5001\n\
             \x20 data_dir: /app/data\n\
             \x20 user: chapkit:chapkit\n\
             \x20 platform: linux/amd64\n\
             \x20 compose_file: compose.chapkit-ewars-model.yml\n",
        )
        .unwrap();

        let loaded = Project::load(dir.path()).unwrap();
        // The old two-entry -f list and the missing api_port both default.
        assert_eq!(loaded.state.api_port, DEFAULT_API_PORT);
        assert_eq!(
            loaded.state.compose_files,
            vec!["compose.yml", "compose.marketplace.yml"]
        );
        assert_eq!(
            loaded.state.models["chapkit_ewars_model"].host_port,
            Some(5001),
            "a recorded number is still a published port"
        );
        assert_eq!(loaded.used_ports(), BTreeSet::from([5001]));
    }

    #[test]
    fn the_api_and_proxy_urls_follow_the_api_port() {
        let project = Project {
            dir: PathBuf::from("/tmp/chapx"),
            state: ProjectState {
                api_port: 8123,
                ..ProjectState::default()
            },
        };
        assert_eq!(project.api_url(), "http://localhost:8123");
        assert_eq!(
            project.proxy_url("chapkit-ewars-model"),
            "http://localhost:8123/v2/services/chapkit-ewars-model/run/"
        );
    }

    /// `.env` is what compose reads, so it is what every URL and every port
    /// reservation in this CLI has to follow. The recorded value is the
    /// fallback, not the answer.
    #[test]
    fn the_api_port_in_effect_comes_from_env_before_the_recorded_state() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState {
                api_port: 8123,
                ..ProjectState::default()
            },
        };

        // No `.env` at all: a project written with `init --no-env`.
        assert_eq!(project.api_port_in_effect(), (8123, ApiPortSource::Project));

        let env = dir.path().join(ENV_FILE);
        std::fs::write(&env, "CHAP_API_PORT=18000\n").unwrap();
        assert_eq!(project.api_port_in_effect(), (18000, ApiPortSource::Env));
        assert_eq!(project.effective_api_port(), 18000);
        assert_eq!(project.api_url(), "http://localhost:18000");
        assert_eq!(
            project.proxy_url("chapkit-ewars-model"),
            "http://localhost:18000/v2/services/chapkit-ewars-model/run/"
        );

        // The shapes compose accepts are the shapes this reads.
        for body in [
            "export CHAP_API_PORT=18000\n",
            "CHAP_API_PORT=\"18000\"\n",
            "CHAP_API_PORT=8000\nCHAP_API_PORT=18000\n",
            "CHAP_API_PORT = 18000\n",
        ] {
            std::fs::write(&env, body).unwrap();
            assert_eq!(
                project.api_port_in_effect(),
                (18000, ApiPortSource::Env),
                "{body:?}"
            );
        }

        // And anything that is not a usable port leaves the recorded one in
        // place: a commented line, an empty assignment, a number no port can
        // hold, 0 (which means "any port" to the kernel and nothing to
        // compose) and plain nonsense.
        for body in [
            "# CHAP_API_PORT=18000\n",
            "CHAP_API_PORT=\n",
            "CHAP_API_PORT=70000\n",
            "CHAP_API_PORT=0\n",
            "CHAP_API_PORT=eight thousand\n",
            "POSTGRES_DB=chap_core\n",
            "",
        ] {
            std::fs::write(&env, body).unwrap();
            assert_eq!(
                project.api_port_in_effect(),
                (8123, ApiPortSource::Project),
                "{body:?}"
            );
        }
    }

    #[test]
    fn the_api_port_source_names_its_file() {
        assert_eq!(ApiPortSource::Env.label(), "from .env");
        assert_eq!(ApiPortSource::Project.label(), "from .chaps/project.yaml");
        // It is a field of `status --json`, so the spelling is part of that
        // document.
        assert_eq!(
            serde_json::to_value(ApiPortSource::Env).unwrap(),
            serde_json::json!("env")
        );
        assert_eq!(
            serde_json::to_value(ApiPortSource::Project).unwrap(),
            serde_json::json!("project")
        );
    }

    #[test]
    fn the_compose_source_round_trips_both_ways() {
        let dir = tempfile::tempdir().unwrap();
        let fetched = ComposeSource::Fetched {
            url: "https://raw.githubusercontent.com/dhis2-chap/chap-core/v2.3.1/compose.ghcr.yml"
                .into(),
            tag: "v2.3.1".into(),
            sha256: "a".repeat(64),
        };
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState {
                chap_image_tag: "v2.3.1".into(),
                chap_compose_source: fetched.clone(),
                ..ProjectState::default()
            },
        };
        project.save().unwrap();

        let body = std::fs::read_to_string(dir.path().join(CHAPS_DIR).join(PROJECT_FILE)).unwrap();
        assert!(body.contains("chap_compose_source:"));
        assert!(body.contains("kind: fetched"));
        assert!(body.contains("tag: v2.3.1"));

        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.state.chap_compose_source, fetched);
        assert_eq!(
            loaded.cached_compose_path(),
            Some(
                dir.path()
                    .join(CHAPS_DIR)
                    .join("compose.chap-core.v2.3.1.yml")
            )
        );

        // The embedded default writes its own tag and has no cached copy.
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState::default(),
        };
        project.save().unwrap();
        let body = std::fs::read_to_string(dir.path().join(CHAPS_DIR).join(PROJECT_FILE)).unwrap();
        assert!(body.contains("kind: embedded"));
        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.state.chap_compose_source, ComposeSource::Embedded);
        assert_eq!(loaded.cached_compose_path(), None);
    }

    #[test]
    fn a_project_file_written_before_the_compose_source_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let chaps = dir.path().join(CHAPS_DIR);
        std::fs::create_dir_all(&chaps).unwrap();
        std::fs::write(
            chaps.join(PROJECT_FILE),
            "schema_version: 1\n\
             generated_by: chaps-cli 0.1.0\n\
             chap_image_tag: latest\n\
             registry_url: https://example.test/registry.yaml\n\
             compose_files:\n\
             - compose.yml\n\
             - compose.marketplace.yml\n\
             port_range:\n\
             - 5001\n\
             - 5999\n",
        )
        .unwrap();
        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.state.chap_compose_source, ComposeSource::Embedded);
        // The same file predates the auth block, which loads as "off".
        assert_eq!(loaded.state.auth, AuthState::default());
        assert!(!loaded.state.auth.is_on());
    }

    #[test]
    fn the_auth_block_round_trips_as_two_booleans_and_no_secret() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState {
                auth: AuthState {
                    api_token: true,
                    registration_key: true,
                },
                ..ProjectState::default()
            },
        };
        project.save().unwrap();

        let body = std::fs::read_to_string(dir.path().join(CHAPS_DIR).join(PROJECT_FILE)).unwrap();
        assert!(body.contains("auth:\n"), "{body}");
        assert!(body.contains("  api_token: true\n"), "{body}");
        assert!(body.contains("  registration_key: true\n"), "{body}");

        let loaded = Project::load(dir.path()).unwrap();
        assert!(loaded.state.auth.api_token && loaded.state.auth.registration_key);
        assert!(loaded.state.auth.is_on());

        // One half on is still on.
        assert!(
            AuthState {
                api_token: false,
                registration_key: true,
            }
            .is_on()
        );
    }

    #[test]
    fn the_compose_project_name_round_trips_and_defaults_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState {
                compose_project: "mychap-1ab2c3".into(),
                ..ProjectState::default()
            },
        };
        project.save().unwrap();
        let body = std::fs::read_to_string(dir.path().join(CHAPS_DIR).join(PROJECT_FILE)).unwrap();
        assert!(
            body.contains("\ncompose_project: mychap-1ab2c3\n"),
            "{body}"
        );

        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.compose_project(), Some("mychap-1ab2c3"));
        assert_eq!(
            loaded.compose_project_name().as_deref(),
            Some("mychap-1ab2c3")
        );
        assert_eq!(loaded.volume_prefix().as_deref(), Some("mychap-1ab2c3_"));
        assert_eq!(
            loaded
                .prefixed_volume(&crate::compose::volume_name("chapkit_ewars_model"))
                .as_deref(),
            Some("mychap-1ab2c3_ck_chapkit_ewars_model_data")
        );
        assert_eq!(
            loaded.prefixed_volume("ocs_data").as_deref(),
            Some("mychap-1ab2c3_ocs_data")
        );

        // A file written before the field existed loads as "not recorded",
        // and the name it has is the one compose derives from the directory.
        let chaps = dir.path().join(CHAPS_DIR);
        std::fs::write(
            chaps.join(PROJECT_FILE),
            "schema_version: 1\n\
             generated_by: chaps-cli 0.2.2\n\
             chap_image_tag: latest\n\
             registry_url: https://example.test/registry.yaml\n\
             compose_files:\n\
             - compose.yml\n\
             - compose.marketplace.yml\n\
             port_range:\n\
             - 5001\n\
             - 5999\n",
        )
        .unwrap();
        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.compose_project(), None);
        assert_eq!(
            loaded.compose_project_name(),
            derived_project_name(dir.path()),
            "an old project is still named after its directory"
        );
    }

    #[test]
    fn a_generated_name_is_a_slug_and_six_hex_characters() {
        let dir = std::path::PathBuf::from("/srv/My CHAP (prod)");
        let name = new_compose_project_name(&dir).unwrap();
        let (slug, suffix) = name.rsplit_once('-').expect("a suffix");
        assert_eq!(slug, "my-chap-prod");
        assert_eq!(suffix.len(), PROJECT_SUFFIX_BYTES * 2);
        assert!(
            suffix
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );

        // Compose's own rule: lowercase, and it starts with a letter or digit.
        let first = name.chars().next().unwrap();
        assert!(
            first.is_ascii_lowercase() || first.is_ascii_digit(),
            "{name}"
        );
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{name}"
        );

        // Two deployments of the same name get two different names, which is
        // the whole point of the suffix.
        let again = new_compose_project_name(&dir).unwrap();
        assert_ne!(name, again);
    }

    #[test]
    fn the_slug_is_what_compose_accepts_whatever_the_directory_is_called() {
        assert_eq!(project_slug("demo"), "demo");
        assert_eq!(project_slug("My CHAP"), "my-chap");
        assert_eq!(project_slug("chap_prod.eu"), "chap-prod-eu");
        assert_eq!(project_slug("--weird--"), "weird");
        assert_eq!(project_slug("2026-deploy"), "2026-deploy");
        // Nothing usable at all still has to yield a legal name.
        assert_eq!(project_slug(""), FALLBACK_SLUG);
        assert_eq!(project_slug("深度"), FALLBACK_SLUG);
        assert_eq!(project_slug("..."), FALLBACK_SLUG);
        // And a very long one is cut where it still reads.
        let long = project_slug(&"a".repeat(100));
        assert_eq!(long.len(), MAX_SLUG);

        assert_eq!(compose_project_name("demo", "1ab2c3"), "demo-1ab2c3");
    }

    #[test]
    fn the_derived_name_is_the_one_compose_would_have_used() {
        // Compose lowercases, drops everything outside [a-z0-9_-] and trims
        // leading separators. Verified against docker compose 5.5.1.
        assert_eq!(normalized_project_name("demo").as_deref(), Some("demo"));
        assert_eq!(
            normalized_project_name("My Chap").as_deref(),
            Some("mychap")
        );
        assert_eq!(normalized_project_name("demo.1").as_deref(), Some("demo1"));
        assert_eq!(
            normalized_project_name("-demo_x").as_deref(),
            Some("demo_x")
        );
        assert_eq!(normalized_project_name("Démo").as_deref(), Some("dmo"));
        assert_eq!(
            normalized_project_name("chap+prod").as_deref(),
            Some("chapprod")
        );
        // Nothing left: compose would refuse to name a project after it, so
        // there is nothing for chaps to record either.
        assert_eq!(normalized_project_name("..."), None);
        assert_eq!(normalized_project_name(""), None);

        assert_eq!(
            derived_project_name(Path::new("/srv/My Chap")).as_deref(),
            Some("mychap")
        );
    }

    #[test]
    fn the_cached_file_name_is_tagged_and_safe() {
        assert_eq!(
            cached_compose_file("v2.3.1"),
            "compose.chap-core.v2.3.1.yml"
        );
        assert_eq!(
            cached_compose_file("master"),
            "compose.chap-core.master.yml"
        );
        assert_eq!(
            cached_compose_file("feature/x y"),
            "compose.chap-core.feature_x_y.yml"
        );
    }

    #[test]
    fn a_missing_or_blank_models_file_means_no_models() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState::default(),
        };
        project.save().unwrap();
        let models = dir.path().join(CHAPS_DIR).join(MODELS_FILE);
        assert!(models.is_file());
        assert!(Project::load(dir.path()).unwrap().state.models.is_empty());

        std::fs::write(&models, "# nothing enabled\n\n").unwrap();
        assert!(Project::load(dir.path()).unwrap().state.models.is_empty());

        std::fs::remove_file(&models).unwrap();
        assert!(Project::load(dir.path()).unwrap().state.models.is_empty());
    }

    #[test]
    fn the_components_file_round_trips_and_shapes_the_f_list() {
        let dir = tempfile::tempdir().unwrap();
        let mut components = Components::default();
        components.ocs.enabled = true;
        components.ocs.port = 9010;
        components.s3.enabled = true;
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState {
                components: components.clone(),
                ..ProjectState::default()
            },
        };
        project.save().unwrap();

        let body = std::fs::read_to_string(dir.path().join(CHAPS_DIR).join(COMPONENTS_FILE))
            .expect("components.yaml is written beside the others");
        assert!(body.starts_with("# .chaps/components.yaml - managed by chaps"));
        assert!(body.contains("\nchap-core:\n"), "{body}");
        assert!(body.contains("  port: 9010\n"), "{body}");

        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.state.components, components);
        assert_eq!(
            compose_files_for(&loaded.state.components),
            vec![
                "compose.yml",
                "compose.chaps.yml",
                "compose.ocs.yml",
                "compose.s3.yml",
                "compose.marketplace.yml"
            ]
        );
        // The component ports are claimed against the model allocator too.
        assert_eq!(loaded.used_ports(), BTreeSet::from([9010]));
        assert_eq!(
            loaded.ocs_config_path(),
            dir.path().join("ocs").join("climate-service.yaml")
        );
    }

    #[test]
    fn a_project_written_before_components_existed_is_chap_core_alone() {
        let dir = tempfile::tempdir().unwrap();
        let chaps = dir.path().join(CHAPS_DIR);
        std::fs::create_dir_all(&chaps).unwrap();
        std::fs::write(
            chaps.join(PROJECT_FILE),
            "schema_version: 1\n\
             generated_by: chaps-cli 0.2.1\n\
             chap_image_tag: latest\n\
             registry_url: https://example.test/registry.yaml\n\
             compose_files:\n\
             - compose.yml\n\
             - compose.chaps.yml\n\
             - compose.marketplace.yml\n\
             port_range:\n\
             - 5001\n\
             - 5999\n",
        )
        .unwrap();

        // No components.yaml at all: nothing to migrate, and the deployment is
        // exactly what it was.
        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.state.components, Components::default());
        assert!(loaded.state.components.chap_core.enabled);
        assert!(!loaded.state.components.ocs.enabled);
        assert_eq!(
            compose_files_for(&loaded.state.components),
            default_compose_files()
        );

        // A comment-only file reads the same way.
        std::fs::write(chaps.join(COMPONENTS_FILE), "# nothing set\n\n").unwrap();
        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.state.components, Components::default());
    }

    #[test]
    fn a_deployment_without_chap_core_drops_the_base_files_from_the_list() {
        let mut components = Components::default();
        components.chap_core.enabled = false;
        components.ocs.enabled = true;
        assert_eq!(
            compose_files_for(&components),
            vec!["compose.ocs.yml", "compose.marketplace.yml"]
        );
    }

    #[test]
    fn load_on_an_empty_dir_is_not_a_project() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!Project::exists(dir.path()));
        let err = Project::load(dir.path()).expect_err("empty dir is not a project");
        match err.downcast_ref::<ChapError>() {
            Some(ChapError::NotAProject(p)) => assert_eq!(p, dir.path()),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn find_walks_up_to_the_nearest_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("deploy");
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        Project {
            dir: root.clone(),
            state: ProjectState {
                chap_image_tag: "v9".into(),
                ..ProjectState::default()
            },
        }
        .save()
        .unwrap();

        for start in [&root, &root.join("a"), &nested] {
            let found = Project::find(start).unwrap();
            assert_eq!(found.dir, root, "from {}", start.display());
            assert_eq!(found.state.chap_image_tag, "v9");
        }
        assert_eq!(Project::find_root(&nested).as_deref(), Some(root.as_path()));

        // A closer project wins over a farther one.
        Project {
            dir: nested.clone(),
            state: ProjectState::default(),
        }
        .save()
        .unwrap();
        assert_eq!(Project::find(&nested).unwrap().dir, nested);
        assert_eq!(Project::find(&root.join("a")).unwrap().dir, root);
    }

    #[test]
    fn find_names_the_start_directory_when_nothing_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let start = dir.path().join("x").join("y");
        std::fs::create_dir_all(&start).unwrap();
        let err = Project::find(&start).expect_err("no project above a temp dir");
        match err.downcast_ref::<ChapError>() {
            Some(ChapError::NotAProject(p)) => assert_eq!(p, &start),
            other => panic!("wrong error: {other:?}"),
        }
        assert!(Project::find_root(&start).is_none());
    }

    #[test]
    fn exists_and_helpers_reflect_the_state() {
        let dir = tempfile::tempdir().unwrap();
        let state = ProjectState {
            models: BTreeMap::from([
                ("a".to_string(), enabled(Some(5001))),
                ("b".to_string(), enabled(Some(5004))),
                ("c".to_string(), enabled(None)),
            ]),
            ..ProjectState::default()
        };
        let project = Project {
            dir: dir.path().to_path_buf(),
            state,
        };
        project.save().unwrap();

        assert!(Project::exists(dir.path()));
        assert_eq!(project.used_ports(), BTreeSet::from([5001, 5004]));
        assert_eq!(
            project.compose_file_paths(),
            vec![
                dir.path().join(BASE_COMPOSE),
                dir.path().join(CHAPS_COMPOSE),
                dir.path().join(MARKETPLACE_COMPOSE),
            ]
        );
    }
}
