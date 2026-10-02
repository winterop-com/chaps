//! Which project files a backup holds, copying them, and the scratch
//! directory an archive is staged in.

use super::Manifest;
use super::TMP_DIR;
use crate::error::Result;
use std::path::{Path, PathBuf};

/// The project files a backup holds, relative to the project directory and in
/// a stable order: `.env`, then `.chaps/**`, then the directory of every
/// component that owns one in [`crate::components::Component::ALL`] order
/// (`ocs/**`, then `dhis2/**`), then the root `compose*.yml`.
///
/// `.chaps/tmp/` is scratch space (this is where the archive is staged) and
/// half-written `.tmp` state files are transient, so neither is included.
/// Compose files are taken from the project root only; a `compose.yml` in a
/// subdirectory belongs to something else.
///
/// The component directories are in here because what they hold is the
/// operator's own, not a rendered artifact: `chaps sync` only ever creates a
/// missing `ocs/climate-service.yaml` or `dhis2/dhis.conf`, so nothing can
/// rebuild the edits made to one, and DHIS2 does not start at all without its.
/// [`crate::components::Component::dir`] is asked rather than any of them being
/// named here, because a name in this function is a name the next component is
/// forgotten from - which is exactly how `dhis2/dhis.conf` came to be missing
/// from every archive.
///
/// A directory is taken whether or not its component is enabled right now: a
/// file that exists is one somebody wrote, `chaps components disable` says the
/// directory is left alone because it is the operator's, and a backup that
/// dropped it the moment the component went off would lose it at the one moment
/// nothing else is looking after it.
pub fn project_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if dir.join(crate::project::ENV_FILE).is_file() {
        out.push(crate::project::ENV_FILE.to_string());
    }

    let chaps = dir.join(crate::project::CHAPS_DIR);
    let mut state = Vec::new();
    collect_under(&chaps, crate::project::CHAPS_DIR, &mut state);
    state.sort();
    out.extend(state);

    for owned in crate::components::Component::ALL
        .iter()
        .filter_map(|component| component.dir())
    {
        let mut files = Vec::new();
        collect_under(&dir.join(owned), owned, &mut files);
        files.sort();
        out.extend(files);
    }

    let mut compose = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_compose_file(&name) && entry.path().is_file() {
                compose.push(name);
            }
        }
    }
    compose.sort();
    out.extend(compose);
    out
}

/// Whether a project-root file name is one of the compose files: `compose*.yml`.
pub fn is_compose_file(name: &str) -> bool {
    name.starts_with("compose") && name.ends_with(".yml")
}

/// Every file under `dir`, as `prefix/...` paths with forward slashes.
fn collect_under(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = format!("{prefix}/{name}");
        if entry.path().is_dir() {
            // The staging directory of the backup being taken right now.
            if prefix == crate::project::CHAPS_DIR && name == TMP_DIR {
                continue;
            }
            collect_under(&entry.path(), &path, out);
        } else if name.starts_with('.') && name.ends_with(".tmp") {
            continue;
        } else if prefix == crate::project::CHAPS_DIR
            && (name == crate::project::LOCK_FILE || name.ends_with(".lock"))
        {
            // The locks of whichever commands are running - the state lock and
            // `chaps run`'s `up-<service>.lock` - belong to this machine's
            // processes, not to the deployment.
            continue;
        } else {
            out.push(path);
        }
    }
}

/// Whether an archive-style `a/b/c` path stays inside the directory it is
/// joined to: no empty, `.` or `..` part, no leading `/`, no backslash or
/// drive. A manifest is read from the archive, so its paths are only as
/// trustworthy as the file, and [`join_relative`] would follow a `..` out.
pub fn is_contained_relative(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains(['\\', ':', '\0'])
        && rel
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Refuse an archive whose manifest names a project file outside the project
/// directory, before a restore touches anything.
pub fn check_manifest_files(manifest: &Manifest, archive: &Path) -> Result<()> {
    match manifest
        .files
        .iter()
        .find(|rel| !is_contained_relative(rel))
    {
        Some(rel) => Err(anyhow::anyhow!(
            "{} lists the project file `{rel}`, which is not inside a deployment directory; \
             chaps never writes such a path, so this archive was changed after `chaps backup \
             create` made it - restore from another one",
            archive.display()
        )),
        None => Ok(()),
    }
}

/// Copy `rel` from `from` to `to`, creating the parent directories.
pub fn copy_file(from: &Path, to: &Path, rel: &str) -> Result<()> {
    if !is_contained_relative(rel) {
        return Err(anyhow::anyhow!(
            "refusing to copy `{rel}`: it is not a path inside {}",
            to.display()
        ));
    }
    let src = join_relative(from, rel);
    let dst = join_relative(to, rel);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", parent.display()))?;
    }
    // Through a temporary file and a rename, so a restore that stops part-way
    // leaves each file either as it was or as the archive has it, never half.
    let name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dst.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    std::fs::copy(&src, &tmp)
        .map_err(|e| anyhow::anyhow!("copying {} to {}: {e}", src.display(), tmp.display()))?;
    std::fs::rename(&tmp, &dst).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow::anyhow!("renaming {} to {}: {e}", tmp.display(), dst.display())
    })?;
    Ok(())
}

/// Resolve an archive-style `a/b/c` path against a directory.
pub fn join_relative(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |acc, part| acc.join(part))
}

/// A scratch directory under `.chaps/tmp/`, removed when it goes out of scope.
///
/// It lives inside the project so the staged copy and the finished archive are
/// on the same filesystem, which keeps a multi-gigabyte model volume off
/// `/tmp` (often a small tmpfs) and makes the final rename cheap.
#[derive(Debug)]
pub struct Stage {
    pub dir: PathBuf,
}

impl Stage {
    /// Create `.chaps/tmp/<prefix>-<pid>` under `chaps_dir`.
    pub fn new(chaps_dir: &Path, prefix: &str) -> Result<Stage> {
        let dir = chaps_dir
            .join(TMP_DIR)
            .join(format!("{prefix}-{}", std::process::id()));
        // A crashed earlier run may have left one behind.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", dir.display()))?;
        Ok(Stage { dir })
    }

    /// `self.dir/rel`, with the parent directories created.
    pub fn path(&self, rel: &str) -> Result<PathBuf> {
        let path = join_relative(&self.dir, rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("creating {}: {e}", parent.display()))?;
        }
        Ok(path)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        // And `.chaps/tmp` itself, when this was the last stage in it: an
        // empty directory left behind would show up in the next backup.
        if let Some(parent) = self.dir.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}
