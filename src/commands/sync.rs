//! `varde sync [--check]` — render the compose files from `.varde/`.

use crate::cli::SyncArgs;
use crate::commands::Ctx;
use crate::compose::{SyncReport, sync};
use crate::error::{ChapError, Result};
use crate::output::Report;
use crate::project::Project;
use std::path::Path;

/// Render `.varde/models.yaml` into the compose files, or with `--check`
/// report whether they are up to date and fail if they are not.
pub fn run(ctx: &Ctx, args: &SyncArgs) -> Result<()> {
    let (mut project, _lock) = ctx.project_mut()?;
    let registry = super::registry_for(ctx, Some(&project))?;
    let report = sync(&mut project, &registry, args.check)?;

    ctx.out
        .report(&report, |lines| say(&report, &project.dir, lines))?;
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

/// The closing lines: a line per file, then the totals.
///
/// Under `--check` the files that would change are what the reader asked
/// about, so they are info; after a real run they are background.
pub fn say(report: &SyncReport, dir: &Path, lines: &mut Report) {
    for warning in &report.warnings {
        lines.warning(warning.as_str());
    }
    for path in &report.written {
        let file = relative(dir, path);
        match report.check {
            true => lines.info(format!("would write {file}")),
            false => lines.hint(format!("wrote {file}")),
        };
    }
    for path in &report.removed {
        let file = relative(dir, path);
        match report.check {
            true => lines.info(format!("would remove {file}")),
            false => lines.hint(format!("removed {file}")),
        };
    }
    for path in &report.unchanged {
        lines.hint(format!("{} is unchanged", relative(dir, path)));
    }
    match report.drift {
        true => lines.info(report.summary()),
        false => lines.info(format!("in sync: {}", report.summary())),
    };
}

fn relative(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir).unwrap_or(path).display().to_string()
}

/// Print the one-line summary `varde up` shows when its sync changed
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
