//! `chaps sync [--check]` — render the compose files from `.chaps/`.

use crate::cli::SyncArgs;
use crate::commands::Ctx;
use crate::compose::{SyncReport, sync};
use crate::error::{ChapError, Result};
use crate::output::Out;
use crate::project::Project;
use std::path::Path;

/// Render `.chaps/models.yaml` into the compose files, or with `--check`
/// report whether they are up to date and fail if they are not.
pub fn run(ctx: &Ctx, args: &SyncArgs) -> Result<()> {
    let (mut project, _lock) = ctx.project_mut()?;
    let registry = super::registry_for(ctx, Some(&project))?;
    let report = sync(&mut project, &registry, args.check)?;

    ctx.out
        .emit(&report, || human(&report, &project.dir, &ctx.out))?;
    if !(args.check && report.drift) {
        return Ok(());
    }
    // The JSON report already says `drift: true`; a second JSON document on
    // stdout would break single-document parsers, so the exit code alone
    // signals it there.
    if ctx.out.json {
        std::process::exit(1);
    }
    Err(ChapError::OutOfSync.into())
}

/// One line per file, then the totals.
///
/// The verb carries the colour: written is a change that landed, removed is a
/// file that is gone, unchanged is the part nobody has to read.
pub fn human(report: &SyncReport, dir: &Path, out: &Out) -> String {
    let mut text = String::new();
    for warning in &report.warnings {
        text.push_str(&format!("{} {warning}\n", out.warn("warning:")));
    }
    let (write, remove) = if report.check {
        ("would write ", "would remove")
    } else {
        ("written     ", "removed     ")
    };
    for path in &report.written {
        text.push_str(&format!(
            "{}  {}\n",
            out.ok(write),
            out.dim(&relative(dir, path))
        ));
    }
    for path in &report.removed {
        text.push_str(&format!(
            "{}  {}\n",
            out.bad(remove),
            out.dim(&relative(dir, path))
        ));
    }
    for path in &report.unchanged {
        text.push_str(&format!(
            "{}     {}\n",
            out.dim("unchanged"),
            out.dim(&relative(dir, path))
        ));
    }
    if report.drift {
        text.push_str(&out.cmd(&report.summary()));
    } else {
        text.push_str(&out.ok("in sync: "));
        text.push_str(&out.cmd(&report.summary()));
    }
    text.push('\n');
    text
}

fn relative(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir).unwrap_or(path).display().to_string()
}

/// Print the one-line summary `chaps up` shows when its sync changed
/// something. Goes to stderr under `--json` so stdout stays docker's.
pub fn announce(ctx: &Ctx, report: &SyncReport, project: &Project) {
    for warning in &report.warnings {
        crate::output::warn(warning);
    }
    if !report.drift {
        return;
    }
    let files: Vec<String> = report
        .written
        .iter()
        .map(|p| relative(&project.dir, p))
        .chain(
            report
                .removed
                .iter()
                .map(|p| format!("-{}", relative(&project.dir, p))),
        )
        .collect();
    let line = format!(
        "synced compose files ({}): {}",
        report.summary(),
        files.join(", ")
    );
    if ctx.out.json {
        eprintln!("{line}");
    } else {
        println!("{}", ctx.out.dim(&line));
    }
}

#[cfg(test)]
mod tests;
