//! Making a group on first use, and taking an emptied one away.

use super::{RUN_BIND, groups_dir, last_line};
use crate::cli::{Cli, Command};
use crate::commands::Ctx;
use crate::docker;
use crate::error::Result;
use crate::project::Project;
use clap::Parser;
use std::path::Path;

/// A group's deployment, created on first use.
///
/// `varde init <dir> --only none --models none`, quietly, and then the one
/// things init has no flag for: model ports on loopback by default, and the
/// group name, which every container's labels carry.
///
/// Two runs into a group that is not there yet would both create it, so the
/// creation holds `.<group>.lock` beside it; the second finds it made.
pub(super) fn ensure_default(ctx: &Ctx, dir: &Path, group: &str) -> Result<()> {
    let _lock = lock_group(group)?;
    if Project::exists(dir) {
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| anyhow::anyhow!("creating {}: {e}", dir.display()))?;
    let argv = [
        "varde",
        "init",
        &dir.to_string_lossy(),
        "--only",
        "none",
        "--models",
        "none",
    ];
    let cli = Cli::try_parse_from(argv).map_err(|e| anyhow::anyhow!("{e}"))?;
    let Command::Init(args) = cli.command else {
        unreachable!("the argv names init");
    };
    crate::commands::init::create(ctx, &args, false)?;
    let (mut project, _lock) = Project::find_locked(dir)?;
    project.state.model_bind = Some(RUN_BIND);
    project.state.group = Some(group.to_string());
    project.save()?;
    ctx.out.verbose(&format!(
        "created the varde run deployment in {}",
        dir.display()
    ));
    Ok(())
}

/// An exclusive hold on creating or removing one group, released when
/// dropped (or when the process ends, however it ends).
fn lock_group(group: &str) -> Result<std::fs::File> {
    let parent = groups_dir();
    std::fs::create_dir_all(&parent)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", parent.display()))?;
    lock_file(&parent.join(format!(".{group}.lock")))
}

/// An exclusive advisory lock on `path`, waiting for whoever holds it.
pub(super) fn lock_file(path: &Path) -> Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(|e| anyhow::anyhow!("opening `{}`: {e}", path.display()))?;
    file.lock()
        .map_err(|e| anyhow::anyhow!("locking `{}`: {e}", path.display()))?;
    Ok(file)
}

/// Take a group away when it holds no model: its containers, its network,
/// every volume compose made for it and its directory. The volumes are found
/// by the compose project label, not by the compose files, because a model
/// stopped earlier left no file that names its volume. The directory goes
/// last, so a volume docker would not remove keeps the group, and another
/// `varde stop --group <group> --purge` can try again.
///
/// `None` when the group still holds a model; otherwise the volumes removed.
pub(super) fn remove_if_empty(group: &str, dir: &Path) -> Result<Option<Vec<String>>> {
    let _lock = lock_group(group)?;
    let Ok(project) = Project::load(dir) else {
        return Ok(None);
    };
    if !project.state.models.is_empty() {
        return Ok(None);
    }
    let down = ["down", "--remove-orphans", "--volumes"].map(str::to_string);
    let (code, _, stderr) = docker::compose_output(&project, &down)?;
    if code != 0 {
        return Err(anyhow::anyhow!(
            "removing group {group}: docker compose down exited with status {code}: {}",
            last_line(&stderr).unwrap_or_default()
        ));
    }
    let mut removed = Vec::new();
    if let Some(name) = project.compose_project_name() {
        for volume in docker::volume_names_of_project(&name) {
            match docker::remove_volume(&volume) {
                docker::Removal::Removed => removed.push(volume),
                docker::Removal::NotFound => {}
                docker::Removal::Refused(why) => {
                    return Err(anyhow::anyhow!(
                        "removing group {group}: docker would not remove volume {volume} ({why}); \
                         run `docker volume rm {volume}`, then `varde stop --group {group} --purge`"
                    ));
                }
            }
        }
        docker::remove_default_network(&name);
    }
    std::fs::remove_dir_all(dir).map_err(|e| anyhow::anyhow!("removing {}: {e}", dir.display()))?;
    Ok(Some(removed))
}
