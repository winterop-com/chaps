//! The ratatui model browser.
//!
//! Owned by agent C. The TUI only produces a [`Selection`]; applying it is
//! done by the caller through [`crate::compose::apply`], so the write path
//! stays shared with `init`, `enable` and `disable`.

pub mod app;
pub mod keys;
pub mod ui;

use crate::commands::Ctx;
use crate::compose::Selection;
use crate::error::Result;
use crate::project::Project;
use crate::registry::Registry;

/// Run the browser. Returns `Ok(None)` when the user quits without saving.
///
/// Owned by agent C.
pub fn run_tui(_ctx: &Ctx, _project: &Project, _registry: &Registry) -> Result<Option<Selection>> {
    Err(anyhow::anyhow!("the model browser is not implemented yet"))
}
