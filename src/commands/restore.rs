//! `chaps backup restore` — put a deployment back from an archive.
//!
//! The order matters and is the whole design:
//!
//! 1. stop `chap`, `worker`, the model services and any component whose
//!    volume is about to be swapped, so nothing writes while their storage
//!    changes under them (skipped when nothing is running),
//! 2. write the project files back and re-render the compose files from the
//!    `.chaps/` that just arrived. Everything after this step works from the
//!    project as it now is - its compose file list, its components, its
//!    credentials - and not from the one this process started with,
//! 3. `pg_restore --clean --if-exists` into a postgres started just for this,
//!    after `select 1` has proved the connection works,
//! 4. empty and refill each model's data volume through its init container,
//!    then hand it back to the model's numeric uid:gid,
//! 5. the same for each component data volume, through a busybox container,
//! 6. `docker compose up -d`.
//!
//! Nothing is touched before the plan has been printed and confirmed, and the
//! one thing a restore never takes from the archive is the compose project
//! name: that is the destination's identity. See
//! [`crate::backup::restored_compose_project`].

use crate::backup::{
    self, ENV_BACKUP_FILE, FILES_MEMBER, MANIFEST_MEMBER, Manifest, PgRestore, PlannedComponent,
    PlannedModel, RestorePlan, Stage,
};
use crate::cli::RestoreArgs;
use crate::commands::Ctx;
use crate::compose::{overrides, sync};
use crate::docker;
use crate::error::Result;
use crate::output::{self, Out};
use crate::project::{CHAPS_DIR, ENV_FILE, PROJECT_FILE, Project};
use serde::Serialize;
use std::collections::BTreeSet;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// How long postgres is given to come up healthy before the restore gives up.
const POSTGRES_TIMEOUT: Duration = Duration::from_secs(180);
/// How often the wait re-reads `docker compose ps`.
const POLL: Duration = Duration::from_secs(2);
/// How many lines of a failed `pg_restore`'s stderr are printed.
const STDERR_TAIL: usize = 10;

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
    /// Component names whose data volume was refilled.
    pub components: Vec<String>,
    /// Services that were stopped first.
    pub stopped: Vec<String>,
    /// Whether CHAP was started again.
    pub started: bool,
}

/// Read the archive, confirm, then restore.
pub fn run(ctx: &Ctx, args: &RestoreArgs) -> Result<()> {
    let mut project = ctx.project()?;
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
    let archived_identity = archived_identity(&archive, &members);
    let plan = plan(
        &project,
        &archive,
        manifest,
        &members,
        &running,
        archived_identity,
        args,
    );

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
        components: Vec::new(),
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
        // Everything below works from the deployment the archive just made of
        // this directory: the `-f` list it renders may now hold
        // compose.ocs.yml, and the credentials the database is reached with
        // are the ones in the `.env` that arrived.
        project = restore_files(ctx, &project, &archive, &stage, args, &mut report)?;
    }
    if report.plan.database {
        restore_database(&project, &archive, &stage, &running, &mut report)?;
    }
    if !report.plan.models.is_empty() {
        restore_models(&project, &archive, &stage, &mut report)?;
    }
    if !report.plan.components.is_empty() {
        restore_components(&project, &archive, &stage, &mut report)?;
    }
    if report.plan.start {
        compose(&project, &["up".to_string(), "-d".to_string()])?;
        report.started = true;
    }

    ctx.out.emit(&report, || human(&report, &ctx.out))
}

/// The compose project name the archive was taken under, when its
/// `.chaps/project.yaml` recorded one.
///
/// Read out of the archive without unpacking it, because the plan says what
/// will happen to this deployment's identity before anything is touched.
fn archived_identity(archive: &Path, members: &BTreeSet<String>) -> Option<String> {
    let member = format!("{FILES_MEMBER}/{CHAPS_DIR}/{PROJECT_FILE}");
    if !members.contains(&member) {
        return None;
    }
    let body = backup::tar_read_member(archive, &member).ok()?;
    backup::archived_compose_project(&body)
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
    archived_identity: Option<String>,
    args: &RestoreArgs,
) -> RestorePlan {
    let want_files = !args.db_only;
    let want_db = !args.files_only && manifest.database.is_some();
    let want_models = !args.files_only && !args.db_only && !args.no_models;
    let want_components = !args.files_only && !args.db_only && !args.no_components;

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
    let components: Vec<PlannedComponent> = if want_components {
        manifest
            .captured_components()
            .filter(|c| c.path.as_deref().is_some_and(|path| members.contains(path)))
            .map(|c| PlannedComponent {
                name: c.name.clone(),
                service: c.service.clone(),
                data_dir: c.data_dir.clone(),
                volume: c.volume.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };

    // Only what is actually up is stopped, and only what this run disturbs:
    // chap and worker hold the database open, each model and each component
    // holds its own volume.
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
        for part in &components {
            if running.contains(&part.service) {
                stop.push(part.service.clone());
            }
        }
    }

    // The identity only comes into it when `.chaps/project.yaml` is one of the
    // files being written; without that nothing can change it.
    let archived_identity = archived_identity.filter(|_| {
        files
            .iter()
            .any(|rel| rel == &format!("{CHAPS_DIR}/{PROJECT_FILE}"))
    });
    let destination = project.compose_project_name().unwrap_or_default();
    let compose_project = backup::restored_compose_project(
        &destination,
        archived_identity.as_deref().unwrap_or(&destination),
        args.adopt_identity,
    );

    RestorePlan {
        archive: archive.to_path_buf(),
        project_dir: project.dir.clone(),
        files,
        database: want_db,
        models,
        components,
        stop,
        start: !args.files_only && !args.no_start,
        compose_project,
        archived_compose_project: archived_identity,
        adopt_identity: args.adopt_identity,
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
///
/// Returns the deployment as it now is, which is what every later step has to
/// work from: the `-f` list, the enabled components and the database
/// credentials all just changed under this process.
fn restore_files(
    ctx: &Ctx,
    project: &Project,
    archive: &Path,
    stage: &Stage,
    args: &RestoreArgs,
    report: &mut RestoreReport,
) -> Result<Project> {
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
    let mut restored = Project::load(&project.dir)?;

    // The `project.yaml` that just arrived carries the compose project name of
    // the deployment the backup was taken from, and that name is the one thing
    // in it this deployment must not adopt: it is what every container and
    // named volume here is prefixed with, so taking it over would point this
    // deployment at the other one's volumes and abandon its own. Everything
    // else in the file is the archive's to restore, the API port included -
    // the `.env` beside it sets that too, and the two have to agree.
    let archived = restored.state.compose_project.clone();
    restored.state.compose_project = backup::restored_compose_project(
        &project.state.compose_project,
        &archived,
        args.adopt_identity,
    );
    if args.adopt_identity {
        if archived.trim().is_empty() {
            output::warn(
                "--adopt-identity was passed, but the archive records no compose project \
                 name; this deployment keeps its own",
            );
        } else {
            output::notice(&format!(
                "compose project name {archived} taken over from the archive \
                 (--adopt-identity)"
            ));
        }
    }

    let registry = super::registry_for(ctx, Some(&restored))?;
    let sync_report = sync(&mut restored, &registry, false)?;
    for warning in &sync_report.warnings {
        output::warn(warning);
    }
    Ok(restored)
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
    check_connection(project, &user, &name)?;

    let dump = stage.path("chap_core.dump")?;
    backup::tar_extract_member_to(archive, &db.path, &dump)?;

    let args = exec_args(
        "postgres",
        &[
            "pg_restore",
            "-U",
            &user,
            "-d",
            &name,
            "--clean",
            "--if-exists",
            "--no-owner",
        ],
    );
    let piped = docker::run_compose_piped(project, &args, Some(&dump), None)?;
    match backup::pg_restore_outcome(piped.code, &piped.stderr) {
        PgRestore::Ok => {}
        // Exit 1 with nothing but the complaints `--clean --if-exists` cannot
        // avoid, and the count that says pg_restore carried on regardless:
        // restored. See `backup::pg_restore_outcome`.
        PgRestore::Warnings => {
            report.database_warnings = backup::pg_restore_warnings(&piped.stderr);
            for warning in &report.database_warnings {
                output::warn(&format!("pg_restore: {warning}"));
            }
        }
        PgRestore::Failed => {
            // What pg_restore said is the only useful thing here, and it says
            // it at the end, so the tail goes out before the one-line error.
            for line in backup::tail_lines(&piped.stderr, STDERR_TAIL) {
                output::warn(&line);
            }
            return Err(anyhow::anyhow!(
                "pg_restore failed (exit {}): {}. The database is as pg_restore left it; \
                 nothing else was restored and CHAP was not started",
                piped.code,
                failure_reason(&piped.stderr)
            ));
        }
    }
    report.database = true;
    Ok(())
}

/// Prove the connection before anything is dropped.
///
/// `pg_restore` reports a connection it never made as exit 1, the same code it
/// uses for the object drops `--clean --if-exists` cannot avoid, so a restore
/// that never reached the server is one exit code away from one that finished
/// with warnings. Asking for `select 1` first turns that into the error
/// PostgreSQL actually gave: the wrong role, a database that is not there, a
/// server still starting up.
fn check_connection(project: &Project, user: &str, name: &str) -> Result<()> {
    let args = exec_args(
        "postgres",
        &["psql", "-U", user, "-d", name, "-tAc", "select 1"],
    );
    let (code, _, stderr) = docker::compose_output(project, &args)?;
    if code != 0 {
        return Err(anyhow::anyhow!(
            "the database {name} could not be reached as {user} (psql exited {code}): {}. \
             Nothing was restored",
            backup::first_line(&stderr)
        ));
    }
    Ok(())
}

/// The line of a failed `pg_restore` that says why: the last error it
/// reported, or the last thing it printed at all.
fn failure_reason(stderr: &str) -> String {
    backup::pg_restore_errors(stderr)
        .pop()
        .or_else(|| backup::tail_lines(stderr, 1).pop())
        .unwrap_or_else(|| "no output".to_string())
}

/// `exec -T <service> <cmd..>`, the form that needs no terminal.
fn exec_args(service: &str, cmd: &[&str]) -> Vec<String> {
    let mut args = vec!["exec".to_string(), "-T".to_string(), service.to_string()];
    args.extend(cmd.iter().map(|s| (*s).to_string()));
    args
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

        // A model that runs as root has no init container - it needs no chown,
        // so the overlay ships none - and its volume is refilled the way a
        // component's is, through a throwaway busybox. The tar then carries
        // the root ownership it was taken with, which is what that model
        // wants, so there is no chown to redo either.
        if crate::compose::overrides::is_root(&model.user) {
            let volume = format!("{}_{}", model_prefix(project)?, model.volume);
            let piped = backup::write_volume(&volume, &tar)?;
            if piped.code != 0 {
                return Err(anyhow::anyhow!(
                    "restoring {} into {volume} failed (exit {}): {}",
                    model.service_id,
                    piped.code,
                    backup::first_line(&piped.stderr)
                ));
            }
            report.models.push(model.service_id.clone());
            continue;
        }

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
        let owner = overrides::numeric_pair(&model.user)
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

/// The compose project name every named volume of this deployment carries.
fn model_prefix(project: &Project) -> Result<String> {
    docker::compose_project_name(project).ok_or_else(|| {
        anyhow::anyhow!(
            "the compose project name could not be read, so the model volumes cannot \
             be named; is Docker running?"
        )
    })
}

/// Empty and refill each component data volume the archive holds.
///
/// `ocs` and `s3` have no init container, so their volumes are reached the
/// same way the backup read them: mounted into a throwaway busybox container.
/// The tar carries the numeric ownership it was taken with, so there is no
/// chown step to undo afterwards.
fn restore_components(
    project: &Project,
    archive: &Path,
    stage: &Stage,
    report: &mut RestoreReport,
) -> Result<()> {
    // The volume names carry the compose project name of this deployment,
    // which after a files restore is the one it kept.
    let Some(prefix) = docker::compose_project_name(project) else {
        return Err(anyhow::anyhow!(
            "the compose project name could not be read, so the component volumes cannot \
             be named; is Docker running?"
        ));
    };

    for planned in report.plan.components.clone() {
        let part = report
            .plan
            .manifest
            .components
            .iter()
            .find(|c| c.name == planned.name)
            .cloned();
        let Some(part) = part else { continue };
        let Some(member) = part.path.clone() else {
            continue;
        };

        let tar = stage.path(&format!("{}.tar", part.name))?;
        backup::tar_extract_member_to(archive, &member, &tar)?;

        let volume = format!("{prefix}_{}", part.volume);
        let piped = backup::write_volume(&volume, &tar)?;
        if piped.code != 0 {
            return Err(anyhow::anyhow!(
                "restoring {} into {volume} failed (exit {}): {}",
                part.name,
                piped.code,
                backup::first_line(&piped.stderr)
            ));
        }
        report.components.push(part.name.clone());
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
fn human(report: &RestoreReport, out: &Out) -> String {
    let mut text = String::new();
    if !report.stopped.is_empty() {
        text.push_str(&format!(
            "{}   {}\n",
            out.key("stopped"),
            out.warn(&report.stopped.join(", "))
        ));
    }
    if report.files.is_empty() {
        text.push_str(&format!(
            "{}     {}\n",
            out.key("files"),
            out.dim("not restored")
        ));
    } else {
        text.push_str(&format!(
            "{}     {} {}\n",
            out.key("files"),
            out.ok(&format!("{} restored:", report.files.len())),
            out.dim(&report.files.join(", "))
        ));
    }
    if let Some(kept) = &report.env_backup {
        text.push_str(&format!(
            "          {}\n",
            out.dim(&format!("the previous .env is kept as {kept}"))
        ));
    }
    if report.database {
        let name = report
            .plan
            .manifest
            .database
            .as_ref()
            .map(|d| d.name.as_str())
            .unwrap_or("the database");
        text.push_str(&format!("{}  ", out.key("database")));
        if report.database_warnings.is_empty() {
            text.push_str(&out.ok(&format!("{name} restored")));
            text.push('\n');
        } else {
            text.push_str(&out.warn(&format!(
                "{name} restored with {} warning(s) from pg_restore",
                report.database_warnings.len()
            )));
            text.push('\n');
        }
    } else {
        text.push_str(&format!(
            "{}  {}\n",
            out.key("database"),
            out.dim("not restored")
        ));
    }
    if report.models.is_empty() {
        text.push_str(&format!(
            "{}    {}\n",
            out.key("models"),
            out.dim("not restored")
        ));
    } else {
        text.push_str(&format!(
            "{}    {}\n",
            out.key("models"),
            out.ok(&report.models.join(", "))
        ));
    }
    if report.components.is_empty() {
        text.push_str(&format!(
            "{}     {}\n",
            out.key("parts"),
            out.dim("not restored")
        ));
    } else {
        text.push_str(&format!(
            "{}     {}\n",
            out.key("parts"),
            out.ok(&report.components.join(", "))
        ));
    }
    let identity = backup::identity_line(&report.plan);
    if !identity.is_empty() {
        // The plan printed this before the confirmation; the summary repeats
        // it, because "restored from another deployment's backup" is the one
        // thing to be sure of afterwards.
        text.push_str(identity.trim_start());
    }
    text.push('\n');
    if report.started {
        text.push_str(
            &out.backticks("CHAP is starting; `chaps status` says when the models are back"),
        );
    } else {
        text.push_str(&out.backticks("CHAP was left as it is; start it with `chaps up`"));
    }
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{
        DB_MEMBER, ManifestComponent, ManifestDatabase, ManifestModel, component_member,
        model_member,
    };
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
                ".chaps/project.yaml".into(),
                "ocs/climate-service.yaml".into(),
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
                    quiesce: Some("paused for 1.4 s".into()),
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
                    quiesce: None,
                },
            ],
            components: vec![
                ManifestComponent {
                    name: "ocs".into(),
                    service: "ocs".into(),
                    volume: "ocs_data".into(),
                    data_dir: "/app/data".into(),
                    path: Some(component_member("ocs")),
                    size_bytes: 4096,
                    skipped: None,
                    quiesce: Some("paused for 0.3 s".into()),
                },
                ManifestComponent {
                    name: "s3".into(),
                    service: "s3".into(),
                    volume: "s3_data".into(),
                    data_dir: "/data".into(),
                    path: None,
                    size_bytes: 0,
                    skipped: Some("no volume yet".into()),
                    quiesce: None,
                },
            ],
        }
    }

    fn members() -> BTreeSet<String> {
        [
            MANIFEST_MEMBER.to_string(),
            "files/.env".to_string(),
            "files/.chaps/models.yaml".to_string(),
            "files/.chaps/project.yaml".to_string(),
            "files/ocs/climate-service.yaml".to_string(),
            "files/compose.yml".to_string(),
            DB_MEMBER.to_string(),
            model_member("chapkit-ewars-model"),
            component_member("ocs"),
        ]
        .into_iter()
        .collect()
    }

    fn project() -> Project {
        Project {
            dir: PathBuf::from("/srv/e2e"),
            state: ProjectState {
                compose_project: "e2e-ab12cd".to_string(),
                ..ProjectState::default()
            },
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
        let up = running(&[
            "chap",
            "worker",
            "postgres",
            "chapkit-ewars-model",
            "ocs",
            "redis",
        ]);
        let plan = plan_with(&up, &[]);
        assert_eq!(plan.files.len(), 5);
        assert!(plan.database);
        assert_eq!(
            plan.models
                .iter()
                .map(|m| &m.service_id)
                .collect::<Vec<_>>(),
            vec!["chapkit-ewars-model"],
            "the model with no data in the archive is not planned"
        );
        assert_eq!(
            plan.components.iter().map(|c| &c.name).collect::<Vec<_>>(),
            vec!["ocs"],
            "the component with no data in the archive is not planned"
        );
        // postgres and redis are left alone: the restore needs postgres, and
        // redis holds nothing this touches. ocs holds its own volume, so it
        // goes down with the models.
        assert_eq!(
            plan.stop,
            vec!["chap", "worker", "chapkit-ewars-model", "ocs"]
        );
        assert!(plan.start);
    }

    #[test]
    fn a_stopped_stack_is_not_stopped_again() {
        let plan = plan_with(&BTreeSet::new(), &[]);
        assert!(plan.stop.is_empty());
    }

    #[test]
    fn files_only_touches_no_docker_at_all() {
        let up = running(&["chap", "worker", "postgres"]);
        let plan = plan_with(&up, &["--files-only"]);
        assert_eq!(plan.files.len(), 5);
        assert!(!plan.database);
        assert!(plan.models.is_empty());
        assert!(plan.components.is_empty());
        assert!(plan.stop.is_empty());
        assert!(!plan.start, "there is nothing to start");
    }

    #[test]
    fn db_only_and_the_no_flags_narrow_the_plan() {
        let up = running(&["chap", "worker", "chapkit-ewars-model", "ocs"]);
        let plan = plan_with(&up, &["--db-only"]);
        assert!(plan.files.is_empty() && plan.database);
        assert!(plan.models.is_empty() && plan.components.is_empty());
        assert_eq!(
            plan.stop,
            vec!["chap", "worker"],
            "no model and no component is disturbed"
        );

        let plan = plan_with(&up, &["--no-models"]);
        assert_eq!(plan.files.len(), 5);
        assert!(plan.database && plan.models.is_empty());
        assert_eq!(
            plan.components.len(),
            1,
            "--no-models is about the models alone"
        );
        assert_eq!(plan.stop, vec!["chap", "worker", "ocs"]);

        let plan = plan_with(&up, &["--no-components"]);
        assert!(plan.components.is_empty() && plan.models.len() == 1);
        assert_eq!(plan.stop, vec!["chap", "worker", "chapkit-ewars-model"]);

        let plan = plan_with(&up, &["--no-start"]);
        assert!(!plan.start);
    }

    fn plan_with(up: &BTreeSet<String>, flags: &[&str]) -> RestorePlan {
        plan_for(up, flags, Some("e2e-ab12cd"))
    }

    fn plan_for(up: &BTreeSet<String>, flags: &[&str], archived: Option<&str>) -> RestorePlan {
        plan(
            &project(),
            Path::new("/backups/x.tar.gz"),
            manifest(),
            &members(),
            up,
            archived.map(str::to_string),
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
            None,
            &args(&[]),
        );
        assert_eq!(
            plan.files,
            vec![
                ".env",
                ".chaps/models.yaml",
                ".chaps/project.yaml",
                "ocs/climate-service.yaml"
            ]
        );
    }

    #[test]
    fn an_archive_with_nothing_in_it_plans_nothing() {
        let mut manifest = manifest();
        manifest.files.clear();
        manifest.database = None;
        manifest.models.clear();
        manifest.components.clear();
        let plan = plan(
            &project(),
            Path::new("/backups/x.tar.gz"),
            manifest,
            &BTreeSet::new(),
            &BTreeSet::new(),
            None,
            &args(&[]),
        );
        assert!(plan.is_empty());
    }

    #[test]
    fn the_plan_keeps_this_deployments_identity_unless_it_is_told_not_to() {
        let up = BTreeSet::new();

        // An archive from another deployment: this one keeps its own name, so
        // its containers and volumes are the ones refilled.
        let plan = plan_for(&up, &[], Some("chapx-9f01bc"));
        assert_eq!(plan.compose_project, "e2e-ab12cd");
        assert_eq!(
            plan.archived_compose_project.as_deref(),
            Some("chapx-9f01bc")
        );
        assert!(!plan.adopt_identity);
        assert!(backup::plan_text(&plan).contains("the archive's own (chapx-9f01bc)"));

        // --adopt-identity is the takeover.
        let plan = plan_for(&up, &["--adopt-identity"], Some("chapx-9f01bc"));
        assert_eq!(plan.compose_project, "chapx-9f01bc");
        assert!(plan.adopt_identity);

        // Nothing to adopt when the archive records no name, or when the file
        // that holds it is not among the ones being written.
        let plan = plan_for(&up, &["--adopt-identity"], None);
        assert_eq!(plan.compose_project, "e2e-ab12cd");
        assert!(plan.archived_compose_project.is_none());

        let plan = plan_for(&up, &["--db-only"], Some("chapx-9f01bc"));
        assert!(
            plan.archived_compose_project.is_none(),
            "--db-only writes no project.yaml, so no identity is at stake"
        );
    }

    fn report(started: bool, warnings: Vec<&str>) -> RestoreReport {
        RestoreReport {
            plan: plan_with(&running(&["chap", "worker"]), &[]),
            files: vec![".env".into(), ".chaps/models.yaml".into()],
            env_backup: Some(ENV_BACKUP_FILE.to_string()),
            database: true,
            database_warnings: warnings.into_iter().map(str::to_string).collect(),
            models: vec!["chapkit-ewars-model".into()],
            components: vec!["ocs".into()],
            stopped: vec!["chap".into(), "worker".into()],
            started,
        }
    }

    #[test]
    fn the_summary_reads_in_the_order_things_happened() {
        let text = human(&report(true, vec![]), &Out::default());
        assert!(text.starts_with("stopped   chap, worker\n"));
        assert!(text.contains("files     2 restored: .env, .chaps/models.yaml"));
        assert!(text.contains("the previous .env is kept as .env.before-restore"));
        assert!(text.contains("database  chap_core restored\n"));
        assert!(text.contains("models    chapkit-ewars-model"));
        assert!(text.contains("parts     ocs"));
        assert!(text.contains("CHAP is starting"));
        // This archive came from this deployment, so there is nothing to say
        // about whose identity it kept.
        assert!(!text.contains("identity"), "{text}");
    }

    #[test]
    fn the_summary_repeats_whose_identity_the_deployment_kept() {
        let mut report = report(true, vec![]);
        report.plan = plan_for(&running(&[]), &[], Some("chapx-9f01bc"));
        let text = human(&report, &Out::default());
        assert!(
            text.contains("identity  e2e-ab12cd is kept; the archive's own (chapx-9f01bc)"),
            "{text}"
        );
    }

    #[test]
    fn pg_restore_warnings_are_counted_not_hidden() {
        let text = human(
            &report(false, vec!["warning: errors ignored on restore: 2"]),
            &Out::default(),
        );
        assert!(text.contains("database  chap_core restored with 1 warning(s) from pg_restore"));
        assert!(text.contains("CHAP was left as it is"));
    }

    #[test]
    fn a_run_that_restored_nothing_says_so_for_every_part() {
        let mut report = report(true, vec![]);
        report.files.clear();
        report.env_backup = None;
        report.database = false;
        report.models.clear();
        report.components.clear();
        report.stopped.clear();
        let text = human(&report, &Out::default());
        assert!(text.starts_with("files     not restored\n"));
        assert!(text.contains("database  not restored"));
        assert!(text.contains("models    not restored"));
        assert!(text.contains("parts     not restored"));
        assert!(!text.contains("stopped"));
    }

    #[test]
    fn a_failed_pg_restore_is_quoted_by_the_line_that_says_why() {
        // The reason is the last error, not the first line of the stderr.
        let stderr = "pg_restore: connecting to database for restore\n\
                      pg_restore: error: connection to server at \"postgres\" failed: \
                      FATAL:  role \"nosuchrole\" does not exist\n";
        assert!(failure_reason(stderr).starts_with("connection to server"));
        // Nothing that looks like an error: the last thing it printed.
        assert_eq!(failure_reason("out of disk\n\n"), "out of disk");
        assert_eq!(failure_reason(""), "no output");
    }

    #[test]
    fn the_database_is_reached_through_exec_without_a_terminal() {
        assert_eq!(
            exec_args("postgres", &["psql", "-U", "chap", "-tAc", "select 1"]),
            vec![
                "exec", "-T", "postgres", "psql", "-U", "chap", "-tAc", "select 1"
            ]
        );
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
