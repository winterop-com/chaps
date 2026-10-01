//! Steps 4 and 5: each model's and each component's data volume, emptied
//! and refilled from the archive.

use super::RestoreReport;
use crate::backup::{self, Stage};
use crate::compose::overrides;
use crate::docker;
use crate::error::Result;
use crate::project::Project;
use std::path::Path;

/// Empty and refill each model's data volume, then hand it back to the model.
pub(super) fn restore_models(
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
pub(super) fn restore_components(
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
pub(super) fn run_args(service: &str, cmd: &[String]) -> Vec<String> {
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
