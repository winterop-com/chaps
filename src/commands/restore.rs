//! `varde backup restore` — put a deployment back from an archive.
//!
//! The order matters and is the whole design:
//!
//! 1. stop `chap`, `worker`, the model services and any component whose
//!    volume is about to be swapped, so nothing writes while their storage
//!    changes under them (skipped when nothing is running),
//! 2. write the project files back and re-render the compose files from the
//!    `.varde/` that just arrived. Everything after this step works from the
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

mod database;
mod dhis2;
mod files;
mod volumes;

use crate::backup::{
    self, FILES_MEMBER, MANIFEST_MEMBER, Manifest, PlannedComponent, PlannedModel, RestorePlan,
    Stage,
};
use crate::cli::RestoreArgs;
use crate::commands::Ctx;
use crate::docker;
use crate::error::Result;
use crate::output::Out;
use crate::project::{PROJECT_FILE, Project, VARDE_DIR};
use database::restore_database;
use files::restore_files;
use serde::Serialize;
use std::collections::BTreeSet;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;
use volumes::{restore_components, restore_models};

/// The shape of `varde backup restore --json`.
#[derive(Debug, Serialize)]
pub struct RestoreReport {
    pub plan: RestorePlan,
    /// Project files written, relative to the project directory.
    pub files: Vec<String>,
    /// Optional `.varde/` state files removed because the archive carries
    /// none: the deployment it came from had none, so keeping this one's
    /// would mix two deployments' state. See [`files::OPTIONAL_STATE`].
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
    /// Whether Chap was started again.
    pub started: bool,
}

/// Read the archive, confirm, then restore.
pub fn run(ctx: &Ctx, args: &RestoreArgs) -> Result<()> {
    let (mut project, _lock) = ctx.project_mut()?;
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

    let stage = Stage::new(&project.varde_dir(), "restore")?;
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
/// `.varde/project.yaml` recorded one.
///
/// Read out of the archive without unpacking it, because the plan says what
/// will happen to this deployment's identity before anything is touched.
fn archived_identity(archive: &Path, members: &BTreeSet<String>) -> Option<String> {
    let member = format!("{FILES_MEMBER}/{VARDE_DIR}/{PROJECT_FILE}");
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
            "{} has no {MANIFEST_MEMBER}; is it a varde backup?",
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

    // The identity only comes into it when `.varde/project.yaml` is one of the
    // files being written; without that nothing can change it.
    let archived_identity = archived_identity.filter(|_| {
        files
            .iter()
            .any(|rel| rel == &format!("{VARDE_DIR}/{PROJECT_FILE}"))
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
            &out.backticks("the deployment is starting; `varde status` says when it answers"),
        );
    } else {
        text.push_str(&out.backticks("the deployment was left as it is; start it with `varde up`"));
    }
    text.push('\n');
    text
}

#[cfg(test)]
mod tests;
