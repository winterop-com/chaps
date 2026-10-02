//! The lock that keeps two chaps commands from changing one deployment at once.
//!
//! A command that changes `.chaps/` reads the state, changes it and writes it
//! back; two of them at the same time would each write over the other, and two
//! `--port auto` picks would land on the same port. The lock is an exclusive
//! advisory lock on `.chaps/lock`, held from before the state is read until
//! the command no longer writes. The operating system releases it when the
//! process ends, however it ends, so there is no stale lock to clear.

use super::{CHAPS_DIR, Project};
use crate::error::Result;
use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

/// The lock file's name inside `.chaps/`.
pub const LOCK_FILE: &str = "lock";

/// An exclusive hold on one deployment's state. Released when dropped.
#[derive(Debug)]
pub struct StateLock {
    _file: File,
}

impl StateLock {
    /// Take the lock for the deployment in `dir`, waiting for another chaps
    /// command that holds it, and saying so once when it has to wait.
    pub fn acquire(dir: &Path) -> Result<StateLock> {
        let path = dir.join(CHAPS_DIR).join(LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| anyhow::anyhow!("opening `{}`: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                crate::output::notice(
                    "another chaps command is changing this deployment; waiting for it to finish",
                );
                file.lock()
                    .map_err(|e| anyhow::anyhow!("locking `{}`: {e}", path.display()))?;
            }
            Err(TryLockError::Error(e)) => {
                return Err(anyhow::anyhow!("locking `{}`: {e}", path.display()));
            }
        }
        Ok(StateLock { _file: file })
    }
}

impl Project {
    /// Load the project that contains `start` with its state locked: the lock
    /// is taken before the state is read, so what is read is what the caller
    /// may change. Hold the [`StateLock`] until the last write.
    pub fn find_locked(start: &Path) -> Result<(Project, StateLock)> {
        // No deployment, nothing to lock: `find` says so in its own words.
        let Some(root) = Project::find_root(start) else {
            return Err(Project::find(start).expect_err("no root, so no project"));
        };
        let lock = StateLock::acquire(&root)?;
        Ok((Project::load(&root)?, lock))
    }

    /// Change the saved state of the deployment in `dir`, under its lock and
    /// on the state as it is on disk now, and save it.
    ///
    /// For a command that read the project long before it writes - a
    /// `dhis2 connect` waiting on DHIS2, a `down` waiting on compose - and
    /// changes one thing: saving its own copy would write back whatever it
    /// read, over anything another chaps saved meanwhile. `self` gets the
    /// same change, so the caller goes on with what it set. Never called by
    /// a command that already holds this deployment's lock: the lock waits
    /// for its holder, which would be the caller itself.
    pub fn update_saved(&mut self, change: impl Fn(&mut super::ProjectState)) -> Result<()> {
        let (mut fresh, _lock) = Project::find_locked(&self.dir)?;
        change(&mut fresh.state);
        fresh.save()?;
        change(&mut self.state);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
