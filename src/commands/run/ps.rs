//! `chaps ps`, and the scope `stop` and `top` share with it.

use super::{group_dir, groups, groups_dir, target};
use crate::cli::ModelPsArgs;
use crate::commands::Ctx;
use crate::commands::enable::ModelRef;
use crate::docker;
use crate::error::Result;
use crate::output::Out;
use crate::project::Project;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

/// One model as `chaps ps` lists it.
#[derive(Debug, Serialize)]
pub struct PsRow {
    /// The group it runs in; `null` inside a deployment of the caller's own.
    pub group: Option<String>,
    #[serde(flatten)]
    pub model: ModelRef,
    /// The `chaps status` STATE word.
    pub state: &'static str,
    pub project_dir: PathBuf,
}

/// What `chaps ps` found, for `--json`.
#[derive(Debug, Serialize)]
pub(super) struct PsReport {
    models: Vec<PsRow>,
}

/// The deployments `ps`, `stop` and `top` look at: the one the caller is
/// inside, else every group (or the one `group` names).
pub(crate) fn scope(ctx: &Ctx, group: Option<&str>) -> Result<Vec<(Option<String>, PathBuf)>> {
    if let Some(dir) = Project::find_root(&ctx.project_dir) {
        target(ctx, group)?;
        return Ok(vec![(None, dir)]);
    }
    Ok(match group {
        Some(name) => {
            let dir = group_dir(name)?;
            match Project::exists(&dir) {
                true => vec![(Some(name.to_string()), dir)],
                false => Vec::new(),
            }
        }
        None => groups()
            .into_iter()
            .map(|(name, dir)| (Some(name), dir))
            .collect(),
    })
}

/// Every model in `scope`, with its state as `chaps status` would say it.
pub(crate) fn rows(scope: &[(Option<String>, PathBuf)]) -> Result<Vec<PsRow>> {
    let mut out = Vec::new();
    for (group, dir) in scope {
        let project = Project::load(dir)?;
        let running: BTreeSet<String> = docker::running_containers(&project)
            .as_deref()
            .map(docker::running_of)
            .unwrap_or_default();
        let token = crate::api::token_for(Some(&project.dir));
        let status = crate::status::status(
            &project,
            &project.api_url(),
            Duration::from_secs(3),
            &running,
            token.as_deref(),
            true,
        );
        out.extend(project.state.models.iter().map(|(id, model)| {
            PsRow {
                group: group.clone(),
                model: ModelRef::of(id, model, &project),
                state: status
                    .models
                    .iter()
                    .find(|row| row.id == model.service_id)
                    .map(|row| row.state.label())
                    .unwrap_or("not running"),
                project_dir: dir.clone(),
            }
        }));
    }
    Ok(out)
}

/// List the models with their state and URL.
pub fn ps(ctx: &Ctx, args: &ModelPsArgs) -> Result<()> {
    let scope = scope(ctx, args.group.as_deref())?;
    let report = PsReport {
        models: rows(&scope)?,
    };
    ctx.out
        .emit(&report, || ps_table(&report, &scope, &ctx.out))
}

/// The human rendering of `chaps ps`.
pub(super) fn ps_table(
    report: &PsReport,
    scope: &[(Option<String>, PathBuf)],
    out: &Out,
) -> String {
    if report.models.is_empty() {
        return out.backticks(match scope.is_empty() {
            true => "nothing has been started with `chaps run` yet; `chaps run <model>` starts one",
            false => "no models are enabled here; `chaps run <model>` starts one",
        });
    }
    let grouped = report.models.iter().any(|row| row.group.is_some());
    let mut headers = vec!["ID", "SERVICE", "STATE", "URL"];
    if grouped {
        headers.insert(0, "GROUP");
    }
    let rows: Vec<Vec<String>> = report
        .models
        .iter()
        .map(|row| {
            let mut cells = vec![
                row.model.id.clone(),
                row.model.service_id.clone(),
                row.state.to_string(),
                row.model
                    .url
                    .clone()
                    .unwrap_or_else(|| "internal".to_string()),
            ];
            if grouped {
                cells.insert(0, row.group.clone().unwrap_or_default());
            }
            cells
        })
        .collect();
    let mut text = out.table(&headers, &rows);
    if grouped {
        text.push_str(&out.dim(&format!("groups live in {}", groups_dir().display())));
    }
    text
}
