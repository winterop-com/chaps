//! Host port allocation for model overlays.
//!
//! Owned by agent B.

use crate::error::Result;
use std::collections::BTreeSet;
use std::path::Path;

/// Hands out free host ports inside a range, never reusing a claimed one.
#[derive(Debug, Clone)]
pub struct PortAllocator {
    used: BTreeSet<u16>,
    lo: u16,
    hi: u16,
}

impl PortAllocator {
    /// Seed an allocator with the ports already in use.
    pub fn new(range: (u16, u16), used: impl IntoIterator<Item = u16>) -> Self {
        PortAllocator {
            used: used.into_iter().collect(),
            lo: range.0,
            hi: range.1,
        }
    }

    /// Host ports published by any `compose*.yml` in `dir`.
    ///
    /// Owned by agent B.
    pub fn scan_compose_dir(_dir: &Path) -> Result<BTreeSet<u16>> {
        Err(anyhow::anyhow!(
            "compose port scanning is not implemented yet"
        ))
    }

    /// Take the lowest free port in the range.
    ///
    /// Owned by agent B.
    pub fn allocate(&mut self) -> Result<u16> {
        Err(anyhow::anyhow!("port allocation is not implemented yet"))
    }

    /// Reserve a specific port.
    ///
    /// Errors with [`crate::error::ChapError::PortOutOfRange`] outside the
    /// range and [`crate::error::ChapError::PortInUse`] when already taken.
    ///
    /// Owned by agent B.
    pub fn claim(&mut self, _p: u16) -> Result<()> {
        Err(anyhow::anyhow!("port claiming is not implemented yet"))
    }

    /// The ports currently considered taken.
    pub fn used(&self) -> &BTreeSet<u16> {
        &self.used
    }

    /// The inclusive range ports are handed out from.
    pub fn range(&self) -> (u16, u16) {
        (self.lo, self.hi)
    }
}
