//! `chap status` — chap-core health and registered services.
//!
//! Owned by agent C.

use crate::cli::StatusArgs;
use crate::commands::Ctx;
use crate::error::Result;

/// Probe the API and print a [`crate::status::StatusReport`].
pub fn run(_ctx: &Ctx, _args: &StatusArgs) -> Result<()> {
    Err(anyhow::anyhow!("`chap status` is not implemented yet"))
}
