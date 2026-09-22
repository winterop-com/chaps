//! `chap models list|search|info` — read-only views of the catalogue.
//!
//! Owned by agent A. `enable` and `disable` live in
//! [`crate::commands::enable`] because they share the write path with `init`.

use crate::cli::{ModelsInfoArgs, ModelsListArgs, ModelsSearchArgs};
use crate::commands::Ctx;
use crate::error::Result;

/// List marketplace models, filtered by the `--all`, `--templates` and
/// `--enabled` flags.
pub fn list(_ctx: &Ctx, _args: &ModelsListArgs) -> Result<()> {
    Err(anyhow::anyhow!("`chap models list` is not implemented yet"))
}

/// Search the catalogue by id, display name or summary.
pub fn search(_ctx: &Ctx, _args: &ModelsSearchArgs) -> Result<()> {
    Err(anyhow::anyhow!(
        "`chap models search` is not implemented yet"
    ))
}

/// Show one model in full.
pub fn info(_ctx: &Ctx, _args: &ModelsInfoArgs) -> Result<()> {
    Err(anyhow::anyhow!("`chap models info` is not implemented yet"))
}
