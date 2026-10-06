//! The instance configs sync scaffolds, `ocs/climate-service.yaml` and
//! `dhis2/dhis.conf`, and the single keys it edits in the OCS one.

use crate::components::{
    DHIS2_CONFIG_FILE, DHIS2_DIR, OCS_CONFIG_FILE, OCS_DIR, OCS_PLUGINS_KEY, OCS_PLUGINS_TARGET,
    OCS_READ_ONLY_KEY,
};
use crate::compose::render::{render_dhis2_config, render_ocs_config};
use crate::compose::spec::{Dhis2ConfigSpec, OcsConfigSpec};
use crate::error::Result;
use crate::project::Project;
use std::path::{Path, PathBuf};

/// Scaffold `ocs/climate-service.yaml` when the `ocs` component is on and the
/// file is not there yet.
///
/// Never overwrites: `varde components enable ocs --ocs-country ...` writes it
/// with the values it was given, and this only covers the case where the
/// component is on and the file has gone missing - a fresh checkout of a
/// deployment whose `ocs/` was never committed, most of all.
pub(super) fn ensure_ocs_config(project: &Project, check: bool) -> Result<Option<PathBuf>> {
    if !project.state.components.ocs.enabled {
        return Ok(None);
    }
    let path = project.ocs_config_path();
    if path.is_file() {
        return Ok(None);
    }
    if check {
        return Ok(Some(path));
    }
    write_ocs_config(&project.dir, &OcsConfigSpec::default())?;
    Ok(Some(path))
}

/// Write `ocs/climate-service.yaml`, creating `ocs/` around it.
///
/// Returns the path when the file was written and `None` when one was already
/// there: the scaffold is a starting point, not something to put back.
pub fn write_ocs_config(dir: &Path, spec: &OcsConfigSpec) -> Result<Option<PathBuf>> {
    let ocs = dir.join(OCS_DIR);
    let path = ocs.join(OCS_CONFIG_FILE);
    if path.is_file() {
        return Ok(None);
    }
    std::fs::create_dir_all(&ocs)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", ocs.display()))?;
    std::fs::write(&path, render_ocs_config(spec))
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(Some(path))
}

/// Scaffold `dhis2/dhis.conf` when the `dhis2` component is on and the file is
/// not there yet.
///
/// Never overwrites: `varde components enable dhis2` writes it the first time,
/// and this covers the case where the component is on and the file has gone -
/// a fresh checkout of a deployment whose `dhis2/` was never committed, most of
/// all. Without the file DHIS2 does not start at all, so a missing one is worth
/// putting back.
pub(super) fn ensure_dhis2_config(project: &Project, check: bool) -> Result<Option<PathBuf>> {
    if !project.state.components.dhis2.enabled {
        return Ok(None);
    }
    let path = dhis2_config_path(&project.dir);
    if path.is_file() {
        return Ok(None);
    }
    if check {
        return Ok(Some(path));
    }
    write_dhis2_config(&project.dir, &Dhis2ConfigSpec::default())?;
    Ok(Some(path))
}

/// Where a deployment keeps its DHIS2 instance config.
pub fn dhis2_config_path(dir: &Path) -> PathBuf {
    dir.join(DHIS2_DIR).join(DHIS2_CONFIG_FILE)
}

/// Write `dhis2/dhis.conf`, creating `dhis2/` around it.
///
/// Returns the path when the file was written and `None` when one was already
/// there: the scaffold is a starting point, not something to put back.
pub fn write_dhis2_config(dir: &Path, spec: &Dhis2ConfigSpec) -> Result<Option<PathBuf>> {
    let dhis2 = dir.join(DHIS2_DIR);
    let path = dhis2.join(DHIS2_CONFIG_FILE);
    if path.is_file() {
        return Ok(None);
    }
    std::fs::create_dir_all(&dhis2)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", dhis2.display()))?;
    std::fs::write(&path, render_dhis2_config(spec))
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(Some(path))
}

/// What [`set_config_key`] found in `ocs/climate-service.yaml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEdit {
    /// The key was already set to this value; nothing changed.
    Unchanged,
    /// Its line was rewritten in place.
    Rewritten,
    /// The file did not have the key, so it was added at the end.
    Appended,
}

/// Set one top-level key of an OCS instance config, and nothing else.
///
/// The file is the operator's: it carries their comments, their dataset list
/// and their scheduler, so this rewrites the one line that assigns `key` and
/// leaves every other byte alone, appending the key when the file does not have
/// it. Text rather than a serde round trip for exactly that reason - parsing
/// and re-emitting the document would drop every comment in it.
///
/// Only a top-level key counts, so a `read_only:` nested inside some other
/// block is not mistaken for this one, and neither is a commented line: the
/// commented `# read_only: true` that a note explains is still only a note.
/// A trailing comment on the line survives the rewrite.
pub fn set_config_key(body: &str, key: &str, value: &str) -> (String, KeyEdit) {
    let wanted = format!("{key}: {value}");
    let mut out = String::with_capacity(body.len() + wanted.len() + 2);
    let mut edit = KeyEdit::Appended;
    for line in body.lines() {
        match top_level_value(line, key) {
            Some(rest) if edit == KeyEdit::Appended => {
                // The operator's comment on this line says why the value is
                // what it is, so it comes across with its own spacing.
                let replacement = format!("{wanted}{}", trailing_comment(rest));
                edit = if replacement == line {
                    KeyEdit::Unchanged
                } else {
                    KeyEdit::Rewritten
                };
                out.push_str(&replacement);
            }
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    if edit != KeyEdit::Appended {
        return (out, edit);
    }
    // A blank line first, so an appended key reads as its own setting rather
    // than as a continuation of whatever the file happened to end on.
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&wanted);
    out.push('\n');
    (out, KeyEdit::Appended)
}

/// Whether an OCS instance config assigns a top-level `key`.
pub fn has_config_key(body: &str, key: &str) -> bool {
    body.lines()
        .any(|line| top_level_value(line, key).is_some())
}

/// The inline comment of a value, whitespace and all, or `""`.
///
/// YAML needs whitespace before an inline `#`, which is also what tells one
/// apart from a `#` inside the value; the whitespace comes along so the
/// rewritten line is aligned exactly as the operator aligned it.
fn trailing_comment(rest: &str) -> &str {
    let Some(at) = rest
        .char_indices()
        .find(|(at, ch)| *ch == '#' && rest[..*at].ends_with([' ', '\t']))
        .map(|(at, _)| at)
    else {
        return "";
    };
    &rest[rest[..at].trim_end_matches([' ', '\t']).len()..]
}

/// The text after `key:` when `line` is that key's top-level assignment.
///
/// Top-level means column zero: YAML nests by indentation, so an indented
/// `read_only:` belongs to some other block and is none of our business.
fn top_level_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    if line.starts_with([' ', '\t', '#']) {
        return None;
    }
    line.strip_prefix(key)?.strip_prefix(':')
}

/// Write `read_only` into `ocs/climate-service.yaml`.
///
/// Returns `None` when there is no file to edit, which is the case a caller
/// reports rather than fails on: the component may not be enabled yet.
pub fn set_read_only(dir: &Path, read_only: bool) -> Result<Option<KeyEdit>> {
    let path = dir.join(OCS_DIR).join(OCS_CONFIG_FILE);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    let (out, edit) = set_config_key(&body, OCS_READ_ONLY_KEY, &read_only.to_string());
    if edit == KeyEdit::Unchanged {
        return Ok(Some(edit));
    }
    std::fs::write(&path, out).map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(Some(edit))
}

/// Point the instance config at the mounted plugin directory when the project
/// has one and the file does not name it yet.
///
/// Only ever adds the key: an operator who pointed `plugins_dir` somewhere else
/// meant it, and the mount is at a fixed path either way. Returns the path when
/// the file was (or with `check`, would be) changed.
pub(super) fn ensure_plugins_key(project: &Project, check: bool) -> Result<Option<PathBuf>> {
    if !project.state.components.ocs.enabled || !project.ocs_plugins_path().is_dir() {
        return Ok(None);
    }
    let path = project.ocs_config_path();
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    if has_config_key(&body, OCS_PLUGINS_KEY) {
        return Ok(None);
    }
    if check {
        return Ok(Some(path));
    }
    let (out, _) = set_config_key(&body, OCS_PLUGINS_KEY, OCS_PLUGINS_TARGET);
    std::fs::write(&path, out).map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(Some(path))
}
