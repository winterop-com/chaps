//! The single state-edit path shared by `init`, `enable`, `disable` and the
//! TUI. It updates `.chaps/models.yaml` in memory and then hands over to
//! [`crate::compose::sync`], which renders the compose files and saves.

use crate::compose::overlay_filename;
use crate::compose::overrides::{DEFAULT_DATA_DIR, DEFAULT_USER, known_override};
use crate::compose::ports::PortAllocator;
use crate::compose::spec::OverlaySpec;
use crate::compose::sync::sync;
use crate::error::{ChapError, Result};
use crate::project::{EnabledModel, Project};
use crate::registry::{Registry, VersionSelector};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// One model the caller wants enabled, with any explicit overrides.
#[derive(Debug, Clone)]
pub struct EnableRequest {
    pub id: String,
    pub selector: VersionSelector,
    pub port: Option<u16>,
    pub data_dir: Option<String>,
    pub user: Option<String>,
    pub allow_template: bool,
}

impl EnableRequest {
    /// A request with no overrides, following the model's stable channel.
    pub fn new(id: impl Into<String>) -> EnableRequest {
        EnableRequest {
            id: id.into(),
            selector: VersionSelector::default(),
            port: None,
            data_dir: None,
            user: None,
            allow_template: false,
        }
    }
}

/// A batch of enables and disables applied together.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    pub enable: Vec<EnableRequest>,
    pub disable: Vec<String>,
}

impl Selection {
    /// Whether the selection would change anything at all.
    pub fn is_empty(&self) -> bool {
        self.enable.is_empty() && self.disable.is_empty()
    }
}

/// What [`apply`] changed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ApplyReport {
    pub enabled: Vec<(String, EnabledModel)>,
    pub updated: Vec<(String, EnabledModel)>,
    pub disabled: Vec<String>,
    pub written: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

impl ApplyReport {
    /// Whether anything changed on disk or in state.
    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
            && self.updated.is_empty()
            && self.disabled.is_empty()
            && self.written.is_empty()
            && self.removed.is_empty()
    }

    /// Every model the run touched, newly enabled first.
    pub fn touched(&self) -> impl Iterator<Item = &(String, EnabledModel)> {
        self.enabled.iter().chain(self.updated.iter())
    }
}

/// Apply a selection: disable first, then enable, then sync the compose
/// files and `.chaps/` from the new state.
///
/// Nothing is written until the whole selection has resolved, so an unknown
/// model or a port clash leaves the directory as it was.
pub fn apply(
    project: &mut Project,
    registry: &Registry,
    sel: &Selection,
    cli_version: &str,
) -> Result<ApplyReport> {
    let mut report = ApplyReport::default();
    let dir = project.dir.clone();

    // Disable first: a model can be removed and another put on its port in
    // the same call, and re-enabling one must see its own slot as free. The
    // overlay itself is removed by sync, which knows the file was ours.
    let mut freed: BTreeSet<u16> = BTreeSet::new();
    for wanted in &sel.disable {
        let id =
            enabled_id(project, wanted).ok_or_else(|| ChapError::UnknownModel(wanted.clone()))?;
        let entry = project
            .state
            .models
            .remove(&id)
            .expect("enabled_id only returns keys that are present");
        freed.insert(entry.host_port);
        report.disabled.push(id);
    }

    // Seed the allocator with what the project records plus what any compose
    // file in the directory already publishes, minus the ports just freed.
    let mut used: BTreeSet<u16> = project.used_ports();
    used.extend(PortAllocator::scan_compose_dir(&dir)?);
    for port in &freed {
        used.remove(port);
    }
    let mut allocator = PortAllocator::new(project.state.port_range, used);
    // A model being re-enabled keeps its own port: its overlay is about to be
    // rewritten, so the port it publishes today is not a conflict.
    for req in &sel.enable {
        if let Some(id) = enabled_id(project, &req.id)
            && let Some(existing) = project.state.models.get(&id)
        {
            allocator.release(existing.host_port);
        }
    }

    for req in &sel.enable {
        let model = registry
            .get(&req.id)
            .ok_or_else(|| ChapError::UnknownModel(req.id.clone()))?;
        if model.is_template() {
            if !req.allow_template {
                return Err(ChapError::IsTemplate(req.id.clone()).into());
            }
            report.warnings.push(format!(
                "{} is a template, not a deployable model; it is scaffolding to copy",
                model.id
            ));
        }
        let version = model.resolve(&req.selector)?;
        let existing = project.state.models.get(&model.id).cloned();

        let host_port = match req.port {
            Some(port) => {
                allocator.claim(port)?;
                port
            }
            None => match &existing {
                // Keep the port the model already had, even if a later
                // --port-base narrowed the range around it.
                Some(entry) => {
                    allocator.reserve(entry.host_port);
                    entry.host_port
                }
                None => allocator.allocate()?,
            },
        };

        let known = known_override(&model.id);
        let data_dir = req
            .data_dir
            .clone()
            .or_else(|| existing.as_ref().map(|e| e.data_dir.clone()))
            .or_else(|| known.map(|k| k.data_dir.to_string()))
            .unwrap_or_else(|| DEFAULT_DATA_DIR.to_string());
        let user = req
            .user
            .clone()
            .or_else(|| existing.as_ref().map(|e| e.user.clone()))
            .or_else(|| known.map(|k| k.user.to_string()))
            .unwrap_or_else(|| DEFAULT_USER.to_string());

        let spec = OverlaySpec::from_model(
            model,
            version,
            host_port,
            Some(&data_dir),
            Some(&user),
            cli_version,
        );
        let entry = EnabledModel {
            service_id: model.service_id.clone(),
            image: model.source.image.clone(),
            image_tag: version.image_tag.clone(),
            version: version.version.clone(),
            channel: match &req.selector {
                VersionSelector::Channel(c) => Some(*c),
                VersionSelector::Exact(_) => None,
            },
            host_port,
            data_dir,
            user,
            platform: spec.platform.clone(),
            compose_file: overlay_filename(&model.service_id),
        };
        if existing.is_some() {
            report.updated.push((model.id.clone(), entry.clone()));
        } else {
            report.enabled.push((model.id.clone(), entry.clone()));
        }
        project.state.models.insert(model.id.clone(), entry);
    }

    // One rendering path: sync writes the overlays, the umbrella and the .env
    // pins, removes the overlays of disabled models, and saves .chaps/.
    let synced = sync(project, registry, cli_version, false)?;
    report.written = synced.written;
    report.removed = synced.removed;
    report.warnings.extend(synced.warnings);
    Ok(report)
}

/// The state key for a marketplace id or a compose service id, when that
/// model is currently enabled.
fn enabled_id(project: &Project, wanted: &str) -> Option<String> {
    if project.state.models.contains_key(wanted) {
        return Some(wanted.to_string());
    }
    project
        .state
        .models
        .iter()
        .find(|(_, m)| m.service_id == wanted)
        .map(|(id, _)| id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::render::NO_TAG_PINS;
    use crate::project::{
        BASE_COMPOSE, DEFAULT_PORT_RANGE, ENV_FILE, MARKETPLACE_COMPOSE, ProjectState,
    };
    use crate::registry::{Channel, load_embedded};
    use serde_yaml_ng::Value;
    use tempfile::TempDir;

    const VERSION: &str = "0.1.0";

    fn project() -> (TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState::default(),
        };
        (dir, project)
    }

    fn enable(ids: &[&str]) -> Selection {
        Selection {
            enable: ids.iter().map(|id| EnableRequest::new(*id)).collect(),
            disable: Vec::new(),
        }
    }

    fn umbrella_includes(project: &Project) -> Vec<String> {
        let body = std::fs::read_to_string(project.dir.join(MARKETPLACE_COMPOSE)).unwrap();
        let doc: Value = serde_yaml_ng::from_str(&body).unwrap();
        doc.get("include")
            .and_then(Value::as_sequence)
            .map(|s| s.iter().map(|v| v.as_str().unwrap().to_string()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn apply_writes_the_overlay_the_umbrella_and_the_state() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        let report = apply(
            &mut project,
            &registry,
            &enable(&["chapkit_ewars_model"]),
            VERSION,
        )
        .unwrap();

        assert_eq!(report.enabled.len(), 1);
        assert!(report.updated.is_empty() && report.disabled.is_empty());
        let (id, entry) = &report.enabled[0];
        assert_eq!(id, "chapkit_ewars_model");
        assert_eq!(entry.host_port, 5001);
        assert_eq!(entry.channel, Some(Channel::Stable));
        assert_eq!(entry.image_tag, "sha-fa880a1");
        assert_eq!(entry.data_dir, "/app/data");
        assert_eq!(entry.user, "chapkit:chapkit");
        assert_eq!(entry.platform.as_deref(), Some("linux/amd64"));
        assert_eq!(entry.compose_file, "compose.chapkit-ewars-model.yml");

        assert!(dir.path().join("compose.chapkit-ewars-model.yml").is_file());
        assert_eq!(
            umbrella_includes(&project),
            vec!["compose.chapkit-ewars-model.yml"]
        );
        assert_eq!(
            project.state.compose_files,
            vec![BASE_COMPOSE.to_string(), MARKETPLACE_COMPOSE.to_string()]
        );

        // .chaps/ is written, not just held in memory.
        let reloaded = Project::load(dir.path()).unwrap();
        assert_eq!(reloaded.state.models["chapkit_ewars_model"], *entry);
    }

    #[test]
    fn a_second_model_takes_the_next_free_port() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        apply(
            &mut project,
            &registry,
            &enable(&["chapkit_ewars_model"]),
            VERSION,
        )
        .unwrap();
        let report = apply(
            &mut project,
            &registry,
            &enable(&["auto_arima_chapkit"]),
            VERSION,
        )
        .unwrap();
        assert_eq!(report.enabled[0].1.host_port, 5002);

        // The umbrella lists both overlays, sorted by marketplace id.
        assert_eq!(
            umbrella_includes(&project),
            vec![
                "compose.auto-arima-chapkit.yml",
                "compose.chapkit-ewars-model.yml",
            ]
        );
    }

    #[test]
    fn an_explicit_port_is_claimed_and_conflicts_are_rejected() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        let mut sel = enable(&["chapkit_ewars_model"]);
        sel.enable[0].port = Some(5100);
        apply(&mut project, &registry, &sel, VERSION).unwrap();
        assert_eq!(project.state.models["chapkit_ewars_model"].host_port, 5100);

        let mut clash = enable(&["auto_arima_chapkit"]);
        clash.enable[0].port = Some(5100);
        let err = apply(&mut project, &registry, &clash, VERSION).expect_err("5100 is taken");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::PortInUse(5100))
        ));

        let mut out_of_range = enable(&["auto_arima_chapkit"]);
        out_of_range.enable[0].port = Some(80);
        let err = apply(&mut project, &registry, &out_of_range, VERSION).expect_err("out of range");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::PortOutOfRange(80))
        ));
    }

    #[test]
    fn re_enabling_keeps_the_port_and_reports_an_update() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        apply(
            &mut project,
            &registry,
            &enable(&["chapkit_ewars_model"]),
            VERSION,
        )
        .unwrap();
        apply(
            &mut project,
            &registry,
            &enable(&["auto_arima_chapkit"]),
            VERSION,
        )
        .unwrap();

        let mut sel = enable(&["chapkit_ewars_model"]);
        sel.enable[0].selector = VersionSelector::Exact("1.0.0".into());
        sel.enable[0].user = Some("1000:1000".into());
        let report = apply(&mut project, &registry, &sel, VERSION).unwrap();
        assert!(report.enabled.is_empty());
        assert_eq!(report.updated.len(), 1);
        let entry = &report.updated[0].1;
        assert_eq!(entry.host_port, 5001, "the port is kept across a rewrite");
        assert_eq!(entry.user, "1000:1000");
        // An exact pin follows no channel.
        assert_eq!(entry.channel, None);
    }

    #[test]
    fn disable_removes_the_overlay_and_frees_its_port() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        apply(
            &mut project,
            &registry,
            &enable(&["chapkit_ewars_model", "auto_arima_chapkit"]),
            VERSION,
        )
        .unwrap();

        // The service id is accepted as well as the marketplace id.
        let sel = Selection {
            enable: Vec::new(),
            disable: vec!["chapkit-ewars-model".into()],
        };
        let report = apply(&mut project, &registry, &sel, VERSION).unwrap();
        assert_eq!(report.disabled, vec!["chapkit_ewars_model"]);
        assert_eq!(report.removed.len(), 1);
        assert!(!dir.path().join("compose.chapkit-ewars-model.yml").exists());
        assert_eq!(
            umbrella_includes(&project),
            vec!["compose.auto-arima-chapkit.yml"]
        );

        // 5001 is free again.
        let report = apply(
            &mut project,
            &registry,
            &enable(&["chapkit_simple_multistep_model"]),
            VERSION,
        )
        .unwrap();
        assert_eq!(report.enabled[0].1.host_port, 5001);
    }

    #[test]
    fn disabling_something_that_is_not_enabled_is_an_error() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        let sel = Selection {
            enable: Vec::new(),
            disable: vec!["auto_arima_chapkit".into()],
        };
        let err = apply(&mut project, &registry, &sel, VERSION).expect_err("not enabled");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownModel(id)) if id == "auto_arima_chapkit"
        ));
    }

    #[test]
    fn an_unknown_model_is_rejected_before_anything_is_written() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        let err = apply(&mut project, &registry, &enable(&["nope"]), VERSION).expect_err("unknown");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownModel(id)) if id == "nope"
        ));
        assert!(!dir.path().join(".chaps").exists());
        assert!(!dir.path().join(MARKETPLACE_COMPOSE).exists());
    }

    #[test]
    fn a_template_needs_allow_template_and_warns() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        let err = apply(
            &mut project,
            &registry,
            &enable(&["chapkit_minimalist_example_py"]),
            VERSION,
        )
        .expect_err("templates are not deployable");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::IsTemplate(_))
        ));

        let mut sel = enable(&["chapkit_minimalist_example_py"]);
        sel.enable[0].allow_template = true;
        let report = apply(&mut project, &registry, &sel, VERSION).unwrap();
        assert_eq!(report.enabled.len(), 1);
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("template"));
    }

    #[test]
    fn the_umbrella_is_written_even_with_no_models() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        let report = apply(&mut project, &registry, &Selection::default(), VERSION).unwrap();
        assert!(!report.is_empty(), "the umbrella was written");
        let body = std::fs::read_to_string(dir.path().join(MARKETPLACE_COMPOSE)).unwrap();
        assert!(body.contains("services: {}"));
        assert!(umbrella_includes(&project).is_empty());
    }

    #[test]
    fn ports_already_published_by_a_hand_written_overlay_are_skipped() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        std::fs::write(
            dir.path().join("compose.mine.yml"),
            "services:\n  mine:\n    ports:\n      - \"5001:8000\"\n",
        )
        .unwrap();
        let report = apply(
            &mut project,
            &registry,
            &enable(&["chapkit_ewars_model"]),
            VERSION,
        )
        .unwrap();
        assert_eq!(report.enabled[0].1.host_port, 5002);
    }

    #[test]
    fn env_pins_are_appended_once_and_nothing_else_is_touched() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        let env = dir.path().join(ENV_FILE);
        std::fs::write(
            &env,
            format!("POSTGRES_PASSWORD=secret\n# pins\n{NO_TAG_PINS}\n"),
        )
        .unwrap();

        apply(
            &mut project,
            &registry,
            &enable(&["chapkit_ewars_model"]),
            VERSION,
        )
        .unwrap();
        let body = std::fs::read_to_string(&env).unwrap();
        assert!(body.starts_with("POSTGRES_PASSWORD=secret\n"));
        assert!(body.contains("\n# CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-fa880a1\n"));

        // A second run adds nothing; only the new model's pin appears.
        apply(
            &mut project,
            &registry,
            &enable(&["auto_arima_chapkit"]),
            VERSION,
        )
        .unwrap();
        let body = std::fs::read_to_string(&env).unwrap();
        assert_eq!(body.matches("CHAPKIT_EWARS_MODEL_IMAGE_TAG").count(), 1);
        assert!(body.contains("\n# AUTO_ARIMA_CHAPKIT_IMAGE_TAG=sha-70c07a9\n"));
        assert!(
            !body.contains("none yet"),
            "the placeholder gives way to pins"
        );
    }

    #[test]
    fn without_an_env_file_nothing_is_created() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        apply(
            &mut project,
            &registry,
            &enable(&["chapkit_ewars_model"]),
            VERSION,
        )
        .unwrap();
        assert!(!dir.path().join(ENV_FILE).exists());
    }

    #[test]
    fn a_port_range_from_a_narrower_port_base_is_honoured() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        project.state.port_range = (5500, DEFAULT_PORT_RANGE.1);
        let report = apply(
            &mut project,
            &registry,
            &enable(&["chapkit_ewars_model"]),
            VERSION,
        )
        .unwrap();
        assert_eq!(report.enabled[0].1.host_port, 5500);
    }
}
