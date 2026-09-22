//! `chaps models enable|disable` — the two single-model write commands.
//!
//! Owned by agent B. Both go through [`crate::compose::apply`], the same path
//! `init` and the TUI use.

use crate::cli::{ModelsDisableArgs, ModelsEnableArgs};
use crate::commands::Ctx;
use crate::compose::{ApplyReport, EnableRequest, Selection, apply};
use crate::error::{ChapError, Result};
use crate::project::Project;
use crate::registry::{self, Channel, VersionSelector};

/// Enable one model: resolve its version, allocate a port, write the overlay,
/// regenerate the umbrella file and update chaps.json.
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
            port: args.port,
            data_dir: args.data_dir.clone(),
            user: args.user.clone(),
            allow_template: args.allow_template,
        }],
        disable: Vec::new(),
    };

    let report = apply(&mut project, &registry, &selection, ctx.cli_version)?;
    ctx.out.emit(&report, || summary(&report, &project))
}

/// Disable one model: remove its overlay, regenerate the umbrella file and
/// update chaps.json.
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
    ctx.out.emit(&report, || summary(&report, &project))
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

fn summary(report: &ApplyReport, project: &Project) -> String {
    let mut out = String::new();
    for (id, model) in report.touched() {
        let verb = if report.enabled.iter().any(|(e, _)| e == id) {
            "enabled"
        } else {
            "updated"
        };
        out.push_str(&format!(
            "{verb} {id} v{} on http://localhost:{} ({})\n",
            model.version, model.host_port, model.compose_file
        ));
    }
    for id in &report.disabled {
        out.push_str(&format!("disabled {id}\n"));
    }
    for path in &report.removed {
        out.push_str(&format!(
            "removed {}\n",
            path.strip_prefix(&project.dir).unwrap_or(path).display()
        ));
    }
    for warning in &report.warnings {
        out.push_str(&format!("warning: {warning}\n"));
    }
    out.push_str("run `chaps up` to apply");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{EnabledModel, ProjectState};
    use std::collections::BTreeMap;

    fn project_with_ewars() -> Project {
        let model = EnabledModel {
            service_id: "chapkit-ewars-model".into(),
            image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
            image_tag: "sha-fa880a1".into(),
            version: "1.0.0".into(),
            channel: Some(Channel::Stable),
            host_port: 5001,
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
        let project = project_with_ewars();
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

    #[test]
    fn the_summary_ends_with_the_next_step() {
        let project = project_with_ewars();
        let report = ApplyReport {
            enabled: vec![(
                "chapkit_ewars_model".to_string(),
                project.state.models["chapkit_ewars_model"].clone(),
            )],
            ..ApplyReport::default()
        };
        let text = summary(&report, &project);
        assert!(text.contains("enabled chapkit_ewars_model v1.0.0 on http://localhost:5001"));
        assert!(text.ends_with("run `chaps up` to apply"));
    }

    #[test]
    fn a_disable_summary_names_the_removed_file() {
        let project = project_with_ewars();
        let report = ApplyReport {
            disabled: vec!["chapkit_ewars_model".to_string()],
            removed: vec![project.dir.join("compose.chapkit-ewars-model.yml")],
            ..ApplyReport::default()
        };
        let text = summary(&report, &project);
        assert!(text.contains("disabled chapkit_ewars_model"));
        assert!(text.contains("removed compose.chapkit-ewars-model.yml"));
    }
}
