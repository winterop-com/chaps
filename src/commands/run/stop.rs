//! `chaps stop`, and the stop key of `chaps top`.

use super::group::remove_if_empty;
use super::ps::scope;
use super::{DEFAULT_GROUP, with_dir};
use crate::cli::ModelStopArgs;
use crate::commands::Ctx;
use crate::commands::enable::enabled_id;
use crate::error::{ChapError, Result};
use crate::project::Project;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// What `chaps stop` did, for `--json`.
#[derive(Debug, Serialize)]
struct StopReport {
    stopped: Vec<StoppedModel>,
    /// The groups `--purge` emptied and took away.
    removed: Vec<String>,
    /// The volumes of those groups that went with them.
    removed_volumes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct StoppedModel {
    group: Option<String>,
    id: String,
    #[serde(flatten)]
    report: crate::commands::enable::DisableReport,
    notes: Vec<String>,
}

/// Stop a model, or every model in scope: its container goes and its overlay
/// with it; the data volume stays unless `--purge` says otherwise, and the
/// definition of a model that was added stays too, so `chaps run` starts it
/// again without asking GitHub.
pub fn stop(ctx: &Ctx, args: &ModelStopArgs) -> Result<()> {
    let scope = scope(ctx, args.group.as_deref())?;
    // Which deployment holds which model, settled before anything is locked.
    let mut wanted: Vec<(Option<String>, PathBuf, String)> = Vec::new();
    for (group, dir) in &scope {
        let project = Project::load(dir)?;
        match &args.id {
            Some(id) => {
                if let Some(id) = enabled_id(&project, id) {
                    wanted.push((group.clone(), dir.clone(), id));
                }
            }
            None => wanted.extend(
                project
                    .state
                    .models
                    .keys()
                    .map(|id| (group.clone(), dir.clone(), id.clone())),
            ),
        }
    }
    if let Some(id) = &args.id {
        if wanted.is_empty() {
            return Err(anyhow::anyhow!(
                "{id} is not running {}; `chaps ps` lists what is",
                match scope.as_slice() {
                    [(None, dir)] => format!("in {}", dir.display()),
                    _ => "in any `chaps run` group".to_string(),
                }
            ));
        }
        if wanted.len() > 1 {
            let names: Vec<String> = wanted
                .iter()
                .filter_map(|(group, _, _)| group.clone())
                .collect();
            return Err(ChapError::Usage(format!(
                "{id} runs in the groups {}; name one with `--group {}`",
                names.join(", "),
                names[0]
            ))
            .into());
        }
    }

    let mut stopped = Vec::new();
    let stopped_dirs: Vec<PathBuf> = wanted.iter().map(|(_, dir, _)| dir.clone()).collect();
    for (group, dir, id) in wanted {
        let (report, notes) = stop_in(ctx, &dir, &id, args.purge)?;
        // The notes name commands for the deployment, which a group is
        // reached with `-C`.
        let notes = match group {
            Some(_) => notes.iter().map(|n| with_dir(n, &dir)).collect(),
            None => notes,
        };
        stopped.push(StoppedModel {
            group,
            id,
            report,
            notes,
        });
    }
    let mut removed = Vec::new();
    let mut removed_volumes = Vec::new();
    if args.purge {
        // `stop ID --purge` takes away only the group it emptied: another
        // group with no model left may be keeping data a plain `stop` kept on
        // purpose. `--group` and `--all` name the groups to empty outright.
        let emptied: Vec<&PathBuf> = stopped_dirs.iter().collect();
        for (group, dir) in &scope {
            if args.id.is_some() && !emptied.contains(&dir) {
                continue;
            }
            if let Some(group) = group
                && let Some(volumes) = remove_if_empty(group, dir)?
            {
                removed.push(group.clone());
                removed_volumes.extend(volumes);
            }
        }
    }
    let report = StopReport {
        stopped,
        removed,
        removed_volumes,
    };
    ctx.out.emit_ok(&report, || {
        let mut text = String::new();
        let removed: String = report
            .removed
            .iter()
            .map(|group| format!("{} group {group}\n", ctx.out.warn("removed")))
            .chain(
                report
                    .removed_volumes
                    .iter()
                    .map(|volume| format!("{} volume {volume}\n", ctx.out.warn("removed"))),
            )
            .collect();
        if report.stopped.is_empty() {
            text.push_str(&removed);
            text.push_str(
                &ctx.out
                    .backticks("nothing was running to stop; `chaps ps` lists what is"),
            );
            return text;
        }
        for model in &report.stopped {
            let place = match model.group.as_deref() {
                Some(group) if group != DEFAULT_GROUP => format!(" (group {group})"),
                _ => String::new(),
            };
            text.push_str(&format!(
                "{} {}{place}\n",
                ctx.out.warn("stopped"),
                model.id
            ));
            for note in &model.notes {
                text.push_str(&format!("{}\n", ctx.out.backticks(note)));
            }
        }
        text.push_str(&removed);
        let again = match report.stopped.as_slice() {
            [one] => match one.group.as_deref() {
                Some(g) if g != DEFAULT_GROUP => {
                    format!("`chaps run {} --group {g}` starts it again", one.id)
                }
                _ => format!("`chaps run {}` starts it again", one.id),
            },
            _ => "`chaps run <model>` starts one again".to_string(),
        };
        text.push_str(&ctx.out.backticks(&again));
        text
    })
}

/// Stop one enabled model of the deployment in `dir`, printing nothing: what
/// `chaps stop` and the stop key of `chaps top` share.
pub(crate) fn stop_in(
    ctx: &Ctx,
    dir: &Path,
    id: &str,
    purge: bool,
) -> Result<(crate::commands::enable::DisableReport, Vec<String>)> {
    let ctx = &Ctx {
        project_dir: dir.to_path_buf(),
        ..ctx.clone()
    };
    let (mut project, _lock) = ctx.project_mut()?;
    let id = enabled_id(&project, id).ok_or_else(|| ChapError::UnknownModel(id.to_string()))?;
    let registry = crate::commands::registry_for(ctx, Some(&project))?;
    crate::commands::enable::disable_enabled(&mut project, &registry, &id, purge, false)
}
