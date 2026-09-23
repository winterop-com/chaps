//! `chaps backup create` — write a deployment into one `tar.gz`.
//!
//! Three parts, each skippable: the project files (a plain copy), the
//! chap-core database (`pg_dump -Fc` through the running postgres container)
//! and one tar per model data volume (read by the overlay's one-shot init
//! container, which mounts the same volume the model does). Everything is
//! staged under `.chaps/tmp/` and packed in one `tar -czf`, so a failure
//! halfway leaves no half-written archive at the destination.

use crate::backup::{
    self, DB_MEMBER, FILES_MEMBER, MANIFEST_MEMBER, MODELS_MEMBER, Manifest, ManifestDatabase,
    ManifestModel, Stage,
};
use crate::cli::BackupCreateArgs;
use crate::commands::Ctx;
use crate::compose::volume_name;
use crate::docker;
use crate::error::Result;
use crate::output;
use crate::project::{ENV_FILE, Project};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The shape of `chaps backup create --json`.
#[derive(Debug, Serialize)]
pub struct BackupReport {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub manifest: Manifest,
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

    let database = if args.no_db {
        None
    } else {
        let dumped = dump_database(&project, &stage)?;
        // The dump is `db/chap_core.dump`; the directory is what tar is given.
        members.push("db".to_string());
        Some(dumped)
    };

    let models = capture_models(&project, &stage, args.no_models)?;
    if models.iter().any(|m| m.path.is_some()) {
        members.push(MODELS_MEMBER.to_string());
    }

    let manifest = Manifest {
        schema_version: backup::SCHEMA_VERSION,
        created_by: format!("chaps-cli {}", ctx.cli_version),
        created_at: backup::timestamp(backup::now()),
        project: project_name(&project.dir),
        chap_image_tag: project.state.chap_image_tag.clone(),
        files: files.clone(),
        database,
        models,
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
    backup::tar_create(&out, &stage.dir, &members)?;

    for model in manifest.models.iter().filter(|m| m.skipped.is_some()) {
        output::warn(&format!(
            "{}: {}",
            model.service_id,
            model.skipped.as_deref().unwrap_or("skipped")
        ));
    }

    let report = BackupReport {
        size_bytes: backup::file_size(&out),
        path: out,
        manifest,
    };
    ctx.out.emit(&report, || human(&report))
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

/// `pg_dump -Fc` the chap-core database into the stage.
///
/// postgres has to be up: the dump goes through `docker compose exec`, which
/// needs a running container. `--no-db` is the way out when it is not.
fn dump_database(project: &Project, stage: &Stage) -> Result<ManifestDatabase> {
    if !docker::running_services(project).contains("postgres") {
        return Err(anyhow::anyhow!(
            "the postgres container is not running, so the database cannot be dumped; \
             start it with `chaps up`, or pass --no-db"
        ));
    }
    let env_body = std::fs::read_to_string(project.dir.join(ENV_FILE)).unwrap_or_default();
    let (user, name) = backup::postgres_credentials(&env_body);

    let dest = stage.path(DB_MEMBER)?;
    let args = compose_exec("postgres", &["pg_dump", "-U", &user, "-Fc", &name]);
    let piped = docker::run_compose_piped(project, &args, None, Some(&dest))?;
    if piped.code != 0 {
        return Err(anyhow::anyhow!(
            "pg_dump failed (exit {}): {}",
            piped.code,
            backup::first_line(&piped.stderr)
        ));
    }
    Ok(ManifestDatabase {
        path: DB_MEMBER.to_string(),
        server_version: server_version(project, &user, &name),
        user,
        name,
        size_bytes: backup::file_size(&dest),
    })
}

/// What the server calls itself, for the manifest. Best effort: a backup is
/// not worth failing over a label.
///
/// The database has to be named: `psql -U chap` alone would connect to a
/// database called `chap`, which chap-core does not have.
fn server_version(project: &Project, user: &str, name: &str) -> Option<String> {
    let args = compose_exec(
        "postgres",
        &[
            "psql",
            "-U",
            user,
            "-d",
            name,
            "-tAc",
            "show server_version",
        ],
    );
    let (code, stdout, _) = docker::compose_output(project, &args).ok()?;
    if code != 0 {
        return None;
    }
    let version = stdout.trim();
    (!version.is_empty()).then(|| version.to_string())
}

/// One tar per enabled model, read out of its data volume.
fn capture_models(project: &Project, stage: &Stage, skip: bool) -> Result<Vec<ManifestModel>> {
    let mut out = Vec::new();
    if project.state.models.is_empty() {
        return Ok(out);
    }
    // The volume names carry the compose project name; without it there is no
    // way to tell "never started" from "empty", so every model is attempted.
    let prefix = if skip {
        None
    } else {
        docker::compose_project_name(project)
    };

    for (id, model) in &project.state.models {
        let volume = volume_name(id);
        let mut entry = ManifestModel {
            id: id.clone(),
            service_id: model.service_id.clone(),
            version: model.version.clone(),
            image_tag: model.image_tag.clone(),
            host_port: model.host_port,
            data_dir: model.data_dir.clone(),
            user: model.user.clone(),
            volume: volume.clone(),
            path: None,
            size_bytes: 0,
            skipped: None,
        };
        if skip {
            entry.skipped = Some("--no-models".to_string());
            out.push(entry);
            continue;
        }
        if let Some(prefix) = &prefix
            && !docker::volume_exists(&format!("{prefix}_{volume}"))
        {
            entry.skipped = Some(format!(
                "no {volume} volume yet, so there is no data to back up; \
                 the service has never started"
            ));
            out.push(entry);
            continue;
        }

        let member = backup::model_member(&model.service_id);
        let dest = stage.path(&member)?;
        // The init service mounts the same volume at the same path as the model
        // and does nothing else, so this reads the data whether the model is
        // running, stopped or has never been started at all.
        let args = compose_run(
            &format!("{}-init", model.service_id),
            &["tar", "cf", "-", "-C", &model.data_dir, "."],
        );
        let piped = docker::run_compose_piped(project, &args, None, Some(&dest))?;
        if piped.code != 0 {
            let _ = std::fs::remove_file(&dest);
            entry.skipped = Some(format!(
                "reading {} failed (exit {}): {}",
                model.data_dir,
                piped.code,
                backup::first_line(&piped.stderr)
            ));
            out.push(entry);
            continue;
        }
        entry.size_bytes = backup::file_size(&dest);
        entry.path = Some(member);
        out.push(entry);
    }
    Ok(out)
}

/// `exec -T <service> <cmd..>`, the form that needs no terminal.
fn compose_exec(service: &str, cmd: &[&str]) -> Vec<String> {
    let mut args = vec!["exec".to_string(), "-T".to_string(), service.to_string()];
    args.extend(cmd.iter().map(|s| s.to_string()));
    args
}

/// `run --rm --no-deps -T <service> <cmd..>`: a throwaway container that
/// starts nothing else, so a stopped stack stays stopped.
fn compose_run(service: &str, cmd: &[&str]) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--no-deps".to_string(),
        "-T".to_string(),
        service.to_string(),
    ];
    args.extend(cmd.iter().map(|s| s.to_string()));
    args
}

/// What went in, with sizes, and where it landed.
fn human(report: &BackupReport) -> String {
    let manifest = &report.manifest;
    let content = manifest.content_bytes();
    let mut text = format!(
        "backup  {}  ({} gzipped, {} of data)\n\n",
        report.path.display(),
        backup::human_size(report.size_bytes),
        backup::human_size(content)
    );

    text.push_str("included\n");
    if manifest.files.is_empty() {
        text.push_str("  files     none\n");
    } else {
        text.push_str(&format!(
            "  files     {} file(s): {}\n",
            manifest.files.len(),
            manifest.files.join(", ")
        ));
    }
    match &manifest.database {
        Some(db) => text.push_str(&format!(
            "  database  {} as {} ({}){}\n",
            db.name,
            db.user,
            backup::human_size(db.size_bytes),
            db.server_version
                .as_deref()
                .map(|v| format!(", PostgreSQL {v}"))
                .unwrap_or_default()
        )),
        None => text.push_str("  database  not included (--no-db)\n"),
    }
    let captured: Vec<&ManifestModel> = manifest.captured_models().collect();
    if captured.is_empty() {
        text.push_str("  models    none\n");
    } else {
        for (i, model) in captured.iter().enumerate() {
            let label = if i == 0 { "  models  " } else { "          " };
            text.push_str(&format!(
                "{label}  {}  {}  ({})\n",
                model.service_id,
                model.data_dir,
                backup::human_size(model.size_bytes)
            ));
        }
    }

    let skipped: Vec<&ManifestModel> = manifest
        .models
        .iter()
        .filter(|m| m.skipped.is_some())
        .collect();
    if !skipped.is_empty() {
        text.push_str("\nskipped\n");
        for model in skipped {
            text.push_str(&format!(
                "  {}  {}\n",
                model.service_id,
                model.skipped.as_deref().unwrap_or_default()
            ));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{DB_MEMBER, model_member};

    fn model(service_id: &str, skipped: Option<&str>) -> ManifestModel {
        ManifestModel {
            id: service_id.replace('-', "_"),
            service_id: service_id.to_string(),
            version: "1.0.0".into(),
            image_tag: "sha-fa880a1".into(),
            host_port: Some(5001),
            data_dir: "/app/data".into(),
            user: "chapkit:chapkit".into(),
            volume: format!("ck_{}_data", service_id.replace('-', "_")),
            path: skipped.is_none().then(|| model_member(service_id)),
            size_bytes: if skipped.is_none() { 40960 } else { 0 },
            skipped: skipped.map(str::to_string),
        }
    }

    fn report(database: bool, models: Vec<ManifestModel>) -> BackupReport {
        BackupReport {
            path: PathBuf::from("/backups/chaps-backup-e2e-20260923-071000.tar.gz"),
            size_bytes: 5 * 1024 * 1024,
            manifest: Manifest {
                schema_version: crate::backup::SCHEMA_VERSION,
                created_by: "chaps-cli 0.1.0".into(),
                created_at: "2026-09-23T07:10:00Z".into(),
                project: "e2e".into(),
                chap_image_tag: "latest".into(),
                files: vec![".env".into(), "compose.yml".into()],
                database: database.then(|| ManifestDatabase {
                    path: DB_MEMBER.into(),
                    user: "chap".into(),
                    name: "chap_core".into(),
                    server_version: Some("17.6".into()),
                    size_bytes: 2048,
                }),
                models,
            },
        }
    }

    #[test]
    fn the_human_output_lists_every_part_with_its_size() {
        let text = human(&report(true, vec![model("chapkit-ewars-model", None)]));
        assert!(text.starts_with(
            "backup  /backups/chaps-backup-e2e-20260923-071000.tar.gz  \
             (5.0 MB gzipped, 42.0 KB of data)\n"
        ));
        assert!(text.contains("files     2 file(s): .env, compose.yml"));
        assert!(text.contains("database  chap_core as chap (2.0 KB), PostgreSQL 17.6"));
        assert!(text.contains("models    chapkit-ewars-model  /app/data  (40.0 KB)"));
        assert!(!text.contains("skipped"));
    }

    #[test]
    fn what_was_left_out_is_said_out_loud() {
        let text = human(&report(
            false,
            vec![model("auto-arima-chapkit", Some("no volume yet"))],
        ));
        assert!(text.contains("database  not included (--no-db)"));
        assert!(text.contains("models    none"));
        assert!(text.contains("skipped\n  auto-arima-chapkit  no volume yet"));
    }

    #[test]
    fn the_compose_argument_lists_never_ask_for_a_terminal() {
        assert_eq!(
            compose_exec("postgres", &["pg_dump", "-U", "chap", "-Fc", "chap_core"]),
            vec![
                "exec",
                "-T",
                "postgres",
                "pg_dump",
                "-U",
                "chap",
                "-Fc",
                "chap_core"
            ]
        );
        assert_eq!(
            compose_run(
                "chapkit-ewars-model-init",
                &["tar", "cf", "-", "-C", "/app/data", "."]
            ),
            vec![
                "run",
                "--rm",
                "--no-deps",
                "-T",
                "chapkit-ewars-model-init",
                "tar",
                "cf",
                "-",
                "-C",
                "/app/data",
                "."
            ]
        );
    }

    #[test]
    fn the_project_name_falls_back_to_something_printable() {
        assert_eq!(project_name(Path::new("/srv/e2e")), "e2e");
        assert_eq!(project_name(Path::new("/")), "chaps");
    }
}
