//! `varde models configs sync`, and the same step inside `up --wait`,
//! `dhis2 connect` and `models test --backtest`.
//!
//! The rules are in [`crate::configs::sync`]; this module picks the models, loads
//! the catalogue when a model needs it, and reports.

use crate::api::Api;
use crate::cli::ConfigsSyncArgs;
use crate::commands::Ctx;
use crate::configs::sync::{self as rules, Folded, ModelOutcome, Source, Target};
use crate::error::{ChapError, Result};
use crate::project::Project;
use crate::registry::Registry;

/// What the process exits with when a model could not be configured. The
/// warning lines have said why, so there is no `error:` line on top.
const EXIT_FAILED: i32 = 1;

/// `varde models configs sync [ID...]`.
pub fn run(ctx: &Ctx, args: &ConfigsSyncArgs) -> Result<()> {
    let project = ctx.project()?;
    crate::components::require_chap_core(&project.state.components, "`varde models configs sync`")?;
    let targets = targets(ctx, &project, &args.ids)?;
    if targets.is_empty() {
        return ctx
            .out
            .report(&serde_json::json!({ "ok": true, "models": [] }), |lines| {
                lines.info(
                    "this deployment enables no models; enable one with `varde models enable ID`",
                );
            });
    }
    let outcomes = step(ctx, &project, &targets)?;
    let failed = rules::any_failed(&outcomes);
    ctx.out.report(
        &serde_json::json!({ "ok": !failed, "models": outcomes }),
        |lines| rules::command_lines(&outcomes, lines),
    )?;
    if failed {
        std::process::exit(EXIT_FAILED);
    }
    Ok(())
}

/// The step for every model this deployment enables, as another command runs
/// it: `None` when there is nothing to configure (no chap-core, or no models).
pub fn auto(ctx: &Ctx, project: &Project) -> Option<Folded> {
    if !project.state.components.has_chap_core_api() || project.state.models.is_empty() {
        return None;
    }
    Some(match step(ctx, project, &all_targets(project)) {
        Ok(models) => Folded {
            models,
            error: None,
        },
        Err(err) => Folded {
            models: Vec::new(),
            error: Some(
                err.to_string()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            ),
        },
    })
}

/// The lines another command folds into its report: what was created, and
/// what failed. A chap-core that could not be asked is a warning too.
pub fn fold(folded: &Option<Folded>, not_registered: bool, lines: &mut crate::output::Report) {
    let Some(folded) = folded else {
        return;
    };
    rules::folded_lines(&folded.models, not_registered, lines);
    if let Some(error) = &folded.error {
        lines.warning(format!(
            "could not check the configured models in chap-core: {error}; run `varde models \
             configs sync`"
        ));
    }
}

/// Run the step for `targets` against this deployment's chap-core.
pub fn step(ctx: &Ctx, project: &Project, targets: &[Target]) -> Result<Vec<ModelOutcome>> {
    let token = crate::api::token_for(Some(&project.dir));
    let api = Api::new(&project.api_url(), token, crate::api::DEFAULT_TIMEOUT);
    ctx.out.verbose(&format!(
        "checking the configured models in chap-core at {}",
        api.base()
    ));
    let mut registry: Option<Registry> = None;
    let mut source = |target: &Target| -> Result<Source> {
        if registry.is_none() {
            registry = Some(crate::commands::registry_for(ctx, Some(project))?);
        }
        let registry = registry.as_ref().expect("loaded above");
        // A model registered from outside runs no image of a marketplace
        // entry that varde knows of, so it gets the one `default`.
        if !project.state.models.contains_key(&target.id) {
            return Ok(Source::Custom);
        }
        Ok(source_of(registry, project, &target.id))
    };
    rules::configure(&api, targets, &mut source)
}

/// What the configured models of the model `id` come from.
///
/// The marketplace entry, unless the model is one this deployment defines
/// itself or it runs an image other than the one the entry names: chap-admin
/// gives a custom image one `default`, and so does this.
pub(crate) fn source_of(registry: &Registry, project: &Project, id: &str) -> Source {
    let entry = registry.models.iter().find(|model| model.id == id);
    let enabled = project.state.models.get(id);
    match entry {
        Some(entry)
            if !entry.manual
                && enabled.is_none_or(|enabled| enabled.image == entry.source.image) =>
        {
            Source::Marketplace(Box::new(entry.clone()))
        }
        _ => Source::Custom,
    }
}

/// Every model this deployment enables, in the order `models.yaml` has them.
pub(super) fn all_targets(project: &Project) -> Vec<Target> {
    project
        .state
        .models
        .iter()
        .map(|(id, enabled)| Target {
            id: id.clone(),
            service_id: enabled.service_id.clone(),
            managed: true,
        })
        .collect()
}

/// The models named on the command line, or every enabled one.
///
/// An id is resolved the way `models test` resolves it: the service id of a
/// model registered from outside this deployment, a marketplace id or a
/// service id. Any other model that is not enabled here is a mistake:
/// nothing registers it.
pub(super) fn targets(ctx: &Ctx, project: &Project, ids: &[String]) -> Result<Vec<Target>> {
    if ids.is_empty() {
        return Ok(all_targets(project));
    }
    let registry = crate::commands::registry_for(ctx, Some(project))?;
    let token = crate::api::token_for(Some(&project.dir));
    let api = Api::new(&project.api_url(), token, crate::api::DEFAULT_TIMEOUT);
    let outside = crate::commands::modeltest::unmanaged_services(project, &api);
    let mut picked: Vec<Target> = Vec::new();
    for given in ids {
        if outside.contains(given) {
            if !picked.iter().any(|target| &target.id == given) {
                picked.push(Target {
                    id: given.clone(),
                    service_id: given.clone(),
                    managed: false,
                });
            }
            continue;
        }
        let model = registry
            .get(given)
            .ok_or_else(|| ChapError::UnknownModel(given.clone()))?;
        let enabled = project.state.models.get(&model.id).ok_or_else(|| {
            anyhow::anyhow!(
                "{} is not enabled in this deployment; enable it with `varde models enable {}`",
                model.id,
                model.id
            )
        })?;
        if picked.iter().any(|target| target.id == model.id) {
            continue;
        }
        picked.push(Target {
            id: model.id.clone(),
            service_id: enabled.service_id.clone(),
            managed: true,
        });
    }
    Ok(picked)
}
