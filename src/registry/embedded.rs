//! The vendored marketplace snapshot, compiled into the binary.
//!
//! Used as the last-resort fallback when the registry can be neither fetched
//! nor read from cache, so `chap` works on a machine that has never had
//! network access.
//!
//! Refresh the files under `vendor/marketplace/` with
//! `scripts/vendor-marketplace.sh`, then update [`FILES`] if the set of model
//! files changed.

/// `(relative path, file contents)` pairs.
///
/// The first entry is always `registry.yaml`; the rest are the model files
/// keyed by the same relative paths the index lists, in index order.
const FILES: &[(&str, &str)] = &[
    (
        "registry.yaml",
        include_str!("../../vendor/marketplace/registry.yaml"),
    ),
    (
        "models/chapkit_ewars_model.yaml",
        include_str!("../../vendor/marketplace/models/chapkit_ewars_model.yaml"),
    ),
    (
        "models/chapkit_rwanda_malaria_bym_model.yaml",
        include_str!("../../vendor/marketplace/models/chapkit_rwanda_malaria_bym_model.yaml"),
    ),
    (
        "models/chapkit_simple_multistep_model.yaml",
        include_str!("../../vendor/marketplace/models/chapkit_simple_multistep_model.yaml"),
    ),
    (
        "models/auto_arima_chapkit.yaml",
        include_str!("../../vendor/marketplace/models/auto_arima_chapkit.yaml"),
    ),
    (
        "models/chapkit_minimalist_example_py.yaml",
        include_str!("../../vendor/marketplace/models/chapkit_minimalist_example_py.yaml"),
    ),
    (
        "models/chapkit_minimalist_example_r.yaml",
        include_str!("../../vendor/marketplace/models/chapkit_minimalist_example_r.yaml"),
    ),
];

/// The embedded snapshot: `registry.yaml` first, then the model files.
pub fn files() -> &'static [(&'static str, &'static str)] {
    FILES
}

/// The embedded `registry.yaml`.
pub fn index_yaml() -> &'static str {
    FILES[0].1
}

/// The embedded model files, without the index.
pub fn model_files() -> Vec<(String, String)> {
    FILES[1..]
        .iter()
        .map(|(name, body)| ((*name).to_string(), (*body).to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_matches_the_index_it_ships_with() {
        let index: crate::registry::model::RegistryIndex =
            serde_yaml_ng::from_str(index_yaml()).expect("registry.yaml parses");
        let embedded: Vec<&str> = FILES[1..].iter().map(|(n, _)| *n).collect();
        for path in &index.models {
            assert!(
                embedded.contains(&path.as_str()),
                "{path} is listed in registry.yaml but not embedded; \
                 re-run scripts/vendor-marketplace.sh and update FILES"
            );
        }
        assert_eq!(embedded.len(), index.models.len());
    }

    #[test]
    fn nothing_is_empty() {
        for (name, body) in files() {
            assert!(!body.trim().is_empty(), "{name} is empty");
        }
    }
}
