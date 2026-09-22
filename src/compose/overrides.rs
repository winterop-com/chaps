//! Per-image data directory and user overrides.
//!
//! Most chapkit services write to `/work/data` as `chapkit:chapkit`, but some
//! images differ; getting this wrong crash-loops the container on the
//! read-only root filesystem. `--data-dir` and `--user` override the table.
//!
//! Owned by agent B.

/// The non-default data dir and user of one image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageOverride {
    pub data_dir: &'static str,
    pub user: &'static str,
}

/// Data directory used by most chapkit images.
pub const DEFAULT_DATA_DIR: &str = "/work/data";
/// User most chapkit images run as.
pub const DEFAULT_USER: &str = "chapkit:chapkit";

/// Look up the override for a marketplace id, if it has one.
///
/// Owned by agent B.
pub fn known_override(_id: &str) -> Option<ImageOverride> {
    None
}
