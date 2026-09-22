//! `chaps update [--dry-run] [--no-restart]` — move channel pins forward.
//!
//! The only other thing that moves a model pin is `chaps models enable`.
//! Exact pins (`--version`) never move here; chap-core's own moving tag is
//! refreshed by the `docker compose pull` this runs.

use crate::cli::UpdateArgs;
use crate::commands::Ctx;
use crate::compose::sync::refresh_env_pin;
use crate::compose::{sync, tag_env_var};
use crate::docker;
use crate::error::{ChapError, Result};
use crate::output;
use crate::project::Project;
use crate::registry::{self, Provenance, Registry, VersionSelector};
use serde::Serialize;

/// The shape of `update --json`, and what the human output is built from.
#[derive(Debug, Serialize)]
pub struct UpdateReport {
    pub registry: RegistryInfo,
    pub models: Vec<ModelUpdate>,
    /// Whether the chap-core images follow a moving tag that the pull refreshes.
    pub chap_core_repulled: bool,
    pub chap_image_tag: String,
    pub pulled: bool,
    pub restarted: bool,
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct RegistryInfo {
    pub url: String,
    pub provenance: Provenance,
}

/// One enabled model's before and after.
#[derive(Debug, Clone, Serialize)]
pub struct ModelUpdate {
    pub id: String,
    pub old_version: String,
    pub old_tag: String,
    pub new_version: String,
    pub new_tag: String,
    pub changed: bool,
    /// Pinned to an exact version, so the channel was not consulted.
    pub pinned: bool,
}

impl UpdateReport {
    pub fn changed(&self) -> impl Iterator<Item = &ModelUpdate> {
        self.models.iter().filter(|m| m.changed)
    }
}

/// Refresh the registry, re-resolve every channel-following model, sync,
/// pull and restart.
pub fn run(ctx: &Ctx, args: &UpdateArgs) -> Result<()> {
    let mut project = ctx.project()?;
    // No fallback on purpose: an update from a stale catalogue is not an update.
    let registry = registry::update(&ctx.registry)?;

    let models = plan(&project, &registry)?;
    let mut report = UpdateReport {
        registry: RegistryInfo {
            url: registry.url.clone(),
            provenance: registry.provenance.clone(),
        },
        models,
        chap_core_repulled: is_moving_tag(&project.state.chap_image_tag),
        chap_image_tag: project.state.chap_image_tag.clone(),
        pulled: false,
        restarted: false,
        dry_run: args.dry_run,
    };
    if args.dry_run {
        return ctx.out.emit(&report, || human(&report));
    }

    for change in report.models.iter().filter(|m| m.changed) {
        let entry = project
            .state
            .models
            .get_mut(&change.id)
            .expect("plan only lists enabled models");
        entry.version = change.new_version.clone();
        entry.image_tag = change.new_tag.clone();
        refresh_env_pin(&project.dir, &tag_env_var(&change.id), &change.new_tag)?;
    }
    let synced = sync(&mut project, &registry, ctx.cli_version, false)?;
    for warning in &synced.warnings {
        output::warn(warning);
    }

    run_compose(&project, &["pull".to_string()])?;
    report.pulled = true;
    if !args.no_restart {
        run_compose(&project, &["up".to_string(), "-d".to_string()])?;
        report.restarted = true;
    }
    ctx.out.emit(&report, || human(&report))
}

/// Resolve every enabled model against the fresh registry without changing
/// anything.
fn plan(project: &Project, registry: &Registry) -> Result<Vec<ModelUpdate>> {
    let mut out = Vec::with_capacity(project.state.models.len());
    for (id, entry) in &project.state.models {
        let mut update = ModelUpdate {
            id: id.clone(),
            old_version: entry.version.clone(),
            old_tag: entry.image_tag.clone(),
            new_version: entry.version.clone(),
            new_tag: entry.image_tag.clone(),
            changed: false,
            pinned: entry.channel.is_none(),
        };
        if let Some(channel) = entry.channel {
            let model = registry
                .get(id)
                .ok_or_else(|| ChapError::UnknownModel(id.clone()))?;
            let version = model.resolve(&VersionSelector::Channel(channel))?;
            update.new_version = version.version.clone();
            update.new_tag = version.image_tag.clone();
            update.changed =
                update.new_version != update.old_version || update.new_tag != update.old_tag;
        }
        out.push(update);
    }
    Ok(out)
}

/// A tag that can point at a different image tomorrow.
fn is_moving_tag(tag: &str) -> bool {
    matches!(tag, "latest" | "main" | "dev" | "edge" | "nightly")
}

fn run_compose(project: &Project, args: &[String]) -> Result<()> {
    let code = docker::run_compose(project, args)?;
    if code != 0 {
        return Err(ChapError::DockerFailed(code).into());
    }
    Ok(())
}

fn human(report: &UpdateReport) -> String {
    let mut text = format!(
        "registry: {} ({})\n",
        report.registry.url,
        report.registry.provenance.describe()
    );
    if report.models.is_empty() {
        text.push_str("no models enabled\n");
    }
    for m in &report.models {
        let line = if m.pinned {
            format!(
                "  {}  v{} ({})  pinned, skipped",
                m.id, m.old_version, m.old_tag
            )
        } else if m.changed {
            format!(
                "  {}  v{} ({}) -> v{} ({})",
                m.id, m.old_version, m.old_tag, m.new_version, m.new_tag
            )
        } else {
            format!("  {}  v{} ({})  unchanged", m.id, m.old_version, m.old_tag)
        };
        text.push_str(&line);
        text.push('\n');
    }
    let changed = report.changed().count();
    if report.dry_run {
        text.push_str(&format!(
            "dry run: {changed} model pin(s) would move; chap-core `{}` {} be re-pulled\n",
            report.chap_image_tag,
            if report.chap_core_repulled {
                "would"
            } else {
                "is an exact tag and would not"
            }
        ));
        text.push_str("nothing written\n");
    } else {
        text.push_str(&format!(
            "{changed} model pin(s) moved; images {}; stack {}\n",
            if report.pulled {
                "pulled"
            } else {
                "not pulled"
            },
            if report.restarted {
                "restarted"
            } else {
                "not restarted (run `chaps up`)"
            }
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{EnableRequest, Selection, apply};
    use crate::project::ProjectState;
    use crate::registry::{Channel, load_embedded};

    fn project_with(ids: &[&str]) -> (tempfile::TempDir, Project, Registry) {
        let dir = tempfile::tempdir().unwrap();
        let registry = load_embedded().unwrap();
        let mut project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState::default(),
        };
        let sel = Selection {
            enable: ids.iter().map(|id| EnableRequest::new(*id)).collect(),
            disable: Vec::new(),
        };
        apply(&mut project, &registry, &sel, "0.1.0").unwrap();
        (dir, project, registry)
    }

    #[test]
    fn plan_reports_unchanged_when_the_channel_still_points_at_the_pin() {
        let (_dir, project, registry) = project_with(&["chapkit_ewars_model"]);
        let plan = plan(&project, &registry).unwrap();
        assert_eq!(plan.len(), 1);
        assert!(!plan[0].changed && !plan[0].pinned);
        assert_eq!(plan[0].old_tag, plan[0].new_tag);
    }

    #[test]
    fn plan_moves_a_channel_pin_and_skips_an_exact_one() {
        let (_dir, mut project, mut registry) =
            project_with(&["chapkit_ewars_model", "auto_arima_chapkit"]);
        // Pretend the project was pinned to an older build of ewars.
        {
            let ewars = project.state.models.get_mut("chapkit_ewars_model").unwrap();
            ewars.version = "0.9.0".into();
            ewars.image_tag = "sha-0000000".into();
            ewars.channel = Some(Channel::Stable);
            let arima = project.state.models.get_mut("auto_arima_chapkit").unwrap();
            arima.channel = None;
            arima.image_tag = "sha-1111111".into();
        }
        // And that the marketplace moved auto_arima too; a pinned model ignores it.
        let arima = registry
            .models
            .iter_mut()
            .find(|m| m.id == "auto_arima_chapkit")
            .unwrap();
        arima.versions[0].image_tag = "sha-2222222".into();

        let plan = plan(&project, &registry).unwrap();
        let ewars = plan.iter().find(|m| m.id == "chapkit_ewars_model").unwrap();
        assert!(ewars.changed);
        assert_eq!(ewars.old_version, "0.9.0");
        assert_eq!(ewars.new_version, "1.0.0");
        assert_eq!(ewars.new_tag, "sha-fa880a1");

        let arima = plan.iter().find(|m| m.id == "auto_arima_chapkit").unwrap();
        assert!(arima.pinned && !arima.changed);
        assert_eq!(arima.new_tag, "sha-1111111");
    }

    #[test]
    fn plan_fails_when_a_channel_model_left_the_registry() {
        let (_dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let entry = project.state.models.remove("chapkit_ewars_model").unwrap();
        project.state.models.insert("vanished".into(), entry);
        let err = plan(&project, &registry).expect_err("unknown model");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownModel(id)) if id == "vanished"
        ));
    }

    #[test]
    fn moving_tags() {
        assert!(is_moving_tag("latest"));
        assert!(!is_moving_tag("v1.2.3"));
        assert!(!is_moving_tag("sha-abcdef0"));
    }

    #[test]
    fn human_dry_run_lists_each_model_and_writes_nothing() {
        let report = UpdateReport {
            registry: RegistryInfo {
                url: "https://example.test/registry.yaml".into(),
                provenance: Provenance::Network,
            },
            models: vec![
                ModelUpdate {
                    id: "a".into(),
                    old_version: "1.0.0".into(),
                    old_tag: "sha-1111111".into(),
                    new_version: "1.1.0".into(),
                    new_tag: "sha-2222222".into(),
                    changed: true,
                    pinned: false,
                },
                ModelUpdate {
                    id: "b".into(),
                    old_version: "1.0.0".into(),
                    old_tag: "sha-3333333".into(),
                    new_version: "1.0.0".into(),
                    new_tag: "sha-3333333".into(),
                    changed: false,
                    pinned: true,
                },
            ],
            chap_core_repulled: true,
            chap_image_tag: "latest".into(),
            pulled: false,
            restarted: false,
            dry_run: true,
        };
        let text = human(&report);
        assert!(text.starts_with("registry: https://example.test/registry.yaml (network)\n"));
        assert!(text.contains("  a  v1.0.0 (sha-1111111) -> v1.1.0 (sha-2222222)\n"));
        assert!(text.contains("  b  v1.0.0 (sha-3333333)  pinned, skipped\n"));
        assert!(text.contains("1 model pin(s) would move; chap-core `latest` would be re-pulled"));
        assert!(text.ends_with("nothing written\n"));

        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["dry_run"], true);
        assert_eq!(value["models"][0]["changed"], true);
        assert_eq!(value["registry"]["provenance"]["kind"], "network");
    }
}
