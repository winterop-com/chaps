//! Give chap-core the configured models of the models this deployment runs.
//!
//! A chapkit model registers itself with chap-core, and up to chap-core 2.3
//! that registration also gave it a configured model: the unit a backtest, a
//! prediction and the Modeling App's model picker name. chap-core 2.4 stopped
//! doing that. A registration now makes a model template at most, and the
//! configured models come from the marketplace entry through `chap-admin
//! install`. A model that varde started is registered, and nothing can run it.
//!
//! This module does what `chap-admin install` does after its service has
//! registered (`_register_service` in chap-core's
//! `cli_endpoints/marketplace.py`): store the template from the service, then
//! post one configured model for each configuration of the marketplace entry
//! (`configured_model_requests` in `services/model_marketplace.py`).
//!
//! The step is idempotent. A model that has a configured model of the version
//! it registered with already is left alone, so an older chap-core that still
//! makes them itself gets no second set, and a second run adds nothing.

use crate::api::Api;
use crate::error::Result;
use crate::modeltest::{ConfiguredModel, configured_models};
use crate::output::Report;
use crate::registry::Model;
use serde::Serialize;

/// The configured models chap-core has.
pub const CONFIGURED_MODELS_PATH: &str = "/v1/crud/configured-models";

/// Where chap-core stores the template of a registered service.
pub const FROM_SERVICE_PATH: &str = "/v1/crud/model-templates/from-service";

/// The keys of a marketplace `config` that are not user options.
///
/// chap-core's `RESERVED_CONFIG_KEYS`: the covariate list is stored as a field
/// of its own, and the horizon is dropped because chap-core sets it per run.
pub const RESERVED_CONFIG_KEYS: &[&str] =
    &["prediction_periods", "additional_continuous_covariates"];

/// The configuration name a model without configurations gets.
///
/// chap-core names a configured model called `default` after its template
/// alone, which is the name a backtest of the service finds first.
pub const DEFAULT_CONFIGURATION: &str = "default";

/// The help line that a model not yet registered gets.
pub const NOT_REGISTERED_WAY_OUT: &str =
    "run `varde models configs sync` again once `varde status` shows it registered";

/// One `POST /v1/crud/configured-models` body.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfiguredModelRequest {
    pub name: String,
    pub model_template_id: i64,
    pub user_option_values: serde_json::Map<String, serde_json::Value>,
    /// Left out for a model without a marketplace entry, which then runs on
    /// the covariates the service itself defaults to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_continuous_covariates: Option<Vec<String>>,
}

/// What the configured models of one model come from.
#[derive(Debug, Clone)]
pub enum Source {
    /// The marketplace entry of the model: one configured model for each of
    /// its configurations.
    Marketplace(Box<Model>),
    /// A model added with `varde models add`, or one that runs an image of
    /// its own: one `default` configured model with the service defaults.
    Custom,
}

/// The bodies to post for one template, in the order they are posted.
///
/// The same rule as chap-core's `configured_model_requests`: a `config` is the
/// flat object chapkit accepts, its covariate list is split out (the entry's
/// `covariates.defaults` when the config has none), and the reserved keys are
/// removed. An entry without configurations gets one `default`.
pub fn requests(source: &Source, template_id: i64) -> Vec<ConfiguredModelRequest> {
    let entry = match source {
        Source::Custom => {
            return vec![ConfiguredModelRequest {
                name: DEFAULT_CONFIGURATION.to_string(),
                model_template_id: template_id,
                user_option_values: serde_json::Map::new(),
                additional_continuous_covariates: None,
            }];
        }
        Source::Marketplace(entry) => entry,
    };
    let mut configurations: Vec<(String, serde_json::Map<String, serde_json::Value>)> = entry
        .configurations
        .iter()
        .map(|(name, configuration)| {
            let config = configuration
                .config
                .as_object()
                .cloned()
                .unwrap_or_default();
            (name.clone(), config)
        })
        .collect();
    if configurations.is_empty() {
        configurations.push((DEFAULT_CONFIGURATION.to_string(), serde_json::Map::new()));
    }
    configurations
        .into_iter()
        .map(|(name, mut config)| {
            let covariates = config
                .remove("additional_continuous_covariates")
                .and_then(|value| string_list(&value))
                .unwrap_or_else(|| entry.covariates.defaults.clone());
            for key in RESERVED_CONFIG_KEYS {
                config.remove(*key);
            }
            ConfiguredModelRequest {
                name,
                model_template_id: template_id,
                user_option_values: config,
                additional_continuous_covariates: Some(covariates),
            }
        })
        .collect()
}

/// A JSON list of strings, or `None` when the value is not one.
fn string_list(value: &serde_json::Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| item.as_str().map(str::to_string))
        .collect()
}

/// Whether chap-core has a configured model of the template `name` at
/// `version`.
///
/// A configured model of a template is named after it: the bare name for a
/// `default`, `<name>:<config>` for the others. An archived row does not
/// count, and a `test_config_` row does not either: a test left it, and
/// nobody can pick it. `version` is the version the service registered
/// with. A model updated to a new version has the rows of the old version,
/// and those do not make the new version runnable. When either side has no
/// version, the name is the whole answer.
pub fn is_configured(models: &[ConfiguredModel], name: &str, version: Option<&str>) -> bool {
    let prefix = format!("{name}:");
    let left_behind = format!("{prefix}test_config_");
    let version = version.filter(|v| !v.is_empty());
    models.iter().any(|model| {
        !model.archived
            && (model.name == name
                || (model.name.starts_with(&prefix) && !model.name.starts_with(&left_behind)))
            && match (version, model.version.as_deref()) {
                (Some(wanted), Some(has)) if !has.is_empty() => wanted == has,
                _ => true,
            }
    })
}

/// One model this step is about: its marketplace id and its service id.
#[derive(Debug, Clone)]
pub struct Target {
    pub id: String,
    pub service_id: String,
}

/// What the step did for one model.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Outcome {
    /// It posted these configured models, by configuration name.
    Created { configured: Vec<String> },
    /// chap-core had a configured model of it already.
    AlreadyConfigured,
    /// chap-core has no registration for it, so there is no template to
    /// configure.
    NotRegistered,
    /// A request failed; the sentence says which and why.
    Failed { error: String },
    /// The template request failed, and the service reports no git revision,
    /// which is the reason chap-core gives no template: running the step
    /// again cannot fix it.
    NoRevision { error: String },
}

/// The way out for a model of this deployment that reports no git revision.
pub const NO_REVISION_WAY_OUT: &str = "build its image again with `--build-arg \
     GIT_REVISION=$(git rev-parse HEAD)`, then run `varde restart`";

/// One model and what the step did for it.
#[derive(Debug, Clone, Serialize)]
pub struct ModelOutcome {
    pub id: String,
    pub service_id: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

/// What the step did inside another command, for its `--json`.
#[derive(Debug, Clone, Serialize)]
pub struct Folded {
    pub models: Vec<ModelOutcome>,
    /// Why chap-core could not be asked, when it could not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Configure every target that chap-core has registered and nothing runs.
///
/// `source` is asked only for a model that needs configured models, so a
/// deployment where everything is configured never loads the catalogue.
/// The error is a chap-core that could not tell which services it has or
/// which configured models: then nothing was asked of it.
pub fn configure(
    api: &Api,
    targets: &[Target],
    source: &mut dyn FnMut(&Target) -> Result<Source>,
) -> Result<Vec<ModelOutcome>> {
    let registered = registered_services(api)?;
    let listed = api.get_json(CONFIGURED_MODELS_PATH)?;
    let models = configured_models(&listed);
    Ok(targets
        .iter()
        .map(|target| {
            let service = registered
                .iter()
                .find(|service| service.id == target.service_id);
            let outcome = match service {
                None => Outcome::NotRegistered,
                Some(service) => {
                    match configure_one(api, target, &service.version, &models, source) {
                        // A failed template request for a service that reports
                        // no revision is that, whatever the status code says.
                        Outcome::Failed { error } if matches!(service.git_revision, Some(None)) => {
                            Outcome::NoRevision { error }
                        }
                        outcome => outcome,
                    }
                }
            };
            ModelOutcome {
                id: target.id.clone(),
                service_id: target.service_id.clone(),
                outcome,
            }
        })
        .collect())
}

/// The services chap-core has registered.
fn registered_services(api: &Api) -> Result<Vec<crate::status::RegisteredService>> {
    let answer = api.send("GET", crate::status::SERVICES_PATH, None)?;
    if !answer.is_success() {
        return Err(api.status_error(crate::status::SERVICES_PATH, &answer));
    }
    crate::status::parse_services(&answer.text()).map_err(|err| {
        anyhow::anyhow!(
            "{} answered with something that is not a service list: {err}",
            api.url(crate::status::SERVICES_PATH)
        )
    })
}

/// Store the template of one registered service, and its configured models.
fn configure_one(
    api: &Api,
    target: &Target,
    version: &str,
    models: &[ConfiguredModel],
    source: &mut dyn FnMut(&Target) -> Result<Source>,
) -> Outcome {
    if is_configured(models, &target.service_id, Some(version)) {
        return Outcome::AlreadyConfigured;
    }
    let body = serde_json::json!({ "service_id": target.service_id }).to_string();
    let template = match post(api, FROM_SERVICE_PATH, &body) {
        Ok(template) => template,
        Err(error) => return Outcome::Failed { error },
    };
    let Some(template_id) = template.get("id").and_then(|id| id.as_i64()) else {
        return Outcome::Failed {
            error: format!(
                "{} answered without a template id",
                api.url(FROM_SERVICE_PATH)
            ),
        };
    };
    // The template is named after what the service reports, which is the
    // service id for every model varde knows; a name of its own is checked too.
    if let Some(name) = template.get("name").and_then(|name| name.as_str())
        && name != target.service_id
        && is_configured(models, name, Some(version))
    {
        return Outcome::AlreadyConfigured;
    }
    let source = match source(target) {
        Ok(source) => source,
        Err(err) => {
            return Outcome::Failed {
                error: err.to_string(),
            };
        }
    };
    let mut created = Vec::new();
    for request in requests(&source, template_id) {
        let body = serde_json::to_string(&request).unwrap_or_default();
        if let Err(error) = post(api, CONFIGURED_MODELS_PATH, &body) {
            return Outcome::Failed {
                error: match created.is_empty() {
                    true => error,
                    false => format!("{error} (after {})", created.join(", ")),
                },
            };
        }
        created.push(request.name);
    }
    Outcome::Created {
        configured: created,
    }
}

/// `POST path` with a JSON body, and the answer as JSON or the sentence that
/// says why not.
fn post(api: &Api, path: &str, body: &str) -> std::result::Result<serde_json::Value, String> {
    let answer = api
        .send_json("POST", path, Some(body))
        .map_err(|err| err.to_string())?;
    if !answer.is_success() {
        return Err(api.status_error(path, &answer).to_string());
    }
    Ok(answer.json().unwrap_or(serde_json::Value::Null))
}

/// The lines of `varde models configs sync`: one for each model.
pub fn command_lines(outcomes: &[ModelOutcome], lines: &mut Report) {
    for model in outcomes {
        let id = &model.id;
        match &model.outcome {
            Outcome::Created { configured } => {
                for name in configured {
                    lines.info(format!("{id}: created configured model {name}"));
                }
            }
            Outcome::AlreadyConfigured => {
                lines.info(format!(
                    "{id}: chap-core has a configured model of it already"
                ));
            }
            Outcome::NotRegistered => {
                lines.info(format!(
                    "{id}: not registered with chap-core; {NOT_REGISTERED_WAY_OUT}"
                ));
            }
            Outcome::Failed { error } => {
                lines.warning(failed_line(id, error));
            }
            Outcome::NoRevision { error } => {
                lines.warning(no_revision_line(id, error));
            }
        }
    }
}

/// The lines another command folds in: one line for what was created, and a
/// warning for each failure. A model that is configured already gets no line.
/// `not_registered` adds a line for each model chap-core has not registered,
/// for a command that says the Modeling App can use Chap.
pub fn folded_lines(outcomes: &[ModelOutcome], not_registered: bool, lines: &mut Report) {
    let created: Vec<String> = outcomes
        .iter()
        .filter_map(|model| match &model.outcome {
            Outcome::Created { configured } => {
                Some(format!("{} ({})", model.id, configured.join(", ")))
            }
            _ => None,
        })
        .collect();
    if !created.is_empty() {
        lines.info(format!(
            "created configured models in chap-core for {}",
            created.join(", ")
        ));
    }
    for model in outcomes {
        match &model.outcome {
            Outcome::Failed { error } => {
                lines.warning(failed_line(&model.id, error));
            }
            Outcome::NoRevision { error } => {
                lines.warning(no_revision_line(&model.id, error));
            }
            Outcome::NotRegistered if not_registered => {
                lines.info(format!(
                    "{}: not registered with chap-core, so the Modeling App cannot use it yet; \
                     {NOT_REGISTERED_WAY_OUT}",
                    model.id
                ));
            }
            _ => {}
        }
    }
}

/// The warning for a model the step could not configure.
fn failed_line(id: &str, error: &str) -> String {
    format!(
        "{id}: could not create its configured models: {error}; run `varde models configs sync {id}` again, and `varde logs chap` says why"
    )
}

/// The warning for a model that reports no git revision.
fn no_revision_line(id: &str, error: &str) -> String {
    format!(
        "{id}: could not create its configured models: {error}; it reports no git revision, so \
         running this again does not help; {NO_REVISION_WAY_OUT}"
    )
}

/// Whether any model failed.
pub fn any_failed(outcomes: &[ModelOutcome]) -> bool {
    outcomes.iter().any(|model| {
        matches!(
            model.outcome,
            Outcome::Failed { .. } | Outcome::NoRevision { .. }
        )
    })
}

#[cfg(test)]
mod tests;
