//! `chaps models enable|disable` — the two single-model write commands.
//!
//! Both go through [`crate::compose::apply()`], the same path `init` and the
//! TUI use.

use crate::cli::{ModelsDisableArgs, ModelsEnableArgs, ModelsExposeArgs, ModelsUnexposeArgs};
use crate::commands::Ctx;
use crate::compose::ports::allocator_for;
use crate::compose::sync::sync;
use crate::compose::{ApplyReport, EnableRequest, PortRequest, Selection, apply};
use crate::error::{ChapError, Result};
use crate::output::Out;
use crate::project::Project;
use crate::registry::{Channel, Registry, VersionSelector};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Enable one model: resolve its version, write the overlay, sync the compose
/// files and update .chaps/models.yaml.
///
/// No host port unless `--port` asks for one: chap-core reaches the service
/// over the compose network, which is the URL the service registers.
pub fn enable(ctx: &Ctx, args: &ModelsEnableArgs) -> Result<()> {
    let mut project = ctx.project()?;
    let registry = super::registry_for(ctx, Some(&project))?;

    // --version and --channel conflict in the parser, so at most one is set.
    let selector = match (&args.version, args.channel) {
        (Some(version), _) => VersionSelector::Exact(version.clone()),
        (None, Some(channel)) => VersionSelector::Channel(channel),
        (None, None) => VersionSelector::Channel(Channel::Stable),
    };
    let selection = Selection {
        enable: vec![EnableRequest {
            id: args.id.clone(),
            selector,
            port: args.port.map(|p| p.0),
            data_dir: args.data_dir.clone(),
            user: args.user.clone(),
            user_from: None,
            allow_template: args.allow_template,
            // `models enable` is where a version is decided, so it always
            // resolves: that is what `--version` and `--channel` are for, and
            // a bare run follows the stable channel.
            keep_version: false,
        }],
        ..Selection::default()
    };

    let endpoints = crate::manual::Endpoints::from_env(ctx.registry.offline);
    let report = apply(&mut project, &registry, &selection, &endpoints)?;
    ctx.out
        .emit(&report, || summary(&report, &[], &project, &ctx.out))
}

/// Disable one model: stop its container, remove its overlay, regenerate the
/// umbrella file and update .chaps/models.yaml.
///
/// The model's data volume is kept, which is what makes disabling a model a
/// reversible thing to do. It is named either way, because once the overlay
/// is gone nothing else knows it: `chaps down --volumes` removes the volumes
/// the compose files still declare, so a volume nobody names again
/// lingers for as long as the machine does. `--purge` is how the data goes
/// with the model.
pub fn disable(ctx: &Ctx, args: &ModelsDisableArgs) -> Result<()> {
    let mut project = ctx.project()?;
    let registry = super::registry_for(ctx, Some(&project))?;

    // Accept either identifier, but only for a model this project enabled:
    // disabling something that was never on is a typo, not a no-op. With
    // `--purge` it is neither, because the volume outlives the model it
    // belonged to and removing it is what the flag is for.
    let Some(id) = enabled_id(&project, &args.id) else {
        if args.purge {
            return purge_only(ctx, &project, &registry, &args.id);
        }
        return Err(ChapError::UnknownModel(args.id.clone()).into());
    };
    let (report, notes) = disable_enabled(&mut project, &registry, &id, args.purge)?;
    ctx.out.emit(&report, || {
        summary(&report.apply, &notes, &project, &ctx.out)
    })
}

/// Disable a model this project has enabled: stop its container, remove its
/// overlay and take it out of `.chaps/models.yaml`, then deal with its data
/// volume.
///
/// The half of [`disable`] that `chaps models remove` needs too: removing a
/// manually added model has to disable it first, and doing that by any other
/// path would leave the container running and the volume unnamed. Returns
/// what was done plus the notes that belong in the closing lines.
pub(crate) fn disable_enabled(
    project: &mut Project,
    registry: &Registry,
    id: &str,
    purge: bool,
) -> Result<(DisableReport, Vec<String>)> {
    let id = id.to_string();
    let service_id = project.state.models[&id].service_id.clone();
    let selection = Selection {
        disable: vec![id.clone()],
        ..Selection::default()
    };
    // Nothing is written before this passes, so the container is only touched
    // for a disable that is going through.
    crate::compose::apply::validate(project, registry, &selection)?;

    // The container goes now, while compose still has the overlay that
    // defines it: a service whose definition has just been deleted cannot be
    // stopped by name, and one left running keeps its host port published
    // long after the model was disabled.
    let stopped = super::docker::stop_and_remove(project, &|service| {
        // The model's own container, and the exited one-shot companion that
        // handed its volume over before it started.
        service.strip_suffix("-init").unwrap_or(service) == service_id
    });

    // And the volume after them: docker refuses to remove one a container
    // still has mounted, so the order here is the difference between a purge
    // that works and one that reports "volume is in use".
    let mut notes: Vec<String> = stopped.iter().cloned().collect();
    let volume = project.prefixed_volume(&crate::compose::volume_name(&id));
    let mut purged = Vec::new();
    let mut kept_volumes = Vec::new();
    match (&volume, purge) {
        (Some(name), true) => {
            let (removed, line) = super::docker::purge_volume(name);
            purged.extend(removed);
            notes.push(line);
        }
        (Some(name), false) => {
            notes.push(super::docker::kept_volume_line(
                name,
                &format!("chaps models disable {id}"),
            ));
            kept_volumes.push(name.clone());
        }
        (None, true) => notes.push(super::docker::UNNAMEABLE_VOLUME.to_string()),
        (None, false) => {}
    }

    let report = DisableReport {
        // A disable enables nothing, so nothing here is ever looked up; the
        // offline endpoints say so rather than leaving it to chance.
        apply: apply(project, registry, &selection, &offline_endpoints())?,
        stopped,
        purged,
        kept_volumes,
    };
    Ok((report, notes))
}

/// `--purge` for a model this project does not have enabled.
///
/// The volume is all that is left of such a model, so removing it is the whole
/// command. Refusing instead would put the one thing `--purge` exists for out
/// of reach: the line a plain `disable` closes with names this very command,
/// and by then the model is already gone from `.chaps/models.yaml`.
///
/// A name the marketplace does not list and that no volume answers to either
/// is still the typo it would be without the flag.
fn purge_only(ctx: &Ctx, project: &Project, registry: &Registry, wanted: &str) -> Result<()> {
    let listed = registry.get(wanted);
    let id = purge_id(listed.map(|model| model.id.as_str()), wanted);
    let volume = project.prefixed_volume(&crate::compose::volume_name(&id));
    if listed.is_none() && !volume.as_deref().is_some_and(crate::docker::volume_exists) {
        return Err(ChapError::UnknownModel(wanted.to_string()).into());
    }

    let mut notes = vec![format!(
        "{id} is not enabled here, so only its data volume was looked for"
    )];
    let mut purged = Vec::new();
    match volume {
        Some(name) => {
            let (removed, line) = super::docker::purge_volume(&name);
            purged.extend(removed);
            notes.push(line);
        }
        None => notes.push(super::docker::UNNAMEABLE_VOLUME.to_string()),
    }
    let report = DisableReport {
        apply: ApplyReport::default(),
        stopped: None,
        purged,
        kept_volumes: Vec::new(),
    };
    ctx.out.emit(&report, || purge_summary(&notes, &ctx.out))
}

/// The marketplace id a `--purge` on a model that is not enabled is about.
///
/// `listed` is the id the marketplace holds for whichever identifier was
/// typed, which is what every other command accepts. For an entry the
/// marketplace no longer lists there is nothing to ask: a service id is the
/// marketplace id with its underscores written as hyphens, so folding them
/// back is the one guess worth making, and the volume of that name either
/// exists or the run says the model is unknown.
fn purge_id(listed: Option<&str>, wanted: &str) -> String {
    match listed {
        Some(id) => id.to_string(),
        None => wanted.replace('-', "_"),
    }
}

/// What `models disable` did: the state edit, what became of the container
/// that was running the model, and what became of its data volume.
#[derive(Debug, serde::Serialize)]
pub(crate) struct DisableReport {
    #[serde(flatten)]
    pub(crate) apply: ApplyReport,
    /// What was done about the container, when there was one to do anything
    /// about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stopped: Option<String>,
    /// Data volumes this run removed: the model's own, when `--purge` asked
    /// for it and it was there to remove.
    pub(crate) purged: Vec<String>,
    /// Data volumes it left in place, which is what a disable without
    /// `--purge` does.
    pub(crate) kept_volumes: Vec<String>,
}

/// Publish a host port for a model that is already enabled.
pub fn expose(ctx: &Ctx, args: &ModelsExposeArgs) -> Result<()> {
    // Omitting --port means "any free one": someone who wanted a specific
    // number would have said so.
    let request = args.port.map(|p| p.0).unwrap_or(PortRequest::Auto);
    set_host_port(ctx, &args.id, request)
}

/// Take an enabled model's host port away again.
pub fn unexpose(ctx: &Ctx, args: &ModelsUnexposeArgs) -> Result<()> {
    set_host_port(ctx, &args.id, PortRequest::None)
}

/// What `expose` and `unexpose` did, for `--json`.
#[derive(Debug, serde::Serialize)]
struct PortChange {
    /// Marketplace id, whichever identifier the caller typed.
    id: String,
    service_id: String,
    /// The port the model publishes now; `null` for an internal-only service.
    host_port: Option<u16>,
    /// What it published before.
    previous: Option<u16>,
    /// How to reach it from this machine now.
    url: String,
    written: Vec<PathBuf>,
}

/// Move one enabled model's host port, then re-render the compose files.
///
/// Deliberately not a [`Selection`]: this command is about the port and
/// nothing else, and its report says so in its own words. A selection can ask
/// for the same thing with `keep_version`, which is what the browser's
/// port-only change carries, but it would still have to resolve the rest of
/// the request it is not making.
fn set_host_port(ctx: &Ctx, wanted: &str, request: PortRequest) -> Result<()> {
    let mut project = ctx.project()?;
    let registry = super::registry_for(ctx, Some(&project))?;
    let id = enabled_id(&project, wanted).ok_or_else(|| ChapError::UnknownModel(wanted.into()))?;
    let previous = project.state.models[&id].host_port;

    // The model's own port is not a conflict with itself: it is about to be
    // replaced, and re-claiming it has to succeed.
    let freed: BTreeSet<u16> = previous.into_iter().collect();
    let host_port = match request {
        PortRequest::None => None,
        PortRequest::Auto => {
            let mut allocator = allocator_for(&project, &freed)?;
            Some(allocator.allocate(&crate::ports::is_busy)?)
        }
        PortRequest::Fixed(port) => {
            let mut allocator = allocator_for(&project, &freed)?;
            allocator.claim(port, &crate::ports::is_busy)?;
            Some(port)
        }
    };

    let entry = project
        .state
        .models
        .get_mut(&id)
        .expect("enabled_id only returns keys that are present");
    entry.host_port = host_port;
    let service_id = entry.service_id.clone();

    let synced = sync(&mut project, &registry, false)?;
    let change = PortChange {
        id,
        url: match host_port {
            Some(port) => format!("http://localhost:{port}"),
            None => project.proxy_url(&service_id),
        },
        service_id,
        host_port,
        previous,
        written: synced.written,
    };
    ctx.out.emit(&change, || {
        port_summary(&change, &project, &synced.warnings, &ctx.out)
    })
}

/// The human rendering of one port change.
fn port_summary(change: &PortChange, project: &Project, warnings: &[String], out: &Out) -> String {
    let mut text = match change.host_port {
        Some(port) => format!(
            "{} {} on {}\n",
            out.ok("exposed"),
            change.service_id,
            out.value(&format!("http://localhost:{port}"))
        ),
        None => format!(
            "{} {}; it stays registered with chap-core and reachable at {}\n",
            out.warn("unexposed"),
            change.service_id,
            out.value(&change.url)
        ),
    };
    if change.host_port == change.previous {
        text.push_str(&out.dim("(that is what it published already)"));
        text.push('\n');
    }
    for path in &change.written {
        text.push_str(&format!(
            "{}  {}\n",
            out.ok("written"),
            out.dim(
                &path
                    .strip_prefix(&project.dir)
                    .unwrap_or(path)
                    .display()
                    .to_string()
            )
        ));
    }
    for warning in warnings {
        text.push_str(&format!("{} {warning}\n", out.warn("warning:")));
    }
    text.push_str(&out.backticks("run `chaps up` to apply"));
    text
}

/// Endpoints for a call that enables nothing and therefore looks nothing up.
fn offline_endpoints() -> crate::manual::Endpoints {
    crate::manual::Endpoints {
        offline: true,
        docker_probe: false,
        ..crate::manual::Endpoints::default()
    }
}

/// The state key for a marketplace id or a compose service id.
pub(crate) fn enabled_id(project: &Project, wanted: &str) -> Option<String> {
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

/// The human rendering of one enable or disable.
///
/// `notes` is what `disable` did beyond the state edit: the container that
/// was running the model, and the data volume it kept or removed. They belong
/// in the closing lines, next to the files that were removed.
pub(crate) fn summary(
    report: &ApplyReport,
    notes: &[String],
    project: &Project,
    out: &Out,
) -> String {
    let mut text = String::new();
    for (id, model) in report.touched() {
        let verb = if report.enabled.iter().any(|(e, _)| e == id) {
            "enabled"
        } else {
            "updated"
        };
        text.push_str(&format!(
            "{} {id} {} {} {}\n",
            out.ok(verb),
            // A manually added model's version is its image tag, so `v` in
            // front of it would read as a version number it does not have.
            out.dim(&match model.version == model.image_tag {
                true => model.image_tag.clone(),
                false => format!("v{}", model.version),
            }),
            match model.host_port {
                Some(port) => format!("on {}", out.value(&format!("http://localhost:{port}"))),
                None => format!("at {}", out.value(&project.proxy_url(&model.service_id))),
            },
            out.dim(&format!("({})", model.compose_file))
        ));
    }
    for id in &report.disabled {
        text.push_str(&format!("{} {id}\n", out.warn("disabled")));
    }
    for path in &report.removed {
        text.push_str(&format!(
            "{} {}\n",
            out.bad("removed"),
            out.dim(
                &path
                    .strip_prefix(&project.dir)
                    .unwrap_or(path)
                    .display()
                    .to_string()
            )
        ));
    }
    for warning in &report.warnings {
        text.push_str(&format!("{} {warning}\n", out.warn("warning:")));
    }
    for note in notes {
        text.push_str(&format!("{} {}\n", out.dim("note:"), out.backticks(note)));
    }
    text.push_str(&out.backticks("run `chaps up` to apply"));
    text
}

/// The human rendering of a `--purge` that had only a volume to remove.
///
/// No closing `chaps up`: nothing was written, so there is nothing to apply.
fn purge_summary(notes: &[String], out: &Out) -> String {
    notes
        .iter()
        .map(|note| format!("{} {}\n", out.dim("note:"), out.backticks(note)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{EnabledModel, ProjectState};
    use std::collections::BTreeMap;

    fn project_with_ewars(host_port: Option<u16>) -> Project {
        let model = EnabledModel {
            service_id: "chapkit-ewars-model".into(),
            image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
            image_tag: "sha-fa880a1".into(),
            version: "1.0.0".into(),
            channel: Some(Channel::Stable),
            host_port,
            data_dir: "/app/data".into(),
            user: "chapkit:chapkit".into(),
            user_from: Default::default(),
            platform: Some("linux/amd64".into()),
            compose_file: "compose.chapkit-ewars-model.yml".into(),
        };
        Project {
            dir: std::path::PathBuf::from("/tmp/chapx"),
            state: ProjectState {
                models: BTreeMap::from([("chapkit_ewars_model".to_string(), model)]),
                ..ProjectState::default()
            },
        }
    }

    /// What `--purge` calls a model that `.chaps/models.yaml` no longer holds,
    /// and therefore which volume it removes.
    #[test]
    fn a_purge_names_the_volume_by_the_marketplace_id() {
        let registry = crate::registry::load_embedded().expect("the embedded snapshot parses");
        for typed in ["chapkit_ewars_model", "chapkit-ewars-model"] {
            let listed = registry.get(typed).map(|model| model.id.as_str());
            assert_eq!(purge_id(listed, typed), "chapkit_ewars_model");
            assert_eq!(
                crate::compose::volume_name(&purge_id(listed, typed)),
                "ck_chapkit_ewars_model_data"
            );
        }

        // An entry the marketplace no longer lists is only the name that was
        // typed: a service id folds back to the id that named the volume, and
        // a marketplace id is already it.
        assert_eq!(purge_id(None, "left-the-market"), "left_the_market");
        assert_eq!(purge_id(None, "left_the_market"), "left_the_market");
    }

    #[test]
    fn enabled_id_accepts_both_identifiers() {
        let project = project_with_ewars(None);
        assert_eq!(
            enabled_id(&project, "chapkit_ewars_model").as_deref(),
            Some("chapkit_ewars_model")
        );
        assert_eq!(
            enabled_id(&project, "chapkit-ewars-model").as_deref(),
            Some("chapkit_ewars_model")
        );
        assert_eq!(enabled_id(&project, "auto_arima_chapkit"), None);
    }

    /// An `enabled` report for the one model `project_with_ewars` holds.
    fn enabled_report(project: &Project) -> ApplyReport {
        ApplyReport {
            enabled: vec![(
                "chapkit_ewars_model".to_string(),
                project.state.models["chapkit_ewars_model"].clone(),
            )],
            ..ApplyReport::default()
        }
    }

    #[test]
    fn the_summary_ends_with_the_next_step() {
        let project = project_with_ewars(Some(5001));
        let text = summary(&enabled_report(&project), &[], &project, &Out::default());
        assert!(text.contains("enabled chapkit_ewars_model v1.0.0 on http://localhost:5001"));
        assert!(text.ends_with("run `chaps up` to apply"));
    }

    #[test]
    fn a_model_with_no_host_port_is_summarised_with_the_proxy_url() {
        let project = project_with_ewars(None);
        let text = summary(&enabled_report(&project), &[], &project, &Out::default());
        assert!(
            text.contains(
                "enabled chapkit_ewars_model v1.0.0 at \
                 http://localhost:8000/v2/services/chapkit-ewars-model/run/"
            ),
            "{text}"
        );
    }

    /// A manually added model's version is its image tag; printing `v` in
    /// front of it would claim a version number it does not have.
    #[test]
    fn a_manual_models_line_names_its_tag_rather_than_a_version() {
        let mut project = project_with_ewars(None);
        let entry = project
            .state
            .models
            .get_mut("chapkit_ewars_model")
            .expect("just built");
        entry.version = "sha-b1d6c31".into();
        entry.image_tag = "sha-b1d6c31".into();
        let text = summary(&enabled_report(&project), &[], &project, &Out::default());
        assert!(
            text.contains("enabled chapkit_ewars_model sha-b1d6c31 at "),
            "{text}"
        );
        assert!(!text.contains("vsha-"), "{text}");
    }

    #[test]
    fn a_disable_summary_names_the_removed_file() {
        let project = project_with_ewars(Some(5001));
        let report = ApplyReport {
            disabled: vec!["chapkit_ewars_model".to_string()],
            removed: vec![project.dir.join("compose.chapkit-ewars-model.yml")],
            ..ApplyReport::default()
        };
        let text = summary(&report, &[], &project, &Out::default());
        assert!(text.contains("disabled chapkit_ewars_model"));
        assert!(text.contains("removed compose.chapkit-ewars-model.yml"));
    }

    #[test]
    fn a_port_change_says_what_happened_and_what_to_do_next() {
        let project = project_with_ewars(None);
        let exposed = PortChange {
            id: "chapkit_ewars_model".into(),
            service_id: "chapkit-ewars-model".into(),
            host_port: Some(5001),
            previous: None,
            url: "http://localhost:5001".into(),
            written: vec![project.dir.join("compose.chapkit-ewars-model.yml")],
        };
        let text = port_summary(&exposed, &project, &[], &Out::default());
        assert!(text.starts_with("exposed chapkit-ewars-model on http://localhost:5001\n"));
        assert!(text.contains("written  compose.chapkit-ewars-model.yml\n"));
        assert!(text.ends_with("run `chaps up` to apply"));
        assert!(!text.contains("published already"));

        let internal = PortChange {
            host_port: None,
            previous: Some(5001),
            url: project.proxy_url("chapkit-ewars-model"),
            written: Vec::new(),
            ..exposed
        };
        let text = port_summary(
            &internal,
            &project,
            &["careful".to_string()],
            &Out::default(),
        );
        assert!(text.starts_with(
            "unexposed chapkit-ewars-model; it stays registered with chap-core and \
             reachable at http://localhost:8000/v2/services/chapkit-ewars-model/run/\n"
        ));
        assert!(text.contains("warning: careful\n"));

        // Asking for what is already there is a no-op worth saying out loud.
        let again = PortChange {
            host_port: None,
            previous: None,
            ..internal
        };
        assert!(
            port_summary(&again, &project, &[], &Out::default())
                .contains("that is what it published already")
        );
    }
}
