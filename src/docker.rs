//! Thin wrappers around `docker compose`.
//!
//! Every invocation passes the project's explicit `-f` list, so the wrappers
//! work from any working directory and never depend on compose's own file
//! discovery.
//!
//! Owned by agent C.

use crate::error::Result;
use crate::project::Project;

/// Minimum compose version that understands `include:`.
pub const MIN_COMPOSE_VERSION: (u32, u32, u32) = (2, 20, 0);

/// The leading `docker` arguments for a project: `compose -f ... -f ...`.
///
/// Owned by agent C.
pub fn compose_args(_project: &Project) -> Vec<String> {
    Vec::new()
}

/// Run `docker compose <args> <extra>` with inherited stdio, returning the
/// child's exit code.
///
/// Owned by agent C.
pub fn run_compose(_project: &Project, _extra: &[String]) -> Result<i32> {
    Err(anyhow::anyhow!(
        "docker compose wrapper is not implemented yet"
    ))
}

/// Capture `docker compose config` for the project.
///
/// Owned by agent C.
pub fn compose_config(_project: &Project) -> Result<String> {
    Err(anyhow::anyhow!(
        "docker compose config is not implemented yet"
    ))
}

/// The installed `docker compose` version.
///
/// Owned by agent C.
pub fn compose_version() -> Result<(u32, u32, u32)> {
    Err(anyhow::anyhow!(
        "docker compose version probing is not implemented yet"
    ))
}
