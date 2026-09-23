//! `chaps backup restore` — put a deployment back from an archive.
//!
//! The order matters and is the whole design:
//!
//! 1. stop `chap`, `worker` and the model services, so nothing writes while
//!    their storage is swapped under them (skipped when nothing is running),
//! 2. write the project files back and re-render the compose files from the
//!    `.chaps/` that just arrived,
//! 3. `pg_restore --clean --if-exists` into a postgres started just for this,
//! 4. empty and refill each model's data volume through its init container,
//!    then hand it back to the model's numeric uid:gid,
//! 5. `docker compose up -d`.
//!
//! Nothing is touched before the plan has been printed and confirmed.

use crate::backup::{
    self, ENV_BACKUP_FILE, FILES_MEMBER, MANIFEST_MEMBER, Manifest, PgRestore, PlannedModel,
    RestorePlan, Stage,
};
use crate::cli::RestoreArgs;
use crate::commands::Ctx;
use crate::compose::{overrides, sync};
use crate::docker;
use crate::error::Result;
use crate::output;
use crate::project::{ENV_FILE, Project};
use crate::registry;
use serde::Serialize;
use std::collections::BTreeSet;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long postgres is given to come up healthy before the restore gives up.
const POSTGRES_TIMEOUT: Duration = Duration::from_secs(180);
/// How often the wait re-reads `docker compose ps`.
const POLL: Duration = Duration::from_secs(2);

/// The shape of `chaps backup restore --json`.
#[derive(Debug, Serialize)]
pub struct RestoreReport {
    pub plan: RestorePlan,
    /// Project files written, relative to the project directory.
    pub files: Vec<String>,
    /// The copy of the replaced `.env`, when one was kept.
    pub env_backup: Option<String>,
    /// Whether the database was restored.
    pub database: bool,
    /// What `pg_restore` complained about while succeeding.
    pub database_warnings: Vec<String>,
    /// Model service ids whose data volume was refilled.
    pub models: Vec<String>,
    /// Services that were stopped first.
    pub stopped: Vec<String>,
    /// Whether the stack was started again.
    pub started: bool,
}

/// Read the archive, confirm, then restore.
pub fn run(ctx: &Ctx, args: &RestoreArgs) -> Result<()> {
    let project = ctx.project()?;
    let archive = std::path::absolute(&args.archive).unwrap_or_else(|_| args.archive.clone());
    if !archive.is_file() {
        return Err(anyhow::anyhow!("{} is not a file", archive.display()));
    }

    let manifest = read_manifest(&archive)?;
    let members: BTreeSet<String> = backup::tar_list(&archive)?.into_iter().collect();
    // Files-only never looks at Docker, so it also never asks it what is up.
    let running = if args.files_only {
        BTreeSet::new()
    } else {
        docker::running_services(&project)
    };
    let plan = plan(&project, &archive, manifest, &members, &running, args);

    if plan.is_empty() {
        return Err(anyhow::anyhow!(
            "{} holds nothing this run would restore",
            archive.display()
        ));
    }
    confirm(ctx, &plan, args.yes)?;

    let mut report = RestoreReport {
        files: Vec::new(),
        env_backup: None,
        database: false,
        database_warnings: Vec::new(),
        models: Vec::new(),
        stopped: Vec::new(),
        started: false,
        plan,
    };

    if !report.plan.stop.is_empty() {
        let mut args = vec!["stop".to_string()];
        args.extend(report.plan.stop.iter().cloned());
        compose(&project, &args)?;
        report.stopped = report.plan.stop.clone();
    }

    let stage = Stage::new(&project.chaps_dir(), "restore")?;
    if !report.plan.files.is_empty() {
        restore_files(ctx, &project, &archive, &stage, &mut report)?;
    }
    if report.plan.database {
        restore_database(&project, &archive, &stage, &running, &mut report)?;
    }
    if !report.plan.models.is_empty() {
        restore_models(&project, &archive, &stage, &mut report)?;
    }
    if report.plan.start {
        compose(&project, &["up".to_string(), "-d".to_string()])?;
        report.started = true;
    }

    ctx.out.emit(&report, || human(&report))
}

/// The manifest, read without unpacking the archive.
fn read_manifest(archive: &Path) -> Result<Manifest> {
    let body = backup::tar_read_member(archive, MANIFEST_MEMBER).map_err(|e| {
        e.context(format!(
            "{} has no {MANIFEST_MEMBER}; is it a chaps backup?",
            archive.display()
        ))
    })?;
    backup::parse_manifest(&body, archive)
}

/// What this invocation would do, given what the archive holds and what is up.
fn plan(
    project: &Project,
    archive: &Path,
    manifest: Manifest,
    members: &BTreeSet<String>,
    running: &BTreeSet<String>,
    args: &RestoreArgs,
) -> RestorePlan {
    let want_files = !args.db_only;
    let want_db = !args.files_only && manifest.database.is_some();
    let want_models = !args.files_only && !args.db_only && !args.no_models;

    let files = if want_files {
        manifest
            .files
            .iter()
            .filter(|rel| members.contains(&format!("{FILES_MEMBER}/{rel}")))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let models: Vec<PlannedModel> = if want_models {
        manifest
            .captured_models()
            .filter(|m| m.path.as_deref().is_some_and(|path| members.contains(path)))
            .map(|m| PlannedModel {
                service_id: m.service_id.clone(),
                data_dir: m.data_dir.clone(),
                volume: m.volume.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };

    // Only what is actually up is stopped, and only what this run disturbs:
    // chap and worker hold the database open, each model holds its volume.
    let mut stop: Vec<String> = Vec::new();
    if !args.files_only {
        for service in ["chap", "worker"] {
            if running.contains(service) {
                stop.push(service.to_string());
            }
        }
        for model in &models {
            if running.contains(&model.service_id) {
                stop.push(model.service_id.clone());
            }
        }
    }

    RestorePlan {
        archive: archive.to_path_buf(),
        project_dir: project.dir.clone(),
        files,
        database: want_db,
        models,
        stop,
        start: !args.files_only && !args.no_start,
        manifest,
    }
}

/// Print the plan and get a yes, or explain why there is no way to ask.
fn confirm(ctx: &Ctx, plan: &RestorePlan, yes: bool) -> Result<()> {
    let text = backup::plan_text(plan);
    // stdout under --json belongs to the report alone.
    if ctx.out.json {
        eprint!("{text}");
    } else {
        print!("{text}");
        let _ = std::io::stdout().flush();
    }
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(anyhow::anyhow!(
            "this overwrites the deployment and there is no terminal to confirm at; \
             pass --yes"
        ));
    }
    eprint!("\nrestore over this deployment? [y/N] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| anyhow::anyhow!("reading the answer: {e}"))?;
    if answer.trim().eq_ignore_ascii_case("y") || answer.trim().eq_ignore_ascii_case("yes") {
        return Ok(());
    }
    Err(anyhow::anyhow!("cancelled; nothing was changed"))
}

/// Unpack `files/` over the project directory, then re-render the compose
/// files from the `.chaps/` that just arrived.
fn restore_files(
    ctx: &Ctx,
    project: &Project,
    archive: &Path,
    stage: &Stage,
    report: &mut RestoreReport,
) -> Result<()> {
    let unpacked = stage.dir.join("unpacked");
    backup::tar_extract_into(archive, &unpacked, &[FILES_MEMBER.to_string()])?;
    let from = unpacked.join(FILES_MEMBER);

    // The operator's secrets are the one thing here that cannot be rebuilt, so
    // a different .env is kept beside the new one rather than dropped.
    let current = std::fs::read(project.dir.join(ENV_FILE)).ok();
    let incoming = std::fs::read(from.join(ENV_FILE)).ok();
    if backup::keep_env_copy(current.as_deref(), incoming.as_deref()) {
        let kept = project.dir.join(ENV_BACKUP_FILE);
        std::fs::write(&kept, current.unwrap_or_default())
            .map_err(|e| anyhow::anyhow!("writing {}: {e}", kept.display()))?;
        report.env_backup = Some(ENV_BACKUP_FILE.to_string());
    }

    for rel in &report.plan.files {
        backup::copy_file(&from, &project.dir, rel)?;
        report.files.push(rel.clone());
    }

    // The compose files in the archive are artifacts; re-rendering them from
    // the restored .chaps/ is what makes the deployment consistent again.
    let mut project = Project::load(&project.dir)?;
    let registry = registry::load(&ctx.registry)?;
    let sync_report = sync(&mut project, &registry, ctx.cli_version, false)?;
    for warning in &sync_report.warnings {
        output::warn(warning);
    }
    Ok(())
}

/// `pg_restore --clean --if-exists` the dump into a running postgres.
fn restore_database(
    project: &Project,
    archive: &Path,
    stage: &Stage,
    running: &BTreeSet<String>,
    report: &mut RestoreReport,
) -> Result<()> {
    let Some(db) = report.plan.manifest.database.clone() else {
        return Ok(());
    };
    // The credentials of the deployment as it is now, which after a files
    // restore is the .env that came out of the archive.
    let env_body = std::fs::read_to_string(project.dir.join(ENV_FILE)).unwrap_or_default();
    let (user, name) = backup::postgres_credentials(&env_body);

    if !running.contains("postgres") {
        compose(
            project,
            &["up".to_string(), "-d".to_string(), "postgres".to_string()],
        )?;
    }
    wait_for_postgres(project)?;

    let dump = stage.path("chap_core.dump")?;
    backup::tar_extract_member_to(archive, &db.path, &dump)?;

    let args = vec![
        "exec".to_string(),
        "-T".to_string(),
        "postgres".to_string(),
        "pg_restore".to_string(),
        "-U".to_string(),
        user,
        "-d".to_string(),
        name,
        "--clean".to_string(),
        "--if-exists".to_string(),
        "--no-owner".to_string(),
    ];
    let piped = docker::run_compose_piped(project, &args, Some(&dump), None)?;
    match backup::pg_restore_outcome(piped.code) {
        PgRestore::Ok => {}
        // Exit 1 is "restored, with complaints": `--clean --if-exists` always
        // has a few, because it drops objects a fresh database never had.
        PgRestore::Warnings => {
            report.database_warnings = backup::pg_restore_warnings(&piped.stderr);
            for warning in &report.database_warnings {
                output::warn(&format!("pg_restore: {warning}"));
            }
        }
        PgRestore::Failed => {
            return Err(anyhow::anyhow!(
                "pg_restore failed (exit {}): {}",
                piped.code,
                backup::first_line(&piped.stderr)
            ));
        }
    }
    report.database = true;
    Ok(())
}

/// Poll `docker compose ps` until postgres reports healthy.
fn wait_for_postgres(project: &Project) -> Result<()> {
    let started = Instant::now();
    let args = ["ps".to_string(), "--format".to_string(), "json".to_string()];
    loop {
        if let Ok((0, stdout, _)) = docker::compose_output(project, &args)
            && docker::service_is_healthy(&stdout, "postgres")
        {
            return Ok(());
        }
        if started.elapsed() >= POSTGRES_TIMEOUT {
            return Err(anyhow::anyhow!(
                "postgres did not become healthy within {} seconds; \
                 look at `chaps logs postgres`",
                POSTGRES_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(POLL);
    }
}

/// Empty and refill each model's data volume, then hand it back to the model.
fn restore_models(
    project: &Project,
    archive: &Path,
    stage: &Stage,
    report: &mut RestoreReport,
) -> Result<()> {
    for planned in report.plan.models.clone() {
        let model = report
            .plan
            .manifest
            .models
            .iter()
            .find(|m| m.service_id == planned.service_id)
            .cloned();
        let Some(model) = model else { continue };
        let Some(member) = model.path.clone() else {
            continue;
        };

        let tar = stage.path(&format!("{}.tar", model.service_id))?;
        backup::tar_extract_member_to(archive, &member, &tar)?;

        let init = format!("{}-init", model.service_id);
        let script = format!(
            "rm -rf {}/* && tar xf - -C {}",
            model.data_dir, model.data_dir
        );
        let args = run_args(&init, &["sh".to_string(), "-c".to_string(), script]);
        let piped = docker::run_compose_piped(project, &args, Some(&tar), None)?;
        if piped.code != 0 {
            return Err(anyhow::anyhow!(
                "restoring {} into {} failed (exit {}): {}",
                model.service_id,
                model.data_dir,
                piped.code,
                backup::first_line(&piped.stderr)
            ));
        }

        // tar restores the archive's ownership, which came out of a container
        // that ran as root; busybox resolves no account names, so the model's
        // user has to go back on as numbers - exactly what the overlay's init
        // container does on a fresh volume.
        let owner = overrides::numeric_user(&model.user)
            .unwrap_or_else(|| overrides::FALLBACK_UID_GID.to_string());
        let args = run_args(
            &init,
            &[
                "chown".to_string(),
                "-R".to_string(),
                owner,
                model.data_dir.clone(),
            ],
        );
        let piped = docker::run_compose_piped(project, &args, None, None)?;
        if piped.code != 0 {
            return Err(anyhow::anyhow!(
                "handing {} back to {} failed (exit {}): {}",
                model.data_dir,
                model.user,
                piped.code,
                backup::first_line(&piped.stderr)
            ));
        }
        report.models.push(model.service_id.clone());
    }
    Ok(())
}

/// `run --rm --no-deps -T <service> <cmd..>`.
fn run_args(service: &str, cmd: &[String]) -> Vec<String> {
    let mut args = vec![
        "run".to_string(),
        "--rm".to_string(),
        "--no-deps".to_string(),
        "-T".to_string(),
        service.to_string(),
    ];
    args.extend(cmd.iter().cloned());
    args
}

/// Run a compose command with inherited stdio, failing on a non-zero exit.
fn compose(project: &Project, args: &[String]) -> Result<()> {
    let code = docker::run_compose(project, args)?;
    if code != 0 {
        return Err(crate::error::ChapError::DockerFailed(code).into());
    }
    Ok(())
}

/// What was restored, in the order it happened.
fn human(report: &RestoreReport) -> String {
    let mut text = String::new();
    if !report.stopped.is_empty() {
        text.push_str(&format!("stopped   {}\n", report.stopped.join(", ")));
    }
    if report.files.is_empty() {
        text.push_str("files     not restored\n");
    } else {
        text.push_str(&format!(
            "files     {} restored: {}\n",
            report.files.len(),
            report.files.join(", ")
        ));
    }
    if let Some(kept) = &report.env_backup {
        text.push_str(&format!("          the previous .env is kept as {kept}\n"));
    }
    if report.database {
        let name = report
            .plan
            .manifest
            .database
            .as_ref()
            .map(|d| d.name.as_str())
            .unwrap_or("the database");
        text.push_str(&format!("database  {name} restored"));
        if report.database_warnings.is_empty() {
            text.push('\n');
        } else {
            text.push_str(&format!(
                " with {} warning(s) from pg_restore\n",
                report.database_warnings.len()
            ));
        }
    } else {
        text.push_str("database  not restored\n");
    }
    if report.models.is_empty() {
        text.push_str("models    not restored\n");
    } else {
        text.push_str(&format!("models    {}\n", report.models.join(", ")));
    }
    if report.started {
        text.push_str("\nthe stack is starting; `chaps status` says when the models are back\n");
    } else {
        text.push_str("\nthe stack was left as it is; start it with `chaps up`\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{DB_MEMBER, ManifestDatabase, ManifestModel, model_member};
    use crate::project::ProjectState;
    use std::path::PathBuf;

    fn manifest() -> Manifest {
        Manifest {
            schema_version: crate::backup::SCHEMA_VERSION,
            created_by: "chaps-cli 0.1.0".into(),
            created_at: "2026-09-23T07:10:00Z".into(),
            project: "e2e".into(),
            chap_image_tag: "latest".into(),
            files: vec![
                ".env".into(),
                ".chaps/models.yaml".into(),
                "compose.yml".into(),
            ],
            database: Some(ManifestDatabase {
                path: DB_MEMBER.into(),
                user: "chap".into(),
                name: "chap_core".into(),
                server_version: Some("17.6".into()),
                size_bytes: 2048,
            }),
            models: vec![
                ManifestModel {
                    id: "chapkit_ewars_model".into(),
                    service_id: "chapkit-ewars-model".into(),
                    version: "1.0.0".into(),
                    image_tag: "sha-fa880a1".into(),
                    host_port: Some(5001),
                    data_dir: "/app/data".into(),
                    user: "chapkit:chapkit".into(),
                    volume: "ck_chapkit_ewars_model_data".into(),
                    path: Some(model_member("chapkit-ewars-model")),
                    size_bytes: 40960,
                    skipped: None,
                },
                ManifestModel {
                    id: "auto_arima_chapkit".into(),
                    service_id: "auto-arima-chapkit".into(),
                    version: "1.0.0".into(),
                    image_tag: "sha-70c07a9".into(),
                    host_port: Some(5002),
                    data_dir: "/work/data".into(),
                    user: "chapkit:chapkit".into(),
                    volume: "ck_auto_arima_chapkit_data".into(),
                    path: None,
                    size_bytes: 0,
                    skipped: Some("no volume yet".into()),
                },
            ],
        }
    }

    fn members() -> BTreeSet<String> {
        [
            MANIFEST_MEMBER.to_string(),
            "files/.env".to_string(),
            "files/.chaps/models.yaml".to_string(),
            "files/compose.yml".to_string(),
            DB_MEMBER.to_string(),
            model_member("chapkit-ewars-model"),
        ]
        .into_iter()
        .collect()
    }

    fn project() -> Project {
        Project {
            dir: PathBuf::from("/srv/e2e"),
            state: ProjectState::default(),
        }
    }

    fn args(flags: &[&str]) -> RestoreArgs {
        use clap::Parser;
        let mut argv = vec!["chap", "backup", "restore", "/backups/x.tar.gz"];
        argv.extend_from_slice(flags);
        let cli = crate::cli::Cli::try_parse_from(argv).unwrap();
        let crate::cli::Command::Backup(b) = cli.command else {
            panic!("expected backup");
        };
        let crate::cli::BackupSub::Restore(args) = b.command else {
            panic!("expected restore");
        };
        args
    }

    fn running(services: &[&str]) -> BTreeSet<String> {
        services.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_full_restore_of_a_running_stack_plans_everything() {
        let up = running(&["chap", "worker", "postgres", "chapkit-ewars-model", "redis"]);
        let plan = plan(
            &project(),
            Path::new("/backups/x.tar.gz"),
            manifest(),
            &members(),
            &up,
            &args(&[]),
        );
        assert_eq!(plan.files.len(), 3);
        assert!(plan.database);
        assert_eq!(
            plan.models
                .iter()
                .map(|m| &m.service_id)
                .collect::<Vec<_>>(),
            vec!["chapkit-ewars-model"],
            "the model with no data in the archive is not planned"
        );
        // postgres and redis are left alone: the restore needs postgres, and
        // redis holds nothing this touches.
        assert_eq!(plan.stop, vec!["chap", "worker", "chapkit-ewars-model"]);
        assert!(plan.start);
    }

    #[test]
    fn a_stopped_stack_is_not_stopped_again() {
        let plan = plan(
            &project(),
            Path::new("/backups/x.tar.gz"),
            manifest(),
            &members(),
            &BTreeSet::new(),
            &args(&[]),
        );
        assert!(plan.stop.is_empty());
    }

    #[test]
    fn files_only_touches_no_docker_at_all() {
        let up = running(&["chap", "worker", "postgres"]);
        let plan = plan(
            &project(),
            Path::new("/backups/x.tar.gz"),
            manifest(),
            &members(),
            &up,
            &args(&["--files-only"]),
        );
        assert_eq!(plan.files.len(), 3);
        assert!(!plan.database);
        assert!(plan.models.is_empty());
        assert!(plan.stop.is_empty());
        assert!(!plan.start, "there is nothing to start");
    }

    #[test]
    fn db_only_and_no_models_narrow_the_plan() {
        let up = running(&["chap", "worker", "chapkit-ewars-model"]);
        let plan = plan(
            &project(),
            Path::new("/backups/x.tar.gz"),
            manifest(),
            &members(),
            &up,
            &args(&["--db-only"]),
        );
        assert!(plan.files.is_empty() && plan.database && plan.models.is_empty());
        assert_eq!(plan.stop, vec!["chap", "worker"], "no model is disturbed");

        let plan = plan_with(&up, &["--no-models"]);
        assert_eq!(plan.files.len(), 3);
        assert!(plan.database && plan.models.is_empty());
        assert_eq!(plan.stop, vec!["chap", "worker"]);

        let plan = plan_with(&up, &["--no-start"]);
        assert!(!plan.start);
    }

    fn plan_with(up: &BTreeSet<String>, flags: &[&str]) -> RestorePlan {
        plan(
            &project(),
            Path::new("/backups/x.tar.gz"),
            manifest(),
            &members(),
            up,
            &args(flags),
        )
    }

    #[test]
    fn a_file_the_manifest_lists_but_the_archive_lacks_is_not_planned() {
        let mut members = members();
        members.remove("files/compose.yml");
        let plan = plan(
            &project(),
            Path::new("/backups/x.tar.gz"),
            manifest(),
            &members,
            &BTreeSet::new(),
            &args(&[]),
        );
        assert_eq!(plan.files, vec![".env", ".chaps/models.yaml"]);
    }

    #[test]
    fn an_archive_with_nothing_in_it_plans_nothing() {
        let mut manifest = manifest();
        manifest.files.clear();
        manifest.database = None;
        manifest.models.clear();
        let plan = plan(
            &project(),
            Path::new("/backups/x.tar.gz"),
            manifest,
            &BTreeSet::new(),
            &BTreeSet::new(),
            &args(&[]),
        );
        assert!(plan.is_empty());
    }

    fn report(started: bool, warnings: Vec<&str>) -> RestoreReport {
        RestoreReport {
            plan: plan_with(&running(&["chap", "worker"]), &[]),
            files: vec![".env".into(), ".chaps/models.yaml".into()],
            env_backup: Some(ENV_BACKUP_FILE.to_string()),
            database: true,
            database_warnings: warnings.into_iter().map(str::to_string).collect(),
            models: vec!["chapkit-ewars-model".into()],
            stopped: vec!["chap".into(), "worker".into()],
            started,
        }
    }

    #[test]
    fn the_summary_reads_in_the_order_things_happened() {
        let text = human(&report(true, vec![]));
        assert!(text.starts_with("stopped   chap, worker\n"));
        assert!(text.contains("files     2 restored: .env, .chaps/models.yaml"));
        assert!(text.contains("the previous .env is kept as .env.before-restore"));
        assert!(text.contains("database  chap_core restored\n"));
        assert!(text.contains("models    chapkit-ewars-model"));
        assert!(text.contains("the stack is starting"));
    }

    #[test]
    fn pg_restore_warnings_are_counted_not_hidden() {
        let text = human(&report(
            false,
            vec!["warning: errors ignored on restore: 2"],
        ));
        assert!(text.contains("database  chap_core restored with 1 warning(s) from pg_restore"));
        assert!(text.contains("the stack was left as it is"));
    }

    #[test]
    fn a_run_that_restored_nothing_says_so_for_every_part() {
        let mut report = report(true, vec![]);
        report.files.clear();
        report.env_backup = None;
        report.database = false;
        report.models.clear();
        report.stopped.clear();
        let text = human(&report);
        assert!(text.starts_with("files     not restored\n"));
        assert!(text.contains("database  not restored"));
        assert!(text.contains("models    not restored"));
        assert!(!text.contains("stopped"));
    }

    #[test]
    fn the_init_container_is_always_run_without_a_terminal_or_dependencies() {
        assert_eq!(
            run_args(
                "chapkit-ewars-model-init",
                &["sh".to_string(), "-c".to_string(), "tar xf -".to_string()]
            ),
            vec![
                "run",
                "--rm",
                "--no-deps",
                "-T",
                "chapkit-ewars-model-init",
                "sh",
                "-c",
                "tar xf -"
            ]
        );
    }
}
