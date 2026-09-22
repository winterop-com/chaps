//! `chaps sync [--check]` — render the compose files from `.chaps/`.

use crate::cli::SyncArgs;
use crate::commands::Ctx;
use crate::compose::{SyncReport, sync};
use crate::error::{ChapError, Result};
use crate::project::Project;
use crate::registry;
use std::path::Path;

/// Render `.chaps/models.yaml` into the compose files, or with `--check`
/// report whether they are up to date and fail if they are not.
pub fn run(ctx: &Ctx, args: &SyncArgs) -> Result<()> {
    let mut project = ctx.project()?;
    let registry = registry::load(&ctx.registry)?;
    let report = sync(&mut project, &registry, ctx.cli_version, args.check)?;

    ctx.out.emit(&report, || human(&report, &project.dir))?;
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
pub fn human(report: &SyncReport, dir: &Path) -> String {
    let mut text = String::new();
    for warning in &report.warnings {
        text.push_str(&format!("warning: {warning}\n"));
    }
    let (write, remove) = if report.check {
        ("would write ", "would remove")
    } else {
        ("written     ", "removed     ")
    };
    for path in &report.written {
        text.push_str(&format!("{write}  {}\n", relative(dir, path)));
    }
    for path in &report.removed {
        text.push_str(&format!("{remove}  {}\n", relative(dir, path)));
    }
    for path in &report.unchanged {
        text.push_str(&format!("unchanged     {}\n", relative(dir, path)));
    }
    if report.drift {
        text.push_str(&report.summary());
    } else {
        text.push_str("in sync: ");
        text.push_str(&report.summary());
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
        println!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn report(check: bool) -> SyncReport {
        SyncReport {
            written: vec![PathBuf::from("/p/compose.chapkit-ewars-model.yml")],
            unchanged: vec![PathBuf::from("/p/compose.marketplace.yml")],
            removed: vec![PathBuf::from("/p/compose.auto-arima-chapkit.yml")],
            drift: true,
            check,
            warnings: vec![],
        }
    }

    #[test]
    fn human_lists_files_relative_to_the_project() {
        let text = human(&report(false), Path::new("/p"));
        assert_eq!(
            text,
            "written       compose.chapkit-ewars-model.yml\n\
             removed       compose.auto-arima-chapkit.yml\n\
             unchanged     compose.marketplace.yml\n\
             1 written, 1 unchanged, 1 removed\n"
        );
    }

    #[test]
    fn check_mode_uses_the_conditional() {
        let text = human(&report(true), Path::new("/p"));
        assert!(text.starts_with("would write   compose.chapkit-ewars-model.yml\n"));
        assert!(text.contains("would remove  compose.auto-arima-chapkit.yml\n"));
        assert!(text.ends_with("1 to write, 1 unchanged, 1 to remove\n"));
    }

    #[test]
    fn a_clean_check_says_in_sync() {
        let clean = SyncReport {
            unchanged: vec![PathBuf::from("/p/compose.marketplace.yml")],
            check: true,
            ..SyncReport::default()
        };
        let text = human(&clean, Path::new("/p"));
        assert!(text.ends_with("in sync: 0 to write, 1 unchanged, 0 to remove\n"));
    }
}
