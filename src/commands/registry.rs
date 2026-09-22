//! `chap registry update|show`.
//!
//! Owned by agent A.

use crate::cli::RegistryCmd;
use crate::commands::Ctx;
use crate::error::Result;

/// Refresh the cached registry, or report what is currently loaded and where
/// it came from.
pub fn run(_ctx: &Ctx, _cmd: &RegistryCmd) -> Result<()> {
    Err(anyhow::anyhow!("`chap registry` is not implemented yet"))
}
