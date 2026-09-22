//! The marketplace catalogue: loading it, and querying what was loaded.

pub mod cache;
pub mod embedded;
pub mod fetch;
pub mod model;

pub use model::{
    AssessedStatus, Channel, Kind, Model, RegistryIndex, Version, VersionSelector, VersionStatus,
};

use crate::error::{ChapError, Result};
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

    /// The label plus the age of the snapshot, where there is one.
    pub fn describe(&self) -> String {
        match self {
            Provenance::Cache { age_secs } | Provenance::StaleCache { age_secs } => format!(
                "{} ({} old)",
                self.label(),
                crate::output::human_age(Duration::from_secs(*age_secs))
            ),
            _ => self.label().to_string(),
        }
    }

    /// Age of a cache-backed snapshot.
    #[cfg(test)]
    pub fn age(&self) -> Option<Duration> {
        match self {
            Provenance::Cache { age_secs } | Provenance::StaleCache { age_secs } => {
                Some(Duration::from_secs(*age_secs))
            }
            _ => None,
        }
    }

    /// Provenance for a cache entry of the given age.
    fn for_age(age: Duration) -> Provenance {
        if age < CACHE_TTL {
            Provenance::Cache {
                age_secs: age.as_secs(),
            }
        } else {
            Provenance::StaleCache {
                age_secs: age.as_secs(),
            }
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
    #[cfg(test)]
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
/// A fresh cache short-circuits the network so the common case costs one
/// directory read. When the cache is cold or stale the network is tried,
/// unless `opts.offline`, and a successful fetch refreshes the cache. If the
/// fetch fails the ladder continues downwards — a stale cache, then the
/// snapshot compiled into the binary — so `chaps` still works on a plane.
///
/// Falling back after a failed fetch warns on stderr, because a silently
/// out-of-date catalogue is how a user ends up pinning a version the
/// marketplace has already yanked. `--offline` does not warn: the user asked.
pub fn load(opts: &RegistryOptions) -> Result<Registry> {
    let cached = cache::read(opts)?;

    if let Some((index, files, age)) = &cached
        && age < &CACHE_TTL
        && let Some(registry) = parse_cached(opts, *age, index, files)
    {
        return Ok(registry);
    }

    if opts.offline {
        if let Some((index, files, age)) = &cached
            && let Some(registry) = parse_cached(opts, *age, index, files)
        {
            return Ok(registry);
        }
        return load_embedded();
    }

    let fetched = match fetch::fetch_registry(opts) {
        Ok(fetched) => fetched,
        Err(err) => return fall_back(opts, cached, &err),
    };

    let (index_yaml, model_files) = fetched;
    // A registry we cannot cache is still a registry: warn, do not fail.
    if let Err(err) = cache::write(opts, &index_yaml, &model_files) {
        crate::output::warn(&format!("could not write the registry cache: {err:#}"));
    }
    Registry::parse(&opts.url, Provenance::Network, &index_yaml, &model_files)
}

/// Force a network refresh and rewrite the cache.
///
/// Unlike [`load`] this has no fallback: the point of `chaps registry update`
/// is to know whether the refresh worked, so every failure surfaces as
/// [`ChapError::RegistryUnavailable`].
pub fn update(opts: &RegistryOptions) -> Result<Registry> {
    if opts.offline {
        return Err(ChapError::RegistryUnavailable(
            "--offline was given, so the registry cannot be refreshed".to_string(),
        )
        .into());
    }

    let (index_yaml, model_files) = fetch::fetch_registry(opts)
        .map_err(|e| ChapError::RegistryUnavailable(format!("{e:#}")))?;
    cache::write(opts, &index_yaml, &model_files)?;
    Registry::parse(&opts.url, Provenance::Network, &index_yaml, &model_files)
}

/// Parse a cache entry, reporting a corrupt one as "no cache" so the caller
/// can continue down the ladder instead of failing outright.
fn parse_cached(
    opts: &RegistryOptions,
    age: Duration,
    index_yaml: &str,
    model_files: &[(String, String)],
) -> Option<Registry> {
    match Registry::parse(&opts.url, Provenance::for_age(age), index_yaml, model_files) {
        Ok(registry) => Some(registry),
        Err(err) => {
            crate::output::warn(&format!("ignoring the registry cache: {err:#}"));
            None
        }
    }
}

/// The network failed: use the stale cache if there is one, else the embedded
/// snapshot, and say which and why.
fn fall_back(
    opts: &RegistryOptions,
    cached: Option<cache::CachedSnapshot>,
    err: &anyhow::Error,
) -> Result<Registry> {
    if let Some((index, files, age)) = &cached
        && let Some(registry) = parse_cached(opts, *age, index, files)
    {
        crate::output::warn(&format!(
            "could not reach {} ({err}); using the cached snapshot from {} ago",
            opts.url,
            crate::output::human_age(*age)
        ));
        return Ok(registry);
    }
    crate::output::warn(&format!(
        "could not reach {} ({err}); using the snapshot built into this binary",
        opts.url
    ));
    load_embedded()
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

    /// Port 9 is the discard service and is not listening, so every test in
    /// this module exercises the "network failed" branch without a network.
    const UNREACHABLE: &str = "http://127.0.0.1:9/registry.yaml";

    fn opts(cache_dir: &std::path::Path, offline: bool) -> RegistryOptions {
        RegistryOptions {
            url: UNREACHABLE.to_string(),
            offline,
            cache_dir: cache_dir.to_path_buf(),
            timeout: Duration::from_secs(2),
        }
    }

    /// Seed the cache with a one-model catalogue, backdated by `age`.
    ///
    /// One model is what makes a cache hit unmistakable: the embedded
    /// snapshot has six, so a count of one cannot have come from the
    /// fallback.
    fn seed_cache(opts: &RegistryOptions, age: Duration) {
        const INDEX: &str = "\
schema_version: 2
marketplace:
  name: test marketplace
  description: seeded by the registry tests
  repository: https://example.test/marketplace
review_policy:
  required_approvals: 3
models:
  - models/chapkit_ewars_model.yaml
";
        let ewars = embedded::model_files()
            .into_iter()
            .find(|(name, _)| name == "models/chapkit_ewars_model.yaml")
            .expect("the snapshot ships ewars");
        cache::write(opts, INDEX, &[ewars]).unwrap();

        // meta.json is the only record of when the entry was written, so
        // rewriting it is how a test makes an entry old.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let meta = serde_json::json!({
            "url": opts.url,
            "fetched_at_unix": now - age.as_secs(),
        });
        std::fs::write(cache::dir_for(opts).join("meta.json"), meta.to_string()).unwrap();
    }

    #[test]
    fn offline_without_a_cache_uses_the_embedded_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let r = load(&opts(tmp.path(), true)).unwrap();
        assert!(
            matches!(r.provenance, Provenance::Embedded),
            "{:?}",
            r.provenance
        );
        assert_eq!(r.models.len(), 6);
    }

    #[test]
    fn an_unreachable_registry_without_a_cache_uses_the_embedded_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let r = load(&opts(tmp.path(), false)).unwrap();
        assert!(
            matches!(r.provenance, Provenance::Embedded),
            "{:?}",
            r.provenance
        );
        assert_eq!(r.models.len(), 6);
    }

    #[test]
    fn an_unreachable_registry_falls_back_to_a_stale_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = opts(tmp.path(), false);
        seed_cache(&opts, CACHE_TTL + Duration::from_secs(3600));

        let r = load(&opts).unwrap();
        match r.provenance {
            Provenance::StaleCache { age_secs } => {
                assert!(age_secs >= CACHE_TTL.as_secs(), "{age_secs}")
            }
            other => panic!("expected a stale cache, got {other:?}"),
        }
        assert_eq!(r.models.len(), 1, "the seeded cache, not the snapshot");
        assert_eq!(r.url, UNREACHABLE);
    }

    #[test]
    fn a_fresh_cache_is_used_without_touching_the_network() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = opts(tmp.path(), false);
        seed_cache(&opts, Duration::from_secs(60));

        // The URL is unreachable, so a cache miss would yield six embedded
        // models; one model proves the cache answered first.
        let r = load(&opts).unwrap();
        match r.provenance {
            Provenance::Cache { age_secs } => assert!((60..600).contains(&age_secs), "{age_secs}"),
            other => panic!("expected a fresh cache, got {other:?}"),
        }
        assert_eq!(r.models.len(), 1);
        assert_eq!(r.models[0].id, "chapkit_ewars_model");
    }

    #[test]
    fn offline_accepts_a_stale_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = opts(tmp.path(), true);
        seed_cache(&opts, CACHE_TTL + Duration::from_secs(60));

        let r = load(&opts).unwrap();
        assert!(
            matches!(r.provenance, Provenance::StaleCache { .. }),
            "{:?}",
            r.provenance
        );
        assert_eq!(r.models.len(), 1);
    }

    #[test]
    fn a_corrupt_cache_does_not_stop_the_ladder() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = opts(tmp.path(), true);
        seed_cache(&opts, Duration::from_secs(60));
        std::fs::write(
            cache::dir_for(&opts).join("models/chapkit_ewars_model.yaml"),
            "schema_version: 2\nnot: a model\n",
        )
        .unwrap();

        let r = load(&opts).unwrap();
        assert!(
            matches!(r.provenance, Provenance::Embedded),
            "{:?}",
            r.provenance
        );
    }

    #[test]
    fn update_fails_loudly_when_the_network_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let err = update(&opts(tmp.path(), false)).expect_err("nothing is listening");
        assert!(
            matches!(
                err.downcast_ref::<ChapError>(),
                Some(ChapError::RegistryUnavailable(_))
            ),
            "{err:#}"
        );
    }

    #[test]
    fn update_refuses_to_run_offline() {
        let tmp = tempfile::tempdir().unwrap();
        let err = update(&opts(tmp.path(), true)).expect_err("--offline cannot refresh");
        match err.downcast_ref::<ChapError>() {
            Some(ChapError::RegistryUnavailable(why)) => {
                assert!(why.contains("--offline"), "{why}")
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn provenance_describes_its_age() {
        assert_eq!(Provenance::Network.describe(), "network");
        assert_eq!(Provenance::Embedded.describe(), "embedded");
        assert_eq!(
            Provenance::Cache { age_secs: 7200 }.describe(),
            "cache (2 hours old)"
        );
        assert_eq!(
            Provenance::StaleCache { age_secs: 172_800 }.describe(),
            "stale cache (2 days old)"
        );
        assert_eq!(
            Provenance::for_age(CACHE_TTL - Duration::from_secs(1)).label(),
            "cache"
        );
        assert_eq!(Provenance::for_age(CACHE_TTL).label(), "stale cache");
        assert_eq!(Provenance::Network.age(), None);
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
