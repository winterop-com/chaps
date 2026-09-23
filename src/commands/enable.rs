//! `chaps models enable|disable` — the two single-model write commands.
//!
//! Owned by agent B. Both go through [`crate::compose::apply`], the same path
//! `init` and the TUI use.

use crate::cli::{ModelsDisableArgs, ModelsEnableArgs, ModelsExposeArgs, ModelsUnexposeArgs};
use crate::commands::Ctx;
use crate::compose::ports::allocator_for;
use crate::compose::sync::sync;
use crate::compose::{ApplyReport, EnableRequest, PortRequest, Selection, apply};
use crate::error::{ChapError, Result};
use crate::output::Out;
use crate::project::Project;
use crate::registry::{self, Channel, VersionSelector};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Enable one model: resolve its version, write the overlay, sync the compose
/// files and update .chaps/models.yaml.
///
/// No host port unless `--port` asks for one: chap-core reaches the service
/// over the compose network, which is the URL the service registers.
pub fn enable(ctx: &Ctx, args: &ModelsEnableArgs) -> Result<()> {
    let mut project = ctx.project()?;
    let registry = registry::load(&ctx.registry)?;

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
            allow_template: args.allow_template,
        }],
        disable: Vec::new(),
    };

    let report = apply(&mut project, &registry, &selection, ctx.cli_version)?;
    ctx.out
        .emit(&report, || summary(&report, &project, &ctx.out))
}

/// Disable one model: remove its overlay, regenerate the umbrella file and
/// update .chaps/models.yaml.
pub fn disable(ctx: &Ctx, args: &ModelsDisableArgs) -> Result<()> {
    let mut project = ctx.project()?;
    let registry = registry::load(&ctx.registry)?;

    // Accept either identifier, but only for a model this project enabled:
    // disabling something that was never on is a typo, not a no-op.
    let id =
        enabled_id(&project, &args.id).ok_or_else(|| ChapError::UnknownModel(args.id.clone()))?;
    let selection = Selection {
        enable: Vec::new(),
        disable: vec![id],
    };

    let report = apply(&mut project, &registry, &selection, ctx.cli_version)?;
    ctx.out
        .emit(&report, || summary(&report, &project, &ctx.out))
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
/// Deliberately not a [`Selection`]: `apply` re-resolves the version from the
/// model's channel, and publishing a port is no reason to move a pin on a
/// deployment that is already running one.
fn set_host_port(ctx: &Ctx, wanted: &str, request: PortRequest) -> Result<()> {
    let mut project = ctx.project()?;
    let registry = registry::load(&ctx.registry)?;
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

    let synced = sync(&mut project, &registry, ctx.cli_version, false)?;
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

/// The state key for a marketplace id or a compose service id.
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

fn summary(report: &ApplyReport, project: &Project, out: &Out) -> String {
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
            out.dim(&format!("v{}", model.version)),
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
    text.push_str(&out.backticks("run `chaps up` to apply"));
    text
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
        let text = summary(&enabled_report(&project), &project, &Out::default());
        assert!(text.contains("enabled chapkit_ewars_model v1.0.0 on http://localhost:5001"));
        assert!(text.ends_with("run `chaps up` to apply"));
    }

    #[test]
    fn a_model_with_no_host_port_is_summarised_with_the_proxy_url() {
        let project = project_with_ewars(None);
        let text = summary(&enabled_report(&project), &project, &Out::default());
        assert!(
            text.contains(
                "enabled chapkit_ewars_model v1.0.0 at \
                 http://localhost:8000/v2/services/chapkit-ewars-model/run/"
            ),
            "{text}"
        );
    }

    #[test]
    fn a_disable_summary_names_the_removed_file() {
        let project = project_with_ewars(Some(5001));
        let report = ApplyReport {
            disabled: vec!["chapkit_ewars_model".to_string()],
            removed: vec![project.dir.join("compose.chapkit-ewars-model.yml")],
            ..ApplyReport::default()
        };
        let text = summary(&report, &project, &Out::default());
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
