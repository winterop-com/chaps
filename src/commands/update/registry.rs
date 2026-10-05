//! `chaps update` where there is no deployment: no pin to move, but the
//! marketplace registry is this machine's, and it can be refreshed.

use crate::cli::UpdateArgs;
use crate::commands::Ctx;
use crate::error::{ChapError, Result};
use crate::output;
use crate::registry as marketplace;
use serde::Serialize;
use std::path::PathBuf;

/// What `chaps update` did without a deployment, for `--json`.
#[derive(Debug, Serialize)]
struct RegistryRefresh {
    /// Whether the registry was fetched now; `false` under `--dry-run`.
    refreshed: bool,
    /// Where the registry came from.
    source: String,
    /// How many models it lists.
    models: usize,
    /// The `chaps run` groups on this machine, which are deployments with
    /// pins of their own.
    groups: Vec<PathBuf>,
}

/// Refresh the registry, say that no pin moved, and name the commands that
/// update the rest.
pub(super) fn refresh(ctx: &Ctx, args: &UpdateArgs) -> Result<()> {
    if let Some(flag) = deployment_flag(args) {
        return Err(ChapError::Usage(format!(
            "{flag} moves the pins of a deployment, and this directory is not one; run it in \
             a deployment's directory or with `-C DIR`, or create one with `chaps init`"
        ))
        .into());
    }
    if ctx.registry.offline && !args.dry_run {
        return Err(ChapError::Usage(
            "outside a deployment, `chaps update` refreshes the marketplace registry, which \
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
        groups: crate::commands::run::groups()
            .into_iter()
            .map(|(_, dir)| dir)
            .collect(),
    };
    ctx.out.emit(&report, || human(ctx, &report))
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

fn human(ctx: &Ctx, report: &RegistryRefresh) -> String {
    let out = &ctx.out;
    let mut text = match report.refreshed {
        true => format!(
            "{} the marketplace registry: {} models ({})\n",
            out.ok("updated"),
            report.models,
            report.source
        ),
        false => format!(
            "the marketplace registry has {} models ({}); --dry-run fetched nothing\n",
            report.models, report.source
        ),
    };
    text.push_str(
        "this is not a deployment, so no pin moved; `chaps self update` updates chaps itself\n",
    );
    for dir in &report.groups {
        text.push_str(&out.backticks(&format!(
            "`chaps -C {} update` moves the pins of that `chaps run` group\n",
            dir.display()
        )));
    }
    if report.groups.is_empty() {
        text.push_str(&output::wrapped(
            "`chaps update` in a deployment's directory moves its pins and pulls its images",
            100,
        ));
    }
    text
}
