//! `chap init` — write a deployment directory.
//!
//! Owned by agent B.

use crate::cli::InitArgs;
use crate::commands::Ctx;
use crate::error::Result;

/// Create compose.yml, compose.marketplace.yml, the model overlays, .env and
/// chap.json in the target directory.
///
/// `args.source` is parsed but must be rejected with "local chap-core build
/// not yet supported"; the flag only reserves the seam.
pub fn run(_ctx: &Ctx, _args: &InitArgs) -> Result<()> {
    Err(anyhow::anyhow!("`chap init` is not implemented yet"))
}
