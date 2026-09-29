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

/// The minor line of a DHIS2 tag: everything up to the second dot.
///
/// `2.42`, `2.42.1` and `2.42.1.1` are all the `2.42` line, which is what
/// [`DHIS2_SEED_DUMPS`] is keyed on. A tag with fewer than two dots is its own
/// key, so a `dev` or a `latest` someone pinned answers rather than panicking.
pub fn dhis2_minor(tag: &str) -> &str {
    let tag = tag.trim();
    let mut dots = tag.match_indices('.');
    match (dots.next(), dots.next()) {
        (Some(_), Some((at, _))) => &tag[..at],
        _ => tag,
    }
}

/// The dump a pinned tag is seeded from, or `None` when chaps knows of none for
/// that minor line.
pub fn dhis2_seed_dump(tag: &str) -> Option<&'static str> {
    let minor = dhis2_minor(tag);
    DHIS2_SEED_DUMPS
        .iter()
        .find(|(line, _)| *line == minor)
        .map(|(_, url)| *url)
}

/// What a `dhis2` component's database is restored from the first time it is
/// created.
///
/// Three answers, written in `components.yaml` with the same words
/// `--dhis2-seed` takes: `default`, `none`, or the dump itself. A dump is a URL
/// the one-shot downloads or a path in the project directory it reads, and the
/// value decides which rather than a second setting: an `http://` or `https://`
/// prefix is the whole of the rule.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Dhis2Seed {
    /// The dump the pinned minor line publishes, from [`DHIS2_SEED_DUMPS`].
    #[default]
    Default,
    /// An empty database, which DHIS2 migrates itself on the first start.
    None,
    /// This dump, whatever the tag says.
    From(String),
}

impl Dhis2Seed {
    /// The value as `components.yaml` writes it and `--dhis2-seed` takes it.
    pub fn as_str(&self) -> &str {
        match self {
            Dhis2Seed::Default => "default",
            Dhis2Seed::None => "none",
            Dhis2Seed::From(source) => source,
        }
    }

    /// The seed one written value names.
    ///
    /// The two words are matched trimmed and without case, because they arrive
    /// from a command line as well as from YAML, and an empty value is the
    /// default rather than a dump whose name is nothing.
    pub fn parse(value: &str) -> Dhis2Seed {
        let value = value.trim();
        match value.to_ascii_lowercase().as_str() {
            "" | "default" => Dhis2Seed::Default,
            "none" => Dhis2Seed::None,
            _ => Dhis2Seed::From(value.to_string()),
        }
    }

    /// Whether this seed is a URL something has to download, which is the one
    /// shape `--offline` refuses.
    pub fn is_url(&self) -> bool {
        matches!(self, Dhis2Seed::From(source)
            if source.starts_with("http://") || source.starts_with("https://"))
    }
}

impl Serialize for Dhis2Seed {
    /// One string, so the file reads as `seed: default` rather than as a tagged
    /// enum nobody would type by hand.
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Dhis2Seed {
    /// An `Option<String>` rather than a `String`, so a hand-edited `seed:` with
    /// nothing after it is the default rather than a type error: YAML reads that
    /// as null, and "no value" is exactly what the default means.
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Dhis2Seed, D::Error> {
        Ok(match Option::<String>::deserialize(deserializer)? {
            Some(value) => Dhis2Seed::parse(&value),
            None => Dhis2Seed::Default,
        })
    }
}

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

/// A DHIS2 this deployment did not start, which `chaps dhis2` talks to in
/// place of the `dhis2` component. Recorded by `chaps dhis2 use`.
///
/// The shape of most real deployments: CHAP beside a DHIS2 that already runs
/// somewhere else. Nothing is rendered from it - there is no container - and
/// it cannot be recorded while the `dhis2` component is on, because the two
/// would each be "this deployment's DHIS2".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalDhis2 {
    /// Where DHIS2 answers, as this machine reaches it: the origin, plus the
    /// context path of an instance that is served under one.
    pub url: String,
    /// Where chap-core answers as DHIS2 reaches it, which is what the `chap`
    /// route points at. Not the compose network's `http://chap:8000`, which
    /// only a container of this deployment can resolve.
    pub chap_url: String,
    /// When `chaps dhis2 connect` last got as far as a verified route and both
    /// apps there. The same record, with the same caveats, as
    /// [`Dhis2Component::connected_at`].
    #[serde(default)]
    pub connected_at: Option<String>,
}

/// A chap-core this deployment did not start, which its model services
/// register with and the chap-core commands talk to. Recorded by
/// `chaps init --chap-core-url` or `chaps components enable chap-core --url`.
///
/// The shape of chap-core development: chap-core runs from its own checkout
/// on this machine, and chaps runs the model services around it. It cannot be
/// recorded while the `chap-core` component is on, because the two would each
/// be "this deployment's chap-core".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExternalChapCore {
    /// Where chap-core answers, as this machine reaches it.
    pub url: String,
    /// The host a model service registers itself under, which is what
    /// chap-core then calls it at, on the model's published host port.
    /// `localhost` for a chap-core running on this machine.
    #[serde(default = "default_models_host")]
    pub models_host: String,
}

fn default_models_host() -> String {
    "localhost".to_string()
}

impl ExternalChapCore {
    /// The URL a container of this deployment reaches chap-core at: a
    /// loopback host names the container itself in there, so it becomes the
    /// host gateway every overlay maps.
    pub fn url_from_containers(&self) -> String {
        for loopback in ["localhost", "127.0.0.1", "[::1]"] {
            for scheme in ["http://", "https://"] {
                let prefix = format!("{scheme}{loopback}");
                if let Some(rest) = self.url.strip_prefix(&prefix)
                    && (rest.is_empty() || rest.starts_with(':') || rest.starts_with('/'))
                {
                    return format!("{scheme}{HOST_GATEWAY}{rest}");
                }
            }
        }
        self.url.clone()
    }
}

/// An [`ExternalChapCore`] for `url`, refused unless it is an absolute
/// `http(s)` URL: the models append a path to it to register.
pub fn external_chap_core(url: &str) -> Result<ExternalChapCore> {
    let url = url.trim().trim_end_matches('/');
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(anyhow::anyhow!(
            "the chap-core URL has to be absolute, such as `http://localhost:8000`, \
             so `{url}` will not do: the models register at <URL>/v2/services/$register"
        ));
    }
    Ok(ExternalChapCore {
        url: url.to_string(),
        models_host: default_models_host(),
    })
}

/// The scheme, host and port of `url`, without any path: what DHIS2's
/// `route.remote_servers_allowed` takes.
pub fn origin(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let host = rest.split('/').next().unwrap_or(rest);
    format!("{scheme}://{host}")
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

    /// `chap-core, ocs` — the enabled set as one cell.
    pub fn label(&self) -> String {
        let names: Vec<&str> = self.enabled().iter().map(|c| c.name()).collect();
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

/// The note `components enable ocs` prints while there is no object store.
///
/// OCS does not read any S3 variable yet, so this is a heads-up rather than a
/// requirement: forcing a second container on a deployment for a contract that
/// does not exist would be worse than saying it is coming.
pub const S3_SOON_NOTE: &str = "OCS will soon need an S3-compatible object store; `chaps components enable s3` \
     adds one, and the OCS service then gets the S3_* variables it will read";

/// The note `components enable s3` prints on a deployment with no OCS.
///
/// The store is there for OCS to keep its objects in, and nothing else in a
/// chaps deployment writes to it - so a deployment that has one and no OCS runs
/// a container with nothing to put in it. The mirror image of
/// [`S3_SOON_NOTE`], and a note for the same reason: it is a container the
/// operator may well be adding first on purpose.
pub const S3_WITHOUT_OCS_NOTE: &str = "the object store is for OCS to keep its objects in, and this deployment has no OCS; \
     `chaps components enable ocs` adds one, and nothing else here writes to the store";

/// The note `components disable s3` prints while OCS is still enabled.
///
/// The next sync re-renders `compose.ocs.yml` without the `S3_*` block, which
/// is a change to a service the operator did not name. OCS does not read those
/// variables yet, so this says what went rather than warning about a breakage
/// there is not.
pub const S3_LEAVES_OCS_NOTE: &str = "the OCS service loses its S3_* variables on this sync; OCS does not read them yet, \
     and `chaps components enable s3` puts them back";

/// The note `components enable ocs` prints on the run that puts the data source
/// variables into `.env`.
///
/// `chaps sync` appends them commented out, and until now the only sign of it
/// was `written .env`. Which datasets need which key is the thing an operator
/// cannot guess from the variable names, so the line says that much and names
/// the command that reports what is set. "One or both" is as much of it as fits
/// on a line: the store alone, the hub alone and both together are each some
/// ERA5-Land dataset's answer, and `docs/components.md` says which is which.
pub const OCS_DATA_SOURCE_NOTE: &str = "the OCS data source variables are now in `.env`, commented out: ERA5-Land needs \
     one or both of ECMWF_DATASTORES_* and EDH_API_KEY, per dataset; WorldPop and \
     CHIRPS3 need none. `chaps auth show` reports which are set";

/// The note `components enable dhis2` and `init --with dhis2` print about what
/// the first start will do with the database.
///
/// The restore is the postgres entrypoint's own, so it happens once, on a data
/// directory it has just created, and never again - which is the half an
/// operator cannot see from the compose file and the half that decides whether
/// changing the seed later does anything. So the line says "once" and names the
/// command that makes the volume fresh again.
pub fn dhis2_seed_note(seed: Option<&str>) -> String {
    match seed {
        Some(source) => format!(
            "the first `chaps up` restores {source} into `dhis2_db`, once, on the database it \
             creates; after that only `chaps components disable dhis2 --purge` makes it happen again"
        ),
        None => "`dhis2_db` starts empty and DHIS2 migrates a new database into it; `seed:` in \
             `.chaps/components.yaml` names a dump instead, then run `chaps sync`"
            .to_string(),
    }
}

/// The note printed when the seed was left at `default` and the pinned minor
/// line publishes no dump chaps knows of.
///
/// Said rather than guessed: the published path does not follow from the tag
/// (see [`DHIS2_SEED_DUMPS`]), so constructing one would download a 404 on the
/// first start and leave the operator with an empty DHIS2 and no explanation.
pub fn dhis2_unknown_seed(tag: &str) -> String {
    format!(
        "chaps knows no DHIS2 demo dump for {}, so `dhis2_db` starts empty; name one with \
         `seed:` in `.chaps/components.yaml` (a URL or a path) and run `chaps sync`",
        dhis2_minor(tag)
    )
}

/// The note about how long the first start takes.
///
/// Every other service in a deployment answers in seconds; DHIS2 migrates its
/// whole schema before it serves a request, and under emulation that is a
/// quarter of an hour. An operator who does not know that reads the first
/// `chaps status` as a broken deployment.
pub const DHIS2_FIRST_START_NOTE: &str = "the first `chaps up` takes minutes before DHIS2 answers - it migrates its schema on the \
     way up - and `chaps logs dhis2` is where that shows";

/// The note that says the two halves are not connected yet, and what connects
/// them.
///
/// A DHIS2 and a CHAP started side by side cannot talk: the Modeling App
/// reaches chap-core through a DHIS2 route, which this deployment does not have
/// until something creates it. `chaps up` does not, on purpose - it is a thin
/// wrapper around compose, DHIS2's API is not ready when it returns, and the
/// steps need DHIS2 credentials and the network - so the command is named here,
/// where the component is added and the reader is already being told what the
/// first start does.
pub const DHIS2_CONNECT_NOTE: &str = "the Modeling App reaches chap-core through a DHIS2 route, and this deployment has none \
     yet; once DHIS2 answers, `chaps dhis2 connect` adds it, generates analytics and installs \
     the apps";

/// The line `chaps up` and `chaps status` close with while
/// [`Components::dhis2_needs_connecting`] holds.
///
/// [`DHIS2_CONNECT_NOTE`] is said where the component is added, which is before
/// DHIS2 exists: the reader then runs `chaps up`, waits out a restore and a
/// migration, and lands on a deployment where every row says `up` with that one
/// line minutes above them. So the two commands that report on a running
/// deployment say it again, and go on saying it until a connect is recorded.
///
/// It claims only what it knows. `chaps` has not connected this DHIS2 - that is
/// [`Dhis2Component::connected_at`], read off `.chaps/components.yaml` - rather
/// than "DHIS2 is not connected", which would be a claim about an instance
/// nothing here asked. Naming the command is safe either way, because every
/// `chaps dhis2` verb is idempotent.
///
/// `answering` is whether the caller has just seen DHIS2 answer. `chaps status`
/// has asked `/api/ping` and only says this when it answered; `chaps up` has
/// asked nothing and DHIS2 is minutes from its first request, so it says when.
pub fn dhis2_connect_hint(answering: bool) -> String {
    match answering {
        true => DHIS2_NOT_CONNECTED.to_string(),
        false => format!("{DHIS2_NOT_CONNECTED} once DHIS2 answers"),
    }
}

/// Why the `dhis2` component cannot go on while an external DHIS2 is recorded.
///
/// Both would be "this deployment's DHIS2", and `chaps dhis2` would have to
/// pick one without saying so.
pub fn dhis2_external_refusal(url: &str) -> String {
    format!(
        "this deployment already uses the external DHIS2 at {url}; run `chaps dhis2 use --clear` \
         first to deploy one of its own"
    )
}

/// The half of [`dhis2_connect_hint`] both shapes share.
const DHIS2_NOT_CONNECTED: &str =
    "chaps has not connected this DHIS2 to CHAP; run `chaps dhis2 connect`";

/// The note `chaps components disable dhis2` prints when it has just forgotten
/// a recorded connect.
///
/// Said only on the run that forgets something: a deployment that was never
/// connected has nothing to report, and the line would be noise on every
/// disable. See [`Dhis2Component::connected_at`] for why it is forgotten at
/// all.
pub const DHIS2_CONNECT_FORGOTTEN: &str = "the record of `chaps dhis2 connect` is forgotten with the component; a DHIS2 enabled \
     here again is asked to connect afresh";

/// The note `chaps down --volumes` prints when it has just removed the database
/// a recorded connect was true of.
///
/// The worst shape a stale record could make: `dhis2_db` is gone, the next
/// `chaps up` restores the seed dump into a new one, and that dump ships its own
/// `chap` route pointing at a server this deployment has nothing to do with. A
/// record that survived would suppress the one line that asks the operator to
/// repoint it.
pub const DHIS2_CONNECT_FORGOTTEN_WITH_VOLUME: &str = "the record of `chaps dhis2 connect` went with `dhis2_db`; the next `chaps up` restores \
     the seed dump, which ships a `chap` route of its own, so run `chaps dhis2 connect` again";

/// The same for a DHIS2 with no seed: the next `chaps up` migrates an empty
/// database, which has no route at all.
pub const DHIS2_CONNECT_FORGOTTEN_UNSEEDED: &str = "the record of `chaps dhis2 connect` went with `dhis2_db`; the next `chaps up` starts \
     an empty DHIS2 with no `chap` route, so run `chaps dhis2 connect` again once it answers";

/// The warning for a DHIS2 image tag that is moving while the database volume
/// is already there.
///
/// DHIS2 migrates a schema forward only. Starting an older image on a database a
/// newer one has migrated gives an instance that passes its own health check
/// while every API request answers 404, which is the worst shape a deployment
/// can be in: up, and wrong. So the line names `chaps backup` before anything
/// else.
pub fn dhis2_tag_change_note(from: &str, to: &str) -> String {
    format!(
        "the DHIS2 image moves from {from} to {to} and `dhis2_db` is already there: DHIS2 \
         migrates a schema forward only, so run `chaps backup` first - an older image on a \
         migrated database answers healthy while every API request 404s"
    )
}

/// The refusal of a command that only chap-core can answer, on a deployment
/// chap-core is not a component of. `what` names the command, in backticks.
pub fn needs_chap_core(what: &str) -> String {
    format!(
        "{what} needs chap-core, and this deployment has no chap-core; \
         `chaps components enable chap-core` adds it, or `--url URL` names one elsewhere"
    )
}

/// Refuse `what` when `components` leaves chap-core out and names none
/// elsewhere.
pub fn require_chap_core(components: &Components, what: &str) -> Result<()> {
    if components.has_chap_core_api() {
        return Ok(());
    }
    Err(anyhow::anyhow!(needs_chap_core(what)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_document_is_chap_core_and_nothing_else() {
        let components: Components = serde_yaml_ng::from_str("{}").unwrap();
        assert_eq!(components, Components::default());
        assert!(components.chap_core.enabled);
        assert!(!components.ocs.enabled);
        assert!(!components.s3.enabled);
        assert_eq!(components.label(), "chap-core");
        assert!(components.compose_files().is_empty());
    }

    /// A loopback chap-core is this machine to the models, which reach it over
    /// the host gateway; any other host is left as it is.
    #[test]
    fn an_external_chap_core_is_reached_from_containers_over_the_gateway() {
        let at = |url: &str| external_chap_core(url).unwrap().url_from_containers();
        assert_eq!(
            at("http://localhost:8000"),
            "http://host.docker.internal:8000"
        );
        assert_eq!(
            at("http://127.0.0.1:8000/"),
            "http://host.docker.internal:8000"
        );
        assert_eq!(at("http://localhost"), "http://host.docker.internal");
        assert_eq!(at("https://chap.example.org"), "https://chap.example.org");
        // A host that merely starts with "localhost" is not the loopback.
        assert_eq!(
            at("http://localhost.example.org"),
            "http://localhost.example.org"
        );
        assert!(external_chap_core("localhost:8000").is_err());
        assert_eq!(
            external_chap_core("http://x:1").unwrap().models_host,
            "localhost"
        );

        // Turning this deployment's own chap-core on forgets the other one.
        let mut components = Components {
            chap_core_external: Some(external_chap_core("http://localhost:8000").unwrap()),
            ..Components::default()
        };
        components.set_enabled(Component::ChapCore, false);
        assert!(components.has_chap_core_api());
        components.set_enabled(Component::ChapCore, true);
        assert!(components.chap_core_external.is_none());
    }

    #[test]
    fn an_origin_drops_the_path() {
        assert_eq!(
            origin("http://host.docker.internal:8000/api"),
            "http://host.docker.internal:8000"
        );
        assert_eq!(
            origin("https://chap.example.org"),
            "https://chap.example.org"
        );
    }

    #[test]
    fn the_settings_round_trip_with_their_defaults() {
        let components = Components {
            chap_core: ChapCoreComponent { enabled: true },
            ocs: OcsComponent {
                enabled: true,
                port: Some(9010),
                base_url: None,
                read_only: false,
                image_tag: "main".into(),
            },
            s3: S3Component {
                enabled: true,
                port: None,
            },
            dhis2: Dhis2Component {
                enabled: true,
                port: Some(18080),
                image_tag: "2.41.7".into(),
                image: "dhis2/core-dev".into(),
                seed: Dhis2Seed::From("dumps/laos.sql.gz".into()),
                connected_at: None,
            },
            dhis2_external: None,
            chap_core_external: None,
        };
        let text = serde_yaml_ng::to_string(&components).unwrap();
        assert!(text.contains("chap-core:\n"), "{text}");
        assert!(text.contains("  port: 9010\n"), "{text}");
        assert!(text.contains("  port: null\n"), "{text}");
        assert!(text.contains("  base_url: null\n"), "{text}");
        assert!(text.contains("  read_only: false\n"), "{text}");
        // The seed is one string, whichever of the three it is, so the block
        // reads as something an operator would type.
        assert!(text.contains("  seed: dumps/laos.sql.gz\n"), "{text}");
        // The one record in the file, and it is written whichever way round it
        // is, so a reader can see that nothing has connected this deployment.
        assert!(text.contains("  connected_at: null\n"), "{text}");
        assert_eq!(
            serde_yaml_ng::from_str::<Components>(&text).unwrap(),
            components
        );

        assert_eq!(components.label(), "chap-core, ocs, s3, dhis2");
        assert_eq!(
            components.compose_files(),
            vec!["compose.ocs.yml", "compose.s3.yml", "compose.dhis2.yml"]
        );
        assert_eq!(components.port_of(Component::Dhis2), Some(18080));
        assert_eq!(components.dhis2_reach(), "http://localhost:18080");
        assert_eq!(components.port_of(Component::Ocs), Some(9010));
        assert_eq!(components.port_of(Component::S3), None);
        assert_eq!(
            components.ocs_url().as_deref(),
            Some("http://localhost:9010")
        );
        assert_eq!(components.ocs_reach(), "http://localhost:9010");
    }

    /// The reverse-proxy shape: no host port, and a base URL in its place.
    #[test]
    fn an_unpublished_instance_reads_as_internal_and_names_its_proxy() {
        let mut components = Components::default();
        components.ocs.enabled = true;
        components.ocs.port = None;
        assert_eq!(components.ocs_url(), None);
        assert_eq!(components.port_of(Component::Ocs), None);
        assert_eq!(components.ocs_reach(), "internal");

        components.ocs.base_url = Some("https://ocs.example.org".to_string());
        assert_eq!(
            components.ocs_reach(),
            "internal (proxy: https://ocs.example.org)"
        );

        // A published port is the address, whatever the base URL says: that is
        // where this machine reaches it.
        components.ocs.port = Some(9000);
        assert_eq!(components.ocs_reach(), "http://localhost:9000");
    }

    /// `port: null` is how a deployment records "no host port", and it has to
    /// survive a round trip: the field defaults to 9000, so a null that read
    /// back as the default would republish the port on the next sync.
    #[test]
    fn a_null_port_survives_the_round_trip_and_a_missing_one_is_the_default() {
        let explicit: Components =
            serde_yaml_ng::from_str("ocs:\n  enabled: true\n  port: null\n").unwrap();
        assert_eq!(explicit.ocs.port, None);
        let text = serde_yaml_ng::to_string(&explicit).unwrap();
        assert_eq!(
            serde_yaml_ng::from_str::<Components>(&text)
                .unwrap()
                .ocs
                .port,
            None
        );

        let omitted: Components = serde_yaml_ng::from_str("ocs:\n  enabled: true\n").unwrap();
        assert_eq!(omitted.ocs.port, Some(OCS_DEFAULT_PORT));
    }

    /// The same invariant on the DHIS2 block: the port defaults to 8080, so a
    /// recorded `null` that read back as the default would republish the port on
    /// the next sync - and put a DHIS2 login page on the host of a deployment
    /// that deliberately keeps it behind a proxy.
    #[test]
    fn a_null_dhis2_port_survives_the_round_trip_and_a_missing_one_is_the_default() {
        let explicit: Components =
            serde_yaml_ng::from_str("dhis2:\n  enabled: true\n  port: null\n").unwrap();
        assert_eq!(explicit.dhis2.port, None);
        assert_eq!(explicit.dhis2_reach(), "internal");
        let text = serde_yaml_ng::to_string(&explicit).unwrap();
        assert_eq!(
            serde_yaml_ng::from_str::<Components>(&text)
                .unwrap()
                .dhis2
                .port,
            None
        );

        let omitted: Components = serde_yaml_ng::from_str("dhis2:\n  enabled: true\n").unwrap();
        assert_eq!(omitted.dhis2.port, Some(DHIS2_DEFAULT_PORT));
    }

    /// A `components.yaml` written before the block existed, and one written
    /// before a field in it did: both load, and neither turns DHIS2 on.
    #[test]
    fn a_components_file_from_before_dhis2_loads_with_the_block_off() {
        let old: Components =
            serde_yaml_ng::from_str("chap-core:\n  enabled: true\nocs:\n  enabled: true\n")
                .unwrap();
        assert!(!old.dhis2.enabled);
        assert_eq!(old.dhis2, Dhis2Component::default());
        assert_eq!(old.label(), "chap-core, ocs");
        assert!(!old.compose_files().contains(&DHIS2_COMPOSE.to_string()));

        // And a block with nothing but `enabled` keeps every default under it,
        // the seed among them.
        let partial: Components = serde_yaml_ng::from_str("dhis2:\n  enabled: true\n").unwrap();
        assert_eq!(partial.dhis2.image_tag, DHIS2_DEFAULT_TAG);
        assert_eq!(partial.dhis2.seed, Dhis2Seed::Default);
        assert_eq!(partial.compose_files(), vec![DHIS2_COMPOSE]);
    }

    /// The three seeds, in the spelling `components.yaml` holds and
    /// `--dhis2-seed` takes.
    #[test]
    fn a_seed_is_one_string_and_reads_back_as_what_was_written() {
        assert_eq!(Dhis2Seed::parse("default"), Dhis2Seed::Default);
        assert_eq!(Dhis2Seed::parse("  DEFAULT "), Dhis2Seed::Default);
        assert_eq!(Dhis2Seed::parse(""), Dhis2Seed::Default, "no value is none");
        assert_eq!(Dhis2Seed::parse("none"), Dhis2Seed::None);
        assert_eq!(Dhis2Seed::parse("None"), Dhis2Seed::None);
        let url = "https://databases.dhis2.org/climate/laos/2.42/laos.sql.gz";
        assert_eq!(Dhis2Seed::parse(url), Dhis2Seed::From(url.to_string()));
        assert_eq!(
            Dhis2Seed::parse(" dumps/mine.sql.gz "),
            Dhis2Seed::From("dumps/mine.sql.gz".to_string()),
            "the shell's whitespace is not part of the path"
        );

        // Only a URL is a download, which is the one shape --offline refuses.
        assert!(Dhis2Seed::parse(url).is_url());
        assert!(Dhis2Seed::parse("http://example.org/d.sql.gz").is_url());
        assert!(!Dhis2Seed::parse("dumps/mine.sql.gz").is_url());
        assert!(
            !Dhis2Seed::Default.is_url(),
            "the table decides, not the flag"
        );
        assert!(!Dhis2Seed::None.is_url());

        // A `seed:` with nothing after it is null in YAML, and "no value" is
        // what the default means - not a type error on a hand-edited file.
        let bare: Components =
            serde_yaml_ng::from_str("dhis2:\n  enabled: true\n  seed:\n").unwrap();
        assert_eq!(bare.dhis2.seed, Dhis2Seed::Default);

        // Round trip through YAML as one scalar each.
        for seed in [
            Dhis2Seed::Default,
            Dhis2Seed::None,
            Dhis2Seed::From(url.to_string()),
        ] {
            let text = serde_yaml_ng::to_string(&seed).unwrap();
            assert_eq!(text.trim(), seed.as_str(), "{text}");
            assert_eq!(serde_yaml_ng::from_str::<Dhis2Seed>(&text).unwrap(), seed);
        }
    }

    /// The published dump is looked up, never built from the tag: a URL
    /// assembled from a version would 404 on the first start.
    #[test]
    fn the_seed_dump_comes_from_the_table_and_an_unlisted_minor_has_none() {
        assert_eq!(dhis2_minor("2.42"), "2.42");
        assert_eq!(dhis2_minor("2.42.1"), "2.42");
        assert_eq!(dhis2_minor("2.42.1.1"), "2.42");
        assert_eq!(dhis2_minor(" 2.41.7 "), "2.41");
        assert_eq!(dhis2_minor("latest"), "latest", "a tag with no dots at all");

        assert_eq!(
            dhis2_seed_dump(DHIS2_DEFAULT_TAG),
            Some(crate::compose::render::DHIS2_DEFAULT_SEED_URL),
            "the minor chaps pins by default has a dump"
        );
        assert_eq!(dhis2_seed_dump("2.42.3"), dhis2_seed_dump("2.42"));
        // The 2.41 dump is no longer published, so 2.41 starts empty.
        assert_eq!(dhis2_seed_dump("2.41"), None);
        // Nothing is guessed for a line the table does not list.
        assert_eq!(dhis2_seed_dump("2.40"), None);
        assert_eq!(dhis2_seed_dump("latest"), None);

        // No two entries claim one minor, or the lookup would have two answers.
        let mut lines: Vec<&str> = DHIS2_SEED_DUMPS.iter().map(|(line, _)| *line).collect();
        let count = lines.len();
        lines.sort_unstable();
        lines.dedup();
        assert_eq!(lines.len(), count, "one minor, one dump");
        for (line, url) in DHIS2_SEED_DUMPS {
            assert_eq!(dhis2_minor(line), *line, "{line} is a minor line");
            assert!(url.starts_with("https://"), "{url}");
        }
    }

    /// What the first `chaps up` will restore, which is the seed setting and the
    /// pinned minor together.
    #[test]
    fn the_resolved_seed_follows_the_setting_and_then_the_pin() {
        let mut components = Components::default();
        components.dhis2.enabled = true;
        assert_eq!(
            components.dhis2_seed_source(),
            dhis2_seed_dump(DHIS2_DEFAULT_TAG)
        );

        assert!(!components.dhis2_seed_is_unknown());

        // A pin the table does not know leaves the database empty, which is the
        // same answer `none` gives - and `dhis2_unknown_seed` is what tells the
        // two apart on the screen, which is what this flag is for.
        components.dhis2.image_tag = "2.40".to_string();
        assert_eq!(components.dhis2_seed_source(), None);
        assert!(components.dhis2_seed_is_unknown());

        components.dhis2.seed = Dhis2Seed::From("dumps/mine.sql.gz".to_string());
        assert_eq!(components.dhis2_seed_source(), Some("dumps/mine.sql.gz"));
        components.dhis2.image_tag = DHIS2_DEFAULT_TAG.to_string();
        assert_eq!(
            components.dhis2_seed_source(),
            Some("dumps/mine.sql.gz"),
            "an explicit dump outranks the table"
        );

        components.dhis2.seed = Dhis2Seed::None;
        assert_eq!(components.dhis2_seed_source(), None);
        assert!(
            !components.dhis2_seed_is_unknown(),
            "an empty database that was asked for is not one chaps has no dump for"
        );
    }

    #[test]
    fn a_partial_block_keeps_the_defaults_of_the_fields_it_omits() {
        let components: Components = serde_yaml_ng::from_str("ocs:\n  enabled: true\n").unwrap();
        assert!(components.ocs.enabled);
        assert_eq!(components.ocs.port, Some(OCS_DEFAULT_PORT));
        assert_eq!(components.ocs.image_tag, OCS_DEFAULT_TAG);
        assert_eq!(components.ocs.base_url, None);
        assert!(!components.ocs.read_only);
        assert!(components.chap_core.enabled, "still on when unmentioned");
    }

    #[test]
    fn chap_core_can_be_turned_off_explicitly() {
        let components: Components =
            serde_yaml_ng::from_str("chap-core:\n  enabled: false\nocs:\n  enabled: true\n")
                .unwrap();
        assert!(!components.chap_core.enabled);
        assert_eq!(components.label(), "ocs");
        assert_eq!(components.enabled(), vec![Component::Ocs]);
    }

    #[test]
    fn names_parse_in_the_spellings_people_type() {
        assert_eq!(Component::from_name("ocs").unwrap(), Component::Ocs);
        assert_eq!(Component::from_name(" OCS ").unwrap(), Component::Ocs);
        assert_eq!(
            Component::from_name("chap_core").unwrap(),
            Component::ChapCore
        );
        assert_eq!(
            Component::from_name("chap-core").unwrap(),
            Component::ChapCore
        );
        assert_eq!(Component::from_name("s3").unwrap(), Component::S3);
        assert_eq!(Component::from_name("dhis2").unwrap(), Component::Dhis2);
        assert_eq!(Component::from_name(" DHIS2 ").unwrap(), Component::Dhis2);
        let err = Component::from_name("nope").expect_err("not a component");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownComponent(name)) if name == "nope"
        ));
    }

    #[test]
    fn a_disabled_component_publishes_nothing() {
        let components = Components::default();
        assert_eq!(components.port_of(Component::Ocs), None);
        assert_eq!(components.port_of(Component::S3), None);
    }

    /// The port field keeps its value while the component is off - that is how
    /// a re-enable puts the instance back where it was - so every reader of it
    /// has to filter on `enabled`. An address for a component nothing is going
    /// to start is a lie a caller would print.
    #[test]
    fn a_disabled_ocs_has_no_address_although_its_port_is_remembered() {
        let mut components = Components::default();
        components.ocs.port = Some(9010);
        assert!(!components.ocs.enabled);
        assert_eq!(components.ocs_url(), None);
        assert_eq!(components.ocs_reach(), "internal");

        components.ocs.enabled = true;
        assert_eq!(
            components.ocs_url().as_deref(),
            Some("http://localhost:9010")
        );
    }

    /// The compose file a component contributes is also the file a `disable` or
    /// a re-init has to delete, so the two answers come from one place.
    #[test]
    fn the_component_compose_files_are_the_enabled_ones_own_files() {
        assert_eq!(Component::ChapCore.compose_file(), None);
        assert_eq!(Component::Ocs.compose_file(), Some(OCS_COMPOSE));
        assert_eq!(Component::S3.compose_file(), Some(S3_COMPOSE));

        let mut components = Components::default();
        assert!(components.compose_files().is_empty());
        components.set_enabled(Component::S3, true);
        assert_eq!(components.compose_files(), vec![S3_COMPOSE]);
        components.set_enabled(Component::Ocs, true);
        assert_eq!(
            components.compose_files(),
            vec![OCS_COMPOSE, S3_COMPOSE],
            "in the order they belong in the -f list"
        );
    }

    /// The volumes a component keeps are what a `disable` names and `--purge`
    /// removes, so they have to be the component's own: chap-core's are
    /// upstream's, declared in a file this CLI does not write, and an empty
    /// slice is how that is said.
    #[test]
    fn a_component_names_the_volumes_it_keeps_and_chap_cores_are_upstreams() {
        assert_eq!(Component::Ocs.volumes(), ["ocs_data"].as_slice());
        assert_eq!(Component::S3.volumes(), ["s3_data"].as_slice());
        assert!(Component::ChapCore.volumes().is_empty());
        // Every volume `compose.dhis2.yml` declares, the download cache
        // included: this is the list the doctor decides leftovers by, so one left
        // out would be reported as a leftover on a deployment that has it and
        // `--purge` would leave it behind.
        assert_eq!(
            Component::Dhis2.volumes(),
            ["dhis2_home", "dhis2_db", "dhis2_dump"].as_slice()
        );

        // No two components may claim one volume: the doctor decides whether a
        // volume is a leftover by asking every component whether it is one of
        // theirs, and two answers would be two verdicts.
        let mut all: Vec<&str> = Component::ALL
            .iter()
            .flat_map(|c| c.volumes().iter().copied())
            .collect();
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), count, "one volume, one component");
    }

    /// Which compose services a component owns, which is what a `disable` stops
    /// before the definitions go away.
    #[test]
    fn a_component_owns_its_own_service_and_the_siblings_beside_it() {
        assert!(Component::Ocs.owns_service("ocs"));
        assert!(Component::Ocs.owns_service("ocs-init"));
        assert!(Component::S3.owns_service("s3") && Component::S3.owns_service("s3-init"));
        assert!(!Component::Ocs.owns_service("s3") && !Component::S3.owns_service("ocs"));

        // The hyphen is the whole of the sibling rule: a name that merely starts
        // with a component's is some other service, and a name the component's
        // own is a prefix of is not the component's either.
        assert!(!Component::S3.owns_service("s3x"));
        assert!(!Component::Ocs.owns_service("ocsx"));
        assert!(!Component::Ocs.owns_service("oc"));

        // chap-core's services share no prefix with the component - `chap-core`
        // is not a service at all - so they can only be the ones no other
        // component claims. Which is also what leaves a model service where it
        // has always been, chap-core's, while `disable` refuses to take
        // chap-core away with one enabled.
        assert!(Component::ChapCore.owns_service("chap"));
        assert!(Component::ChapCore.owns_service("chap-worker"));
        assert!(Component::ChapCore.owns_service("postgres"));
        assert!(Component::ChapCore.owns_service("chapkit-ewars-model"));
        assert!(Component::ChapCore.owns_service("chapkit-ewars-model-init"));
        assert!(!Component::ChapCore.owns_service("ocs"));
        assert!(!Component::ChapCore.owns_service("ocs-init"));
        assert!(!Component::ChapCore.owns_service("s3-init"));

        // And the invariant the whole rule exists for: exactly one component
        // owns any given service, whatever the component is called and however
        // many services it brings. A fourth component's `<name>-db` and
        // `<name>-dump` are its own on the same terms `ocs-init` is, which a
        // hand-kept list of the names that are not chap-core's would not give
        // them - chap-core would claim them and a `disable chap-core` would
        // stop them.
        for component in Component::ALL {
            for suffix in ["", "-init", "-db", "-dump"] {
                let service = format!("{}{suffix}", component.name());
                let owners: Vec<&str> = Component::ALL
                    .iter()
                    .filter(|c| c.owns_service(&service))
                    .map(|c| c.name())
                    .collect();
                assert_eq!(owners, [component.name()], "who owns {service}");
            }
        }
    }

    /// Every note `ocs` and `s3` print names the command that acts on it,
    /// which is the project's message rule, and none of them overstates the S3
    /// contract OCS does not have yet.
    #[test]
    fn the_component_notes_name_the_command_that_answers_them() {
        assert!(S3_WITHOUT_OCS_NOTE.contains("`chaps components enable ocs`"));
        assert!(S3_LEAVES_OCS_NOTE.contains("`chaps components enable s3`"));
        assert!(
            S3_LEAVES_OCS_NOTE.contains("does not read them yet"),
            "the S3_* block is forward-looking, and the note says so"
        );
        assert!(OCS_DATA_SOURCE_NOTE.contains("`chaps auth show`"));
        // The two ERA5-Land accounts are not alternatives, and the note must not
        // read as though picking one were enough.
        assert!(
            OCS_DATA_SOURCE_NOTE
                .contains("ERA5-Land needs one or both of ECMWF_DATASTORES_* and EDH_API_KEY"),
            "{OCS_DATA_SOURCE_NOTE}"
        );
        assert!(
            !OCS_DATA_SOURCE_NOTE.contains("needs one of"),
            "{OCS_DATA_SOURCE_NOTE}"
        );
        for note in [
            S3_SOON_NOTE,
            S3_WITHOUT_OCS_NOTE,
            S3_LEAVES_OCS_NOTE,
            OCS_DATA_SOURCE_NOTE,
        ] {
            assert!(!note.contains('\n'), "one line each: {note}");
        }
    }

    /// Each DHIS2 note says one thing an operator cannot see for themselves, and
    /// names the command or the file that answers it.
    #[test]
    fn the_dhis2_notes_say_what_the_first_start_does_and_where_to_change_it() {
        let seeded = dhis2_seed_note(Some("https://databases.dhis2.org/x.sql.gz"));
        assert!(
            seeded.contains("https://databases.dhis2.org/x.sql.gz"),
            "{seeded}"
        );
        // The restore is the postgres entrypoint's, so it happens once - which is
        // the half that decides whether changing the seed later does anything.
        assert!(seeded.contains("once"), "{seeded}");
        assert!(
            seeded.contains("`chaps components disable dhis2 --purge`"),
            "{seeded}"
        );

        let empty = dhis2_seed_note(None);
        assert!(empty.contains("starts empty"), "{empty}");
        assert!(empty.contains("`.chaps/components.yaml`"), "{empty}");
        assert!(empty.contains("`chaps sync`"), "{empty}");

        let unknown = dhis2_unknown_seed("2.40.1");
        assert!(
            unknown.contains("2.40"),
            "the minor line, not the tag: {unknown}"
        );
        assert!(!unknown.contains("2.40.1"), "{unknown}");
        assert!(unknown.contains("`.chaps/components.yaml`"), "{unknown}");

        assert!(
            DHIS2_FIRST_START_NOTE.contains("minutes"),
            "{DHIS2_FIRST_START_NOTE}"
        );
        assert!(
            DHIS2_FIRST_START_NOTE.contains("`chaps logs dhis2`"),
            "{DHIS2_FIRST_START_NOTE}"
        );

        // A DHIS2 beside a CHAP cannot talk to it until the route exists, so
        // the note that says so names the command that makes it.
        assert!(
            DHIS2_CONNECT_NOTE.contains("`chaps dhis2 connect`"),
            "{DHIS2_CONNECT_NOTE}"
        );
        assert!(DHIS2_CONNECT_NOTE.contains("route"), "{DHIS2_CONNECT_NOTE}");
        // CHAP is not a DHIS2 product, and nothing here is a "stack".
        assert!(
            !DHIS2_CONNECT_NOTE.contains("stack"),
            "{DHIS2_CONNECT_NOTE}"
        );

        // A downgrade is the one case that has to be stopped before `chaps up`,
        // so the line leads with the command that makes it recoverable.
        let moved = dhis2_tag_change_note("2.42", "2.41");
        assert!(moved.contains("from 2.42 to 2.41"), "{moved}");
        assert!(moved.contains("`chaps backup`"), "{moved}");
        assert!(moved.contains("404"), "{moved}");

        for note in [
            seeded,
            empty,
            unknown,
            DHIS2_FIRST_START_NOTE.to_string(),
            moved,
        ] {
            assert!(!note.contains('\n'), "one line each: {note}");
        }
    }

    /// A `components.yaml` written before the field loads as "no connect
    /// recorded", which is what such a deployment was, and the rest of the
    /// block is untouched.
    #[test]
    fn an_older_components_file_loads_with_no_connect_recorded() {
        let before = "chap-core:\n  enabled: true\ndhis2:\n  enabled: true\n  port: 8080\n  \
                      image_tag: '2.42'\n  seed: default\n";
        let components: Components = serde_yaml_ng::from_str(before).unwrap();
        assert!(components.dhis2.enabled);
        assert_eq!(components.dhis2.port, Some(8080));
        assert_eq!(components.dhis2.image_tag, "2.42");
        assert_eq!(components.dhis2.seed, Dhis2Seed::Default);
        assert_eq!(components.dhis2.connected_at, None);
        assert!(components.dhis2_needs_connecting());
    }

    /// The record round-trips as one string, and a deployment that has it is
    /// not asked to connect again.
    #[test]
    fn a_recorded_connect_round_trips_and_stops_the_hint() {
        let mut components = Components::default();
        components.set_enabled(Component::Dhis2, true);
        assert!(components.dhis2_needs_connecting());

        components.dhis2.connected_at = Some("2026-09-27T09:12:33Z".to_string());
        assert!(!components.dhis2_needs_connecting());

        let text = serde_yaml_ng::to_string(&components).unwrap();
        assert!(
            text.contains("connected_at: 2026-09-27T09:12:33Z"),
            "{text}"
        );
        assert_eq!(
            serde_yaml_ng::from_str::<Components>(&text).unwrap(),
            components
        );
    }

    /// Switching the component off forgets the record, wherever the switch was
    /// thrown: the CLI, the browser's components page and `init --force` all go
    /// through this one method.
    #[test]
    fn disabling_dhis2_forgets_the_recorded_connect() {
        let mut components = Components::default();
        components.set_enabled(Component::Dhis2, true);
        components.dhis2.connected_at = Some("2026-09-27T09:12:33Z".to_string());

        components.set_enabled(Component::Dhis2, false);
        assert_eq!(components.dhis2.connected_at, None);
        // The settings themselves are not touched: a re-enable keeps the port
        // and the tag it had.
        assert_eq!(components.dhis2.port, Some(DHIS2_DEFAULT_PORT));
        assert_eq!(components.dhis2.image_tag, DHIS2_DEFAULT_TAG);

        // And re-enabling asks for a connect again, because the database the
        // record was true of may be gone and a restored seed dump ships a
        // `chap` route of its own.
        components.set_enabled(Component::Dhis2, true);
        assert!(components.dhis2_needs_connecting());
    }

    /// A deployment with no chap-core is never asked to connect: the route
    /// would point at a service that is not there, and `chaps dhis2 connect`
    /// refuses on exactly those grounds.
    #[test]
    fn a_deployment_without_chap_core_is_not_asked_to_connect() {
        let mut components = Components::default();
        components.set_enabled(Component::Dhis2, true);
        components.set_enabled(Component::ChapCore, false);
        assert!(!components.dhis2_needs_connecting());
    }

    /// The hint says what chaps knows rather than what DHIS2 is, names the
    /// command, and says when to run it on the one caller that has not seen
    /// DHIS2 answer.
    #[test]
    fn the_connect_hint_claims_only_what_it_read() {
        let answering = dhis2_connect_hint(true);
        let waiting = dhis2_connect_hint(false);
        for line in [&answering, &waiting] {
            assert!(line.contains("`chaps dhis2 connect`"), "{line}");
            assert!(line.starts_with("chaps has not connected"), "{line}");
            assert!(!line.contains("stack"), "{line}");
            assert!(!line.contains('\n'), "one line each: {line}");
        }
        assert!(!answering.contains("once DHIS2 answers"), "{answering}");
        assert!(waiting.contains("once DHIS2 answers"), "{waiting}");

        for note in [DHIS2_CONNECT_FORGOTTEN, DHIS2_CONNECT_FORGOTTEN_WITH_VOLUME] {
            assert!(note.contains("`chaps dhis2 connect`"), "{note}");
            assert!(!note.contains('\n'), "one line each: {note}");
        }
        assert!(
            DHIS2_CONNECT_FORGOTTEN_WITH_VOLUME.contains("`dhis2_db`"),
            "{DHIS2_CONNECT_FORGOTTEN_WITH_VOLUME}"
        );
    }
}
