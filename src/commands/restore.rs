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
use crate::components::COMPONENTS_FILE;
use crate::compose::{overrides, sync};
use crate::docker;
use crate::error::Result;
use crate::output::{self, Out};
use crate::project::{CHAPS_DIR, ENV_FILE, MANUAL_MODELS_FILE, MODELS_FILE, PROJECT_FILE, Project};
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
    /// Optional `.chaps/` state files removed because the archive carries
    /// none: the deployment it came from had none, so keeping this one's
    /// would mix two deployments' state. See [`OPTIONAL_STATE`].
    pub removed_state: Vec<String>,
    /// The copy of the replaced `.env`, when one was kept.
    pub env_backup: Option<String>,
    /// `.env` variables left at this deployment's values rather than the
    /// archive's, because the database volume they open stayed. See
    /// [`backup::keep_credentials`].
    pub kept_credentials: Vec<String>,
    /// Whether the database was restored.
    pub database: bool,
    /// What `pg_restore` complained about while succeeding.
    pub database_warnings: Vec<String>,
    /// Model service ids whose data volume was refilled.
    pub models: Vec<String>,
    /// The service of each component volume that was refilled, as the plan
    /// names it: `dhis2` and `dhis2-db` for DHIS2's two, where the component
    /// name alone would say `dhis2` twice.
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
    backup::check_manifest_files(&manifest, &archive)?;
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
        removed_state: Vec::new(),
        env_backup: None,
        kept_credentials: Vec::new(),
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
        std::fs::write(&kept, current.clone().unwrap_or_default())
            .map_err(|e| anyhow::anyhow!("writing {}: {e}", kept.display()))?;
        report.env_backup = Some(ENV_BACKUP_FILE.to_string());
    }

    for rel in &report.plan.files {
        backup::copy_file(&from, &project.dir, rel)?;
        report.files.push(rel.clone());
    }
    report.removed_state = remove_state_not_in(&project.dir, &report.plan.files)?;

    // A database's credentials live in its volume as well as in `.env`, and a
    // restore that keeps the volume has to keep the `.env` half with it:
    // `pg_restore` puts the archive's rows into this deployment's `postgres`,
    // whose role still has this deployment's password. Only a volume the
    // restore replaces wholesale - a component tar, or all of them under
    // `--adopt-identity`, which points this directory at the archive's
    // volumes - takes the archive's credentials.
    if let (Some(current), Some(_)) = (&current, &incoming) {
        let adopting = args.adopt_identity
            && archived_identity_differs(&project.state.compose_project, &report.plan);
        let dhis2_db_replaced = report
            .plan
            .components
            .iter()
            .any(|c| c.volume == crate::compose::render::DHIS2_DB_VOLUME);
        let mut kept = Vec::new();
        if !adopting {
            kept.extend_from_slice(backup::CHAP_DB_CREDENTIALS);
            if !dhis2_db_replaced {
                kept.extend_from_slice(backup::DHIS2_DB_CREDENTIALS);
            }
        }
        let env = project.dir.join(ENV_FILE);
        let restored_env = std::fs::read_to_string(&env).unwrap_or_default();
        let (body, moved) =
            backup::keep_credentials(&restored_env, &String::from_utf8_lossy(current), &kept);
        if !moved.is_empty() {
            std::fs::write(&env, body)
                .map_err(|e| anyhow::anyhow!("writing {}: {e}", env.display()))?;
            report.kept_credentials = moved;
        }
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

/// The `.chaps/` state files a deployment may or may not have: each one read
/// as empty when it is missing. `project.yaml` is not among them; every
/// deployment has one, and so does every archive.
const OPTIONAL_STATE: &[&str] = &[MODELS_FILE, MANUAL_MODELS_FILE, COMPONENTS_FILE];

/// Remove the optional state files the restored files do not include.
///
/// A file a backup does not carry is one the deployment it came from did not
/// have - its models, its own model definitions or its components were simply
/// the defaults. Restoring that deployment means having none here either: a
/// `models-manual.yaml` left over from this deployment would keep definitions
/// the archive never had, beside the archive's `models.yaml`.
fn remove_state_not_in(dir: &Path, restored: &[String]) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for name in OPTIONAL_STATE {
        let rel = format!("{CHAPS_DIR}/{name}");
        let path = dir.join(CHAPS_DIR).join(name);
        if restored.contains(&rel) || !path.is_file() {
            continue;
        }
        std::fs::remove_file(&path)
            .map_err(|e| anyhow::anyhow!("removing {}: {e}", path.display()))?;
        removed.push(rel);
    }
    Ok(removed)
}

/// Whether the archive was taken under a compose project name other than this
/// deployment's, which is what `--adopt-identity` then switches to.
fn archived_identity_differs(current: &str, plan: &RestorePlan) -> bool {
    plan.archived_compose_project
        .as_deref()
        .map(str::trim)
        .is_some_and(|archived| !archived.is_empty() && archived != current)
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

/// The line of a failed `pg_restore` that says why: the first error that
/// lost something, with the statement it came from, else the last error it
/// reported, else the last thing it printed at all.
fn failure_reason(stderr: &str) -> String {
    let errors = backup::pg_restore_error_entries(stderr);
    if let Some(error) = errors.iter().find(|error| !error.is_harmless()) {
        return match &error.command {
            Some(command) => format!("{} (while running `{command}`)", error.message),
            None => error.message.clone(),
        };
    }
    errors
        .last()
        .map(|error| error.message.clone())
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

        let init = format!("{}-init", model.service_id);
        let script = backup::refill_script(&model.data_dir);
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
        let owner = overrides::chown_pair(&model.user);
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

/// Empty and refill each component data volume the archive holds.
///
/// No component one-shot mounts a volume the archive holds, so these are
/// reached the same way the backup read them: mounted into a throwaway busybox
/// container. The tar carries the numeric ownership it was taken with, so
/// there is no chown step to undo afterwards.
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
        // By volume, not by name: a component that keeps several volumes has an
        // entry per volume, all under the one component name, and the volume is
        // what no two of them share.
        let part = report
            .plan
            .manifest
            .components
            .iter()
            .find(|c| c.volume == planned.volume)
            .cloned();
        let Some(part) = part else { continue };
        let Some(member) = part.path.clone() else {
            continue;
        };

        // Staged under the member's own path, which is unique per volume where
        // the component name is not.
        let tar = stage.path(&member)?;
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
        report.components.push(part.service.clone());
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
    if !report.removed_state.is_empty() {
        text.push_str(&format!(
            "          {}\n",
            out.dim(&format!(
                "removed {}: the archive has none, so neither does this deployment now",
                report.removed_state.join(", ")
            ))
        ));
    }
    if let Some(kept) = &report.env_backup {
        text.push_str(&format!(
            "          {}\n",
            out.dim(&format!("the previous .env is kept as {kept}"))
        ));
    }
    if !report.kept_credentials.is_empty() {
        text.push_str(&format!(
            "          {}\n",
            out.dim(&format!(
                "kept this deployment's {} in .env: its database volumes stayed, and they only \
                 open with these",
                report.kept_credentials.join(", ")
            ))
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
mod tests;
