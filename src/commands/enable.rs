//! `chap models enable|disable` — the two single-model write commands.
//!
//! Owned by agent B. Both go through [`crate::compose::apply`], the same path
//! `init` and the TUI use.

use crate::cli::{ModelsDisableArgs, ModelsEnableArgs};
use crate::commands::Ctx;
use crate::error::Result;

/// Enable one model: resolve its version, allocate a port, write the overlay,
/// regenerate the umbrella file and update chap.json.
pub fn enable(_ctx: &Ctx, _args: &ModelsEnableArgs) -> Result<()> {
    Err(anyhow::anyhow!(
        "`chap models enable` is not implemented yet"
    ))
}

/// Disable one model: remove its overlay, regenerate the umbrella file and
/// update chap.json.
pub fn disable(_ctx: &Ctx, _args: &ModelsDisableArgs) -> Result<()> {
    Err(anyhow::anyhow!(
        "`chap models disable` is not implemented yet"
    ))
}
