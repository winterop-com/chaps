//! `chaps run` in the foreground: follow the model's log until Ctrl-C, then
//! stop what this command started. And Ctrl-C part-way through a start, in
//! either mode: take back out what the start put in.

use super::{RunReport, stop_in};
use crate::cli::ModelRunArgs;
use crate::commands::Ctx;
use crate::docker;
use crate::error::{ChapError, Result};
use crate::output;
use crate::project::Project;
use std::io::IsTerminal;

/// How many lines of the model's log the foreground shows before it follows.
const TAIL: &str = "20";

/// Whether this run follows the log in the foreground.
///
/// `--attach` and `--detach` decide when given. Otherwise the foreground is
/// for a person at a terminal running a model of a group: a deployment of
/// one's own lives on its own, and a script, `--json` or `--no-wait` wants
/// the command to return.
pub(super) fn wanted(ctx: &Ctx, args: &ModelRunArgs, in_group: bool) -> bool {
    if args.attach {
        return true;
    }
    if args.detach || args.no_wait || ctx.out.json || !in_group {
        return false;
    }
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

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
            "{id} ran before this command, so it keeps running; `chaps stop {id}` stops it"
        ));
        return Ok(());
    }
    if !crate::interrupt::requested() {
        output::notice(&format!("{service} stopped by itself; its log is above"));
    }
    match stop_in(ctx, &report.project_dir, id, purge) {
        Ok(_) if purge => output::notice(&format!("stopped {id}, and removed its data")),
        Ok(_) => output::notice(&format!(
            "stopped {id}; its data stays, and `chaps run {id}` starts it again with it"
        )),
        Err(err) => output::warn(&format!(
            "{id} did not stop: {err:#}; `chaps stop {id}` stops it"
        )),
    }
    Ok(())
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
            Err(err) => format!("{id} stays enabled ({err:#}); `chaps stop {id}` takes it out"),
        }
    } else {
        let args = ["stop", service].map(str::to_string);
        match docker::compose_output(project, &args) {
            Ok((0, _, _)) => format!("{service} is stopped again"),
            _ => format!("{service} may still run; `chaps stop {id}` stops it"),
        }
    };
    ChapError::Interrupted(what).into()
}
