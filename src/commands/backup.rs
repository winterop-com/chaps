//! `chaps backup create` — write a deployment into one `tar.gz`.
//!
//! Four parts, each skippable: the project files (a plain copy), the chap-core
//! database (`pg_dump -Fc` through the running postgres container), one tar
//! per model data volume (read by the overlay's one-shot init container, which
//! mounts the same volume the model does) and one tar per component data
//! volume (read through a busybox container, since no component one-shot
//! mounts a volume the archive holds). Everything is staged under
//! `.chaps/tmp/`, packed in one `tar -czf` into a temporary sibling of the
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
    self, COMPONENTS_MEMBER, FILES_MEMBER, MANIFEST_MEMBER, MODELS_MEMBER, Manifest,
    ManifestComponent, ManifestModel, Stage,
};
use crate::cli::BackupCreateArgs;
use crate::commands::Ctx;
use crate::error::Result;
use crate::output::{self, Out};
use crate::project::Project;
use capture::{capture_components, capture_models, dump_database};
use quiesce::Running;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The shape of `chaps backup create --json`.
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
pub fn run(ctx: &Ctx, args: &BackupCreateArgs) -> Result<()> {
    let project = ctx.project()?;
    let out = destination(&project, args.out.as_deref())?;

    let stage = Stage::new(&project.chaps_dir(), "backup")?;
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

    let mut running = Running::new(&project);
    let no_chap_core = !project
        .state
        .components
        .is_enabled(crate::components::Component::ChapCore);
    let database = if args.no_db || no_chap_core {
        None
    } else {
        let dumped = dump_database(&project, &stage, &mut running)?;
        // The dump is `db/chap_core.dump`; the directory is what tar is given.
        members.push("db".to_string());
        Some(dumped)
    };

    let models = capture_models(&project, &stage, &mut running, args.no_models)?;
    if models.iter().any(|m| m.path.is_some()) {
        members.push(MODELS_MEMBER.to_string());
    }

    let components = capture_components(&project, &stage, &mut running, args.no_components)?;
    if components.iter().any(|c| c.path.is_some()) {
        members.push(COMPONENTS_MEMBER.to_string());
    }

    let manifest = Manifest {
        schema_version: backup::SCHEMA_VERSION,
        created_by: format!("chaps {}", ctx.cli_version),
        created_at: backup::timestamp(backup::now()),
        project: project_name(&project.dir),
        chap_image_tag: project.state.chap_image_tag.clone(),
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

    for model in manifest.models.iter().filter(|m| m.skipped.is_some()) {
        output::warn(&format!(
            "{}: {}",
            model.service_id,
            model.skipped.as_deref().unwrap_or("skipped")
        ));
    }
    for part in manifest.components.iter().filter(|c| c.skipped.is_some()) {
        output::warn(&format!(
            "{}: {}",
            part.name,
            part.skipped.as_deref().unwrap_or("skipped")
        ));
    }

    let report = BackupReport {
        size_bytes: backup::file_size(&out),
        path: out,
        manifest,
        no_chap_core,
    };
    ctx.out.emit(&report, || human(&report, &ctx.out))
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

/// Where the archive lands, as an absolute path.
fn destination(project: &Project, out: Option<&Path>) -> Result<PathBuf> {
    let cwd = std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("reading the working directory: {e}"))?;
    let name = backup::archive_name(&project_name(&project.dir), &backup::stamp(backup::now()));
    let path = backup::resolve_out_path(out, out.is_some_and(Path::is_dir), &cwd, &name);
    Ok(std::path::absolute(&path).unwrap_or(path))
}

/// The project directory's own name, which names the archive and identifies
/// the deployment in the manifest.
fn project_name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "chaps".to_string())
}

/// What went in, with sizes, and where it landed.
fn human(report: &BackupReport, out: &Out) -> String {
    let manifest = &report.manifest;
    let content = manifest.content_bytes();
    let mut text = format!(
        "{}  {}  {}\n\n",
        out.heading("backup"),
        out.value(&report.path.display().to_string()),
        out.dim(&format!(
            "({} gzipped, {} of data)",
            backup::human_size(report.size_bytes),
            backup::human_size(content)
        ))
    );

    text.push_str(&format!("{}\n", out.heading("included")));
    if manifest.files.is_empty() {
        text.push_str(&format!("  {}     {}\n", out.key("files"), out.dim("none")));
    } else {
        text.push_str(&format!(
            "  {}     {} {}\n",
            out.key("files"),
            format_args!("{} file(s):", manifest.files.len()),
            out.dim(&manifest.files.join(", "))
        ));
    }
    match &manifest.database {
        Some(db) => text.push_str(&format!(
            "  {}  {} as {} {}\n",
            out.key("database"),
            db.name,
            db.user,
            out.dim(&format!(
                "({}){}",
                backup::human_size(db.size_bytes),
                db.server_version
                    .as_deref()
                    .map(|v| format!(", PostgreSQL {v}"))
                    .unwrap_or_default()
            ))
        )),
        None => text.push_str(&format!(
            "  {}  {}\n",
            out.key("database"),
            out.dim(if report.no_chap_core {
                "none (this deployment has no chap-core)"
            } else {
                "not included (--no-db)"
            })
        )),
    }
    let captured: Vec<&ManifestModel> = manifest.captured_models().collect();
    if captured.is_empty() {
        text.push_str(&format!("  {}    {}\n", out.key("models"), out.dim("none")));
    } else {
        for (i, model) in captured.iter().enumerate() {
            let label = if i == 0 {
                format!("  {}  ", out.key("models"))
            } else {
                "          ".to_string()
            };
            text.push_str(&format!(
                "{label}  {}  {}  {}\n",
                model.service_id,
                model.data_dir,
                out.dim(&size_note(model.size_bytes, model.quiesce.as_deref()))
            ));
        }
    }

    let parts: Vec<&ManifestComponent> = manifest.captured_components().collect();
    if parts.is_empty() {
        text.push_str(&format!("  {}     {}\n", out.key("parts"), out.dim("none")));
    } else {
        for (i, part) in parts.iter().enumerate() {
            let label = if i == 0 {
                format!("  {}   ", out.key("parts"))
            } else {
                "          ".to_string()
            };
            text.push_str(&format!(
                "{label}  {}  {}  {}\n",
                part.name,
                part.data_dir,
                out.dim(&size_note(part.size_bytes, part.quiesce.as_deref()))
            ));
        }
    }

    let skipped: Vec<(&str, &str)> = manifest
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
    if !skipped.is_empty() {
        text.push_str(&format!("\n{}\n", out.heading("skipped")));
        for (name, why) in skipped {
            text.push_str(&format!("  {name}  {}\n", out.warn(why)));
        }
    }
    // What to do with it: the one command that reads it back. The path is
    // the one the reader just saw, and a restore prints what it would
    // overwrite and asks before it does anything.
    text.push_str(&format!(
        "\nrestore it with {}\n",
        out.cmd(&format!("`chaps backup restore {}`", report.path.display()))
    ));
    text
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
