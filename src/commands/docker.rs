//! `chap up|down|ps|logs|pull|compose` — the docker compose wrappers.
//!
//! Owned by agent C.

use crate::cli::DockerCmd;
use crate::commands::Ctx;
use crate::error::Result;

/// Run one docker compose wrapper against the project's explicit `-f` list.
///
/// A non-zero child exit status must surface as
/// [`crate::error::ChapError::DockerFailed`] so `main` can mirror the code.
pub fn run(_ctx: &Ctx, _cmd: &DockerCmd) -> Result<()> {
    Err(anyhow::anyhow!(
        "the docker compose wrappers are not implemented yet"
    ))
}
