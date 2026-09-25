//! Serde types for the CHAP model marketplace schema (schema_version 2).
//!
//! These mirror the YAML in <https://github.com/dhis2-chap/model-marketplace>
//! verbatim. Unknown fields are tolerated on purpose: the marketplace may add
//! fields before the CLI learns about them, and an old CLI must keep working.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `registry.yaml` — the marketplace index.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegistryIndex {
    pub schema_version: u32,
    pub marketplace: Marketplace,
    pub review_policy: ReviewPolicy,
    /// Relative paths of the model files, e.g. `models/chapkit_ewars_model.yaml`.
    pub models: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Marketplace {
    pub name: String,
    pub description: String,
    pub repository: String,
    #[serde(default)]
    pub documentation: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReviewPolicy {
    pub required_approvals: u32,
    #[serde(default)]
    pub note: Option<String>,
}

/// `models/<id>.yaml` — one marketplace entry.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Model {
    pub schema_version: u32,
    /// Marketplace identifier, e.g. `chapkit_ewars_model`.
    pub id: String,
    /// Compose service name and DNS name, e.g. `chapkit-ewars-model`.
    pub service_id: String,
    pub display_name: String,
    pub kind: Kind,
    pub assessed_status: AssessedStatus,
    pub summary: String,
    pub source: Source,
    pub attribution: Attribution,
    #[serde(default)]
    pub maintainers: Vec<String>,
    pub compatibility: Compatibility,
    pub covariates: Covariates,
    pub channels: Channels,
    pub versions: Vec<Version>,
    #[serde(default)]
    pub configurations: BTreeMap<String, Configuration>,
    /// Whether this entry is one the deployment defines itself
    /// ([`crate::project::ManualModel`]) rather than one the marketplace
    /// lists.
    ///
    /// Never read from a model file: a catalogue that started publishing a
    /// `manual:` key must not be able to make one of its own entries claim it
    /// is local. It is written to `--json`, which is where a caller reads it.
    #[serde(default, skip_deserializing)]
    pub manual: bool,
}

/// Whether an entry is deployable or only scaffolding to copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Model,
    Template,
}

/// The author's own chapkit assessment, not a marketplace verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AssessedStatus {
    Green,
    Yellow,
    Orange,
    Red,
    Gray,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Source {
    pub repository: String,
    /// Tagless image reference; the tag comes from the selected [`Version`].
    pub image: String,
    /// chapkit base image the service is built on, e.g. `ghcr.io/dhis2-chap/chapkit-r-inla`.
    pub runtime_image: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Attribution {
    pub author: String,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub contact: Option<String>,
    #[serde(default)]
    pub citation: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Compatibility {
    pub period_types: Vec<String>,
    pub min_prediction_periods: u32,
    pub max_prediction_periods: u32,
    #[serde(default)]
    pub requires_geo: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Covariates {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub defaults: Vec<String>,
    #[serde(default)]
    pub allow_free_additional: bool,
}

/// Channel pointers into [`Model::versions`].
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Channels {
    pub stable: String,
    pub latest: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Version {
    pub version: String,
    pub commit: String,
    /// Image tag for this pin, e.g. `sha-fa880a1`.
    pub image_tag: String,
    /// chapkit version requirement, e.g. `>=2.0.0,<3`.
    pub chapkit: String,
    pub status: VersionStatus,
    #[serde(default)]
    pub verified_by: Vec<String>,
    #[serde(default)]
    pub changelog: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VersionStatus {
    Verified,
    Unstable,
    Deprecated,
    Yanked,
}

/// A verified configuration; `config` is the flat object chapkit accepts as
/// `data` on `POST /api/v1/configs`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Configuration {
    #[serde(default)]
    pub description: Option<String>,
    pub config: serde_json::Value,
}

/// Release channel a model can be pinned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Latest,
}

impl Channel {
    /// Lowercase name, as used on the command line and in `.chaps/models.yaml`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Latest => "latest",
        }
    }
}

/// How the caller asked for a version: follow a channel, or pin exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionSelector {
    Channel(Channel),
    Exact(String),
}

impl Default for VersionSelector {
    fn default() -> Self {
        VersionSelector::Channel(Channel::Stable)
    }
}

/// The chapkit R-INLA runtime base image; amd64 only.
pub const R_INLA_RUNTIME: &str = "ghcr.io/dhis2-chap/chapkit-r-inla";

impl Model {
    /// Scaffolding rather than a deployable forecasting model.
    pub fn is_template(&self) -> bool {
        self.kind == Kind::Template
    }

    /// Look up a version by its `version` string.
    pub fn version(&self, v: &str) -> Option<&Version> {
        self.versions
            .iter()
            .find(|candidate| candidate.version == v)
    }

    /// Resolve a selector to a concrete version.
    ///
    /// Errors with [`crate::error::ChapError::UnknownVersion`] when the version
    /// (or the version a channel points at) is absent, and with
    /// [`crate::error::ChapError::YankedVersion`] when it is yanked.
    pub fn resolve(&self, sel: &VersionSelector) -> crate::error::Result<&Version> {
        let wanted = match sel {
            VersionSelector::Channel(Channel::Stable) => self.channels.stable.clone(),
            VersionSelector::Channel(Channel::Latest) => self.channels.latest.clone(),
            VersionSelector::Exact(v) => v.clone(),
        };
        let found =
            self.version(&wanted)
                .ok_or_else(|| crate::error::ChapError::UnknownVersion {
                    id: self.id.clone(),
                    version: wanted.clone(),
                })?;
        if found.status == VersionStatus::Yanked {
            return Err(crate::error::ChapError::YankedVersion {
                id: self.id.clone(),
                version: wanted,
            }
            .into());
        }
        Ok(found)
    }

    /// Fully qualified image reference for a version.
    pub fn image_ref(&self, v: &Version) -> String {
        crate::compose::image_ref(&self.source.image, &v.image_tag)
    }

    /// Whether the service needs `platform: linux/amd64`, i.e. whether it is
    /// built on the R-INLA runtime.
    pub fn needs_amd64(&self) -> bool {
        runtime_base(&self.source.runtime_image) == R_INLA_RUNTIME
    }
}

/// Strip a `:tag` suffix from an image reference, leaving the repository.
///
/// A colon inside the registry host's port (`host:5000/img`) is not a tag, so
/// only a colon after the last `/` counts.
fn runtime_base(image: &str) -> &str {
    let last_slash = image.rfind('/').map(|i| i + 1).unwrap_or(0);
    match image[last_slash..].find(':') {
        Some(i) => &image[..last_slash + i],
        None => image,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::embedded;

    fn parse_all() -> Vec<Model> {
        embedded::files()
            .iter()
            .filter(|(name, _)| *name != "registry.yaml")
            .map(|(name, body)| {
                serde_yaml_ng::from_str::<Model>(body)
                    .unwrap_or_else(|e| panic!("{name} failed to parse: {e}"))
            })
            .collect()
    }

    fn model(id: &str) -> Model {
        parse_all()
            .into_iter()
            .find(|m| m.id == id)
            .unwrap_or_else(|| panic!("no vendored model {id}"))
    }

    #[test]
    fn vendored_index_parses() {
        let (name, body) = embedded::files()[0];
        assert_eq!(name, "registry.yaml");
        let index: RegistryIndex = serde_yaml_ng::from_str(body).expect("registry.yaml parses");
        assert_eq!(index.schema_version, 2);
        assert_eq!(index.models.len(), 7);
        assert!(index.models.iter().all(|p| p.starts_with("models/")));
        assert!(!index.marketplace.documentation.is_empty());
        assert_eq!(index.review_policy.required_approvals, 3);
    }

    #[test]
    fn every_vendored_model_parses() {
        let models = parse_all();
        assert_eq!(models.len(), 7);
        for m in &models {
            assert_eq!(m.schema_version, 2);
            assert!(!m.versions.is_empty(), "{} has no versions", m.id);
            assert!(!m.configurations.is_empty(), "{} has no configs", m.id);
            // Every channel pointer must resolve.
            assert!(m.version(&m.channels.stable).is_some(), "{}", m.id);
            assert!(m.version(&m.channels.latest).is_some(), "{}", m.id);
        }
        assert_eq!(models.iter().filter(|m| m.is_template()).count(), 2);
    }

    #[test]
    fn needs_amd64_tracks_the_r_inla_runtime() {
        for id in [
            "chapkit_ewars_model",
            "chapkit_rwanda_malaria_bym_model",
            "chapkit_ghr_model",
            "chapkit_minimalist_example_r",
        ] {
            assert!(model(id).needs_amd64(), "{id} should need amd64");
        }
        for id in [
            "auto_arima_chapkit",
            "chapkit_simple_multistep_model",
            "chapkit_minimalist_example_py",
        ] {
            assert!(!model(id).needs_amd64(), "{id} should not need amd64");
        }
    }

    #[test]
    fn runtime_base_strips_only_a_real_tag() {
        assert_eq!(runtime_base(R_INLA_RUNTIME), R_INLA_RUNTIME);
        assert_eq!(
            runtime_base("ghcr.io/dhis2-chap/chapkit-r-inla:2.0.0"),
            R_INLA_RUNTIME
        );
        assert_eq!(runtime_base("localhost:5000/img"), "localhost:5000/img");
    }

    #[test]
    fn resolve_stable_yields_a_verified_version() {
        let m = model("chapkit_ewars_model");
        let v = m
            .resolve(&VersionSelector::Channel(Channel::Stable))
            .unwrap();
        assert_eq!(v.version, m.channels.stable);
        assert_eq!(v.status, VersionStatus::Verified);
        assert_eq!(
            m.image_ref(v),
            format!("ghcr.io/chap-models/chapkit_ewars_model:{}", v.image_tag)
        );
    }

    #[test]
    fn resolve_exact_and_unknown() {
        let m = model("auto_arima_chapkit");
        // The pin the snapshot carries, asked for by its exact number rather
        // than through a channel.
        let pinned = m.channels.stable.clone();
        let v = m
            .resolve(&VersionSelector::Exact(pinned.clone()))
            .expect("the pinned version exists");
        assert_eq!(v.version, pinned);
        assert_eq!(v.image_tag, m.version(&pinned).unwrap().image_tag);

        let err = m
            .resolve(&VersionSelector::Exact("9.9.9".into()))
            .expect_err("unknown version errors");
        match err.downcast_ref::<crate::error::ChapError>() {
            Some(crate::error::ChapError::UnknownVersion { id, version }) => {
                assert_eq!(id, "auto_arima_chapkit");
                assert_eq!(version, "9.9.9");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn resolve_rejects_a_yanked_version() {
        let mut m = model("auto_arima_chapkit");
        m.versions[0].status = VersionStatus::Yanked;
        let err = m
            .resolve(&VersionSelector::Channel(Channel::Stable))
            .expect_err("yanked version errors");
        assert!(matches!(
            err.downcast_ref::<crate::error::ChapError>(),
            Some(crate::error::ChapError::YankedVersion { .. })
        ));
    }

    #[test]
    fn configurations_keep_their_json_shape() {
        let m = model("chapkit_ewars_model");
        let cfg = &m.configurations["monthly_climate"].config;
        assert_eq!(cfg["prediction_periods"], serde_json::json!(3));
        assert_eq!(cfg["n_lags"], serde_json::json!([3, 3]));
        assert_eq!(cfg["region_seasonal"], serde_json::json!(false));
    }
}
