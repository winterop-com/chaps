//! Finding, loading and saving the state files in `.chaps/`.

use super::{
    CHAPS_DIR, MANUAL_MODELS_FILE, MODELS_FILE, ManualModels, PROJECT_FILE, Project, ProjectState,
};
use crate::components::{COMPONENTS_FILE, Components};
use crate::error::{ChapError, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The first line of every file in `.chaps/`.
///
/// One line, the same in each of them: what each file holds and which command
/// edits it is documented, and a generated file that carries the explanation
/// too is one more copy to keep in step.
pub(super) const MANAGED_HEADER: &str =
    "# Managed by chaps; change it with the chaps commands, not by hand.\n";

impl Project {
    /// Whether `dir` itself holds a `.chaps/project.yaml`.
    pub fn exists(dir: &Path) -> bool {
        dir.join(CHAPS_DIR).join(PROJECT_FILE).is_file()
    }

    /// The project directory that contains `start`: `start` itself or the
    /// nearest ancestor holding a `.chaps/project.yaml`, like git's discovery
    /// of `.git`.
    pub fn find_root(start: &Path) -> Option<PathBuf> {
        let start = std::path::absolute(start).ok()?;
        let mut dir: &Path = &start;
        loop {
            if Project::exists(dir) {
                return Some(dir.to_path_buf());
            }
            dir = dir.parent()?;
        }
    }

    /// The marketplace registry `project.yaml` under `root` records, which
    /// `chaps init --registry-url` wrote there.
    ///
    /// Read on its own rather than through [`Project::load`], because it is
    /// asked before any command runs: a deployment whose other state files do
    /// not load still has a registry, and the command that reports the broken
    /// file needs it. `None` for a file that is missing or does not parse, and
    /// the command then goes on with the default.
    pub fn saved_registry_url(root: &Path) -> Option<String> {
        let body = std::fs::read_to_string(root.join(CHAPS_DIR).join(PROJECT_FILE)).ok()?;
        let state: serde_yaml_ng::Value = serde_yaml_ng::from_str(&body).ok()?;
        state
            .get("registry_url")?
            .as_str()
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_string)
    }

    /// The compose project name `dir`'s `project.yaml` records, read on its
    /// own like [`Project::saved_registry_url`]: a typo in another state file
    /// must not make a deployment look like it has no name, because what
    /// follows from "no name" - a fresh one, or "this deployment is gone" -
    /// strands or deletes its data.
    ///
    /// `Ok(None)` when there is no `project.yaml`, or it records no name;
    /// an error when it is there and cannot be read or parsed.
    pub fn recorded_name(dir: &Path) -> Result<Option<String>> {
        let path = dir.join(CHAPS_DIR).join(PROJECT_FILE);
        let body = match std::fs::read_to_string(&path) {
            Ok(body) => body,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(anyhow::anyhow!("reading {}: {e}", path.display())),
        };
        let state: serde_yaml_ng::Value = serde_yaml_ng::from_str(&body)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        Ok(state
            .get("compose_project")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string))
    }

    /// Load the project that contains `start`, walking up parent directories.
    ///
    /// Errors with [`ChapError::NotAProject`] naming `start` when no ancestor
    /// is a project.
    pub fn find(start: &Path) -> Result<Project> {
        match Project::find_root(start) {
            Some(root) => Project::load(&root),
            None => {
                let shown = std::path::absolute(start).unwrap_or_else(|_| start.to_path_buf());
                Err(ChapError::NotAProject(shown).into())
            }
        }
    }

    /// Read `dir/.chaps/project.yaml` and `dir/.chaps/models.yaml`.
    ///
    /// Errors with [`ChapError::NotAProject`] when `project.yaml` is absent;
    /// a missing `models.yaml` means no models are enabled.
    pub fn load(dir: &Path) -> Result<Project> {
        let chaps = dir.join(CHAPS_DIR);
        let project_path = chaps.join(PROJECT_FILE);
        let body = match std::fs::read_to_string(&project_path) {
            Ok(body) => body,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ChapError::NotAProject(dir.to_path_buf()).into());
            }
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context(format!("reading {}", project_path.display()))
                );
            }
        };
        let mut state: ProjectState = serde_yaml_ng::from_str(&body).map_err(|e| {
            anyhow::anyhow!("{}: invalid project.yaml: {e}", project_path.display())
        })?;

        let models_path = chaps.join(MODELS_FILE);
        state.models = match std::fs::read_to_string(&models_path) {
            Ok(body) if is_blank_yaml(&body) => BTreeMap::new(),
            Ok(body) => serde_yaml_ng::from_str(&body).map_err(|e| {
                anyhow::anyhow!("{}: invalid models.yaml: {e}", models_path.display())
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context(format!("reading {}", models_path.display()))
                );
            }
        };

        // Definitions, not enablements: a deployment that has added no model
        // of its own has no such file, which is not a state to migrate.
        let manual_path = chaps.join(MANUAL_MODELS_FILE);
        state.manual = match std::fs::read_to_string(&manual_path) {
            Ok(body) if is_blank_yaml(&body) => ManualModels::new(),
            Ok(body) => serde_yaml_ng::from_str(&body).map_err(|e| {
                anyhow::anyhow!("{}: invalid models-manual.yaml: {e}", manual_path.display())
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ManualModels::new(),
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context(format!("reading {}", manual_path.display()))
                );
            }
        };

        // Every field of `Components` defaults, so a missing or comment-only
        // file is "chap-core and nothing else" rather than an error.
        let components_path = chaps.join(COMPONENTS_FILE);
        state.components = match std::fs::read_to_string(&components_path) {
            Ok(body) if is_blank_yaml(&body) => Components::default(),
            Ok(body) => serde_yaml_ng::from_str(&body).map_err(|e| {
                anyhow::anyhow!(
                    "{}: invalid components.yaml: {e}",
                    components_path.display()
                )
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Components::default(),
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context(format!("reading {}", components_path.display()))
                );
            }
        };
        check_file_names(&state)?;
        Ok(Project {
            dir: dir.to_path_buf(),
            state,
        })
    }

    /// Write `.chaps/project.yaml` and `.chaps/models.yaml`.
    ///
    /// Each file goes to a temporary sibling first and is renamed into place,
    /// so a crash never leaves a half-written state file behind.
    pub fn save(&self) -> Result<()> {
        let chaps = self.dir.join(CHAPS_DIR);
        std::fs::create_dir_all(&chaps)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", chaps.display()))?;

        let project_body = format!("{MANAGED_HEADER}{}", serde_yaml_ng::to_string(&self.state)?);
        write_atomically(&chaps.join(PROJECT_FILE), &project_body)?;

        let models_body = format!(
            "{MANAGED_HEADER}{}",
            serde_yaml_ng::to_string(&self.state.models)?
        );
        write_atomically(&chaps.join(MODELS_FILE), &models_body)?;

        // Written only by a deployment that has one: an empty file in every
        // other project would be a file to explain, and `chaps models remove`
        // leaves the (now empty) one it emptied rather than deleting a file
        // the operator can see.
        let manual_path = chaps.join(MANUAL_MODELS_FILE);
        if !self.state.manual.is_empty() || manual_path.is_file() {
            let manual_body = format!(
                "{MANAGED_HEADER}{}",
                serde_yaml_ng::to_string(&self.state.manual)?
            );
            write_atomically(&manual_path, &manual_body)?;
        }

        let components_body = format!(
            "{MANAGED_HEADER}{}",
            serde_yaml_ng::to_string(&self.state.components)?
        );
        write_atomically(&chaps.join(COMPONENTS_FILE), &components_body)?;
        // So `chaps cleanup` can tell this deployment's volumes from a
        // deleted one's.
        crate::known::record(self);
        Ok(())
    }
}

/// Whether `name` is one file directly in the deployment directory: no
/// separator, no drive, nothing that climbs out of it.
///
/// Every compose file chaps renders is such a name (`compose.yml`,
/// `compose.<service>.yml`), and `sync` writes and removes them with
/// `dir.join(name)` - which an absolute path or a `..` would take outside the
/// directory. State that arrives from elsewhere, a restored backup above all,
/// is held to this before anything joins it.
pub fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':', '\0'])
}

/// Refuse state whose compose file names are not plain file names. See
/// [`is_plain_file_name`].
fn check_file_names(state: &ProjectState) -> Result<()> {
    let way_out = "chaps only writes `compose.<service>.yml` there; correct the entry by hand, \
                   or run `chaps init --force` to rebuild the state";
    for (id, model) in &state.models {
        if !is_plain_file_name(&model.compose_file) {
            return Err(anyhow::anyhow!(
                "`{CHAPS_DIR}/{MODELS_FILE}` gives {id} the compose file `{}`, which is not a \
                 file in this deployment's directory; {way_out}",
                model.compose_file
            ));
        }
    }
    for name in state.compose_files.iter().chain(&state.rendered_files) {
        if !is_plain_file_name(name) {
            return Err(anyhow::anyhow!(
                "`{CHAPS_DIR}/{PROJECT_FILE}` lists the compose file `{name}`, which is not a \
                 file in this deployment's directory; {way_out}"
            ));
        }
    }
    Ok(())
}

/// A YAML document with nothing but comments and blank lines.
fn is_blank_yaml(body: &str) -> bool {
    body.lines()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
}

fn write_atomically(path: &Path, body: &str) -> Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, body).map_err(|e| anyhow::anyhow!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| anyhow::anyhow!("renaming {} to {}: {e}", tmp.display(), path.display()))?;
    Ok(())
}
