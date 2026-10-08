//! `varde run -a`, in the foreground: follow the model's log until Ctrl-C, then
//! stop what this command started. And Ctrl-C part-way through a start, in
//! either mode: take back out what the start put in.

use super::{RunReport, stop_in};
use crate::commands::Ctx;
use crate::docker;
use crate::error::{ChapError, Result};
use crate::output;
use crate::project::Project;

/// How many lines of the model's log the foreground shows before it follows.
const TAIL: &str = "20";

/// Follow the model's log until Ctrl-C, or until the container stops, then
/// stop the model when this command started it.
pub(super) fn follow(ctx: &Ctx, report: &RunReport, purge: bool) -> Result<()> {
    let project = Project::load(&report.project_dir)?;
    let id = &report.model.id;
    let service = &report.model.service_id;
    output::notice(&match report.was_running {
        true => format!(
            "following the log of {service}; it ran before this command, so Ctrl-C stops \
             only the log"
        ),
        false => format!("following the log of {service}; Ctrl-C stops it"),
    });
    let args = ["logs", "-f", "--tail", TAIL, service.as_str()].map(str::to_string);
    let _ = docker::run_compose(&project, &args)?;

    if report.was_running {
        output::notice(&format!(
            "{id} ran before this command, so it keeps running; `varde stop {id}` stops it"
        ));
        return Ok(());
    }
    if !crate::interrupt::requested() {
        output::notice(&format!("{service} stopped by itself; its log is above"));
    }
    match stop_in(ctx, &report.project_dir, id, purge) {
        Ok(_) if purge => {
            output::notice(&format!("stopped {id}, and removed its data"));
            remove_group(report);
        }
        Ok(_) => output::notice(&format!(
            "stopped {id}; its data stays, and `varde run {id}` starts it again with it"
        )),
        Err(err) => output::warn(&format!(
            "{id} did not stop: {err:#}; `varde stop {id}` stops it"
        )),
    }
    Ok(())
}

/// After a `--rm` stop: take the group away when it holds no model now, the
/// way `varde stop --purge` does, with its network and its directory.
fn remove_group(report: &RunReport) {
    let Some(group) = report.group.as_deref() else {
        return;
    };
    match super::group::remove_if_empty(group, &report.project_dir) {
        Ok(Some(_)) => output::notice(&format!("removed group {group}")),
        Ok(None) => {}
        // A volume docker would not remove names its own way out.
        Err(err) if err.to_string().contains("`varde stop") => output::warn(&format!("{err:#}")),
        Err(err) => output::warn(&format!(
            "{err:#}; `varde stop --group {group} --purge` removes the group"
        )),
    }
}

/// After Ctrl-C part-way through a start: take back out what this start put
/// in, and give the error that names what is left. A model this command
/// enabled goes back out, as after a failed start; one that was enabled and
/// stopped is stopped again; one that ran before is left running.
pub(super) fn take_back(
    ctx: &Ctx,
    project: &Project,
    id: &str,
    service: &str,
    enabled: bool,
    was_running: bool,
) -> anyhow::Error {
    let what = if was_running {
        format!("{id} ran before this command, so it keeps running")
    } else if enabled {
        match stop_in(ctx, &project.dir, id, false) {
            Ok(_) => format!("{id} is taken back out of {}", project.dir.display()),
            Err(err) => format!("{id} stays enabled ({err:#}); `varde stop {id}` takes it out"),
        }
    } else {
        let args = ["stop", service].map(str::to_string);
        match docker::compose_output(project, &args) {
            Ok((0, _, _)) => format!("{service} is stopped again"),
            _ => format!("{service} may still run; `varde stop {id}` stops it"),
        }
    };
    ChapError::Interrupted(what).into()
}
