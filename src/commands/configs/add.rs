//! `varde models configs add ID --name NAME [--set KEY=VALUE ...]
//! [--covariates a,b]`, `add ID --from FILE`, and `add ID` alone, which
//! opens the form at a terminal.

use super::{Session, open_form, require_terminal};
use crate::cli::ConfigsAddArgs;
use crate::commands::Ctx;
use crate::configs::{self, Draft, export, options};
use crate::error::{ChapError, Result};

/// `varde models configs add`.
pub fn add(ctx: &Ctx, args: &ConfigsAddArgs) -> Result<()> {
    if let Some(file) = &args.from {
        return add_from(ctx, args, file);
    }
    // The usage mistakes come before anything is asked of chap-core.
    let sets = args
        .set
        .iter()
        .map(|text| options::parse_set(text))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(ChapError::Usage)?;
    let form = args.name.is_none() && args.set.is_empty() && args.covariates.is_none();
    if form {
        require_terminal(
            ctx,
            "name the configuration with --name and its values with --set KEY=VALUE, or run \
             it at a terminal for the form",
        )?;
    } else if args.name.is_none() {
        return Err(ChapError::Usage(
            "a configured model needs a name; give it with --name NAME".to_string(),
        )
        .into());
    }

    let session = Session::open(ctx, &args.id)?;
    let id = &session.target.id;
    let template = &session.template;
    let given = options::values_of(&template.options, &sets, id).map_err(ChapError::Usage)?;
    let draft = match form {
        true => {
            let blank = crate::tui::form::form_for(template);
            match open_form(id, blank, template, &session.existing)? {
                Some(draft) => draft,
                None => return super::left_form(ctx, id),
            }
        }
        false => Draft {
            variant: args.name.clone().unwrap_or_default(),
            values: given,
            covariates: args
                .covariates
                .as_deref()
                .map(configs::parse_covariates)
                .unwrap_or_default(),
        },
    };
    configs::check(&draft, template, &session.existing, id).map_err(ChapError::Usage)?;

    let first = first_of_version(&session);
    let created = configs::create(&session.api, template, &draft)?;
    let value = serde_json::json!({ "model": id, "configured": [created] });
    ctx.out.report_ok(&value, |lines| {
        lines.info(format!(
            "created configured model {} of {id}",
            draft.variant
        ));
        if first {
            lines.hint(sync_hint(id));
        }
    })
}

/// Whether the model has no configured model of the version it runs, which
/// is what `configs sync` looks for.
fn first_of_version(session: &Session) -> bool {
    !configs::sync::is_configured(
        &crate::modeltest::configured_models(&session.listed),
        &session.template.name,
        session.template.version.as_deref(),
    )
}

fn sync_hint(id: &str) -> String {
    format!(
        "{id} has a configured model of this version now, so `varde models configs sync` adds \
         no marketplace configuration to it"
    )
}

/// `add ID --from FILE`: every configuration in a file of the marketplace
/// format. Each one is checked before the first is created, so a mistake
/// in the file creates nothing.
fn add_from(ctx: &Ctx, args: &ConfigsAddArgs, file: &std::path::Path) -> Result<()> {
    let text = std::fs::read_to_string(file)
        .map_err(|err| ChapError::Usage(format!("could not read `{}`: {err}", file.display())))?;
    let configurations = export::from_yaml(&text)
        .map_err(|why| ChapError::Usage(format!("`{}` cannot be read: {why}", file.display())))?;
    let session = Session::open(ctx, &args.id)?;
    let id = &session.target.id;
    let mut drafts = Vec::new();
    for (name, configuration) in &configurations {
        let draft = export::draft_of(name, configuration).map_err(ChapError::Usage)?;
        export::check_values(&draft, &session.template, id).map_err(ChapError::Usage)?;
        if session
            .existing
            .iter()
            .any(|config| !config.archived && &config.variant == name)
        {
            return Err(ChapError::Usage(format!(
                "{id} has a configured model {name} already; remove it from `{}`, or archive it \
                 first with `varde models configs archive {id} {name}`",
                file.display()
            ))
            .into());
        }
        configs::check(&draft, &session.template, &session.existing, id)
            .map_err(|why| ChapError::Usage(format!("{name}: {why}")))?;
        drafts.push(draft);
    }
    let first = first_of_version(&session);
    let mut created = Vec::new();
    for draft in &drafts {
        match configs::create(&session.api, &session.template, draft) {
            Ok(row) => created.push((draft.variant.clone(), row)),
            Err(err) => {
                let done: Vec<&str> = created.iter().map(|(name, _)| name.as_str()).collect();
                return Err(match done.is_empty() {
                    true => err,
                    false => anyhow::anyhow!(
                        "{err}; created {} before it, and `varde models configs add {id} --from \
                         {}` refuses those names now",
                        done.join(", "),
                        file.display()
                    ),
                });
            }
        }
    }
    let rows: Vec<&serde_json::Value> = created.iter().map(|(_, row)| row).collect();
    let value = serde_json::json!({ "model": id, "configured": rows });
    ctx.out.report_ok(&value, |lines| {
        for (name, _) in &created {
            lines.info(format!("created configured model {name} of {id}"));
        }
        lines.hint(
            "chap-core sets prediction_periods for each run, so the value in the file is not \
             stored",
        );
        if first {
            lines.hint(sync_hint(id));
        }
    })
}
