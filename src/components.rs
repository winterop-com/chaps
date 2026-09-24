//! `.chaps/components.yaml` — which pieces a deployment is made of.
//!
//! A deployment is chap-core plus whatever else was asked for. chap-core is a
//! component like the others, on by default; OCS (Open Climate Service) and an
//! S3-compatible object store are opt-in. The file is intent, like
//! `project.yaml` and `models.yaml`: `chaps sync` renders one compose file per
//! enabled component from it.
//!
//! Every field carries a `serde` default, so a project written before this
//! file existed loads as "chap-core on, nothing else" — which is exactly what
//! it was. Nothing has to be migrated.

use crate::error::{ChapError, Result};
use serde::{Deserialize, Serialize};

/// The enabled component set, inside [`crate::project::CHAPS_DIR`].
pub const COMPONENTS_FILE: &str = "components.yaml";

/// Compose file rendered for the `ocs` component.
pub const OCS_COMPOSE: &str = "compose.ocs.yml";
/// Compose file rendered for the `s3` component.
pub const S3_COMPOSE: &str = "compose.s3.yml";

/// Directory, relative to the project, holding the OCS instance config.
pub const OCS_DIR: &str = "ocs";
/// The OCS instance config, inside [`OCS_DIR`]. Mounted read-only at
/// `/app/climate-service.yaml`.
pub const OCS_CONFIG_FILE: &str = "climate-service.yaml";

/// Image OCS is deployed from. No release tags are published yet, so the
/// default pin is the moving `main` tag.
pub const OCS_IMAGE: &str = "ghcr.io/dhis2/open-climate-service";
/// Tag `ocs` follows unless `.env` pins another.
pub const OCS_DEFAULT_TAG: &str = "main";
/// Container port OCS listens on, and the port chap-core reaches it at.
pub const OCS_CONTAINER_PORT: u16 = 9000;
/// Host port `ocs` publishes unless `--port` says otherwise.
pub const OCS_DEFAULT_PORT: u16 = 9000;
/// The `.env` variable that moves the OCS image pin.
pub const OCS_TAG_ENV_VAR: &str = "OCS_IMAGE_TAG";

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
}

impl Component {
    /// Every component, in the order they are listed and rendered.
    pub const ALL: &'static [Component] = &[Component::ChapCore, Component::Ocs, Component::S3];

    /// The name this component is spelled with everywhere.
    pub fn name(self) -> &'static str {
        match self {
            Component::ChapCore => "chap-core",
            Component::Ocs => "ocs",
            Component::S3 => "s3",
        }
    }

    /// One line saying what it is, for `chaps components list`.
    pub fn summary(self) -> &'static str {
        match self {
            Component::ChapCore => "CHAP itself: chap-core, its worker, Valkey and PostgreSQL",
            Component::Ocs => "Open Climate Service: climate data, reachable at http://ocs:9000",
            Component::S3 => "RustFS, an S3-compatible object store OCS will keep objects in",
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

    /// Whether this component takes a host port at all.
    pub fn takes_port(self) -> bool {
        matches!(self, Component::Ocs | Component::S3)
    }

    /// The named volume this component keeps its data in, as the compose file
    /// `chaps sync` renders declares it.
    ///
    /// `None` for chap-core: its volumes are upstream's own, declared in a
    /// compose file this CLI does not write, and `chaps down --volumes` is
    /// what removes them.
    pub fn volume(self) -> Option<&'static str> {
        match self {
            Component::Ocs => Some(crate::compose::render::OCS_VOLUME),
            Component::S3 => Some(crate::compose::render::S3_VOLUME),
            Component::ChapCore => None,
        }
    }
}

/// `true`, for the `serde` default of a flag that is on unless said otherwise.
fn enabled_by_default() -> bool {
    true
}

fn default_ocs_port() -> u16 {
    OCS_DEFAULT_PORT
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
    /// published by default rather than hidden on the compose network.
    #[serde(default = "default_ocs_port")]
    pub port: u16,
    /// The tag the compose file defaults to, and the one `.env` pins.
    #[serde(default = "default_ocs_tag")]
    pub image_tag: String,
}

impl Default for OcsComponent {
    fn default() -> OcsComponent {
        OcsComponent {
            enabled: false,
            port: OCS_DEFAULT_PORT,
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

/// The whole of `components.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Components {
    #[serde(rename = "chap-core", default)]
    pub chap_core: ChapCoreComponent,
    #[serde(default)]
    pub ocs: OcsComponent,
    #[serde(default)]
    pub s3: S3Component,
}

impl Components {
    /// Whether one component is on.
    pub fn is_enabled(&self, component: Component) -> bool {
        match component {
            Component::ChapCore => self.chap_core.enabled,
            Component::Ocs => self.ocs.enabled,
            Component::S3 => self.s3.enabled,
        }
    }

    /// Turn one on or off, leaving its settings alone.
    pub fn set_enabled(&mut self, component: Component, on: bool) {
        match component {
            Component::ChapCore => self.chap_core.enabled = on,
            Component::Ocs => self.ocs.enabled = on,
            Component::S3 => self.s3.enabled = on,
        }
    }

    /// The host port one component publishes, when it publishes one.
    pub fn port_of(&self, component: Component) -> Option<u16> {
        match component {
            Component::ChapCore => None,
            Component::Ocs => self.ocs.enabled.then_some(self.ocs.port),
            Component::S3 => self.s3.port.filter(|_| self.s3.enabled),
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
        let mut files = Vec::new();
        if self.ocs.enabled {
            files.push(OCS_COMPOSE.to_string());
        }
        if self.s3.enabled {
            files.push(S3_COMPOSE.to_string());
        }
        files
    }

    /// The URL a human reaches OCS at from this machine.
    pub fn ocs_url(&self) -> String {
        format!("http://localhost:{}", self.ocs.port)
    }
}

/// The note `components enable ocs` prints while there is no object store.
///
/// OCS does not read any S3 variable yet, so this is a heads-up rather than a
/// requirement: forcing a second container on a deployment for a contract that
/// does not exist would be worse than saying it is coming.
pub const S3_SOON_NOTE: &str = "OCS will soon need an S3-compatible object store; `chaps components enable s3` \
     adds one, and the OCS service then gets the S3_* variables it will read";

/// Why a model cannot be enabled while `chap-core` is off.
///
/// The mirror image of [`models_need_chap_core`]: the same dependency, seen
/// from the model's side. [`crate::compose::apply::validate`] raises it, so
/// `models enable`, the browser and every other caller of the shared apply
/// path all say the same thing.
pub const MODELS_NEED_CHAP_CORE: &str =
    "models need the chap-core component; run `chaps components enable chap-core`";

/// Why `chap-core` cannot be turned off while models are enabled.
pub fn models_need_chap_core(models: &[String]) -> String {
    format!(
        "chap-core cannot be disabled while {} enabled: model services register with \
         chap-core and are reached through it. Disable them first with `chaps models disable ID`",
        match models.len() {
            1 => format!("`{}` is", models[0]),
            _ => format!("{} models are ({})", models.len(), models.join(", ")),
        }
    )
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

    #[test]
    fn the_settings_round_trip_with_their_defaults() {
        let components = Components {
            chap_core: ChapCoreComponent { enabled: true },
            ocs: OcsComponent {
                enabled: true,
                port: 9010,
                image_tag: "main".into(),
            },
            s3: S3Component {
                enabled: true,
                port: None,
            },
        };
        let text = serde_yaml_ng::to_string(&components).unwrap();
        assert!(text.contains("chap-core:\n"), "{text}");
        assert!(text.contains("  port: 9010\n"), "{text}");
        assert!(text.contains("  port: null\n"), "{text}");
        assert_eq!(
            serde_yaml_ng::from_str::<Components>(&text).unwrap(),
            components
        );

        assert_eq!(components.label(), "chap-core, ocs, s3");
        assert_eq!(
            components.compose_files(),
            vec!["compose.ocs.yml", "compose.s3.yml"]
        );
        assert_eq!(components.port_of(Component::Ocs), Some(9010));
        assert_eq!(components.port_of(Component::S3), None);
        assert_eq!(components.ocs_url(), "http://localhost:9010");
    }

    #[test]
    fn a_partial_block_keeps_the_defaults_of_the_fields_it_omits() {
        let components: Components = serde_yaml_ng::from_str("ocs:\n  enabled: true\n").unwrap();
        assert!(components.ocs.enabled);
        assert_eq!(components.ocs.port, OCS_DEFAULT_PORT);
        assert_eq!(components.ocs.image_tag, OCS_DEFAULT_TAG);
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

    #[test]
    fn the_refusal_names_the_models_in_the_way() {
        let one = models_need_chap_core(&["chapkit_ewars_model".to_string()]);
        assert!(one.contains("`chapkit_ewars_model` is enabled"), "{one}");
        let two = models_need_chap_core(&["a".to_string(), "b".to_string()]);
        assert!(two.contains("2 models are (a, b)"), "{two}");
        assert!(two.contains("chaps models disable"));
    }
}
