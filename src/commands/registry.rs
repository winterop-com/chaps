//! `varde registry update|show`.

use crate::cli::RegistryCmd;
use crate::commands::Ctx;
use crate::error::Result;
use crate::output;
use crate::registry::{self, Provenance, Registry};
use std::path::PathBuf;

/// What both subcommands report, and the shape of their `--json`.
#[derive(Debug, serde::Serialize)]
struct RegistryReport {
    url: String,
    provenance: Provenance,
    /// Directory holding this registry's cache entry, whether or not it exists.
    cache_dir: PathBuf,
    models: usize,
    /// Marketplace ids, in index order. Only `show` fills this in.
    #[serde(skip_serializing_if = "Option::is_none")]
    ids: Option<Vec<String>>,
}

/// Refresh the cached registry, or report what is currently loaded and where
/// it came from.
pub fn run(ctx: &Ctx, cmd: &RegistryCmd) -> Result<()> {
    match cmd {
        RegistryCmd::Update(_) => update(ctx),
        RegistryCmd::Show(_) => show(ctx),
    }
}

/// Force a network refresh. Failure is an error, not a fallback: the point of
/// the command is to find out whether the refresh worked.
fn update(ctx: &Ctx) -> Result<()> {
    let registry = registry::update(&ctx.registry)?;
    let report = report(ctx, &registry, false);
    ctx.out.report(&report, |lines| updated(&report, lines))
}

/// The lines of `registry update`: the count, then where it came from.
fn updated(report: &RegistryReport, lines: &mut output::Report) {
    lines
        .info(format!(
            "updated the marketplace registry: {}",
            models(report.models)
        ))
        .hint(format!("fetched from {}", report.url))
        .hint(format!("cached in {}", report.cache_dir.display()));
}

/// Report the catalogue the rest of the CLI would use right now, without
/// forcing a refresh.
fn show(ctx: &Ctx) -> Result<()> {
    let registry = registry::load(&ctx.registry)?;
    let report = report(ctx, &registry, true);
    // The fields and the ids are the table this command shows; the closing
    // lines come after it.
    if !ctx.out.json {
        let mut text = output::fields_with(
            2,
            &[
                ("url", report.url.clone()),
                ("source", report.provenance.describe()),
                ("models", report.models.to_string()),
                (
                    "cache",
                    ctx.out.dim(&report.cache_dir.display().to_string()),
                ),
            ],
            &|label| ctx.out.key(label),
        );
        for (i, id) in report.ids.iter().flatten().enumerate() {
            if i == 0 {
                text.push('\n');
            }
            text.push_str(&format!("  {}\n", ctx.out.dim(id)));
        }
        println!("{text}");
    }
    ctx.out.report(&report, |lines| shown(&report, lines))
}

/// The lines `registry show` ends on: what the catalogue holds and where it
/// came from.
fn shown(report: &RegistryReport, lines: &mut output::Report) {
    let source = report.provenance.describe();
    match report.models {
        0 => lines.info(format!(
            "the catalogue ({source}) holds no models; \
             run `varde registry update` to fetch it again"
        )),
        count => lines
            .info(format!("{} in the catalogue ({source})", models(count)))
            .hint("`varde models info ID` describes one"),
    };
}

fn models(count: usize) -> String {
    match count {
        1 => "1 model".to_string(),
        n => format!("{n} models"),
    }
}

fn report(ctx: &Ctx, registry: &Registry, with_ids: bool) -> RegistryReport {
    RegistryReport {
        url: registry.url.clone(),
        provenance: registry.provenance.clone(),
        cache_dir: registry::cache::dir_for(&ctx.registry),
        models: registry.models.len(),
        ids: with_ids.then(|| registry.models.iter().map(|m| m.id.clone()).collect()),
    }
}

#[cfg(test)]
mod tests;
