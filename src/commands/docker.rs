//! `chaps up|down|logs|restart` and the `chaps docker` group — the docker
//! compose wrappers.
//!
//! Every one of them builds an argument list and hands it to the same runner,
//! so they all get the project's explicit `-f` list and the same exit-code
//! behaviour. `up` syncs the compose files from `.chaps/` first; the others,
//! `restart` included, run against whatever is on disk.

mod args;
mod down;
mod preflight;
mod removal;
mod report;
mod wait;

pub use removal::{UNNAMEABLE_VOLUME, kept_volume_line, purge_volume, stop_and_remove};

use crate::cli::DockerCmd;
use crate::commands::Ctx;
use crate::compose::sync;
use crate::docker;
use crate::error::{ChapError, Result};
use crate::project::Project;
use args::{Shell, args_for};
use down::{confirm_volumes, misused_volumes_flag, volumes_flag_message};
use preflight::preflight;
use report::{not_running, nothing_running, report_what_changed, unknown_service_message};

/// Run one docker compose wrapper against the project's explicit `-f` list.
///
/// A non-zero child exit status surfaces as [`ChapError::DockerFailed`] so
/// `main` can mirror the code: `chaps up` is then a drop-in for
/// `docker compose up` in scripts.
///
/// Around the child run sit the two things docker will not say: what there was
/// to work with before it ran, and what changed by the time it finished.
pub fn run(ctx: &Ctx, cmd: &DockerCmd) -> Result<()> {
    // Ahead of the project and of docker: a `-v` meant for compose is a
    // mistake about this CLI's own flags, and the answer to it is the same in
    // any directory and on a machine with no daemon at all.
    if let DockerCmd::Down(args) = cmd {
        let argv: Vec<String> = std::env::args().collect();
        if let Some(token) = misused_volumes_flag(&argv, &args.extra) {
            return Err(anyhow::Error::new(ChapError::Usage(volumes_flag_message(
                token,
            ))));
        }
    }
    // `up` renders the compose files from `.chaps/` and `down` may clear a
    // record in it, so both hold the state lock; `up` lets go of it once the
    // files are written, before compose starts anything.
    let (mut project, mut lock) = match cmd {
        DockerCmd::Up(_) | DockerCmd::Down(_) => {
            let (project, lock) = ctx.project_mut()?;
            (project, Some(lock))
        }
        _ => (ctx.project()?, None),
    };
    if let DockerCmd::Up(args) = cmd {
        let registry = super::registry_for(ctx, Some(&project))?;
        let report = sync(&mut project, &registry, false)?;
        super::sync::announce(ctx, &report, &project);
        // Compose answers an empty file list with `no service selected` and a
        // failure, which says nothing about what to do; this does.
        if nothing_to_start(&project) {
            note(ctx, &ctx.out.backticks(NOTHING_TO_START));
            return Ok(());
        }
        // After the sync, because the files it just wrote are the ones whose
        // ports we are about to probe.
        if !args.no_preflight {
            preflight(ctx, &project, args.replace)?;
        }
        lock.take();
    }
    warn_about_old_compose();

    // `down --volumes` is the only wrapper that destroys data, so it names
    // what will go and asks first. The list is the closing line's too: what
    // docker no longer holds afterwards is what this run removed.
    let volumes = match cmd {
        DockerCmd::Down(args) if args.volumes => confirm_volumes(ctx, &project, args)?,
        _ => Vec::new(),
    };

    let (before, unasked) = match prepare(ctx, &project, cmd)? {
        Pre::Run { before, unasked } => (before, unasked),
        // Everything there was to say has been said; the exit code is all
        // that is left, and an `error:` line on top of it would repeat it.
        Pre::Skip(0) => return Ok(()),
        Pre::Skip(code) => std::process::exit(code),
    };

    // A config file edited under a running service is the one change compose
    // cannot see, so those services are recreated first, on their own; the
    // plain run after it then handles whatever else moved.
    if let DockerCmd::Restart(args) = cmd
        && !args.all
    {
        let edited = edited_configs(&project, &before, &args.services);
        if !edited.is_empty() {
            note(
                ctx,
                &ctx.out.backticks(&format!(
                    "recreating {} to apply its edited config file",
                    edited.join(", ")
                )),
            );
            let mut forced = vec![
                "up".to_string(),
                "-d".to_string(),
                "--no-deps".to_string(),
                "--force-recreate".to_string(),
            ];
            forced.extend(edited);
            let code = docker::run_compose(&project, &forced)?;
            if code != 0 {
                return Err(docker_failed(code, unasked));
            }
        }
    }

    let args = args_for(cmd, Shell::detect(ctx.out.json));
    // `up -d` and `restart` are the two that can end on "dependency failed to
    // start", and compose says which container that was on stderr and nowhere
    // else, so those two are run with stderr read as it goes. The rest keep
    // docker's streams exactly as they were.
    let code = if reads_stderr(cmd) {
        let ran = docker::run_compose_teed(&project, &args)?;
        if ran.code != 0 {
            explain_unhealthy(ctx, &project, &ran.stderr);
        }
        ran.code
    } else if matches!(cmd, DockerCmd::Logs(_)) && !ctx.out.color {
        // Containers colour their own lines whatever compose is told, so a
        // reader that is not a terminal gets them with the escapes removed.
        docker::run_compose_plain(&project, &args)?
    } else {
        docker::run_compose(&project, &args)?
    };
    if code != 0 {
        return Err(docker_failed(code, unasked));
    }
    report_what_changed(ctx, &mut project, cmd, &before, &volumes);
    if let DockerCmd::Up(args) = cmd
        && !args.attach
    {
        let readiness = match args.wait {
            true => Some(wait_for(ctx, &project, args.timeout)?),
            false => None,
        };
        if ctx.out.json {
            emit_up(ctx, &project, &before, readiness)?;
        }
    }
    Ok(())
}

/// `up --json`: what runs now, what this run started, and every model with
/// where it answers.
fn emit_up(
    ctx: &Ctx,
    project: &Project,
    before: &[docker::Container],
    readiness: Option<wait::Readiness>,
) -> Result<()> {
    let after = docker::running_containers(project).unwrap_or_default();
    let was: std::collections::BTreeSet<String> = docker::running_of(before);
    let running: Vec<String> = docker::running_of(&after).into_iter().collect();
    let started: Vec<&String> = running.iter().filter(|s| !was.contains(*s)).collect();
    let models: Vec<super::enable::ModelRef> = project
        .state
        .models
        .iter()
        .map(|(id, model)| super::enable::ModelRef::of(id, model, project))
        .collect();
    let value = serde_json::json!({
        "api_url": project.state.components.has_chap_core_api().then(|| project.api_url()),
        "running": running,
        "started": started,
        "models": models,
        "wait": readiness,
    });
    ctx.out.emit_ok(&value, String::new)
}

/// `up --wait`: wait, say what answers where, and fail on the deadline naming
/// what never did.
fn wait_for(ctx: &Ctx, project: &Project, timeout: u64) -> Result<wait::Readiness> {
    note(
        ctx,
        &format!("waiting up to {timeout}s for chap-core and the models to answer"),
    );
    let readiness = wait::wait_until_ready(ctx, project, std::time::Duration::from_secs(timeout));
    note(ctx, &ready_lines(&ctx.out, &readiness));
    if readiness.ready {
        return Ok(readiness);
    }
    Err(anyhow::anyhow!(
        "not ready after {timeout}s: {}; `chaps status` shows each one, and \
         `chaps logs <service>` says why",
        wait::pending(&readiness).join(", ")
    ))
}

/// What `--wait` found, one line per thing it waited for.
fn ready_lines(out: &crate::output::Out, readiness: &wait::Readiness) -> String {
    let mut text = match readiness.ready {
        true => format!("{} in {}s", out.ok("ready"), readiness.waited_s),
        false => format!("{} after {}s", out.warn("not ready"), readiness.waited_s),
    };
    if let Some(url) = &readiness.api_url {
        let state = if readiness.api_up {
            "up"
        } else {
            "not answering"
        };
        text.push_str(&format!("\n  chap-core  {state}  {}", out.value(url)));
    }
    let width = readiness
        .models
        .iter()
        .map(|m| m.service_id.len())
        .max()
        .unwrap_or(0);
    for model in &readiness.models {
        text.push_str(&format!(
            "\n  {:width$}  {}  {}",
            model.service_id,
            model.state,
            out.value(&model.url)
        ));
    }
    text
}

/// Whether this wrapper is one whose stderr is worth reading: a detached `up`
/// or a `restart`, which is an `up -d` under another name.
///
/// An attached `up` is streaming the logs the operator asked for, so its
/// streams stay docker's own.
fn reads_stderr(cmd: &DockerCmd) -> bool {
    match cmd {
        DockerCmd::Up(args) => !args.attach,
        DockerCmd::Restart(_) => true,
        _ => false,
    }
}

/// Say why the service compose gave up on is unhealthy, in its own words.
///
/// `dependency failed to start: container demo-chap-1 is unhealthy` names
/// which container failed and nothing about why; the container has been
/// printing the reason all along. Best-effort: a log that cannot be read, or
/// one with nothing failure-shaped in it, prints nothing at all rather than a
/// heading over an empty block.
fn explain_unhealthy(ctx: &Ctx, project: &Project, stderr: &str) {
    let unhealthy = crate::diagnose::from_stderr(project, stderr);
    let block = crate::diagnose::block(&unhealthy, &ctx.out);
    if block.is_empty() {
        return;
    }
    eprintln!();
    eprint!("{block}");
}

/// What the wrapper decided before letting docker run.
enum Pre {
    /// Run docker. Carries the containers as they were, for the wrappers that
    /// report the difference they made, and the query docker refused, for the
    /// error its exit code alone would not explain.
    Run {
        before: Vec<docker::Container>,
        unasked: Option<docker::QueryFailure>,
    },
    /// Do not run docker at all, and exit with this code: the caller has
    /// already been told what there was to know.
    Skip(i32),
}

impl Pre {
    /// Run docker, with the containers as docker said they were.
    fn run(before: Vec<docker::Container>) -> Pre {
        Pre::Run {
            before,
            unasked: None,
        }
    }

    /// Run docker without having been able to ask what was there first,
    /// remembering why for the error that follows if docker fails too.
    fn run_blind(why: docker::QueryFailure) -> Pre {
        Pre::Run {
            before: Vec::new(),
            unasked: Some(why),
        }
    }
}

/// The error a wrapper ends on when docker exited non-zero.
///
/// `docker compose exited with status 1` is the whole message on its own, and
/// it sends the reader looking at their stack when the answer is that docker
/// would not talk about this project at all. The `ps` that was refused first
/// is the missing half, in docker's own words, so it goes in front of the
/// status - which stays in the chain, because the exit code is mirrored from
/// it.
fn docker_failed(code: i32, unasked: Option<docker::QueryFailure>) -> anyhow::Error {
    let err = anyhow::Error::new(ChapError::DockerFailed(code));
    match unasked {
        Some(why) => err.context(format!(
            "docker could not be asked about this project: {why}"
        )),
        None => err,
    }
}

/// Ask docker what there is before running the wrapper, and answer for it
/// when the command it would run has nothing to print.
///
/// `docker compose logs` and `ps` on a deployment that was never started both
/// exit 0 having said nothing, which is the one thing a wrapper must not do.
fn prepare(ctx: &Ctx, project: &Project, cmd: &DockerCmd) -> Result<Pre> {
    match cmd {
        DockerCmd::Logs(args) => {
            // An `Err` is "docker could not be asked", which is not the same
            // as "this project has no containers"; docker itself says that
            // best, so the wrapper runs anyway - and keeps what docker said
            // about `ps` for the error that follows if it fails too.
            let containers = match docker::all_containers_or_why(project) {
                Ok(containers) => containers,
                Err(why) => return Ok(Pre::run_blind(why)),
            };
            if containers.is_empty() {
                note(ctx, &nothing_running(&ctx.out));
                return Ok(Pre::Skip(1));
            }
            reject_unknown_services(project, &args.services)?;
            Ok(Pre::run(Vec::new()))
        }
        // Nothing to recreate is not a failure of docker's to report: it is
        // the one thing this command cannot do, so it says which command can.
        DockerCmd::Restart(args) => {
            let before = match docker::running_containers_or_why(project) {
                Ok(before) => before,
                Err(why) => return Ok(Pre::run_blind(why)),
            };
            if before.is_empty() {
                note(ctx, &not_running(&ctx.out));
                return Ok(Pre::Skip(1));
            }
            reject_unknown_services(project, &args.services)?;
            Ok(Pre::run(before))
        }
        DockerCmd::Ps(_) => match docker::all_containers_or_why(project) {
            Ok(containers) if containers.is_empty() => {
                note(ctx, &nothing_running(&ctx.out));
                // A parser asked for a list of containers; there are none.
                if ctx.out.json {
                    println!("[]");
                }
                // Nothing is wrong with a stack that is not running: `ps` on
                // an empty project is a question answered, not a failure.
                Ok(Pre::Skip(0))
            }
            Ok(_) => Ok(Pre::run(Vec::new())),
            Err(why) => Ok(Pre::run_blind(why)),
        },
        // The snapshot `up` and `down` compare against afterwards.
        DockerCmd::Up(_) | DockerCmd::Down(_) => match docker::running_containers_or_why(project) {
            Ok(before) => Ok(Pre::run(before)),
            Err(why) => Ok(Pre::run_blind(why)),
        },
        _ => Ok(Pre::run(Vec::new())),
    }
}

/// Fail on a service name this project does not have, naming the ones it does.
///
/// Compose answers a name it does not know with `no such service`, which does
/// not say what the services are; this does, and it does it before docker is
/// asked to do anything. A docker that cannot list the services is not a
/// reason to refuse: the name may well be right.
fn reject_unknown_services(project: &Project, names: &[String]) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    let Some(services) = docker::config_services(project) else {
        return Ok(());
    };
    let unknown: Vec<String> = names
        .iter()
        .filter(|name| !services.contains(name))
        .cloned()
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(anyhow::anyhow!(unknown_service_message(
        &unknown, &services
    )))
}

/// The config files a component reads once, at startup, from a bind mount,
/// by the service that reads them.
const MOUNTED_CONFIGS: [(&str, &str, &str); 2] = [
    (
        crate::compose::DHIS2_SERVICE,
        crate::components::DHIS2_DIR,
        crate::components::DHIS2_CONFIG_FILE,
    ),
    (
        crate::compose::OCS_SERVICE,
        crate::components::OCS_DIR,
        crate::components::OCS_CONFIG_FILE,
    ),
];

/// The running services whose mounted config file was written after their
/// container was created, among `asked` (every service when it is empty).
///
/// Compose compares the compose files, and a bind-mounted file is not in them:
/// after an edit to `dhis2/dhis.conf` a plain `docker compose up -d` finds
/// nothing to do, while the service is still running on what it read at
/// startup. These are the services `restart` recreates anyway.
pub fn edited_configs(
    project: &Project,
    before: &[docker::Container],
    asked: &[String],
) -> Vec<String> {
    MOUNTED_CONFIGS
        .iter()
        .filter(|(service, ..)| asked.is_empty() || asked.iter().any(|a| a == service))
        .filter_map(|(service, dir, file)| {
            let container = before.iter().find(|c| c.service == *service)?;
            let created = container.created_unix()?;
            let written = std::fs::metadata(project.dir.join(dir).join(file))
                .and_then(|meta| meta.modified())
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs();
            (written > created).then(|| service.to_string())
        })
        .collect()
}

/// What `chaps up` says in a deployment that has nothing in it.
pub const NOTHING_TO_START: &str = "nothing to start: this deployment has no components and no \
    models; add one with `chaps models add URL`, `chaps models enable ID` or `chaps components \
    enable NAME`, then run `chaps up`";

/// Whether this deployment has nothing for `up` to start: no component of its
/// own (a chap-core elsewhere is not one) and no model.
fn nothing_to_start(project: &Project) -> bool {
    project.state.components.enabled().is_empty() && project.state.models.is_empty()
}

/// Print a line of the wrapper's own, as opposed to docker's output.
///
/// Under `--json` it goes to stderr: stdout is whatever docker printed, and a
/// parser reading it must not find a sentence in the middle.
fn note(ctx: &Ctx, line: &str) {
    if ctx.out.json {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}

/// Print the "compose is too old for `include:`" warning at most once.
///
/// Failing to probe the version is not reported here: the wrapper itself is
/// about to run `docker` and will produce a much better error.
fn warn_about_old_compose() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);

    if WARNED.swap(true, Ordering::Relaxed) {
        return;
    }
    if let Ok(Some(warning)) = docker::check_compose_version() {
        crate::output::warn(&warning);
    }
}

#[cfg(test)]
mod tests;
