//! `varde models configs update ID NAME [--set KEY=VALUE ...] [--unset KEY]
//! [--covariates a,b]`, and `update ID NAME` alone, which opens the form at
//! a terminal with the values the configured model has.
//!
//! How chap-core is asked is [`configs::replace`].

use super::{Session, open_form, require_terminal};
use crate::cli::ConfigsUpdateArgs;
use crate::commands::Ctx;
use crate::configs::{self, Config, Draft, options};
use crate::error::{ChapError, Result};

/// `varde models configs update`.
pub fn update(ctx: &Ctx, args: &ConfigsUpdateArgs) -> Result<()> {
    let sets = args
        .set
        .iter()
        .map(|text| options::parse_set(text))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(ChapError::Usage)?;
    let form = args.set.is_empty() && args.unset.is_empty() && args.covariates.is_none();
    if form {
        require_terminal(
            ctx,
            "give the new values with --set KEY=VALUE, --unset KEY or --covariates, or run it \
             at a terminal for the form",
        )?;
    }
    let session = Session::open(ctx, &args.id)?;
    let id = session.target.id.clone();
    let current = session.live(&args.name)?.clone();
    let template = &session.template;

    let mut start = merged(&current, &sets, &args.unset, template, &id)?;
    if let Some(text) = &args.covariates {
        start.covariates = configs::parse_covariates(text);
    }
    let others: Vec<Config> = session
        .existing
        .iter()
        .filter(|config| config.id != current.id)
        .cloned()
        .collect();
    let draft = match form {
        true => {
            let filled = crate::tui::form::form_for_update(template, &current);
            match open_form(&id, filled, template, &others)? {
                Some(draft) => draft,
                None => return super::left_form(ctx, &id),
            }
        }
        false => start,
    };
    configs::check(&draft, template, &others, &id).map_err(ChapError::Usage)?;

    if draft.values == current.values && draft.covariates == current.covariates {
        return ctx.out.report(
            &serde_json::json!({ "model": id, "changed": false, "configured": current }),
            |lines| {
                lines.info(format!(
                    "configured model {} of {id} has these values already; nothing changed",
                    current.variant
                ));
            },
        );
    }

    let (created, archived) = configs::replace(&session.api, template, current.id, &draft)?;
    let value = serde_json::json!({
        "model": id,
        "changed": true,
        "configured": created,
        "archived": current,
    });
    ctx.out.report_ok(&value, |lines| {
        lines.info(configs::updated_line(&current.variant, &id));
        if let Err(err) = &archived {
            lines.warning(configs::not_archived(current.id, err));
        }
    })
}

/// The values of `current` with `sets` on top and `unsets` left out.
///
/// An unset key must be an option of the template or a value it has; the
/// model then uses its own default.
pub fn merged(
    current: &Config,
    sets: &[(String, String)],
    unsets: &[String],
    template: &configs::Template,
    model: &str,
) -> Result<Draft> {
    let mut values = current.values.clone();
    for key in unsets {
        let known = template.options.iter().any(|option| &option.key == key);
        if values.remove(key).is_none() && !known {
            return Err(ChapError::Usage(format!(
                "`{key}` is not an option of {model}; its options are {}",
                options::options_list(&template.options)
            ))
            .into());
        }
    }
    let given = options::values_of(&template.options, sets, model).map_err(ChapError::Usage)?;
    for (key, value) in given {
        if unsets.contains(&key) {
            return Err(ChapError::Usage(format!(
                "`{key}` is given to --set and to --unset; give it to one"
            ))
            .into());
        }
        values.insert(key, value);
    }
    Ok(Draft {
        variant: current.variant.clone(),
        values,
        covariates: current.covariates.clone(),
    })
}
