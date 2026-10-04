//! The compose project name: the `<slug>-<suffix>` one `chaps init`
//! generates.

use crate::error::Result;
use std::path::Path;

/// How many random hex characters a generated compose project name ends in.
///
/// Three bytes: short enough to keep a container name readable, and 16 million
/// values is far more than the handful of deployments one machine ever holds.
pub const PROJECT_SUFFIX_BYTES: usize = 3;

/// The longest slug a generated compose project name starts with.
///
/// Compose puts the project name in front of every container and volume name,
/// and a name nobody can read on a `docker ps` line helps no one.
pub(super) const MAX_SLUG: usize = 32;

/// What a generated name falls back to when the directory name yields no
/// usable slug at all (`~/深度`, say).
pub(super) const FALLBACK_SLUG: &str = "chaps";

/// The readable half of a generated compose project name.
///
/// Lowercase `[a-z0-9-]` starting with a letter or a digit, which is what
/// Compose accepts and what reads as the deployment's own name on a
/// `docker ps` line. Anything else folds to a single `-`.
pub fn project_slug(dir_name: &str) -> String {
    let mut out = String::with_capacity(dir_name.len());
    for ch in dir_name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    let out: String = out.chars().take(MAX_SLUG).collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        return FALLBACK_SLUG.to_string();
    }
    out
}

/// `<slug>-<suffix>`, the compose project name a new deployment gets.
pub fn compose_project_name(dir_name: &str, suffix: &str) -> String {
    format!("{}-{suffix}", project_slug(dir_name))
}

/// A compose project name for a deployment being created in `dir`.
///
/// The directory name is only half of it: two directories both called `demo`
/// would otherwise share every named volume, so a fresh deployment gets six
/// random hex characters of its own. See [`ProjectState::compose_project`].
///
/// [`ProjectState::compose_project`]: super::ProjectState::compose_project
pub fn new_compose_project_name(dir: &Path) -> Result<String> {
    let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(compose_project_name(
        &name,
        &crate::auth::random_hex(PROJECT_SUFFIX_BYTES)?,
    ))
}
