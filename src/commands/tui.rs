//! `chap tui` / `chap models tui` — the model browser.
//!
//! Owned by agent C. The browser returns a
//! [`Selection`](crate::compose::Selection); applying it goes through
//! [`crate::compose::apply`] outside the terminal, and the resulting
//! [`ApplyReport`](crate::compose::ApplyReport) is printed afterwards.

use crate::cli::TuiArgs;
use crate::commands::Ctx;
use crate::error::Result;

/// Open the browser against the current project and catalogue.
///
/// `--json` is rejected before this is reached.
pub fn run(_ctx: &Ctx, _args: &TuiArgs) -> Result<()> {
    Err(anyhow::anyhow!("`chap tui` is not implemented yet"))
}
