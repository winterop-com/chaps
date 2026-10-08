//! Which configured model a backtest is of: the one `--config` names, or the
//! one [`modeltest::configured_model_for`] picks.

use super::first_line;
use crate::api::Api;
use crate::commands::Ctx;
use crate::modeltest::{self, UsedModel};
use serde_json::Value as Json;

/// What a backtest of one service names, or why it has nothing to name.
pub(super) enum Choice {
    /// The `modelId` to send, the covariates it hands the model, and the
    /// configured model the row says it used.
    Chosen {
        model_id: Json,
        covariates: Vec<String>,
        used: UsedModel,
    },
    /// chap-core has no live configured model of the registered version.
    NoneLive,
    /// chap-core has no live configured model `--config` names; these are
    /// the variant names of the ones it has.
    NoVariant(Vec<String>),
    /// `--config` was given, and chap-core could not list its configured
    /// models, so the name cannot be checked.
    Unlisted,
}

/// What `create-backtest` is given as its `modelId` for this service.
///
/// chap-core takes either the integer key of a configured model or a string it
/// resolves against their names, and the string only works while the service
/// has a configured model named plainly after it. That stops being true the
/// moment it registers with a new version, so the row is chosen here and its
/// id is what goes out, with the covariates the backtest will hand the model.
///
/// `version` is the version the service registered with; only a live row of
/// it counts. `wanted` is `--config`: a variant name, as `varde models
/// configs` shows it.
///
/// Without `--config`, a chap-core that could not be asked at all is no
/// skip: the service id is the spelling that worked before this, and a
/// listing that failed is no reason to refuse to backtest.
pub(super) fn choose(
    ctx: &Ctx,
    api: &Api,
    service_id: &str,
    version: Option<&str>,
    wanted: Option<&str>,
) -> Choice {
    const PATH: &str = crate::configs::sync::CONFIGURED_MODELS_PATH;
    let listed = match api.send("GET", PATH, None) {
        Ok(answer) if answer.is_success() => answer.json(),
        Ok(answer) => {
            ctx.out.verbose(&format!(
                "{service_id}: {} answered {}",
                api.url(PATH),
                answer.status_line()
            ));
            None
        }
        Err(err) => {
            ctx.out
                .verbose(&format!("{service_id}: {}", first_line(&err.to_string())));
            None
        }
    };
    let Some(listed) = listed else {
        if wanted.is_some() {
            return Choice::Unlisted;
        }
        ctx.out.verbose(&format!(
            "{service_id}: could not read chap-core's configured models, sending the service id"
        ));
        return Choice::Chosen {
            model_id: serde_json::json!(service_id),
            covariates: Vec::new(),
            used: UsedModel {
                id: None,
                name: service_id.to_string(),
                variant: crate::configs::sync::DEFAULT_CONFIGURATION.to_string(),
            },
        };
    };
    let models = modeltest::configured_models(&listed);
    let chosen = match wanted {
        Some(wanted) => match modeltest::configured_variant(&models, service_id, version, wanted) {
            Ok(chosen) => chosen,
            Err(names) if names.is_empty() => return Choice::NoneLive,
            Err(names) => return Choice::NoVariant(names),
        },
        None => match modeltest::configured_model_for(&models, service_id, version) {
            Some(chosen) => chosen,
            None => return Choice::NoneLive,
        },
    };
    ctx.out.verbose(&format!(
        "{service_id}: configured model {} {}",
        chosen.id, chosen.name
    ));
    Choice::Chosen {
        model_id: serde_json::json!(chosen.id),
        covariates: chosen.covariates.clone(),
        used: UsedModel {
            id: Some(chosen.id),
            name: chosen.name.clone(),
            variant: chosen
                .variant(service_id)
                .unwrap_or_else(|| chosen.name.clone()),
        },
    }
}
