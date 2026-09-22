//! The single write path shared by `init`, `enable`, `disable` and the TUI.
//!
//! Owned by agent B.

use crate::error::Result;
use crate::project::{EnabledModel, Project};
use crate::registry::{Registry, VersionSelector};
use serde::Serialize;
use std::path::PathBuf;

/// One model the caller wants enabled, with any explicit overrides.
#[derive(Debug, Clone)]
pub struct EnableRequest {
    pub id: String,
    pub selector: VersionSelector,
    pub port: Option<u16>,
    pub data_dir: Option<String>,
    pub user: Option<String>,
    pub allow_template: bool,
}

/// A batch of enables and disables applied together.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub enable: Vec<EnableRequest>,
    pub disable: Vec<String>,
}

/// What [`apply`] changed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ApplyReport {
    pub enabled: Vec<(String, EnabledModel)>,
    pub updated: Vec<(String, EnabledModel)>,
    pub disabled: Vec<String>,
    pub written: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

impl ApplyReport {
    /// Whether anything changed on disk or in state.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
            && self.updated.is_empty()
            && self.disabled.is_empty()
            && self.written.is_empty()
            && self.removed.is_empty()
    }
}

/// Apply a selection: disable first, then enable, then rewrite the umbrella
/// file and `chap.json`.
///
/// Owned by agent B.
pub fn apply(
    _project: &mut Project,
    _registry: &Registry,
    _sel: &Selection,
    _cli_version: &str,
) -> Result<ApplyReport> {
    Err(anyhow::anyhow!("compose apply is not implemented yet"))
}

/// Regenerate `compose.marketplace.yml` from the project state.
///
/// Owned by agent B.
pub fn write_umbrella(_project: &Project) -> Result<PathBuf> {
    Err(anyhow::anyhow!(
        "umbrella compose generation is not implemented yet"
    ))
}
