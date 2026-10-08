//! `varde models configs export [ID] [--out FILE]`: the configured models of
//! one model as the `configurations:` block of a marketplace entry.
//!
//! One model per export: the block goes under one entry, and `configs add
//! --from` reads it into one model. Without an ID the deployment must run
//! exactly one model, so the output is never a guess.

use super::{api_of, project_of, source_for, sync};
use crate::cli::ConfigsExportArgs;
use crate::commands::Ctx;
use crate::configs::{self, export, sync::Source};
use crate::error::{ChapError, Result};
use std::collections::BTreeSet;

/// `varde models configs export`.
pub fn export(ctx: &Ctx, args: &ConfigsExportArgs) -> Result<()> {
    let project = project_of(ctx)?;
    let ids: Vec<String> = args.id.iter().cloned().collect();
    let targets = sync::targets(ctx, &project, &ids)?;
    let target = match targets.as_slice() {
        [one] => one.clone(),
        [] => {
            return Err(ChapError::Usage(
                "this deployment enables no models; enable one with `varde models enable ID`"
                    .to_string(),
            )
            .into());
        }
        many => {
            let names: Vec<&str> = many.iter().map(|t| t.id.as_str()).collect();
            return Err(ChapError::Usage(format!(
                "an export is the configurations of one model; name one of {}",
                names.join(", ")
            ))
            .into());
        }
    };
    let id = &target.id;
    let api = api_of(&project);
    let listed = configs::listing(&api)?;
    let live: Vec<configs::Config> = configs::configs_of(&listed, &target.service_id)
        .into_iter()
        .filter(|config| !config.archived && !config.is_test())
        .collect();
    if live.is_empty() {
        return Err(anyhow::anyhow!(
            "chap-core has no configured model of {id} to export; `varde models configs sync \
             {id}` creates them"
        ));
    }
    let source = source_for(ctx, &project, &target);
    let entry = match &source {
        Some(Source::Marketplace(entry)) => Some(entry.as_ref()),
        _ => None,
    };
    // The service is asked only when a configuration has no marketplace twin.
    let needs_service = live.iter().any(|config| {
        entry.is_none_or(|entry| {
            entry
                .configurations
                .get(&config.variant)
                .and_then(|c| c.config.get(export::PERIODS_KEY))
                .is_none()
        })
    });
    let service_periods = match needs_service {
        true => service_periods(&api, &target.service_id),
        false => None,
    };
    let (configurations, from) = export::configurations(&live, id, entry, service_periods);
    let yaml = export::to_yaml(&configurations);
    let sources: BTreeSet<export::PeriodsFrom> = from.values().copied().collect();
    let hints: Vec<String> = sources
        .iter()
        .map(|source| {
            let names: Vec<&str> = from
                .iter()
                .filter(|(_, s)| *s == source)
                .map(|(name, _)| name.as_str())
                .collect();
            format!(
                "prediction_periods of {} comes from {}",
                names.join(", "),
                source.words()
            )
        })
        .collect();
    let value = serde_json::json!({
        "model": id,
        "configurations": configurations,
        "prediction_periods_from": from,
    });

    let Some(file) = &args.out else {
        // Standard output is the YAML alone, so it pipes; the hints go to
        // stderr with -v.
        if ctx.out.json {
            return ctx.out.report(&value, |_| {});
        }
        print!("{yaml}");
        if ctx.out.shows_hints() {
            for hint in &hints {
                eprintln!("hint: {hint}");
            }
        }
        return Ok(());
    };
    std::fs::write(file, &yaml)
        .map_err(|err| anyhow::anyhow!("could not write `{}`: {err}", file.display()))?;
    ctx.out.report_ok(&value, |lines| {
        lines.info(format!(
            "wrote {} configuration{} of {id} to {}",
            configurations.len(),
            if configurations.len() == 1 { "" } else { "s" },
            file.display()
        ));
        for hint in &hints {
            lines.hint(hint.clone());
        }
    })
}

/// The default horizon of the service, from its config schema through
/// chap-core's proxy; `None` when it does not say.
fn service_periods(api: &crate::api::Api, service: &str) -> Option<i64> {
    let path = format!(
        "/v2/services/{}/run/api/v1/configs/$schema",
        crate::api::encode(service)
    );
    let schema = api.get_json(&path).ok()?;
    export::schema_periods(&schema)
}
