//! The single state-edit path shared by `init`, `enable`, `disable` and the
//! TUI. It updates `.chaps/models.yaml` in memory and then hands over to
//! [`crate::compose::sync`], which renders the compose files and saves.

use crate::components::{Component, MODELS_NEED_CHAP_CORE};
use crate::compose::overlay_filename;
use crate::compose::overrides::{DEFAULT_DATA_DIR, DEFAULT_USER, known_override};
use crate::compose::ports::allocator_for;
use crate::compose::resolve::{self, ResolveFn, UserSource};
use crate::compose::spec::OverlaySpec;
use crate::compose::sync::sync;
use crate::error::{ChapError, Result};
use crate::manual::Endpoints;
use crate::project::{EnabledModel, Project};
use crate::registry::{Registry, VersionSelector};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// What a caller wants done with one model's host port.
///
/// A model needs none to work, so the whole type is opt-in: it only appears
/// when someone typed `--port`, `expose` or pressed `p` in the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortRequest {
    /// Publish nothing: the overlay only `expose`s port 8000, chap-core
    /// reaches the model over the compose network, and a human goes through
    /// chap-core's proxy. This is what a fresh `models enable` does.
    #[default]
    None,
    /// Publish on the lowest port in the project's range that no compose file
    /// claims and nothing is listening on.
    Auto,
    /// Publish on exactly this port, or fail saying who has it.
    Fixed(u16),
}

/// One model the caller wants enabled, with any explicit overrides.
#[derive(Debug, Clone)]
pub struct EnableRequest {
    pub id: String,
    pub selector: VersionSelector,
    /// `None` leaves the model's host port exactly as it is - which for a
    /// model being enabled for the first time means none at all. `Some` is an
    /// explicit decision, [`PortRequest::None`] included.
    pub port: Option<PortRequest>,
    pub data_dir: Option<String>,
    pub user: Option<String>,
    /// Where [`user`] came from, for a caller that resolved it already.
    ///
    /// `chaps models add` reads the image itself and hands the answer over;
    /// leaving this `None` records the flag as [`UserSource::Flag`], which is
    /// what it is for everyone who typed `--user`.
    ///
    /// [`user`]: EnableRequest::user
    pub user_from: Option<UserSource>,
    pub allow_template: bool,
    /// Leave the version the project recorded exactly as it is, [`selector`]
    /// included.
    ///
    /// This is what a request that changes something *around* an enabled model
    /// carries: publishing a host port is no reason to move the pin a running
    /// deployment follows, and re-resolving the channel here would upgrade it
    /// as a side effect. Ignored for a model that is not enabled yet, which
    /// has no version to keep.
    ///
    /// [`selector`]: EnableRequest::selector
    pub keep_version: bool,
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
            user_from: None,
            allow_template: false,
            keep_version: false,
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

/// Check a whole selection against the registry and the project state,
/// changing nothing.
///
/// [`apply`] starts here, so every caller gets these answers before a file is
/// written. `chaps init --force` calls it directly as well: it deletes the
/// previous deployment's overlays before handing over to [`apply`], and a
/// selection that was never going to work must not take them with it.
pub fn validate(project: &Project, registry: &Registry, sel: &Selection) -> Result<()> {
    for wanted in &sel.disable {
        if enabled_id(project, wanted).is_none() {
            return Err(ChapError::UnknownModel(wanted.clone()).into());
        }
    }
    // A model service registers with chap-core and is reached through it, so a
    // deployment with models and no chap-core would start and do nothing.
    // `init` and `components disable` each say this in their own words, about
    // the flag or the component the caller named; this is the one that catches
    // `models enable` and the browser.
    if !sel.enable.is_empty() && !project.state.components.is_enabled(Component::ChapCore) {
        return Err(anyhow::anyhow!(MODELS_NEED_CHAP_CORE));
    }
    for req in &sel.enable {
        let model = registry
            .get(&req.id)
            .ok_or_else(|| ChapError::UnknownModel(req.id.clone()))?;
        if model.is_template() && !req.allow_template {
            return Err(ChapError::IsTemplate(req.id.clone()).into());
        }
        // A request that keeps the recorded version asks the registry for
        // nothing; every other one needs a version that exists and is not
        // yanked.
        if !(req.keep_version && project.state.models.contains_key(&model.id)) {
            model.resolve(&req.selector)?;
        }
    }
    Ok(())
}

/// Apply a selection: disable first, then enable, then sync the compose
/// files and `.chaps/` from the new state.
///
/// Nothing is written until the whole selection has resolved, so an unknown
/// model or a port clash leaves the directory as it was.
///
/// `endpoints` is where the user of a newly enabled marketplace model is read
/// from: the registry, or the local daemon, or - for an `--offline` run that
/// has neither - the table compiled into this binary. See
/// [`crate::compose::resolve`].
pub fn apply(
    project: &mut Project,
    registry: &Registry,
    sel: &Selection,
    endpoints: &Endpoints,
) -> Result<ApplyReport> {
    apply_with(project, registry, sel, &crate::ports::is_busy, &|req| {
        resolve::from_image(req, endpoints)
    })
}

/// [`apply`] with the host port probe and the user resolution injected, so
/// tests can decide what the machine is listening on without binding anything
/// and what an image declares without pulling one.
pub fn apply_with(
    project: &mut Project,
    registry: &Registry,
    sel: &Selection,
    busy: &dyn Fn(u16) -> bool,
    resolve: ResolveFn,
) -> Result<ApplyReport> {
    validate(project, registry, sel)?;
    let mut report = ApplyReport::default();

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
        freed.extend(entry.host_port);
        report.disabled.push(id);
    }

    // A model being re-enabled keeps its own port: its overlay is about to be
    // rewritten, so the port it publishes today is not a conflict.
    for req in &sel.enable {
        if let Some(id) = enabled_id(project, &req.id)
            && let Some(existing) = project.state.models.get(&id)
        {
            freed.extend(existing.host_port);
        }
    }
    let mut allocator = allocator_for(project, &freed)?;

    for req in &sel.enable {
        let model = registry
            .get(&req.id)
            .ok_or_else(|| ChapError::UnknownModel(req.id.clone()))?;
        // A template nobody allowed was refused by validate() above; what is
        // left is to say out loud that this one was allowed.
        if model.is_template() {
            report.warnings.push(format!(
                "{} is a template, not a deployable model; it is scaffolding to copy",
                model.id
            ));
        }
        let existing = project.state.models.get(&model.id).cloned();
        // What a `keep_version` request stands on: the entry as the project
        // recorded it, pin and all. A model that is not enabled yet has
        // nothing to keep, so it resolves like any other.
        let pinned = existing.clone().filter(|_| req.keep_version);

        let host_port = match req.port {
            // No decision made: keep whatever the model has, which for a new
            // one is no host port at all. A port it already had is kept even
            // if a later --port-base narrowed the range around it.
            None => {
                let kept = existing.as_ref().and_then(|e| e.host_port);
                if let Some(port) = kept {
                    allocator.reserve(port);
                }
                kept
            }
            Some(PortRequest::None) => None,
            Some(PortRequest::Auto) => Some(allocator.allocate(busy)?),
            Some(PortRequest::Fixed(port)) => {
                allocator.claim(port, busy)?;
                Some(port)
            }
        };

        let known = known_override(&model.id);
        // A manually added model carries its own answers: `models add` read
        // them off the image when the entry was written, and the built-in
        // table cannot have an entry for an image this CLI has never seen.
        let manual = project.state.manual.get(&model.id).cloned();
        // A marketplace model's user comes from the image itself, re-read on
        // every enable, because an image can change what it runs as between
        // two tags. A port-only change (`keep_version`) asks nothing: it is
        // not about the image, and re-resolving here would make publishing a
        // port a network operation.
        let resolved = match (&manual, &pinned) {
            (None, None) => {
                let version = model.resolve(&req.selector)?;
                Some(resolve(&resolve::Request {
                    id: &model.id,
                    image: &model.source.image,
                    image_tag: &version.image_tag,
                    data_dir_flag: req.data_dir.as_deref(),
                    user_flag: req.user.as_deref(),
                }))
            }
            _ => None,
        };
        let (data_dir, user, user_from) = match &resolved {
            Some(resolution) => {
                report.warnings.extend(resolution.notes.iter().cloned());
                (
                    resolution.data_dir.clone(),
                    resolution.user.clone(),
                    resolution.user_from,
                )
            }
            None => {
                let data_dir = req
                    .data_dir
                    .clone()
                    .or_else(|| existing.as_ref().map(|e| e.data_dir.clone()))
                    .or_else(|| manual.as_ref().and_then(|m| m.data_dir.clone()))
                    .or_else(|| known.map(|k| k.data_dir.to_string()))
                    .unwrap_or_else(|| DEFAULT_DATA_DIR.to_string());
                let user = req
                    .user
                    .clone()
                    .or_else(|| existing.as_ref().map(|e| e.user.clone()))
                    .or_else(|| manual.as_ref().and_then(|m| m.user.clone()))
                    .or_else(|| known.map(|k| k.user.to_string()))
                    .unwrap_or_else(|| DEFAULT_USER.to_string());
                let user_from = req
                    .user_from
                    .or_else(|| req.user.as_ref().map(|_| UserSource::Flag))
                    .or_else(|| existing.as_ref().map(|e| e.user_from))
                    .unwrap_or_default();
                (data_dir, user, user_from)
            }
        };

        let entry = match pinned {
            // A port change, and nothing about the image: the recorded pin,
            // the channel it follows and the platform all stay as they are.
            Some(previous) => EnabledModel {
                host_port,
                data_dir,
                user,
                user_from,
                ..previous
            },
            None => {
                let version = model.resolve(&req.selector)?;
                let spec = OverlaySpec::from_model(
                    model,
                    version,
                    host_port,
                    Some(&data_dir),
                    Some(&user),
                );
                EnabledModel {
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
                    user_from,
                    platform: spec.platform.clone(),
                    compose_file: overlay_filename(&model.service_id),
                }
            }
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
    let synced = sync(project, registry, false)?;
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
    use crate::error::PortHolder;
    use crate::project::{
        DEFAULT_PORT_RANGE, ENV_FILE, MARKETPLACE_COMPOSE, ProjectState, default_compose_files,
    };
    use crate::registry::{Channel, load_embedded};
    use serde_yaml_ng::Value;
    use tempfile::TempDir;

    /// Nothing is listening on this machine, as far as these tests care.
    fn all_free(_: u16) -> bool {
        false
    }

    /// [`apply`] with the host probe and the user lookup stubbed out, which is
    /// how every test here runs: either one would make the result depend on
    /// the machine - what it listens on, and what it has pulled.
    fn apply(project: &mut Project, registry: &Registry, sel: &Selection) -> Result<ApplyReport> {
        apply_with(project, registry, sel, &all_free, &resolve::from_table)
    }

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

    /// The same, asking for an automatically allocated host port.
    fn publish(ids: &[&str]) -> Selection {
        let mut sel = enable(ids);
        for req in &mut sel.enable {
            req.port = Some(PortRequest::Auto);
        }
        sel
    }

    /// The version and image tag the snapshot's `stable` channel points at,
    /// so the assertions below follow a marketplace refresh instead of
    /// pinning a sha that the next one invalidates.
    fn stable_pin(registry: &Registry, id: &str) -> (String, String) {
        let version = registry
            .get(id)
            .expect("vendored model")
            .resolve(&VersionSelector::Channel(Channel::Stable))
            .expect("stable resolves");
        (version.version.clone(), version.image_tag.clone())
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
        let report = apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();

        assert_eq!(report.enabled.len(), 1);
        assert!(report.updated.is_empty() && report.disabled.is_empty());
        let (id, entry) = &report.enabled[0];
        assert_eq!(id, "chapkit_ewars_model");
        assert_eq!(
            entry.host_port, None,
            "a model publishes no host port unless one was asked for"
        );
        assert_eq!(entry.channel, Some(Channel::Stable));
        assert_eq!(
            entry.image_tag,
            stable_pin(&registry, "chapkit_ewars_model").1
        );
        assert_eq!(entry.data_dir, "/app/data");
        assert_eq!(entry.user, "1000:1000");
        assert_eq!(entry.user_from, UserSource::Table, "the test resolver");
        assert_eq!(entry.platform.as_deref(), Some("linux/amd64"));
        assert_eq!(entry.compose_file, "compose.chapkit-ewars-model.yml");

        assert!(dir.path().join("compose.chapkit-ewars-model.yml").is_file());
        assert_eq!(
            umbrella_includes(&project),
            vec!["compose.chapkit-ewars-model.yml"]
        );
        assert_eq!(project.state.compose_files, default_compose_files());

        // .chaps/ is written, not just held in memory.
        let reloaded = Project::load(dir.path()).unwrap();
        assert_eq!(reloaded.state.models["chapkit_ewars_model"], *entry);
    }

    #[test]
    fn auto_hands_out_the_lowest_free_port_and_none_publishes_nothing() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        let report = apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
        assert_eq!(report.enabled[0].1.host_port, Some(5001));

        let report = apply(&mut project, &registry, &publish(&["auto_arima_chapkit"])).unwrap();
        assert_eq!(report.enabled[0].1.host_port, Some(5002));

        // A third one, left internal, takes no port at all.
        let report = apply(
            &mut project,
            &registry,
            &enable(&["chapkit_simple_multistep_model"]),
        )
        .unwrap();
        assert_eq!(report.enabled[0].1.host_port, None);
        assert_eq!(project.used_ports(), BTreeSet::from([5001, 5002]));

        // The umbrella lists every overlay, sorted by marketplace id.
        assert_eq!(
            umbrella_includes(&project),
            vec![
                "compose.auto-arima-chapkit.yml",
                "compose.chapkit-ewars-model.yml",
                "compose.chapkit-simple-multistep-model.yml",
            ]
        );
    }

    #[test]
    fn auto_skips_a_port_something_on_this_machine_is_listening_on() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        let busy = |port: u16| (5001..=5003).contains(&port);
        let report = apply_with(
            &mut project,
            &registry,
            &publish(&["chapkit_ewars_model"]),
            &busy,
            &resolve::from_table,
        )
        .unwrap();
        assert_eq!(report.enabled[0].1.host_port, Some(5004));
    }

    #[test]
    fn an_explicit_port_is_claimed_and_conflicts_say_who_holds_it() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        let mut sel = enable(&["chapkit_ewars_model"]);
        sel.enable[0].port = Some(PortRequest::Fixed(5100));
        apply(&mut project, &registry, &sel).unwrap();
        assert_eq!(
            project.state.models["chapkit_ewars_model"].host_port,
            Some(5100)
        );

        let mut clash = enable(&["auto_arima_chapkit"]);
        clash.enable[0].port = Some(PortRequest::Fixed(5100));
        let err = apply(&mut project, &registry, &clash).expect_err("5100 is taken");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::PortInUse {
                port: 5100,
                holder: PortHolder::ComposeFile
            })
        ));

        // A port nothing in the project claims, but that the machine does.
        let mut listening = enable(&["auto_arima_chapkit"]);
        listening.enable[0].port = Some(PortRequest::Fixed(5200));
        let err = apply_with(
            &mut project,
            &registry,
            &listening,
            &|port| port == 5200,
            &resolve::from_table,
        )
        .expect_err("something is listening on 5200");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::PortInUse {
                port: 5200,
                holder: PortHolder::Host
            })
        ));
        assert!(!project.state.models.contains_key("auto_arima_chapkit"));

        let mut out_of_range = enable(&["auto_arima_chapkit"]);
        out_of_range.enable[0].port = Some(PortRequest::Fixed(80));
        let err = apply(&mut project, &registry, &out_of_range).expect_err("out of range");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::PortOutOfRange(80))
        ));
    }

    #[test]
    fn re_enabling_keeps_the_port_and_reports_an_update() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
        apply(&mut project, &registry, &publish(&["auto_arima_chapkit"])).unwrap();

        let mut sel = enable(&["chapkit_ewars_model"]);
        sel.enable[0].selector =
            VersionSelector::Exact(stable_pin(&registry, "chapkit_ewars_model").0);
        sel.enable[0].user = Some("1000:1000".into());
        let report = apply(&mut project, &registry, &sel).unwrap();
        assert!(report.enabled.is_empty());
        assert_eq!(report.updated.len(), 1);
        let entry = &report.updated[0].1;
        assert_eq!(
            entry.host_port,
            Some(5001),
            "the port is kept across a rewrite"
        );
        assert_eq!(entry.user, "1000:1000");
        // An exact pin follows no channel.
        assert_eq!(entry.channel, None);

        // An explicit request is what takes the port away again, and frees it.
        let mut sel = enable(&["chapkit_ewars_model"]);
        sel.enable[0].port = Some(PortRequest::None);
        let report = apply(&mut project, &registry, &sel).unwrap();
        assert_eq!(report.updated[0].1.host_port, None);
        assert_eq!(project.used_ports(), BTreeSet::from([5002]));

        // And asking for it back lands on 5001 once more.
        let report = apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
        assert_eq!(report.updated[0].1.host_port, Some(5001));
    }

    #[test]
    fn disable_removes_the_overlay_and_frees_its_port() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        apply(
            &mut project,
            &registry,
            &publish(&["chapkit_ewars_model", "auto_arima_chapkit"]),
        )
        .unwrap();

        // The service id is accepted as well as the marketplace id.
        let sel = Selection {
            enable: Vec::new(),
            disable: vec!["chapkit-ewars-model".into()],
        };
        let report = apply(&mut project, &registry, &sel).unwrap();
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
            &publish(&["chapkit_simple_multistep_model"]),
        )
        .unwrap();
        assert_eq!(report.enabled[0].1.host_port, Some(5001));
    }

    #[test]
    fn disabling_something_that_is_not_enabled_is_an_error() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        let sel = Selection {
            enable: Vec::new(),
            disable: vec!["auto_arima_chapkit".into()],
        };
        let err = apply(&mut project, &registry, &sel).expect_err("not enabled");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownModel(id)) if id == "auto_arima_chapkit"
        ));
    }

    #[test]
    fn an_unknown_model_is_rejected_before_anything_is_written() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        let err = apply(&mut project, &registry, &enable(&["nope"])).expect_err("unknown");
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
        )
        .expect_err("templates are not deployable");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::IsTemplate(_))
        ));

        let mut sel = enable(&["chapkit_minimalist_example_py"]);
        sel.enable[0].allow_template = true;
        let report = apply(&mut project, &registry, &sel).unwrap();
        assert_eq!(report.enabled.len(), 1);
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("template"));
    }

    /// A model service registers with chap-core and is reached through it, so
    /// a model in a deployment that has no chap-core would start and talk to
    /// nothing. `init` and `components disable` guard their own side of this;
    /// the shared path is what catches `models enable` and the browser.
    #[test]
    fn a_model_cannot_be_enabled_without_the_chap_core_component() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        project
            .state
            .components
            .set_enabled(Component::ChapCore, false);

        let err = apply(&mut project, &registry, &enable(&["chapkit_ewars_model"]))
            .expect_err("a model has nowhere to register");
        assert_eq!(
            err.to_string(),
            "models need the chap-core component; run `chaps components enable chap-core`"
        );
        assert!(project.state.models.is_empty());
        assert!(!dir.path().join(".chaps").exists());
        assert!(!dir.path().join(MARKETPLACE_COMPOSE).exists());

        // A selection that enables no model is not about models at all.
        apply(&mut project, &registry, &Selection::default()).unwrap();
    }

    /// Publishing a host port is no reason to move the version a deployment
    /// runs: that is what the browser's port-only change carries
    /// `keep_version` for.
    #[test]
    fn keep_version_leaves_the_recorded_pin_exactly_where_it_is() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();

        // A deployment running a version pinned exactly, which is what the
        // marketplace moving on looks like from here.
        let entry = project
            .state
            .models
            .get_mut("chapkit_ewars_model")
            .expect("just enabled");
        entry.version = "0.9.0".to_string();
        entry.image_tag = "sha-older".to_string();
        entry.channel = None;

        let mut sel = publish(&["chapkit_ewars_model"]);
        sel.enable[0].keep_version = true;
        let report = apply(&mut project, &registry, &sel).unwrap();
        let entry = &report.updated[0].1;
        assert_eq!(entry.host_port, Some(5001), "the port change went through");
        assert_eq!(entry.version, "0.9.0");
        assert_eq!(entry.image_tag, "sha-older");
        assert_eq!(entry.channel, None, "an exact pin follows no channel");
        let overlay = std::fs::read_to_string(dir.path().join("compose.chapkit-ewars-model.yml"))
            .expect("the overlay was rewritten");
        assert!(overlay.contains("sha-older"), "{overlay}");

        // The same request without it is what asks the registry again.
        let report = apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
        let entry = &report.updated[0].1;
        let (version, tag) = stable_pin(&registry, "chapkit_ewars_model");
        assert_eq!(entry.version, version);
        assert_eq!(entry.image_tag, tag);
        assert_eq!(entry.channel, Some(Channel::Stable));

        // A model that is not enabled has no version to keep, so it resolves.
        let mut sel = enable(&["auto_arima_chapkit"]);
        sel.enable[0].keep_version = true;
        let report = apply(&mut project, &registry, &sel).unwrap();
        assert_eq!(
            report.enabled[0].1.version,
            stable_pin(&registry, "auto_arima_chapkit").0
        );
    }

    /// `init --force` deletes the previous deployment's overlays before
    /// handing over to `apply`, so it asks first. Every answer has to be
    /// available while all of it is still on disk.
    #[test]
    fn validate_answers_for_the_whole_selection_and_touches_nothing() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();

        let err = validate(
            &project,
            &registry,
            &enable(&["chapkit_minimalist_example_py"]),
        )
        .expect_err("templates are not deployable");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::IsTemplate(_))
        ));
        let err = validate(&project, &registry, &enable(&["nope"])).expect_err("unknown");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownModel(id)) if id == "nope"
        ));
        let sel = Selection {
            enable: Vec::new(),
            disable: vec!["auto_arima_chapkit".into()],
        };
        let err = validate(&project, &registry, &sel).expect_err("not enabled");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownModel(_))
        ));
        let mut sel = enable(&["chapkit_ewars_model"]);
        sel.enable[0].selector = VersionSelector::Exact("9.9.9".into());
        let err = validate(&project, &registry, &sel).expect_err("no such version");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownVersion { .. })
        ));

        // A selection that would apply says nothing, and none of these
        // questions wrote anything.
        validate(&project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();
        assert!(!dir.path().join(".chaps").exists());
        assert!(!dir.path().join(MARKETPLACE_COMPOSE).exists());
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());

        // And what it allows, apply() goes on to do.
        apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();
    }

    #[test]
    fn the_umbrella_is_written_even_with_no_models() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        let report = apply(&mut project, &registry, &Selection::default()).unwrap();
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
        let report = apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
        assert_eq!(report.enabled[0].1.host_port, Some(5002));
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

        apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();
        let body = std::fs::read_to_string(&env).unwrap();
        assert!(body.starts_with("POSTGRES_PASSWORD=secret\n"));
        assert!(body.contains(&format!(
            "\n# CHAPKIT_EWARS_MODEL_IMAGE_TAG={}\n",
            stable_pin(&registry, "chapkit_ewars_model").1
        )));

        // A second run adds nothing; only the new model's pin appears.
        apply(&mut project, &registry, &enable(&["auto_arima_chapkit"])).unwrap();
        let body = std::fs::read_to_string(&env).unwrap();
        assert_eq!(body.matches("CHAPKIT_EWARS_MODEL_IMAGE_TAG").count(), 1);
        assert!(body.contains(&format!(
            "\n# AUTO_ARIMA_CHAPKIT_IMAGE_TAG={}\n",
            stable_pin(&registry, "auto_arima_chapkit").1
        )));
        assert!(
            !body.contains("none yet"),
            "the placeholder gives way to pins"
        );
    }

    #[test]
    fn without_an_env_file_nothing_is_created() {
        let registry = load_embedded().unwrap();
        let (dir, mut project) = project();
        apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();
        assert!(!dir.path().join(ENV_FILE).exists());
    }

    #[test]
    fn a_port_range_from_a_narrower_port_base_is_honoured() {
        let registry = load_embedded().unwrap();
        let (_dir, mut project) = project();
        project.state.port_range = (5500, DEFAULT_PORT_RANGE.1);
        let report = apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
        assert_eq!(report.enabled[0].1.host_port, Some(5500));
    }
}
