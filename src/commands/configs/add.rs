//! `varde models configs add ID --name NAME [--set KEY=VALUE ...]
//! [--covariates a,b]`, and the same with `-i`.

use super::{api_of, one_target, project_of, prompt};
use crate::cli::ConfigsAddArgs;
use crate::commands::Ctx;
use crate::configs::{self, Draft, options};
use crate::error::{ChapError, Result};
use std::io::IsTerminal;

/// `varde models configs add`.
pub fn add(ctx: &Ctx, args: &ConfigsAddArgs) -> Result<()> {
    // The usage mistakes come before anything is asked of chap-core.
    let sets = args
        .set
        .iter()
        .map(|text| options::parse_set(text))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(ChapError::Usage)?;
    if args.interactive && (ctx.out.json || !std::io::stdin().is_terminal()) {
        return Err(ChapError::Usage(
            "-i asks in a terminal, and there is none here; give the values with --name, --set \
             and --covariates"
                .to_string(),
        )
        .into());
    }
    if !args.interactive && args.name.is_none() {
        return Err(ChapError::Usage(
            "a configured model needs a name; give it with --name NAME, or use -i".to_string(),
        )
        .into());
    }

    let project = project_of(ctx)?;
    let target = one_target(ctx, &project, &args.id)?;
    let id = &target.id;
    let api = api_of(&project);
    let template = configs::template_of_service(&api, id, &target.service_id)
        .map_err(|err| anyhow::anyhow!(err))?;
    let listed = configs::listing(&api)?;
    let existing = configs::configs_of(&listed, &template.name);

    let given = options::values_of(&template.options, &sets, id).map_err(ChapError::Usage)?;
    let covariates = args
        .covariates
        .as_deref()
        .map(configs::parse_covariates)
        .unwrap_or_default();
    let draft = match args.interactive {
        true => prompt::draft(
            &template,
            &existing,
            id,
            Draft {
                variant: args.name.clone().unwrap_or_default(),
                values: given,
                covariates,
            },
        )?,
        false => Draft {
            variant: args.name.clone().unwrap_or_default(),
            values: given,
            covariates,
        },
    };
    configs::check(&draft, &template, &existing, id).map_err(ChapError::Usage)?;

    // The first configured model of this version is what `configs sync`
    // looks for, so after this one it adds nothing for the model.
    let first = !crate::configs::sync::is_configured(
        &crate::modeltest::configured_models(&listed),
        &template.name,
        template.version.as_deref(),
    );
    let created = configs::create(&api, &template, &draft)?;
    let value = serde_json::json!({ "model": id, "configured": created });
    ctx.out.report_ok(&value, |lines| {
        lines.info(format!(
            "created configured model {} of {id}",
            draft.variant
        ));
        if first {
            lines.hint(format!(
                "{id} has a configured model of this version now, so `varde models configs \
                 sync` adds no marketplace configuration to it"
            ));
        }
    })
}
