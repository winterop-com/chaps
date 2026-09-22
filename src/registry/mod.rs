//! The marketplace catalogue: loading it, and querying what was loaded.

pub mod cache;
pub mod embedded;
pub mod fetch;
pub mod model;

// Re-exports for the modules A, B and C fill in; remove the allow once they land.
#[allow(unused_imports)]
pub use model::{
    AssessedStatus, Attribution, Channel, Channels, Compatibility, Configuration, Covariates, Kind,
    Marketplace, Model, R_INLA_RUNTIME, RegistryIndex, ReviewPolicy, Source, Version,
    VersionSelector, VersionStatus,
};

use crate::error::Result;
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

/// Raw URL of the upstream marketplace index.
pub const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/dhis2-chap/model-marketplace/main/registry.yaml";

/// How long a cached snapshot is considered fresh.
pub const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

/// Default network timeout for registry requests.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Where a loaded [`Registry`] came from.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Provenance {
    /// Fetched over the network just now.
    Network,
    /// Read from a cache entry younger than [`CACHE_TTL`].
    Cache { age_secs: u64 },
    /// Read from a cache entry older than [`CACHE_TTL`] because the network failed.
    StaleCache { age_secs: u64 },
    /// The snapshot compiled into the binary.
    Embedded,
}

impl Provenance {
    /// One-word label for human output.
    pub fn label(&self) -> &'static str {
        match self {
            Provenance::Network => "network",
            Provenance::Cache { .. } => "cache",
            Provenance::StaleCache { .. } => "stale cache",
            Provenance::Embedded => "embedded",
        }
    }
}

/// Everything [`load`] and [`update`] need to find the catalogue.
#[derive(Debug, Clone)]
pub struct RegistryOptions {
    pub url: String,
    pub offline: bool,
    pub cache_dir: PathBuf,
    pub timeout: Duration,
}

impl Default for RegistryOptions {
    fn default() -> Self {
        RegistryOptions {
            url: DEFAULT_REGISTRY_URL.to_string(),
            offline: false,
            cache_dir: crate::paths::cache_dir(),
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

/// A parsed marketplace catalogue.
#[derive(Debug, Clone, Serialize)]
pub struct Registry {
    pub url: String,
    pub provenance: Provenance,
    pub index: RegistryIndex,
    pub models: Vec<Model>,
}

impl Registry {
    /// Parse an index plus its model files.
    ///
    /// `model_files` are `(relative path as listed in the index, contents)`
    /// pairs; every path the index lists must be present.
    pub fn parse(
        url: &str,
        provenance: Provenance,
        index_yaml: &str,
        model_files: &[(String, String)],
    ) -> Result<Registry> {
        let index: RegistryIndex = serde_yaml_ng::from_str(index_yaml)
            .map_err(|e| anyhow::anyhow!("{url}: invalid registry index: {e}"))?;

        let mut models = Vec::with_capacity(index.models.len());
        for path in &index.models {
            let body = model_files
                .iter()
                .find(|(name, _)| name == path)
                .map(|(_, body)| body)
                .ok_or_else(|| anyhow::anyhow!("registry lists {path} but it is missing"))?;
            let model: Model = serde_yaml_ng::from_str(body)
                .map_err(|e| anyhow::anyhow!("{path}: invalid model file: {e}"))?;
            models.push(model);
        }

        Ok(Registry {
            url: url.to_string(),
            provenance,
            index,
            models,
        })
    }

    /// Look a model up by `id` or by `service_id`.
    pub fn get(&self, id: &str) -> Option<&Model> {
        self.models
            .iter()
            .find(|m| m.id == id)
            .or_else(|| self.models.iter().find(|m| m.service_id == id))
    }

    /// Entries that can actually be deployed, i.e. everything but templates.
    pub fn deployable(&self) -> impl Iterator<Item = &Model> {
        self.models.iter().filter(|m| m.kind == Kind::Model)
    }

    /// Case-insensitive substring search over id, display name and summary.
    pub fn search(&self, q: &str) -> Vec<&Model> {
        let needle = q.to_lowercase();
        self.models
            .iter()
            .filter(|m| {
                m.id.to_lowercase().contains(&needle)
                    || m.service_id.to_lowercase().contains(&needle)
                    || m.display_name.to_lowercase().contains(&needle)
                    || m.summary.to_lowercase().contains(&needle)
            })
            .collect()
    }
}

/// Load the catalogue: fresh cache > network > stale cache > embedded.
///
/// Owned by agent A. Until the fetch and cache layers land this always returns
/// the embedded snapshot, which keeps the rest of the CLI usable.
pub fn load(opts: &RegistryOptions) -> Result<Registry> {
    // TODO(agent A): honour `opts` — fresh cache, then network (unless
    // `opts.offline`), then stale cache, then embedded.
    let _ = opts;
    load_embedded()
}

/// Force a network refresh and rewrite the cache.
///
/// Owned by agent A.
pub fn update(_opts: &RegistryOptions) -> Result<Registry> {
    Err(anyhow::anyhow!("registry update is not implemented yet"))
}

/// Parse the snapshot compiled into the binary.
pub fn load_embedded() -> Result<Registry> {
    Registry::parse(
        DEFAULT_REGISTRY_URL,
        Provenance::Embedded,
        embedded::index_yaml(),
        &embedded::model_files(),
    )
}

/// Resolve a model file's URL relative to the index URL.
///
/// Strips the filename from `index_url` and appends `rel`.
pub fn model_url(index_url: &str, rel: &str) -> String {
    let base = match index_url.rfind('/') {
        Some(i) => &index_url[..i + 1],
        None => "",
    };
    format!("{base}{rel}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        load_embedded().expect("embedded snapshot parses")
    }

    #[test]
    fn embedded_snapshot_parses_into_a_registry() {
        let r = registry();
        assert_eq!(r.models.len(), 6);
        assert_eq!(r.url, DEFAULT_REGISTRY_URL);
        assert!(matches!(r.provenance, Provenance::Embedded));
        // Model order follows the index order.
        assert_eq!(r.models[0].id, "chapkit_ewars_model");
    }

    #[test]
    fn get_matches_id_and_service_id() {
        let r = registry();
        assert_eq!(
            r.get("chapkit_ewars_model").unwrap().id,
            "chapkit_ewars_model"
        );
        assert_eq!(
            r.get("chapkit-ewars-model").unwrap().id,
            "chapkit_ewars_model"
        );
        assert!(r.get("nope").is_none());
    }

    #[test]
    fn deployable_excludes_templates() {
        let r = registry();
        let ids: Vec<&str> = r.deployable().map(|m| m.id.as_str()).collect();
        assert_eq!(ids.len(), 4);
        assert!(!ids.iter().any(|id| id.contains("minimalist_example")));
    }

    #[test]
    fn search_is_case_insensitive_across_fields() {
        let r = registry();
        assert_eq!(r.search("EWARS").len(), 1);
        assert_eq!(
            r.search("chapkit_ewars_model")[0].display_name,
            "CHAP-EWARS"
        );
        assert!(
            r.search("arima")
                .iter()
                .any(|m| m.id == "auto_arima_chapkit")
        );
        assert!(r.search("zzzz").is_empty());
    }

    #[test]
    fn parse_rejects_a_missing_model_file() {
        let err = Registry::parse(
            DEFAULT_REGISTRY_URL,
            Provenance::Embedded,
            embedded::index_yaml(),
            &[],
        )
        .expect_err("missing model files are an error");
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn model_url_replaces_the_index_filename() {
        assert_eq!(
            model_url(DEFAULT_REGISTRY_URL, "models/chapkit_ewars_model.yaml"),
            "https://raw.githubusercontent.com/dhis2-chap/model-marketplace/main/models/chapkit_ewars_model.yaml"
        );
        assert_eq!(model_url("registry.yaml", "models/x.yaml"), "models/x.yaml");
    }
}
