//! `chaps.json` — the state file that describes a generated deployment.

use crate::error::{ChapError, Result};
use crate::registry::Channel;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// State file name, at the root of a project directory.
pub const STATE_FILE: &str = "chaps.json";
/// Base compose file: chap-core, worker, valkey, postgres.
pub const BASE_COMPOSE: &str = "compose.yml";
/// Umbrella file that `include:`s one overlay per enabled model.
pub const MARKETPLACE_COMPOSE: &str = "compose.marketplace.yml";
/// Environment file docker compose picks up automatically.
pub const ENV_FILE: &str = ".env";

/// `chaps.json` schema version written by this CLI.
pub const SCHEMA_VERSION: u32 = 1;
/// Host port range model overlays are allocated from.
pub const DEFAULT_PORT_RANGE: (u16, u16) = (5001, 5999);

/// The contents of `chaps.json`.
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
    /// Enabled models, keyed by marketplace `id`.
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
            models: BTreeMap::new(),
        }
    }
}

/// One enabled model, as recorded in `chaps.json`.
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
    /// Whether `dir` holds a `chaps.json`.
    pub fn exists(dir: &Path) -> bool {
        dir.join(STATE_FILE).is_file()
    }

    /// Read `dir/chaps.json`.
    ///
    /// Errors with [`ChapError::NotAProject`] when the state file is absent.
    pub fn load(dir: &Path) -> Result<Project> {
        let path = dir.join(STATE_FILE);
        let body = match std::fs::read_to_string(&path) {
            Ok(body) => body,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(ChapError::NotAProject(dir.to_path_buf()).into());
            }
            Err(e) => {
                return Err(anyhow::Error::new(e).context(format!("reading {}", path.display())));
            }
        };
        let state: ProjectState = serde_json::from_str(&body)
            .map_err(|e| anyhow::anyhow!("{}: invalid chaps.json: {e}", path.display()))?;
        Ok(Project {
            dir: dir.to_path_buf(),
            state,
        })
    }

    /// Write `chaps.json` as pretty JSON with a trailing newline.
    ///
    /// Written to a temporary file in the same directory and renamed, so a
    /// crash never leaves a half-written state file behind.
    pub fn save(&self) -> Result<()> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", self.dir.display()))?;
        let path = self.dir.join(STATE_FILE);
        let tmp = self.dir.join(format!(".{STATE_FILE}.tmp"));
        let mut body = serde_json::to_string_pretty(&self.state)?;
        body.push('\n');
        std::fs::write(&tmp, body)
            .map_err(|e| anyhow::anyhow!("writing {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            anyhow::anyhow!("renaming {} to {}: {e}", tmp.display(), path.display())
        })?;
        Ok(())
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
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let state = ProjectState {
            chap_image_tag: "v1.2.3".into(),
            models: BTreeMap::from([("chapkit_ewars_model".to_string(), enabled(5001))]),
            ..ProjectState::default()
        };
        let project = Project {
            dir: dir.path().to_path_buf(),
            state,
        };
        project.save().unwrap();

        let body = std::fs::read_to_string(dir.path().join(STATE_FILE)).unwrap();
        assert!(body.ends_with("}\n"), "trailing newline");
        assert!(body.contains("\n  \"schema_version\""), "pretty printed");
        assert!(
            !dir.path().join(format!(".{STATE_FILE}.tmp")).exists(),
            "temp file is renamed away"
        );

        let loaded = Project::load(dir.path()).unwrap();
        assert_eq!(loaded.dir, dir.path());
        assert_eq!(loaded.state.chap_image_tag, "v1.2.3");
        assert_eq!(loaded.state.port_range, DEFAULT_PORT_RANGE);
        assert_eq!(loaded.state.models["chapkit_ewars_model"], enabled(5001));
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
