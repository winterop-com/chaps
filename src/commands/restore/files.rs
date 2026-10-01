//! Step 2: the project files back, and the compose files re-rendered from
//! the `.chaps/` that arrived with them.

use super::RestoreReport;
use crate::backup::{self, ENV_BACKUP_FILE, FILES_MEMBER, RestorePlan, Stage};
use crate::cli::RestoreArgs;
use crate::commands::Ctx;
use crate::components::COMPONENTS_FILE;
use crate::compose::sync;
use crate::error::Result;
use crate::output;
use crate::project::{CHAPS_DIR, ENV_FILE, MANUAL_MODELS_FILE, MODELS_FILE, Project};
use std::path::Path;

/// Unpack `files/` over the project directory, then re-render the compose
/// files from the `.chaps/` that just arrived.
///
/// Returns the deployment as it now is, which is what every later step has to
/// work from: the `-f` list, the enabled components and the database
/// credentials all just changed under this process.
pub(super) fn restore_files(
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

    let registry = crate::commands::registry_for(ctx, Some(&restored))?;
    let sync_report = sync(&mut restored, &registry, false)?;
    for warning in &sync_report.warnings {
        output::warn(warning);
    }
    Ok(restored)
}

/// The `.chaps/` state files a deployment may or may not have: each one read
/// as empty when it is missing. `project.yaml` is not among them; every
/// deployment has one, and so does every archive.
pub(super) const OPTIONAL_STATE: &[&str] = &[MODELS_FILE, MANUAL_MODELS_FILE, COMPONENTS_FILE];

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
