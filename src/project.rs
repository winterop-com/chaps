//! `.chaps/` — the directory that records what a deployment is meant to be.
//!
//! Two YAML files: `project.yaml` holds the project-wide settings and
//! `models.yaml` the enabled model set. They are intent; the compose files at
//! the project root are artifacts rendered from them by `chaps sync`.

use crate::error::{ChapError, Result};
use crate::registry::Channel;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Directory that marks a project, at the root of a project directory.
pub const CHAPS_DIR: &str = ".chaps";
/// Project-wide settings, inside [`CHAPS_DIR`].
pub const PROJECT_FILE: &str = "project.yaml";
/// The enabled model set, inside [`CHAPS_DIR`].
pub const MODELS_FILE: &str = "models.yaml";
/// Base compose file: chap-core, worker, valkey, postgres.
pub const BASE_COMPOSE: &str = "compose.yml";
/// Umbrella file that `include:`s one overlay per enabled model.
pub const MARKETPLACE_COMPOSE: &str = "compose.marketplace.yml";
/// Environment file docker compose picks up automatically.
pub const ENV_FILE: &str = ".env";

/// `project.yaml` schema version written by this CLI.
pub const SCHEMA_VERSION: u32 = 1;
/// Host port range model overlays are allocated from.
pub const DEFAULT_PORT_RANGE: (u16, u16) = (5001, 5999);

const PROJECT_HEADER: &str = "\
# .chaps/project.yaml - managed by chaps. Written by `chaps init`; `rendered_files` is
# updated by `chaps sync` (which `chaps up` runs first). Compose files at the project
# root are rendered from this directory; edit here, then run `chaps sync`.
";

const MODELS_HEADER: &str = "\
# .chaps/models.yaml - managed by chaps. The enabled model set, edited by
# `chaps models enable|disable`, `chaps tui` and `chaps update`.
# `chaps sync` renders one compose.<service_id>.yml per entry plus compose.marketplace.yml.
";

/// The in-memory project state: `project.yaml` plus `models.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub schema_version: u32,
    /// e.g. `chaps-cli 0.1.0`.
    pub generated_by: String,
    pub chap_image_tag: String,
    pub registry_url: String,
    /// Ordered `-f` list, relative to the project directory.
    pub compose_files: Vec<String>,
    pub port_range: (u16, u16),
    /// Files at the project root that `chaps sync` wrote last time, relative
    /// to the project directory. Only these are ever removed by a later sync.
    #[serde(default)]
    pub rendered_files: Vec<String>,
    /// Enabled models, keyed by marketplace `id`. Lives in `models.yaml`.
    #[serde(skip)]
    pub models: BTreeMap<String, EnabledModel>,
}

impl Default for ProjectState {
    fn default() -> Self {
        ProjectState {
            schema_version: SCHEMA_VERSION,
            generated_by: format!("chaps-cli {}", env!("CARGO_PKG_VERSION")),
            chap_image_tag: "latest".to_string(),
            registry_url: crate::registry::DEFAULT_REGISTRY_URL.to_string(),
            compose_files: vec![BASE_COMPOSE.to_string(), MARKETPLACE_COMPOSE.to_string()],
            port_range: DEFAULT_PORT_RANGE,
            rendered_files: Vec::new(),
            models: BTreeMap::new(),
        }
    }
}

/// One enabled model, as recorded in `models.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnabledModel {
    /// Compose service name and DNS name.
    pub service_id: String,
    /// Tagless image reference.
    pub image: String,
    /// Image tag of the pinned version, e.g. `sha-fa880a1`.
    pub image_tag: String,
    pub version: String,
    /// `Some` when the pin follows a channel, `None` when it is exact.
    pub channel: Option<Channel>,
    pub host_port: u16,
    pub data_dir: String,
    /// `user:group` the container runs as.
    pub user: String,
    /// `Some("linux/amd64")` for R-INLA services.
    pub platform: Option<String>,
    /// Overlay file name, relative to the project directory.
    pub compose_file: String,
}

/// A project directory plus its parsed state.
#[derive(Debug, Clone)]
pub struct Project {
    pub dir: PathBuf,
    pub state: ProjectState,
}

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

        let project_body = format!("{PROJECT_HEADER}{}", serde_yaml_ng::to_string(&self.state)?);
        write_atomically(&chaps.join(PROJECT_FILE), &project_body)?;

        let models_body = format!(
            "{MODELS_HEADER}{}",
            serde_yaml_ng::to_string(&self.state.models)?
        );
        write_atomically(&chaps.join(MODELS_FILE), &models_body)
    }

    /// Absolute paths of the ordered `-f` list.
    pub fn compose_file_paths(&self) -> Vec<PathBuf> {
        self.state
            .compose_files
            .iter()
            .map(|f| self.dir.join(f))
            .collect()
    }

    /// Host ports already claimed by enabled models.
    pub fn used_ports(&self) -> BTreeSet<u16> {
        self.state.models.values().map(|m| m.host_port).collect()
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled(port: u16) -> EnabledModel {
        EnabledModel {
            service_id: "chapkit-ewars-model".into(),
            image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
            image_tag: "sha-fa880a1".into(),
            version: "1.0.0".into(),
            channel: Some(Channel::Stable),
            host_port: port,
            data_dir: "/app/data".into(),
            user: "chapkit:chapkit".into(),
            platform: Some("linux/amd64".into()),
            compose_file: "compose.chapkit-ewars-model.yml".into(),
        }
    }

    #[test]
    fn save_then_load_round_trips_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let state = ProjectState {
            chap_image_tag: "v1.2.3".into(),
            rendered_files: vec!["compose.marketplace.yml".into()],
            models: BTreeMap::from([("chapkit_ewars_model".to_string(), enabled(5001))]),
            ..ProjectState::default()
        };
        let project = Project {
            dir: dir.path().to_path_buf(),
            state,
        };
        project.save().unwrap();

        let chaps = dir.path().join(CHAPS_DIR);
        let project_body = std::fs::read_to_string(chaps.join(PROJECT_FILE)).unwrap();
        assert!(project_body.starts_with("# .chaps/project.yaml - managed by chaps"));
        assert!(project_body.contains("\nschema_version: 1\n"));
        assert!(project_body.contains("chap_image_tag: v1.2.3"));
        assert!(
            !project_body.contains("models:"),
            "models live in their own file"
        );
        let models_body = std::fs::read_to_string(chaps.join(MODELS_FILE)).unwrap();
        assert!(models_body.starts_with("# .chaps/models.yaml - managed by chaps"));
        assert!(models_body.contains("\nchapkit_ewars_model:\n"));
        assert!(models_body.contains("host_port: 5001"));
        assert!(models_body.contains("channel: stable"));
        assert!(
            std::fs::read_dir(&chaps).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")),
            "temp files are renamed away"
        );

        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.dir, dir.path());
        assert_eq!(loaded.state.chap_image_tag, "v1.2.3");
        assert_eq!(loaded.state.port_range, DEFAULT_PORT_RANGE);
        assert_eq!(loaded.state.rendered_files, vec!["compose.marketplace.yml"]);
        assert_eq!(loaded.state.models["chapkit_ewars_model"], enabled(5001));
    }

    #[test]
    fn a_missing_or_blank_models_file_means_no_models() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState::default(),
        };
        project.save().unwrap();
        let models = dir.path().join(CHAPS_DIR).join(MODELS_FILE);
        assert!(models.is_file());
        assert!(Project::load(dir.path()).unwrap().state.models.is_empty());

        std::fs::write(&models, "# nothing enabled\n\n").unwrap();
        assert!(Project::load(dir.path()).unwrap().state.models.is_empty());

        std::fs::remove_file(&models).unwrap();
        assert!(Project::load(dir.path()).unwrap().state.models.is_empty());
    }

    #[test]
    fn load_on_an_empty_dir_is_not_a_project() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!Project::exists(dir.path()));
        let err = Project::load(dir.path()).expect_err("empty dir is not a project");
        match err.downcast_ref::<ChapError>() {
            Some(ChapError::NotAProject(p)) => assert_eq!(p, dir.path()),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn find_walks_up_to_the_nearest_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("deploy");
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        Project {
            dir: root.clone(),
            state: ProjectState {
                chap_image_tag: "v9".into(),
                ..ProjectState::default()
            },
        }
        .save()
        .unwrap();

        for start in [&root, &root.join("a"), &nested] {
            let found = Project::find(start).unwrap();
            assert_eq!(found.dir, root, "from {}", start.display());
            assert_eq!(found.state.chap_image_tag, "v9");
        }
        assert_eq!(Project::find_root(&nested).as_deref(), Some(root.as_path()));

        // A closer project wins over a farther one.
        Project {
            dir: nested.clone(),
            state: ProjectState::default(),
        }
        .save()
        .unwrap();
        assert_eq!(Project::find(&nested).unwrap().dir, nested);
        assert_eq!(Project::find(&root.join("a")).unwrap().dir, root);
    }

    #[test]
    fn find_names_the_start_directory_when_nothing_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let start = dir.path().join("x").join("y");
        std::fs::create_dir_all(&start).unwrap();
        let err = Project::find(&start).expect_err("no project above a temp dir");
        match err.downcast_ref::<ChapError>() {
            Some(ChapError::NotAProject(p)) => assert_eq!(p, &start),
            other => panic!("wrong error: {other:?}"),
        }
        assert!(Project::find_root(&start).is_none());
    }

    #[test]
    fn exists_and_helpers_reflect_the_state() {
        let dir = tempfile::tempdir().unwrap();
        let state = ProjectState {
            models: BTreeMap::from([
                ("a".to_string(), enabled(5001)),
                ("b".to_string(), enabled(5004)),
            ]),
            ..ProjectState::default()
        };
        let project = Project {
            dir: dir.path().to_path_buf(),
            state,
        };
        project.save().unwrap();

        assert!(Project::exists(dir.path()));
        assert_eq!(project.used_ports(), BTreeSet::from([5001, 5004]));
        assert_eq!(
            project.compose_file_paths(),
            vec![
                dir.path().join(BASE_COMPOSE),
                dir.path().join(MARKETPLACE_COMPOSE),
            ]
        );
    }
}
