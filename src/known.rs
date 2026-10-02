//! The deployments chaps has written on this machine, by compose project.
//!
//! A compose project name ends in random characters
//! ([`crate::project::new_compose_project_name`]), so a volume docker holds
//! cannot be traced back to the directory that made it. `chaps cleanup` needs
//! exactly that to tell the data of a deployment that was deleted from the
//! data of one that is only down, so every save of a project records its
//! name and directory here, in `<data dir>/deployments.yaml`.
//!
//! Best-effort throughout: a record that could not be written only means
//! `chaps cleanup` leaves that deployment's volumes alone, which is the safe
//! way to be wrong.

use crate::project::Project;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file the record lives in.
pub fn file() -> PathBuf {
    crate::paths::data_dir().join("deployments.yaml")
}

/// Every recorded deployment: compose project name to directory. Empty when
/// nothing was recorded or the file cannot be read.
pub fn load() -> BTreeMap<String, PathBuf> {
    load_from(&file())
}

fn load_from(path: &Path) -> BTreeMap<String, PathBuf> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_yaml_ng::from_str(&text).ok())
        .unwrap_or_default()
}

/// Record the deployment in `project`, unless it is recorded already.
///
/// Unit tests save projects in temporary directories by the hundred, and none
/// of them may touch the record of the machine they run on.
pub fn record(project: &Project) {
    if cfg!(test) {
        return;
    }
    let Some(name) = project.compose_project_name() else {
        return;
    };
    let dir = std::path::absolute(&project.dir).unwrap_or_else(|_| project.dir.clone());
    let path = file();
    let mut known = load_from(&path);
    if known.get(&name) == Some(&dir) {
        return;
    }
    known.insert(name, dir);
    let _ = write(&path, &known);
}

/// Drop these projects from the record: what `chaps cleanup` took away.
pub fn forget(names: &[String]) -> crate::error::Result<()> {
    let path = file();
    let mut known = load_from(&path);
    let before = known.len();
    known.retain(|name, _| !names.contains(name));
    if known.len() == before {
        return Ok(());
    }
    write(&path, &known)
}

/// Whether the deployment recorded as `name` in `dir` is still there: the
/// directory holds a chaps project, and that project still has this name.
/// A directory `chaps init --force` wrote over holds a new name, so the old
/// one's volumes are left behind just as if it had been deleted.
pub fn is_present(name: &str, dir: &Path) -> bool {
    Project::exists(dir)
        && Project::load(dir)
            .ok()
            .and_then(|p| p.compose_project_name())
            .is_some_and(|n| n == name)
}

/// Write the record through a temporary file of this process's own, so two
/// chaps saving at once cannot rename each other's half-written file.
fn write(path: &Path, known: &BTreeMap<String, PathBuf>) -> crate::error::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", parent.display()))?;
    }
    let body = serde_yaml_ng::to_string(known)?;
    let tmp = path.with_file_name(format!(".deployments.{}.tmp", std::process::id()));
    std::fs::write(&tmp, body).map_err(|e| anyhow::anyhow!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| anyhow::anyhow!("renaming {} to {}: {e}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests;
