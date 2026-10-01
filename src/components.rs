//! `.chaps/components.yaml` — which pieces a deployment is made of.
//!
//! A deployment is chap-core plus whatever else was asked for. chap-core is a
//! component like the others, on by default; OCS (Open Climate Service), an
//! S3-compatible object store and DHIS2 are opt-in. The file is intent, like
//! `project.yaml` and `models.yaml`: `chaps sync` renders one compose file per
//! enabled component from it.
//!
//! Every field carries a `serde` default, so a project written before this
//! file existed loads as "chap-core on, nothing else" — which is exactly what
//! it was. Nothing has to be migrated.
//!
//! Two fields in here are records rather than intent, and each says so where it
//! is declared: [`OcsComponent::read_only`] mirrors what
//! `ocs/climate-service.yaml` says, and [`Dhis2Component::connected_at`] notes
//! when `chaps dhis2 connect` last finished. Neither is ever read as evidence -
//! OCS reads its own config file, and only `chaps dhis2 show` asks DHIS2.

use crate::error::{ChapError, Result};
use serde::{Deserialize, Serialize};

mod external;
mod notes;
mod seed;

pub use external::*;
pub use notes::*;
pub use seed::*;

/// The enabled component set, inside [`crate::project::CHAPS_DIR`].
pub const COMPONENTS_FILE: &str = "components.yaml";

/// Compose file rendered for the `ocs` component.
pub const OCS_COMPOSE: &str = "compose.ocs.yml";
/// Compose file rendered for the `s3` component.
pub const S3_COMPOSE: &str = "compose.s3.yml";
/// Compose file rendered for the `dhis2` component.
pub const DHIS2_COMPOSE: &str = "compose.dhis2.yml";

/// Directory, relative to the project, holding the OCS instance config.
pub const OCS_DIR: &str = "ocs";
/// The OCS instance config, inside [`OCS_DIR`]. Mounted read-only at
/// `/app/climate-service.yaml`.
pub const OCS_CONFIG_FILE: &str = "climate-service.yaml";
/// Dataset plugins, inside [`OCS_DIR`]. Mounted read-only at
/// [`OCS_PLUGINS_TARGET`] when the directory exists.
pub const OCS_PLUGINS_DIR: &str = "plugins";
/// Where [`OCS_PLUGINS_DIR`] is mounted, and what the `plugins_dir` key of the
/// instance config then points at.
pub const OCS_PLUGINS_TARGET: &str = "/app/plugins";
/// The instance config key naming the plugin directory.
pub const OCS_PLUGINS_KEY: &str = "plugins_dir";
/// The instance config key that turns ingestion over HTTP off.
pub const OCS_READ_ONLY_KEY: &str = "read_only";

/// Image OCS is deployed from. No release tags are published yet, so the
/// default pin is the moving `main` tag.
pub const OCS_IMAGE: &str = "ghcr.io/dhis2/open-climate-service";
/// Tag `ocs` follows unless `.env` pins another.
pub const OCS_DEFAULT_TAG: &str = "main";
/// Container port OCS listens on, and the port chap-core reaches it at.
pub const OCS_CONTAINER_PORT: u16 = 9000;
/// Host port `ocs` publishes unless `--port` says otherwise: chaps' 87xx block,
/// away from the 9000 so much else defaults to (see
/// [`crate::project::DEFAULT_API_PORT`]).
pub const OCS_DEFAULT_PORT: u16 = 8790;
/// The `.env` variable that moves the OCS image pin.
pub const OCS_TAG_ENV_VAR: &str = "OCS_IMAGE_TAG";
/// The variable naming the public origin OCS builds its STAC and openEO links
/// from, which is what a deployment behind a reverse proxy needs.
pub const OCS_BASE_URL_ENV_VAR: &str = "CLIMATE_SERVICE_BASE_URL";

/// The dataset credential variables an OCS deployment is given, in the order
/// they are written and reported.
///
/// `ECMWF_DATASTORES_*` is the Copernicus Climate Data Store and `EDH_API_KEY`
/// is the Earth Data Hub. They are not alternatives: the ERA5-Land monthly
/// datasets come from the store alone, the 1991-2020 day-of-year normals from
/// the hub alone, and every daily dataset reads both, choosing per period, so a
/// daily ingestion reaching into the present needs both accounts. `CDSE_S3_*` is
/// the Copernicus Data Space Ecosystem, for the CLMS GPP dataset plugin - which
/// is the operator's own, in `ocs/plugins/`, not part of OCS. WorldPop and
/// CHIRPS3 are public and need none of them. `docs/components.md` maps each
/// dataset family to the account it needs.
///
/// Each of the five is read as `os.getenv(...) or <a credentials file>`, so a
/// variable that arrives empty is a variable nothing has: the compose file can
/// pass all five unconditionally and a deployment that sets none behaves exactly
/// as one whose compose file never mentioned them. Which code does the reading
/// differs, and only one of them is OCS: the `ecmwf-datastores-client` library
/// OCS calls reads `ECMWF_DATASTORES_*` and falls back to `~/.ecmwfdatastoresrc`,
/// OCS itself reads `EDH_API_KEY` and falls back to `~/.netrc`, and the GPP
/// plugin reads `CDSE_S3_*` and falls back to the `[cdse]` profile in
/// `~/.aws/credentials`.
pub const OCS_DATA_SOURCE_ENV_VARS: &[&str] = &[
    "ECMWF_DATASTORES_URL",
    "ECMWF_DATASTORES_KEY",
    "EDH_API_KEY",
    "CDSE_S3_ACCESS_KEY",
    "CDSE_S3_SECRET_KEY",
];

/// The Copernicus Climate Data Store endpoint, as the commented `.env`
/// placeholder carries it: the one data source variable with a value worth
/// pre-filling, since it is the same for everyone.
pub const OCS_ECMWF_URL: &str = "https://cds.climate.copernicus.eu/api";

/// Image the `s3` component runs: RustFS, an S3-compatible object store.
pub const S3_IMAGE: &str = "rustfs/rustfs";
/// Tag `s3` follows unless `.env` pins another.
pub const S3_DEFAULT_TAG: &str = "latest";
/// Container port the object store listens on.
pub const S3_CONTAINER_PORT: u16 = 9000;
/// The `.env` variable that moves the object store's image pin.
pub const S3_TAG_ENV_VAR: &str = "S3_IMAGE_TAG";
/// The `.env` variables holding the object store's root credentials.
pub const S3_ACCESS_KEY_ENV_VAR: &str = "S3_ACCESS_KEY";
pub const S3_SECRET_KEY_ENV_VAR: &str = "S3_SECRET_KEY";
/// Bucket the one-shot init service creates for OCS.
pub const S3_BUCKET: &str = "ocs";

/// Directory, relative to the project, holding the DHIS2 instance config.
pub const DHIS2_DIR: &str = "dhis2";
/// The DHIS2 instance config, inside [`DHIS2_DIR`]. Mounted read-only at
/// `/opt/dhis2/dhis.conf`, where DHIS2 requires it.
pub const DHIS2_CONFIG_FILE: &str = "dhis.conf";

/// Image the `dhis2` component runs. Multi-arch since 2.39.1-rc, so no overlay
/// or component file pins a platform for it.
pub const DHIS2_IMAGE: &str = "dhis2/core";
/// Minor line `dhis2` follows unless `.env` pins another. A two-part tag rather
/// than a full version, so a patch release arrives with a `docker pull`.
pub const DHIS2_DEFAULT_TAG: &str = "2.42";
/// The `.env` variable that moves the DHIS2 image pin.
pub const DHIS2_TAG_ENV_VAR: &str = "DHIS2_IMAGE_TAG";
/// Host port `dhis2` publishes unless something says otherwise: chaps' 87xx
/// block rather than the container's 8080, which a DHIS2 already running on
/// this machine is likely to hold (see [`crate::project::DEFAULT_API_PORT`]).
pub const DHIS2_DEFAULT_PORT: u16 = 8780;
/// The `.env` variable holding the DHIS2 database password, generated once and
/// never rewritten: the database volume was created with it.
pub const DHIS2_DB_PASSWORD_ENV_VAR: &str = "DHIS2_DB_PASSWORD";
/// The `.env` variable holding the key DHIS2 encrypts stored credentials with.
///
/// At least 24 characters, or DHIS2 stops on `ENCRYPTION_PASSWORD_TOO_SHORT`;
/// empty and it encrypts with a password compiled into its own source, which
/// every other DHIS2 in the world also has.
pub const DHIS2_ENCRYPTION_PASSWORD_ENV_VAR: &str = "DHIS2_ENCRYPTION_PASSWORD";
/// The `.env` variable that moves the seed dump the one-shot fetches.
pub const DHIS2_SEED_ENV_VAR: &str = "DHIS2_DB_DUMP_URL";
/// The `.env` variable that replaces the JVM options, the heap sizes among them.
pub const DHIS2_JAVA_ENV_VAR: &str = "DHIS2_JAVA_TOOL_OPTIONS";
/// What the compose file passes when [`DHIS2_JAVA_ENV_VAR`] is unset, written
/// into `.env` as the commented starting point an operator edits.
pub const DHIS2_DEFAULT_JAVA_OPTIONS: &str = "-Xms2g -Xmx4g -XX:+UseG1GC";

/// The DHIS2 climate demo database published for each minor line, keyed by the
/// minor.
///
/// A table rather than a URL built from the tag, because the published path does
/// not follow from it (the 2.42 line publishes `climate/laos/2.42/laos.sql.gz`).
/// Only dumps that are there belong in it: the 2.41 one this used to list is
/// gone, and a dead URL here fails the seed one-shot and with it `chaps up`. A
/// minor this does not list has no dump chaps can name, so such a deployment
/// starts empty and is told so.
pub const DHIS2_SEED_DUMPS: &[(&str, &str)] =
    &[("2.42", crate::compose::render::DHIS2_DEFAULT_SEED_URL)];

/// The DHIS2 versions chaps offers to pick between, newest first: the minor
/// lines it has been run against. Any other tag still works when given.
pub const DHIS2_VERSIONS: &[&str] = &["2.43", "2.42", "2.41"];

/// One component, by name.
///
/// The names are what `chaps components enable NAME` and `init --with NAME`
/// accept, and what `components.yaml` keys its blocks on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Component {
    /// chap-core, its worker, Valkey and PostgreSQL. On by default.
    ChapCore,
    /// Open Climate Service, beside chap-core.
    Ocs,
    /// An S3-compatible object store, for OCS to keep its objects in.
    S3,
    /// DHIS2 and its own database, beside CHAP.
    Dhis2,
}

impl Component {
    /// Every component, in the order they are listed and rendered.
    pub const ALL: &'static [Component] = &[
        Component::ChapCore,
        Component::Ocs,
        Component::S3,
        Component::Dhis2,
    ];

    /// The name this component is spelled with everywhere.
    pub fn name(self) -> &'static str {
        match self {
            Component::ChapCore => "chap-core",
            Component::Ocs => "ocs",
            Component::S3 => "s3",
            Component::Dhis2 => "dhis2",
        }
    }

    /// One line saying what it is, for `chaps components list`.
    pub fn summary(self) -> &'static str {
        match self {
            Component::ChapCore => "CHAP itself: chap-core, its worker, Valkey and PostgreSQL",
            Component::Ocs => "Open Climate Service: climate data, reachable at http://ocs:9000",
            Component::S3 => "RustFS, an S3-compatible object store OCS will keep objects in",
            Component::Dhis2 => "DHIS2 and its own database, for a deployment that wants one",
        }
    }

    /// The component a name stands for.
    ///
    /// Accepts the underscore spelling of `chap-core` as well, since the
    /// marketplace ids around it use underscores.
    pub fn from_name(name: &str) -> Result<Component> {
        let wanted = name.trim().to_ascii_lowercase().replace('_', "-");
        Component::ALL
            .iter()
            .copied()
            .find(|c| c.name() == wanted)
            .ok_or_else(|| ChapError::UnknownComponent(name.trim().to_string()).into())
    }

    /// The compose service whose container says whether this component is
    /// running.
    ///
    /// The three opt-in components name their own service, so this is
    /// [`Component::name`] for them; chap-core's is upstream's `chap`, which is
    /// the one reason this is a method rather than that one. Every caller that
    /// asks docker whether a component is up goes through here, so
    /// `chaps status`, `chaps doctor` and `chaps open` all judge it by the same
    /// container.
    pub fn service(self) -> &'static str {
        match self {
            Component::ChapCore => crate::compose::API_SERVICE,
            Component::Ocs => crate::compose::OCS_SERVICE,
            Component::S3 => crate::compose::S3_SERVICE,
            Component::Dhis2 => crate::compose::DHIS2_SERVICE,
        }
    }

    /// Whether this component takes a host port at all.
    pub fn takes_port(self) -> bool {
        matches!(self, Component::Ocs | Component::S3 | Component::Dhis2)
    }

    /// The compose file `chaps sync` renders for this component.
    ///
    /// `None` for chap-core: its services come from upstream's `compose.yml`
    /// and `compose.chaps.yml`, which are the deployment itself rather than one
    /// component's file, so there is nothing a `disable` or a re-init could
    /// remove on its own.
    pub fn compose_file(self) -> Option<&'static str> {
        match self {
            Component::ChapCore => None,
            Component::Ocs => Some(OCS_COMPOSE),
            Component::S3 => Some(S3_COMPOSE),
            Component::Dhis2 => Some(DHIS2_COMPOSE),
        }
    }

    /// The directory this component keeps its own files in, relative to the
    /// project directory.
    ///
    /// The operator's half of a component, and the opposite of
    /// [`Component::compose_file`] in every way that matters: the compose file is
    /// rendered from `.chaps/` and `chaps sync` can always write it again, while
    /// what is in here is scaffolded once and never rewritten, so an edit made
    /// to it exists nowhere else. `ocs/climate-service.yaml` and
    /// `dhis2/dhis.conf` are the two, and the whole directory is named rather
    /// than the file, because what an operator puts beside that file -
    /// `ocs/plugins/` is the case in point - is theirs on the same grounds.
    ///
    /// The one place this is written down, because a name in one place is a name
    /// the next component is forgotten from: [`crate::backup::project_files`]
    /// collects the directory of every component that has one, so a fifth
    /// component's files are in the archive the moment this method answers for
    /// it.
    ///
    /// `None` for chap-core, whose every setting is in `.chaps/` and `.env`, and
    /// for `s3`, which is configured by its compose file and two `.env` keys and
    /// has nothing on disk for anyone to edit.
    pub fn dir(self) -> Option<&'static str> {
        match self {
            Component::ChapCore | Component::S3 => None,
            Component::Ocs => Some(OCS_DIR),
            Component::Dhis2 => Some(DHIS2_DIR),
        }
    }

    /// The named volumes this component keeps its data in, as the compose file
    /// `chaps sync` renders declares them, in the order they are reported.
    ///
    /// A slice rather than one name because a component is not limited to one
    /// volume: `ocs` and `s3` have a single data volume each, and a component
    /// that is several services can keep one volume per service. Empty for
    /// chap-core: its volumes are upstream's own, declared in a compose file
    /// this CLI does not write, and `chaps down --volumes` is what removes
    /// them. So an empty slice is "nothing chaps names here", which is what
    /// `components disable --purge` refuses on.
    ///
    /// Every volume the component's compose file declares belongs here, whether
    /// or not it holds anything worth keeping: this is the list `chaps doctor`
    /// decides leftovers by, so a volume left out would be reported as one on a
    /// deployment that legitimately has it, and `--purge` would leave it behind.
    /// `dhis2_dump` is the case in point - a download cache the one-shot refills
    /// on its own - and [`crate::backup::COMPONENT_VOLUMES`] is where it is left
    /// out, because an archive is the one place it costs something.
    pub fn volumes(self) -> &'static [&'static str] {
        match self {
            Component::Ocs => &[crate::compose::render::OCS_VOLUME],
            Component::S3 => &[crate::compose::render::S3_VOLUME],
            Component::Dhis2 => &[
                crate::compose::render::DHIS2_HOME_VOLUME,
                crate::compose::render::DHIS2_DB_VOLUME,
                crate::compose::render::DHIS2_DUMP_VOLUME,
            ],
            Component::ChapCore => &[],
        }
    }

    /// Whether the compose service `service` belongs to this component.
    ///
    /// The one place this rule lives, because every caller that stops or
    /// inspects a component's containers has to draw the same line. A component
    /// owns the service named after it and every `<name>-<suffix>` sibling
    /// beside it: the one-shots that prepare it, `s3-init` and `dhis2-prep`
    /// among them, and the further services a component that is more than one
    /// container brings. The hyphen is the whole of the boundary, so `s3` owns
    /// `s3-init` and not some service whose name merely starts with those two
    /// characters.
    ///
    /// chap-core is the exception, and the reason this is a method rather than
    /// a table of names: its services are upstream's - `chap`, `chap-worker`,
    /// Valkey, PostgreSQL - named in a file this CLI does not write and sharing
    /// no prefix with the component, so the only thing they have in common is
    /// that no other component claims them. Asking the other components is what
    /// keeps that true: a hand-kept list of the names that are not chap-core's
    /// would hand a fourth component's services to chap-core, and `chaps
    /// components disable chap-core` would stop them. A model service falls to
    /// chap-core on the same rule, which is the answer it has always given, and
    /// harmless because `disable` refuses while any model is enabled.
    pub fn owns_service(self, service: &str) -> bool {
        match self {
            Component::ChapCore => !Component::ALL
                .iter()
                .any(|other| *other != Component::ChapCore && other.owns_service(service)),
            named => service
                .strip_prefix(named.name())
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('-')),
        }
    }
}

/// `true`, for the `serde` default of a flag that is on unless said otherwise.
fn enabled_by_default() -> bool {
    true
}

fn default_ocs_port() -> Option<u16> {
    Some(OCS_DEFAULT_PORT)
}

fn default_ocs_tag() -> String {
    OCS_DEFAULT_TAG.to_string()
}

/// chap-core's own block. It has no settings of its own: the API port, the
/// image tag and the model set all live in `project.yaml` and `models.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapCoreComponent {
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

impl Default for ChapCoreComponent {
    fn default() -> ChapCoreComponent {
        ChapCoreComponent { enabled: true }
    }
}

/// The `ocs` block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OcsComponent {
    #[serde(default)]
    pub enabled: bool,
    /// Host port OCS is published on. OCS has a web interface, so it is
    /// published by default rather than hidden on the compose network; `None`
    /// keeps it on the compose network alone, for a deployment whose way in is
    /// a reverse proxy.
    #[serde(default = "default_ocs_port")]
    pub port: Option<u16>,
    /// The public origin OCS builds absolute links from, for an instance
    /// reached through a proxy rather than on its own port. `None` leaves OCS
    /// to compose them from the request, which behind a proxy names the
    /// internal address.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Whether the instance config turns ingestion over HTTP off.
    ///
    /// A record of what `ocs/climate-service.yaml` says, not the setting
    /// itself: that file is the operator's, and OCS reads it and not this.
    #[serde(default)]
    pub read_only: bool,
    /// The tag the compose file defaults to, and the one `.env` pins.
    #[serde(default = "default_ocs_tag")]
    pub image_tag: String,
}

impl Default for OcsComponent {
    fn default() -> OcsComponent {
        OcsComponent {
            enabled: false,
            port: Some(OCS_DEFAULT_PORT),
            base_url: None,
            read_only: false,
            image_tag: OCS_DEFAULT_TAG.to_string(),
        }
    }
}

/// The `s3` block.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct S3Component {
    #[serde(default)]
    pub enabled: bool,
    /// Host port the object store is published on. `None` — the default —
    /// keeps it inside the compose network, which is where OCS reaches it.
    #[serde(default)]
    pub port: Option<u16>,
}

fn default_dhis2_port() -> Option<u16> {
    Some(DHIS2_DEFAULT_PORT)
}

fn default_dhis2_tag() -> String {
    DHIS2_DEFAULT_TAG.to_string()
}

fn default_dhis2_image() -> String {
    DHIS2_IMAGE.to_string()
}

/// The `dhis2` block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Dhis2Component {
    #[serde(default)]
    pub enabled: bool,
    /// Host port DHIS2 is published on. DHIS2 is a web application people log
    /// into, so it is published by default rather than hidden on the compose
    /// network; `None` keeps it inside, for a deployment whose way in is a
    /// reverse proxy.
    #[serde(default = "default_dhis2_port")]
    pub port: Option<u16>,
    /// The tag the compose file defaults to, and the one `.env` pins.
    #[serde(default = "default_dhis2_tag")]
    pub image_tag: String,
    /// The image repository: `dhis2/core` for releases, and another (such as
    /// `dhis2/core-dev` with tag `master`, the next version) for trying one
    /// that is not released yet.
    #[serde(default = "default_dhis2_image")]
    pub image: String,
    /// What the database is restored from the first time it is created.
    #[serde(default)]
    pub seed: Dhis2Seed,
    /// When `chaps dhis2 connect` last got as far as a verified route and both
    /// apps here, as a UTC timestamp. `None` for a deployment no such run has
    /// been recorded for, which is every deployment until one happens.
    ///
    /// **A record of a command that ran, not a fact about DHIS2, and never
    /// evidence.** It exists for one purpose: to stop `chaps up` and
    /// `chaps status` naming [`dhis2_connect_hint`] on a deployment that has
    /// already been through it. Nothing decides anything else by it. The route
    /// can be deleted, repointed or disabled in DHIS2's own interface a minute
    /// later and this timestamp will not move, because nothing here asks - only
    /// `chaps dhis2 show` does, and it asks DHIS2 rather than this file.
    ///
    /// So it is cleared wherever the thing it was true of can have gone:
    /// [`Components::set_enabled`] forgets it when the component is switched
    /// off - the seed dump a re-enabled DHIS2 restores ships a `chap` route
    /// pointing at somebody else's CHAP, and a stale record would hide exactly
    /// that - and `chaps down --volumes` forgets it with `dhis2_db` itself.
    ///
    /// A `components.yaml` written before this field loads as `None`, which is
    /// what it was: nothing had recorded a connect.
    #[serde(default)]
    pub connected_at: Option<String>,
}

impl Default for Dhis2Component {
    fn default() -> Dhis2Component {
        Dhis2Component {
            enabled: false,
            port: Some(DHIS2_DEFAULT_PORT),
            image_tag: DHIS2_DEFAULT_TAG.to_string(),
            image: DHIS2_IMAGE.to_string(),
            seed: Dhis2Seed::Default,
            connected_at: None,
        }
    }
}

/// A DHIS2 image tag as given on the command line, or why it will not do.
/// `flag` is the flag's spelling, for the message.
pub fn dhis2_tag_arg(tag: &str, flag: &str) -> Result<String> {
    let tag = tag.trim();
    if tag.is_empty() || tag.contains(char::is_whitespace) {
        return Err(anyhow::anyhow!(
            "{flag} takes an image tag such as `2.42` or `2.43.1`"
        ));
    }
    Ok(tag.to_string())
}

/// A DHIS2 image repository as given on the command line, or why it will not
/// do. The tag has a flag of its own, so a `repo:tag` is split for the
/// operator rather than rendered as a reference with two tags.
pub fn dhis2_image_arg(image: &str, flag: &str, tag_flag: &str) -> Result<String> {
    let image = image.trim();
    let last = image.rsplit('/').next().unwrap_or(image);
    if image.is_empty() || image.contains(char::is_whitespace) || last.contains(':') {
        return Err(anyhow::anyhow!(
            "{flag} takes a repository such as `dhis2/core-dev`, and the version goes in \
             {tag_flag}: `{flag} dhis2/core-dev {tag_flag} master`"
        ));
    }
    Ok(image.to_string())
}

/// The name every container that has to reach this machine resolves it by.
pub const HOST_GATEWAY: &str = "host.docker.internal";

/// The whole of `components.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Components {
    #[serde(rename = "chap-core", default)]
    pub chap_core: ChapCoreComponent,
    #[serde(default)]
    pub ocs: OcsComponent,
    #[serde(default)]
    pub s3: S3Component,
    #[serde(default)]
    pub dhis2: Dhis2Component,
    /// A DHIS2 somewhere else, when one has been recorded. Absent from the
    /// file otherwise, so a deployment that never used it reads as it did.
    #[serde(
        rename = "dhis2-external",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub dhis2_external: Option<ExternalDhis2>,
    /// A chap-core somewhere else, when one has been recorded. Absent from the
    /// file otherwise.
    #[serde(
        rename = "chap-core-external",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub chap_core_external: Option<ExternalChapCore>,
}

impl Components {
    /// Whether one component is on.
    pub fn is_enabled(&self, component: Component) -> bool {
        match component {
            Component::ChapCore => self.chap_core.enabled,
            Component::Ocs => self.ocs.enabled,
            Component::S3 => self.s3.enabled,
            Component::Dhis2 => self.dhis2.enabled,
        }
    }

    /// Turn one on or off, leaving its settings alone.
    ///
    /// [`Dhis2Component::connected_at`] is the one thing a switch-off does not
    /// leave alone, and it is not a setting: it records that a
    /// `chaps dhis2 connect` finished against a DHIS2 this deployment no longer
    /// has. Forgetting it here rather than in `chaps components disable` is
    /// what keeps the browser's components page and `chaps init --force` from
    /// each having to remember to do it.
    pub fn set_enabled(&mut self, component: Component, on: bool) {
        match component {
            // Either this deployment runs chap-core or it names one elsewhere,
            // never both, so turning its own on forgets the other.
            Component::ChapCore => {
                self.chap_core.enabled = on;
                if on {
                    self.chap_core_external = None;
                }
            }
            Component::Ocs => self.ocs.enabled = on,
            Component::S3 => self.s3.enabled = on,
            Component::Dhis2 => {
                self.dhis2.enabled = on;
                if !on {
                    self.dhis2.connected_at = None;
                }
            }
        }
    }

    /// Whether there is a chap-core API to talk to: this deployment's own, or
    /// one elsewhere that it names.
    pub fn has_chap_core_api(&self) -> bool {
        self.chap_core.enabled || self.chap_core_external.is_some()
    }

    /// Whether `chaps up` and `chaps status` should still name
    /// `chaps dhis2 connect`.
    ///
    /// Local knowledge and nothing else: this deployment has a DHIS2, it has a
    /// chap-core for the route to point at, and
    /// [`Dhis2Component::connected_at`] records no connect. chap-core is part
    /// of it because `chaps dhis2 connect` refuses without one - the route
    /// would aim at a service that is not there - so a `--without chap-core`
    /// deployment is never asked for something it cannot do.
    ///
    /// An external DHIS2 ([`ExternalDhis2`]) is asked the same question of its
    /// own record: it is the DHIS2 `chaps dhis2 connect` would connect.
    pub fn dhis2_needs_connecting(&self) -> bool {
        self.has_chap_core_api()
            && match &self.dhis2_external {
                Some(external) => external.connected_at.is_none(),
                None => self.dhis2.enabled && self.dhis2.connected_at.is_none(),
            }
    }

    /// The `connected_at` record of whichever DHIS2 `chaps dhis2` talks to:
    /// the external one when there is one, the component otherwise.
    pub fn dhis2_connected_at_mut(&mut self) -> &mut Option<String> {
        match &mut self.dhis2_external {
            Some(external) => &mut external.connected_at,
            None => &mut self.dhis2.connected_at,
        }
    }

    /// The host port one component publishes, when it publishes one.
    pub fn port_of(&self, component: Component) -> Option<u16> {
        match component {
            Component::ChapCore => None,
            Component::Ocs => self.ocs.port.filter(|_| self.ocs.enabled),
            Component::S3 => self.s3.port.filter(|_| self.s3.enabled),
            Component::Dhis2 => self.dhis2.port.filter(|_| self.dhis2.enabled),
        }
    }

    /// Every enabled component, in [`Component::ALL`] order.
    pub fn enabled(&self) -> Vec<Component> {
        Component::ALL
            .iter()
            .copied()
            .filter(|c| self.is_enabled(*c))
            .collect()
    }

    /// `chap-core, ocs` — the enabled set as one cell. A chap-core elsewhere
    /// leads it as `chap-core (elsewhere)`: not a component this deployment
    /// runs, and still the one its models and its DHIS2 route talk to.
    pub fn label(&self) -> String {
        let mut names: Vec<&str> = self.enabled().iter().map(|c| c.name()).collect();
        if self.chap_core_external.is_some() {
            names.insert(0, "chap-core (elsewhere)");
        }
        if names.is_empty() {
            return "none".to_string();
        }
        names.join(", ")
    }

    /// The compose files the enabled components contribute, in the order they
    /// belong in the `-f` list: after `compose.chaps.yml`, before the
    /// marketplace umbrella.
    pub fn compose_files(&self) -> Vec<String> {
        self.enabled()
            .into_iter()
            .filter_map(Component::compose_file)
            .map(str::to_string)
            .collect()
    }

    /// The URL a human reaches OCS at from this machine, or `None` for an
    /// instance that publishes no host port - or none at all.
    ///
    /// A disabled component reaches nothing, so this answers on the same terms
    /// as [`Components::port_of`]: a recorded port belongs to an instance that
    /// is switched off until something turns it on.
    pub fn ocs_url(&self) -> Option<String> {
        self.port_of(Component::Ocs)
            .map(|port| format!("http://localhost:{port}"))
    }

    /// The one cell that says how OCS is reached: its own host port, or the
    /// compose network plus whatever proxy the base URL names.
    ///
    /// An unpublished instance is not unreachable, it is reached somewhere
    /// else, so the base URL is printed where the address would be rather than
    /// being a setting nothing on the screen mentions.
    pub fn ocs_reach(&self) -> String {
        match (&self.ocs_url(), &self.ocs.base_url) {
            (Some(url), _) => url.clone(),
            (None, Some(base)) => format!("internal (proxy: {base})"),
            (None, None) => "internal".to_string(),
        }
    }

    /// The one cell that says how DHIS2 is reached: its own host port, or the
    /// compose network.
    ///
    /// The same terms [`Components::ocs_reach`] answers on, minus the proxy: the
    /// origin DHIS2 builds absolute links from is `server.base.url` in
    /// `dhis2/dhis.conf`, which is the operator's file, so chaps records no
    /// second copy of it to print here.
    pub fn dhis2_reach(&self) -> String {
        match self.port_of(Component::Dhis2) {
            Some(port) => format!("http://localhost:{port}"),
            None => "internal".to_string(),
        }
    }

    /// The dump the first `chaps up` would restore, or `None` for a database
    /// DHIS2 migrates from empty.
    ///
    /// `Dhis2Seed::Default` resolves through [`dhis2_seed_dump`], so a minor line
    /// with no published dump answers `None` exactly as `none` does - and
    /// [`dhis2_unknown_seed`] is the line that tells the operator which of the
    /// two happened.
    pub fn dhis2_seed_source(&self) -> Option<&str> {
        match &self.dhis2.seed {
            Dhis2Seed::Default => dhis2_seed_dump(&self.dhis2.image_tag),
            Dhis2Seed::None => None,
            Dhis2Seed::From(source) => Some(source.as_str()),
        }
    }

    /// Whether the seed was left at `default` and the pinned minor line has no
    /// dump chaps knows of.
    ///
    /// The database then starts empty, and [`dhis2_unknown_seed`] is the line
    /// that says so *with the reason*. One rule in one place, because that line
    /// and the general seed note would otherwise both say "starts empty" and a
    /// reader would get the same sentence twice.
    pub fn dhis2_seed_is_unknown(&self) -> bool {
        self.dhis2.seed == Dhis2Seed::Default && self.dhis2_seed_source().is_none()
    }
}

#[cfg(test)]
mod tests;
