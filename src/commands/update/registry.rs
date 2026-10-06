//! `varde update` where there is no deployment: no pin to move, but the
//! marketplace registry is this machine's, and it can be refreshed.

use crate::cli::UpdateArgs;
use crate::commands::Ctx;
use crate::error::{ChapError, Result};
use crate::output;
use crate::registry as marketplace;
use serde::Serialize;

/// What `varde update` did without a deployment, for `--json`.
#[derive(Debug, Serialize)]
struct RegistryRefresh {
    /// Whether the registry was fetched now; `false` under `--dry-run`.
    refreshed: bool,
    /// Where the registry came from.
    source: String,
    /// How many models it lists.
    models: usize,
}

/// Refresh the registry and say so.
pub(super) fn refresh(ctx: &Ctx, args: &UpdateArgs) -> Result<()> {
    if let Some(flag) = deployment_flag(args) {
        return Err(ChapError::Usage(format!(
            "{flag} moves the pins of a deployment, and this directory is not one; run it in \
             a deployment's directory or with `-C DIR`, or create one with `varde init`"
        ))
        .into());
    }
    if ctx.registry.offline && !args.dry_run {
        return Err(ChapError::Usage(
            "outside a deployment, `varde update` refreshes the marketplace registry, which \
             needs the network; drop --offline, or use --dry-run to see the cached one"
                .to_string(),
        )
        .into());
    }
    let loaded = match args.dry_run {
        true => marketplace::load(&ctx.registry)?,
        false => marketplace::update(&ctx.registry)?,
    };
    let report = RegistryRefresh {
        refreshed: !args.dry_run,
        source: loaded.provenance.describe(),
        models: loaded.models.len(),
    };
    ctx.out.report(&report, |lines| say(&report, lines))
}

/// The flag that only means something in a deployment, when one was given.
fn deployment_flag(args: &UpdateArgs) -> Option<&'static str> {
    if args.chap_tag.is_some() {
        Some("--chap-tag")
    } else if args.pin_chap_core {
        Some("--pin-chap-core")
    } else if args.list_tags {
        Some("--list-tags")
    } else {
        None
    }
}

fn say(report: &RegistryRefresh, lines: &mut output::Report) {
    let models = match report.models {
        1 => "1 model".to_string(),
        n => format!("{n} models"),
    };
    match report.refreshed {
        true => lines.info(format!("updated the marketplace registry: {models}")),
        false => lines.info(format!(
            "the marketplace registry has {models} ({}); --dry-run fetched nothing",
            report.source
        )),
    };
    lines
        .hint("this directory is not a deployment, so no version pin changed")
        .hint("`varde self update` updates varde itself");
}
