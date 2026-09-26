//! `chaps registry update|show`.

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
    ctx.out.emit(&report, || {
        format!(
            "{}\n{}",
            ctx.out.ok("updated the registry"),
            output::fields_with(
                2,
                &[
                    ("url", report.url.clone()),
                    ("source", report.provenance.describe()),
                    ("models", report.models.to_string()),
                    (
                        "cache",
                        ctx.out.dim(&report.cache_dir.display().to_string())
                    ),
                ],
                &|label| ctx.out.key(label),
            )
        )
    })
}

/// Report the catalogue the rest of the CLI would use right now, without
/// forcing a refresh.
fn show(ctx: &Ctx) -> Result<()> {
    let registry = registry::load(&ctx.registry)?;
    let report = report(ctx, &registry, true);
    ctx.out.emit(&report, || {
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
        // A list of ids is not a statement about the catalogue, and an empty
        // one says nothing at all.
        text.push_str(&format!("\n{}\n", ctx.out.cmd(&catalogue_line(&report))));
        text
    })
}

/// The line `registry show` ends on: what the catalogue holds, where it came
/// from, and how to move it on.
fn catalogue_line(report: &RegistryReport) -> String {
    let source = report.provenance.describe();
    match report.models {
        0 => format!(
            "the catalogue ({source}) holds no models; \
             run `chaps registry update` to fetch it again"
        ),
        1 => format!("1 model in the catalogue ({source}); `chaps models info ID` describes one"),
        count => {
            format!(
                "{count} models in the catalogue ({source}); `chaps models info ID` describes one"
            )
        }
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
mod tests {
    use super::*;
    use crate::output::Out;

    fn ctx(cache_dir: &std::path::Path) -> Ctx {
        Ctx {
            out: Out::default(),
            project_dir: std::path::PathBuf::from("."),
            registry: registry::RegistryOptions {
                url: "https://example.test/registry.yaml".to_string(),
                offline: true,
                cache_dir: cache_dir.to_path_buf(),
                timeout: registry::DEFAULT_TIMEOUT,
            },
            cli_version: "0.0.0-test",
        }
    }

    #[test]
    fn the_report_names_the_cache_entry_for_this_url() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ctx(tmp.path());
        let registry = registry::load_embedded().unwrap();
        let report = report(&ctx, &registry, true);

        assert_eq!(report.models, registry.models.len());
        assert_eq!(report.ids.as_ref().unwrap().len(), registry.models.len());
        assert_eq!(report.ids.as_ref().unwrap()[0], "chapkit_ewars_model");
        assert!(matches!(report.provenance, Provenance::Embedded));
        assert!(
            report.cache_dir.starts_with(tmp.path().join("registries")),
            "{}",
            report.cache_dir.display()
        );
    }

    #[test]
    fn update_only_reports_the_ids_on_show() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ctx(tmp.path());
        let registry = registry::load_embedded().unwrap();

        let value = serde_json::to_value(report(&ctx, &registry, false)).unwrap();
        assert!(value.get("ids").is_none(), "update omits the id list");
        assert_eq!(value["models"], registry.models.len());
        assert_eq!(value["provenance"]["kind"], "embedded");

        let value = serde_json::to_value(report(&ctx, &registry, true)).unwrap();
        assert_eq!(
            value["ids"].as_array().unwrap().len(),
            registry.models.len()
        );
    }
}
