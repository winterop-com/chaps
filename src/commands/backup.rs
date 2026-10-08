//! `varde backup create` — write a deployment into one `tar.gz`.
//!
//! Four parts, each skippable: the project files (a plain copy), the chap-core
//! database (`pg_dump -Fc` through the running postgres container), one tar
//! per model data volume (read by the overlay's one-shot init container, which
//! mounts the same volume the model does) and one tar per component data
//! volume (read through a busybox container, since no component one-shot
//! mounts a volume the archive holds). Everything is staged under
//! `.varde/tmp/`, packed in one `tar -czf` into a temporary sibling of the
//! destination and renamed into place, so a failure halfway leaves no
//! half-written archive and any archive already at that path exactly as it was.
//!
//! A service that is running is paused for the seconds its volume takes to
//! read: a model keeps a live SQLite database in there, and tar reading a file
//! that is being written to produces a tar of a torn database. `pg_dump` needs
//! none of that - it reads one transactional snapshot - and a service that is
//! not running cannot write, so neither is disturbed.

mod capture;
mod quiesce;

use crate::backup::{
    self, COMPONENTS_MEMBER, FILES_MEMBER, MANIFEST_MEMBER, MODELS_MEMBER, Manifest, Stage,
};
use crate::cli::BackupCreateArgs;
use crate::commands::Ctx;
use crate::error::Result;
use crate::output;
use crate::project::Project;
use capture::{capture_components, capture_models, dump_database};
use quiesce::Running;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The shape of `varde backup create --json`.
#[derive(Debug, Serialize)]
pub struct BackupReport {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub manifest: Manifest,
    /// The deployment has no chap-core, so there was no database to dump.
    #[serde(skip)]
    pub no_chap_core: bool,
}

/// Stage, capture and pack.
///
/// Ctrl-C stops the run at the next step: the service held still is let go,
/// no archive is written and the stage is removed. A second Ctrl-C exits at
/// once, and `varde up` then resumes what is still paused.
pub fn run(ctx: &Ctx, args: &BackupCreateArgs) -> Result<()> {
    let project = ctx.project()?;
    let out = destination(&project, args.out.as_deref())?;
    crate::interrupt::install();
    let mut running = Running::new(&project);
    let result = create(ctx, &project, &out, &mut running, args);
    match result {
        Err(_) if crate::interrupt::requested() => {
            Err(interrupted(&running.resumed, &running.stuck))
        }
        result => result,
    }
}

/// The error of a run that Ctrl-C stopped: what it leaves behind. `resumed`
/// are the services it paused and let go again, `stuck` the ones it could
/// not let go.
fn interrupted(resumed: &[String], stuck: &[String]) -> anyhow::Error {
    let mut said = vec!["no archive was written".to_string()];
    match resumed.len() {
        0 => {}
        1 => said.push(format!(
            "{} was paused for the backup and runs again",
            resumed.join(", ")
        )),
        _ => said.push(format!(
            "{} were paused for the backup and run again",
            resumed.join(", ")
        )),
    }
    if !stuck.is_empty() {
        said.push(format!(
            "{} could not be started again; run `varde up` to resume {}",
            stuck.join(", "),
            if stuck.len() == 1 { "it" } else { "them" }
        ));
    }
    crate::error::ChapError::Interrupted(said.join("; ")).into()
}

/// [`run`] up to the report, with the stage that is removed when it returns.
fn create(
    ctx: &Ctx,
    project: &Project,
    out: &Path,
    running: &mut Running,
    args: &BackupCreateArgs,
) -> Result<()> {
    let out = out.to_path_buf();
    let stage = Stage::new(&project.varde_dir(), "backup")?;
    let mut members = vec![MANIFEST_MEMBER.to_string()];

    // Files. Always: without them the archive says nothing about what it is a
    // backup of, and they are the cheapest part by orders of magnitude.
    let files = backup::project_files(&project.dir);
    for rel in &files {
        let dst = stage.path(&format!("{FILES_MEMBER}/{rel}"))?;
        std::fs::copy(backup::join_relative(&project.dir, rel), &dst)
            .map_err(|e| anyhow::anyhow!("copying {rel}: {e}"))?;
    }
    if !files.is_empty() {
        members.push(FILES_MEMBER.to_string());
    }

    let no_chap_core = !project
        .state
        .components
        .is_enabled(crate::components::Component::ChapCore);
    let database = if args.no_db || no_chap_core {
        None
    } else {
        let dumped = dump_database(project, &stage, running)?;
        // The dump is `db/chap_core.dump`; the directory is what tar is given.
        members.push("db".to_string());
        Some(dumped)
    };

    let models = capture_models(project, &stage, running, args.no_models)?;
    if models.iter().any(|m| m.path.is_some()) {
        members.push(MODELS_MEMBER.to_string());
    }

    let components = capture_components(project, &stage, running, args.no_components)?;
    if components.iter().any(|c| c.path.is_some()) {
        members.push(COMPONENTS_MEMBER.to_string());
    }

    // Stopped part-way: the archive would not hold what was asked for.
    if crate::interrupt::requested() {
        return Err(anyhow::anyhow!("stopped by Ctrl-C"));
    }

    let manifest = Manifest {
        schema_version: backup::SCHEMA_VERSION,
        created_by: format!("varde {}", ctx.cli_version),
        created_at: backup::timestamp(backup::now()),
        project: project_name(&project.dir),
        chap_image_tag: (!no_chap_core).then(|| project.state.chap_image_tag.clone()),
        files: files.clone(),
        database,
        models,
        components,
    };
    std::fs::write(
        stage.dir.join(MANIFEST_MEMBER),
        backup::render_manifest(&manifest)?,
    )
    .map_err(|e| anyhow::anyhow!("writing the manifest: {e}"))?;

    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", parent.display()))?;
    }
    write_archive(&out, &stage.dir, &members)?;

    // A volume that could not be read is a backup that is not whole: the
    // archive stays, for what it does hold, and the run fails so a cron job
    // or a script hears about it.
    let failed = failed_reads(&manifest);
    if !failed.is_empty() {
        for (name, why) in skip_reasons(&manifest) {
            output::warn(&format!("{name}: {why}"));
        }
        return Err(anyhow::anyhow!(
            "{} was written without the data of {} (see the warnings above); fix what stopped \
             the read, then `varde backup create` again",
            out.display(),
            failed.join(", ")
        ));
    }

    let report = BackupReport {
        size_bytes: backup::file_size(&out),
        path: out,
        manifest,
        no_chap_core,
    };
    ctx.out.report(&report, |lines| say(&report, lines))
}

/// The models and volumes whose read failed, as opposed to data there was no
/// reason to read: a model never started has nothing to lose.
fn failed_reads(manifest: &backup::Manifest) -> Vec<&str> {
    manifest
        .models
        .iter()
        .filter(|m| m.failed)
        .map(|m| m.service_id.as_str())
        .chain(
            manifest
                .components
                .iter()
                .filter(|c| c.failed)
                .map(|c| c.volume.as_str()),
        )
        .collect()
}

/// Pack the stage into a temporary sibling of `out` and rename it into place.
///
/// tar writes straight into the file it is given, so packing into `out` would
/// truncate whatever is there before it knows whether it can finish: a backup
/// that fails halfway would take last night's with it. The temporary file is
/// removed on failure, and `out` is then exactly as it was found - missing, or
/// the older archive, untouched.
fn write_archive(out: &Path, stage: &Path, members: &[String]) -> Result<()> {
    let tmp = backup::temp_archive_path(out);
    let packed = backup::tar_create(&tmp, stage, members).and_then(|()| {
        std::fs::rename(&tmp, out)
            .map_err(|e| anyhow::anyhow!("renaming {} to {}: {e}", tmp.display(), out.display()))
    });
    if packed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    packed
}

/// Each model and component that the archive does not hold, and why.
///
/// A component with several volumes has an entry per volume under one name,
/// so the same reason for each of them is one line: `dhis2: not included
/// (--no-components)` once, not once per volume.
fn skip_reasons(manifest: &Manifest) -> Vec<(&str, &str)> {
    let mut reasons: Vec<(&str, &str)> = manifest
        .models
        .iter()
        .filter_map(|m| Some((m.service_id.as_str(), m.skipped.as_deref()?)))
        .chain(
            manifest
                .components
                .iter()
                .filter_map(|c| Some((c.name.as_str(), c.skipped.as_deref()?))),
        )
        .collect();
    reasons.dedup();
    reasons
}

/// Where the archive lands, as an absolute path.
fn destination(project: &Project, out: Option<&Path>) -> Result<PathBuf> {
    let cwd = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("reading the working directory: {e}"))?;
    let name = backup::archive_name(&project_name(&project.dir), &backup::stamp(backup::now()));
    let mut path = backup::resolve_out_path(out, out.is_some_and(Path::is_dir), &cwd, &name);
    if out != Some(path.as_path()) {
        path = backup::unused_path(path, Path::exists);
    }
    // Absolute and without `..`: the path is printed, and `chapx/../archives`
    // is not how anyone names that directory.
    Ok(crate::ports::real_path(&path))
}

/// The project directory's own name, which names the archive and identifies
/// the deployment in the manifest.
fn project_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "varde".to_string())
}

/// Where the archive landed, then what went in, with sizes, under `-v`.
fn say(report: &BackupReport, lines: &mut output::Report) {
    let manifest = &report.manifest;
    lines.info(format!(
        "wrote {} ({} gzipped, {} of data)",
        report.path.display(),
        backup::human_size(report.size_bytes),
        backup::human_size(manifest.content_bytes())
    ));
    if !manifest.files.is_empty() {
        lines.hint(format!("files: {}", manifest.files.join(", ")));
    }
    lines.hint(match &manifest.database {
        Some(db) => format!(
            "database: {} as {} ({}){}",
            db.name,
            db.user,
            backup::human_size(db.size_bytes),
            db.server_version
                .as_deref()
                .map(|v| format!(", PostgreSQL {v}"))
                .unwrap_or_default()
        ),
        None if report.no_chap_core => {
            "database: none (this deployment has no chap-core)".to_string()
        }
        None => "database: not included (--no-db)".to_string(),
    });
    for model in manifest.captured_models() {
        lines.hint(format!(
            "model {}: {} {}",
            model.service_id,
            model.data_dir,
            size_note(model.size_bytes, model.quiesce.as_deref())
        ));
    }
    for part in manifest.captured_components() {
        let what = match part.path.as_deref().is_some_and(backup::is_dhis2_db_dump) {
            true => format!("the database as a {}", backup::DHIS2_DB_DUMP_SOURCE),
            false => part.data_dir.clone(),
        };
        lines.hint(format!(
            "component {}: {what} {}",
            part.name,
            size_note(part.size_bytes, part.quiesce.as_deref())
        ));
    }
    for (name, why) in skip_reasons(manifest) {
        // A part left out by a flag is what the reader asked for; any other
        // reason is data that the archive does not hold.
        match why.starts_with("--") {
            true => lines.hint(format!("{name}: not included ({why})")),
            false => lines.warning(format!("{name}: {why}")),
        };
    }
    lines.hint(format!(
        "`varde backup restore {}` restores it",
        report.path.display()
    ));
}

/// `(40.0 KB)`, and how long the service was held still when it was:
/// `(40.0 KB, paused for 1.4 s)`.
fn size_note(size_bytes: u64, quiesce: Option<&str>) -> String {
    match quiesce {
        Some(held) => format!("({}, {held})", backup::human_size(size_bytes)),
        None => format!("({})", backup::human_size(size_bytes)),
    }
}

#[cfg(test)]
mod tests;
