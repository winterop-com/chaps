//! Network fetch of the marketplace index and the model files it lists.
//!
//! Owned by agent A.

use crate::error::Result;
use crate::registry::RegistryOptions;

/// Fetch `registry.yaml` and every model file it lists over HTTP.
///
/// Returns `(index_yaml, model_files)` where each model file is
/// `(relative path as listed in the index, contents)`.
///
/// Contract: uses `ureq` with `opts.timeout`, resolves model URLs through
/// [`crate::registry::model_url`], and surfaces non-2xx responses as
/// [`crate::error::ChapError::Http`].
pub fn fetch_registry(_opts: &RegistryOptions) -> Result<(String, Vec<(String, String)>)> {
    Err(anyhow::anyhow!("registry fetch is not implemented yet"))
}
