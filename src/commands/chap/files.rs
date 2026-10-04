//! Which files a run of the chap CLI wrote, so the closing line can name
//! them: a look at the mounted directories before the run and after it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How deep the look goes below each mounted directory. Outputs are written
/// where the arguments say, which is seldom deeper than this.
const MAX_DEPTH: usize = 3;

/// How many files the look reads at most, so a run from a large directory
/// does not spend its time listing it.
const MAX_FILES: usize = 20_000;

/// Directories that hold no output of chap, and can hold very many files.
const SKIPPED: &[&str] = &["node_modules", "target", "__pycache__"];

/// Every file under the mounted directories, with the time it was last
/// changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot(BTreeMap<PathBuf, SystemTime>);

impl Snapshot {
    /// Look at `roots`, at most [`MAX_DEPTH`] levels down.
    pub fn take(roots: &[PathBuf]) -> Snapshot {
        let mut files = BTreeMap::new();
        for root in roots {
            walk(root, 0, &mut files);
        }
        Snapshot(files)
    }

    /// The files that are new in `after`, or changed since `self`.
    pub fn written(&self, after: &Snapshot) -> Vec<PathBuf> {
        after
            .0
            .iter()
            .filter(|(path, changed)| self.0.get(*path) != Some(*changed))
            .map(|(path, _)| path.clone())
            .collect()
    }

    #[cfg(test)]
    pub fn from_entries(entries: &[(&str, u64)]) -> Snapshot {
        Snapshot(
            entries
                .iter()
                .map(|(path, secs)| {
                    let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(*secs);
                    (PathBuf::from(path), at)
                })
                .collect(),
        )
    }
}

fn walk(dir: &Path, depth: usize, files: &mut BTreeMap<PathBuf, SystemTime>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= MAX_FILES {
            return;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            let hidden = name.starts_with('.');
            if depth + 1 < MAX_DEPTH && !hidden && !SKIPPED.contains(&name.as_ref()) {
                walk(&path, depth + 1, files);
            }
        } else if kind.is_file()
            && let Ok(changed) = entry.metadata().and_then(|meta| meta.modified())
        {
            files.insert(path, changed);
        }
    }
}

/// The written files as the closing line names them: relative to `cwd`
/// where they are inside it, and at most `limit` of them.
pub fn describe(written: &[PathBuf], cwd: &Path, limit: usize) -> String {
    let mut names: Vec<String> = written
        .iter()
        .take(limit)
        .map(|path| {
            path.strip_prefix(cwd)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    if written.len() > limit {
        names.push(format!("and {} more", written.len() - limit));
    }
    names.join(", ")
}
