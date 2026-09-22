//! On-disk cache of the fetched marketplace snapshot.
//!
//! Owned by agent A.
//!
//! Layout under [`RegistryOptions::cache_dir`](crate::registry::RegistryOptions):
//! `registry.yaml` plus `models/<id>.yaml`, i.e. the same relative paths the
//! index uses, so a cache directory is interchangeable with the vendored
//! snapshot.

use crate::error::Result;
use std::path::Path;
use std::time::Duration;

/// A cached snapshot: `(index_yaml, model_files, age of the cache entry)`.
///
/// The alias only exists to keep clippy's type-complexity lint quiet; the
/// shape is exactly the tuple it names.
pub type CachedSnapshot = (String, Vec<(String, String)>, Duration);

/// Read a cached snapshot.
///
/// Returns `Ok(None)` when there is no usable cache.
pub fn read(_dir: &Path) -> Result<Option<CachedSnapshot>> {
    Err(anyhow::anyhow!(
        "registry cache read is not implemented yet"
    ))
}

/// Write a snapshot to the cache directory, creating it if needed.
pub fn write(_dir: &Path, _index_yaml: &str, _model_files: &[(String, String)]) -> Result<()> {
    Err(anyhow::anyhow!(
        "registry cache write is not implemented yet"
    ))
}
