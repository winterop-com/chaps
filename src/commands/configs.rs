//! `varde models configs`: the configured models chap-core has of each model.
//!
//! The rules are in [`crate::configs`]; this module picks the models, asks
//! chap-core and reports. `sync` is the step that other commands run too.

mod add;
mod export;
pub mod sync;
mod update;

pub use add::add;
pub use export::export;
pub use update::update;

use crate::api::Api;
use crate::cli::{ConfigsArchiveArgs, ConfigsListArgs};
use crate::commands::Ctx;
use crate::configs::{self, Config, sync::Source, sync::Target};
use crate::error::Result;
use crate::project::Project;
use serde::Serialize;

/// The table of `configs list`, without ARCHIVED.
pub const HEADERS: &[&str] = &["NAME", "COVARIATES", "OPTIONS", "SOURCE"];

/// How wide the OPTIONS column may be.
const OPTIONS_WIDTH: usize = 48;

/// The client for this deployment's chap-core.
pub(crate) fn api_of(project: &Project) -> Api {
    let token = crate::api::token_for(Some(&project.dir));
    Api::new(&project.api_url(), token, crate::api::DEFAULT_TIMEOUT)
}

/// The deployment, refused when it has no chap-core.
fn project_of(ctx: &Ctx) -> Result<Project> {
    let project = ctx.project()?;
    crate::components::require_chap_core(&project.state.components, "`varde models configs`")?;
    Ok(project)
}

/// The one model a command names.
fn one_target(ctx: &Ctx, project: &Project, id: &str) -> Result<Target> {
    sync::targets(ctx, project, &[id.to_string()])?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("{id} is not a model of this deployment"))
}

/// Where the configured models of `target` come from, when the catalogue
/// loads. A model registered from outside gets the one `default`.
fn source_for(ctx: &Ctx, project: &Project, target: &Target) -> Option<Source> {
    if !project.state.models.contains_key(&target.id) {
        return Some(Source::Custom);
    }
    let registry = crate::commands::registry_for(ctx, Some(project)).ok()?;
    Some(sync::source_of(&registry, project, &target.id))
}

/// The covariates a template allows without free covariates: the defaults
/// of the marketplace entry.
pub(crate) fn allow_from(template: &mut configs::Template, source: Option<&Source>) {
    if let Some(Source::Marketplace(entry)) = source {
        template.allowed_covariates = entry.covariates.defaults.clone();
    }
}

/// What `add` and `update` work on: one model, the template of the version
/// it runs, and the configured models chap-core has of it.
pub(crate) struct Session {
    pub target: Target,
    pub api: Api,
    pub template: configs::Template,
    pub listed: serde_json::Value,
    pub existing: Vec<Config>,
}

impl Session {
    pub(crate) fn open(ctx: &Ctx, id: &str) -> Result<Session> {
        let project = project_of(ctx)?;
        let target = one_target(ctx, &project, id)?;
        let api = api_of(&project);
        let mut template = configs::template_of_service(&api, &target.id, &target.service_id)
            .map_err(|err| anyhow::anyhow!(err))?;
        let source = source_for(ctx, &project, &target);
        allow_from(&mut template, source.as_ref());
        let listed = configs::listing(&api)?;
        let existing = configs::configs_of(&listed, &template.name);
        Ok(Session {
            target,
            api,
            template,
            listed,
            existing,
        })
    }

    /// The configured model `name` that is not archived, or the error that
    /// names the ones there are.
    pub(crate) fn live(&self, name: &str) -> Result<&Config> {
        let id = &self.target.id;
        let wanted = |config: &&Config| config.variant == name || config.name == name;
        if let Some(config) = self.existing.iter().filter(wanted).find(|c| !c.archived) {
            return Ok(config);
        }
        if self.existing.iter().any(|config| wanted(&config)) {
            return Err(anyhow::anyhow!(
                "the configured model {name} of {id} is archived; `varde models configs list \
                 {id} --all` shows it"
            ));
        }
        let live: Vec<&str> = self
            .existing
            .iter()
            .filter(|config| !config.archived)
            .map(|config| config.variant.as_str())
            .collect();
        Err(anyhow::anyhow!(match live.is_empty() {
            true => format!(
                "{id} has no configured model {name}, and no other one; `varde models configs \
                 list {id} --all` shows the archived ones"
            ),
            false => format!(
                "{id} has no configured model {name}; its configured models are {}",
                live.join(", ")
            ),
        }))
    }
}

/// The form needs a terminal; without one, the way out names the options.
pub(crate) fn require_terminal(ctx: &Ctx, way_out: &str) -> Result<()> {
    use std::io::IsTerminal;
    if ctx.out.json || !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(crate::error::ChapError::Usage(format!(
            "there is no terminal here; {way_out}"
        ))
        .into());
    }
    Ok(())
}

/// The form at the terminal, checked the way the options are.
pub(crate) fn open_form(
    model: &str,
    form: crate::tui::form::Form,
    template: &configs::Template,
    existing: &[Config],
) -> Result<Option<configs::Draft>> {
    crate::tui::run_form(model, form, |form| {
        crate::tui::form::draft_of(form, template, existing, model)
    })
}

/// The report of a form that Esc left.
pub(crate) fn left_form(ctx: &Ctx, model: &str) -> Result<()> {
    ctx.out.report(
        &serde_json::json!({ "model": model, "changed": false }),
        |lines| {
            lines.info(crate::tui::form::LEFT);
        },
    )
}

/// One configured model as `--json` gives it.
#[derive(Debug, Serialize)]
struct Listed {
    #[serde(flatten)]
    config: Config,
    source: &'static str,
}

/// One model and its configured models, for `--json`.
#[derive(Debug, Serialize)]
struct ModelConfigs {
    id: String,
    service_id: String,
    configs: Vec<Listed>,
}

/// `varde models configs [list] [ID] [--all]`.
pub fn list(ctx: &Ctx, args: &ConfigsListArgs) -> Result<()> {
    let project = project_of(ctx)?;
    let ids: Vec<String> = args.id.iter().cloned().collect();
    let targets = sync::targets(ctx, &project, &ids)?;
    if targets.is_empty() {
        return ctx
            .out
            .report(&serde_json::json!({ "models": [] }), |lines| {
                lines.info(
                    "this deployment enables no models; enable one with `varde models enable ID`",
                );
            });
    }
    let api = api_of(&project);
    ctx.out.verbose(&format!(
        "asking chap-core at {} for its configured models",
        api.base()
    ));
    let listed = configs::listing(&api)?;
    let models: Vec<ModelConfigs> = targets
        .iter()
        .map(|target| {
            let source = source_for(ctx, &project, target);
            let configs = configs::configs_of(&listed, &target.service_id)
                .into_iter()
                .filter(|config| args.all || !config.archived)
                .map(|config| Listed {
                    source: configs::source_label(&config, source.as_ref()),
                    config,
                })
                .collect();
            ModelConfigs {
                id: target.id.clone(),
                service_id: target.service_id.clone(),
                configs,
            }
        })
        .collect();
    if !ctx.out.json {
        print!("{}", human_list(&models, args.all, &ctx.out));
    }
    ctx.out
        .report(&serde_json::json!({ "models": models }), |lines| {
            for model in models.iter().filter(|model| model.configs.is_empty()) {
                lines.info(format!(
                    "{}: chap-core has no configured model of it; `varde models configs sync \
                     {}` creates them",
                    model.id, model.id
                ));
            }
            let count: usize = models.iter().map(|model| model.configs.len()).sum();
            if count > 0 {
                let with = models.iter().filter(|m| !m.configs.is_empty()).count();
                lines.info(format!(
                    "{count} configured model{} of {with} model{}",
                    plural(count),
                    plural(with)
                ));
            }
        })
}

/// A table under the id of each model that has configured models.
fn human_list(models: &[ModelConfigs], all: bool, out: &crate::output::Out) -> String {
    let mut text = String::new();
    for model in models.iter().filter(|model| !model.configs.is_empty()) {
        text.push_str(&out.heading(&model.id));
        text.push('\n');
        text.push_str(&out.table(&headers(all), &rows(&model.configs, all)));
        text.push('\n');
    }
    text
}

/// The headers, with ARCHIVED for `--all`.
fn headers(all: bool) -> Vec<&'static str> {
    let mut headers = HEADERS.to_vec();
    if all {
        headers.push("ARCHIVED");
    }
    headers
}

/// The rows of one model's table.
fn rows(configs: &[Listed], all: bool) -> Vec<Vec<String>> {
    configs
        .iter()
        .map(|listed| {
            let config = &listed.config;
            let mut row = vec![
                config.variant.clone(),
                match config.covariates.is_empty() {
                    true => "-".to_string(),
                    false => config.covariates.join(","),
                },
                match config.values.is_empty() {
                    true => "-".to_string(),
                    false => configs::options::short(&config.values, OPTIONS_WIDTH),
                },
                listed.source.to_string(),
            ];
            if all {
                row.push(if config.archived { "yes" } else { "no" }.to_string());
            }
            row
        })
        .collect()
}

/// `varde models configs archive ID NAME`.
pub fn archive(ctx: &Ctx, args: &ConfigsArchiveArgs) -> Result<()> {
    let project = project_of(ctx)?;
    let target = one_target(ctx, &project, &args.id)?;
    let api = api_of(&project);
    let listed = configs::listing(&api)?;
    let existing = configs::configs_of(&listed, &target.service_id);
    let id = &target.id;
    let wanted = |config: &&Config| config.variant == args.name || config.name == args.name;
    let Some(config) = existing.iter().filter(wanted).find(|c| !c.archived) else {
        if existing.iter().any(|config| wanted(&config)) {
            return Err(anyhow::anyhow!(
                "the configured model {} of {id} is archived already; `varde models configs \
                 list {id} --all` shows it",
                args.name
            ));
        }
        let live: Vec<&str> = existing
            .iter()
            .filter(|config| !config.archived)
            .map(|config| config.variant.as_str())
            .collect();
        return Err(anyhow::anyhow!(match live.is_empty() {
            true => format!(
                "{id} has no configured model {}, and no other one; `varde models configs list \
                 {id} --all` shows the archived ones",
                args.name
            ),
            false => format!(
                "{id} has no configured model {}; its configured models are {}",
                args.name,
                live.join(", ")
            ),
        }));
    };
    configs::archive(&api, config.id)?;
    let leaves_alone = configs::sync_leaves_alone(&existing, config);
    let source = source_for(ctx, &project, &target);
    let label = configs::source_label(config, source.as_ref());
    let value = serde_json::json!({ "model": id, "archived": config });
    ctx.out.report_ok(&value, |lines| {
        lines.info(format!(
            "archived configured model {} of {id}",
            config.variant
        ));
        if !leaves_alone {
            lines.info(format!(
                "{id} has no other configured model now, so `varde models configs sync` and \
                 `varde up --wait` create its configured models again"
            ));
        } else if label != "by hand" {
            lines.hint(format!(
                "`varde models configs sync` does not create {} again while {id} has other \
                 configured models",
                config.variant
            ));
        }
        lines.hint(
            "chap-core keeps an archived configured model for the backtests that use it, and \
             the Modeling App shows it as Archived",
        );
    })
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests;
