//! `chaps up|down|logs|restart` and the `chaps docker` group — the docker
//! compose wrappers.
//!
//! Every one of them builds an argument list and hands it to the same runner,
//! so they all get the project's explicit `-f` list and the same exit-code
//! behaviour. `up` syncs the compose files from `.chaps/` first; the others,
//! `restart` included, run against whatever is on disk.

use crate::cli::DockerCmd;
use crate::commands::Ctx;
use crate::compose::sync;
use crate::docker;
use crate::error::{ChapError, Result};
use crate::output::{Out, PanelKind};
use crate::ports;
use crate::project::Project;
use crate::registry;
use std::io::IsTerminal;

/// The command run inside the container when `chaps docker exec` is given none.
const DEFAULT_EXEC_CMD: &str = "sh";

/// What `logs` and `docker ps` say for a project that has no containers at
/// all. Both would otherwise print nothing whatsoever.
const NOTHING_RUNNING: &str = "nothing is running for this project; start CHAP with `chaps up`";

/// What `restart` says when there is nothing to recreate. Recreating is not
/// starting: a deployment that is down is `chaps up`'s to bring up, the same
/// answer `chaps status` gives.
const NOT_RUNNING: &str = "CHAP is not running; start it with `chaps up`";

/// The hint that closes a detached `up`.
const AFTER_UP: &str = "run `chaps status` to check chap-core and the models";

/// What the caller's terminal adds to the argument list.
///
/// Split out from the arguments themselves so [`args_for`] stays a pure
/// function that tests can drive without a real terminal.
#[derive(Debug, Clone, Copy)]
pub struct Shell {
    /// The global `--json` flag.
    pub json: bool,
    /// Whether this process's stdin is a terminal. Compose refuses to allocate
    /// a TTY when it is not, so `exec` has to ask for `-T` instead.
    pub tty: bool,
}

impl Shell {
    /// The real environment: `--json` as given, stdin probed for a terminal.
    pub fn detect(json: bool) -> Shell {
        Shell {
            json,
            tty: std::io::stdin().is_terminal(),
        }
    }
}

/// Run one docker compose wrapper against the project's explicit `-f` list.
///
/// A non-zero child exit status surfaces as [`ChapError::DockerFailed`] so
/// `main` can mirror the code: `chaps up` is then a drop-in for
/// `docker compose up` in scripts.
///
/// Around the child run sit the two things docker will not say: what there was
/// to work with before it ran, and what changed by the time it finished.
pub fn run(ctx: &Ctx, cmd: &DockerCmd) -> Result<()> {
    let mut project = ctx.project()?;
    if let DockerCmd::Up(args) = cmd {
        let registry = registry::load(&ctx.registry)?;
        let report = sync(&mut project, &registry, ctx.cli_version, false)?;
        super::sync::announce(ctx, &report, &project);
        // After the sync, because the files it just wrote are the ones whose
        // ports we are about to probe.
        if !args.no_preflight {
            preflight(&project)?;
        }
    }
    warn_about_old_compose();

    let (before, unasked) = match prepare(ctx, &project, cmd)? {
        Pre::Run { before, unasked } => (before, unasked),
        // Everything there was to say has been said; the exit code is all
        // that is left, and an `error:` line on top of it would repeat it.
        Pre::Skip(0) => return Ok(()),
        Pre::Skip(code) => std::process::exit(code),
    };

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
    } else {
        docker::run_compose(&project, &args)?
    };
    if code != 0 {
        return Err(docker_failed(code, unasked));
    }
    report_what_changed(ctx, &project, cmd, &before);
    Ok(())
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

/// Say what the wrapper did, now that docker has finished.
fn report_what_changed(
    ctx: &Ctx,
    project: &Project,
    cmd: &DockerCmd,
    before: &[docker::Container],
) {
    match cmd {
        // An attached `up` has just streamed the logs and been interrupted;
        // there is nothing left running to summarise.
        DockerCmd::Up(args) if !args.attach => {
            let after = docker::running_containers(project).unwrap_or_default();
            note(ctx, &up_summary(&ctx.out, before, &after));
        }
        DockerCmd::Restart(_) => {
            let after = docker::running_containers(project).unwrap_or_default();
            note(ctx, &restart_summary(&ctx.out, before, &after));
        }
        DockerCmd::Down(args) => note(
            ctx,
            &down_summary(
                &ctx.out,
                &docker::service_names(before),
                &args.extra,
                project.compose_project_name().as_deref(),
            ),
        ),
        DockerCmd::Pull(_) => note(
            ctx,
            &ctx.out.ok(&pull_summary(docker::image_count(project))),
        ),
        _ => {}
    }
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

/// What a detached `up` changed, from the containers before and after.
///
/// Compose prints one line per service as it goes, in no particular order and
/// in the language of its own steps ("Created", "Running"); this is the one
/// line that says which services are new to this run.
pub fn up_summary(out: &Out, before: &[docker::Container], after: &[docker::Container]) -> String {
    let (started, unchanged) = docker::diff_containers(before, after);
    let started_cell = |names: &[String]| {
        format!(
            "{} {}",
            out.ok("started/recreated:"),
            out.value(&names.join(", "))
        )
    };
    let unchanged_cell = |names: &[String]| out.dim(&format!("unchanged: {}", names.join(", ")));
    let summary = match (started.is_empty(), unchanged.is_empty()) {
        (true, true) => {
            return out.backticks("nothing is running after `up`; run `chaps logs` to see why");
        }
        (false, true) => started_cell(&started),
        (true, false) => unchanged_cell(&unchanged),
        (false, false) => format!("{}; {}", started_cell(&started), unchanged_cell(&unchanged)),
    };
    format!("{summary}\n{}", out.backticks(AFTER_UP))
}

/// What `restart` recreated, from the containers before and after.
///
/// Compose recreates only the containers that no longer match the files, and
/// says nothing at all about the ones it skipped; this is the line that says
/// which half each service fell in. A run that recreated nothing is the
/// answer "nothing had moved on under you", not a failure.
pub fn restart_summary(
    out: &Out,
    before: &[docker::Container],
    after: &[docker::Container],
) -> String {
    let (recreated, unchanged) = docker::diff_containers(before, after);
    if recreated.is_empty() {
        return out.backticks("nothing needed a restart");
    }
    let head = format!(
        "{} {}",
        out.ok("recreated:"),
        out.value(&recreated.join(", "))
    );
    if unchanged.is_empty() {
        return head;
    }
    format!(
        "{head}; {}",
        out.dim(&format!("unchanged: {}", unchanged.join(", ")))
    )
}

/// What `down` stopped, and what it left behind.
///
/// The volumes are the point of the second half: `down` is the command people
/// reach for to "reset" a deployment, and it keeps the database. The compose
/// project name goes with them, because it is the prefix those volumes carry
/// and therefore what `docker volume ls` has to be asked about.
pub fn down_summary(
    out: &Out,
    stopped: &[String],
    extra: &[String],
    project_name: Option<&str>,
) -> String {
    if stopped.is_empty() {
        return out.dim("nothing was running");
    }
    let kept = match project_name {
        Some(name) => {
            format!("volumes kept: {name}_* (`chaps docker run -- down -v` removes them)")
        }
        None => "volumes kept (`chaps docker run -- down -v` removes them)".to_string(),
    };
    let volumes = if extra.iter().any(|a| a == "-v" || a == "--volumes") {
        "volumes removed too (-v)".to_string()
    } else {
        kept
    };
    format!(
        "{} {}; {}",
        out.warn("stopped CHAP:"),
        out.dim(&format!(
            "{} ({} container{})",
            stopped.join(", "),
            stopped.len(),
            if stopped.len() == 1 { "" } else { "s" }
        )),
        out.backticks(&volumes)
    )
}

/// The "there is nothing here" answer `logs` and `ps` give a project whose
/// containers have never been created, as a panel on a terminal.
fn nothing_running(out: &Out) -> String {
    out.panel_or(
        "Not running",
        &crate::output::hint_lines(NOTHING_RUNNING)
            .iter()
            .map(|line| out.backticks(line))
            .collect::<Vec<_>>()
            .join("\n"),
        PanelKind::Warning,
        NOTHING_RUNNING,
    )
}

/// The "there is nothing to recreate" answer `restart` gives a deployment
/// that is not running, as a panel on a terminal.
fn not_running(out: &Out) -> String {
    out.panel_or(
        "Not running",
        &crate::output::hint_lines(NOT_RUNNING)
            .iter()
            .map(|line| out.backticks(line))
            .collect::<Vec<_>>()
            .join("\n"),
        PanelKind::Warning,
        NOT_RUNNING,
    )
}

/// What `docker pull` fetched. Compose's own output is progress, not a result.
pub fn pull_summary(images: Option<usize>) -> String {
    match images {
        Some(1) => "pulled 1 image".to_string(),
        Some(count) => format!("pulled {count} images"),
        // `config --images` is the only thing that could have failed here, and
        // the pull itself succeeded, so the count is all that is missing.
        None => "pulled the images this project pins".to_string(),
    }
}

/// What `logs SERVICE` says about a name the project does not have.
pub fn unknown_service_message(unknown: &[String], services: &[String]) -> String {
    let named: Vec<String> = unknown.iter().map(|s| format!("`{s}`")).collect();
    format!(
        "no service {} in this project; the services are {}",
        named.join(", "),
        services.join(", ")
    )
}

/// The arguments appended after `compose -f ... -f ...`.
///
/// The global `--json` flag only reaches `ps` and `config`, the two wrappers
/// whose output docker itself can serialise; `shell.tty` only reaches `exec`.
pub fn args_for(cmd: &DockerCmd, shell: Shell) -> Vec<String> {
    match cmd {
        DockerCmd::Up(args) => {
            let mut out = vec!["up".to_string()];
            if !args.attach {
                out.push("-d".to_string());
            }
            if args.pull {
                out.push("--pull".to_string());
                out.push("always".to_string());
            }
            out.extend(args.extra.iter().cloned());
            out
        }
        DockerCmd::Down(args) => {
            let mut out = vec!["down".to_string()];
            out.extend(args.extra.iter().cloned());
            out
        }
        DockerCmd::Logs(args) => {
            let mut out = vec!["logs".to_string()];
            if args.follow {
                out.push("-f".to_string());
            }
            out.extend(args.services.iter().cloned());
            out
        }
        // `up -d` rather than `restart`: compose's own `restart` stops and
        // starts the container it has, which is exactly what does not pick up
        // a new image or a changed file. `up -d` recreates what no longer
        // matches and leaves the rest alone.
        DockerCmd::Restart(args) => {
            let mut out = vec!["up".to_string(), "-d".to_string()];
            if args.all {
                out.push("--force-recreate".to_string());
            }
            // Named services are the ones asked for: --no-deps keeps compose
            // from recreating what they depend on as well.
            if !args.services.is_empty() {
                out.push("--no-deps".to_string());
                out.extend(args.services.iter().cloned());
            }
            out
        }
        DockerCmd::Ps(args) => {
            let mut out = vec!["ps".to_string()];
            if shell.json {
                out.push("--format".to_string());
                out.push("json".to_string());
            }
            out.extend(args.extra.iter().cloned());
            out
        }
        DockerCmd::Pull(_) => vec!["pull".to_string()],
        DockerCmd::Exec(args) => {
            let mut out = vec!["exec".to_string()];
            // Without a terminal compose cannot allocate one; -T says so up
            // front rather than letting the command fail inside a script.
            if !shell.tty {
                out.push("-T".to_string());
            }
            out.push(args.service.clone());
            if args.cmd.is_empty() {
                out.push(DEFAULT_EXEC_CMD.to_string());
            } else {
                out.extend(args.cmd.iter().cloned());
            }
            out
        }
        DockerCmd::Run(args) => args.args.clone(),
        DockerCmd::Config(args) => {
            let mut out = vec!["config".to_string()];
            if shell.json {
                out.push("--format".to_string());
                out.push("json".to_string());
            }
            out.extend(args.extra.iter().cloned());
            out
        }
    }
}

/// Refuse to start when a host port the stack publishes is already taken.
///
/// Docker would find the same conflict, several seconds in, and name a
/// container rather than a port; this says which port, who wanted it and how
/// to move it, before anything has started. Ports held by this project's own
/// running containers are ours, so they are skipped: `chaps up` on a running
/// stack has to stay a no-op.
fn preflight(project: &Project) -> Result<()> {
    let claims = ports::claims(project);
    if claims.is_empty() {
        return Ok(());
    }
    let running = docker::running_services(project);
    let busy = ports::busy_claims(&claims, &running, &ports::is_busy);
    if busy.is_empty() {
        return Ok(());
    }
    // A free port to point at, found the way `init` finds one.
    let suggestion = busy
        .iter()
        .find(|claim| claim.service == crate::compose::API_SERVICE)
        .and_then(|claim| {
            ports::first_free(claim.port.saturating_add(1), u16::MAX, &ports::is_busy)
        });
    Err(anyhow::anyhow!(ports::preflight_message(&busy, suggestion)))
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
mod tests {
    use super::*;
    use crate::cli::{ConfigArgs, DownArgs, ExecArgs, LogsArgs, PsArgs, PullArgs, RunArgs, UpArgs};

    /// A `docker compose ps` that ran and was refused.
    fn refused_ps() -> docker::QueryFailure {
        docker::QueryFailure {
            query: "ps -a --format json".to_string(),
            code: Some(1),
            detail: "error during connect: the daemon is not running".to_string(),
        }
    }

    #[test]
    fn a_wrapper_that_could_not_ask_docker_first_says_what_docker_said() {
        let err = docker_failed(1, Some(refused_ps()));
        assert_eq!(
            err.to_string(),
            "docker could not be asked about this project: `docker compose ps -a --format json` \
             exited with status 1: error during connect: the daemon is not running"
        );
        // The status stays in the chain: `main` reads the code off it.
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::DockerFailed(1))
        ));
        assert_eq!(
            err.chain().nth(1).map(|c| c.to_string()),
            Some("docker compose exited with status 1".to_string())
        );
    }

    #[test]
    fn a_wrapper_docker_answered_for_ends_on_the_status_alone() {
        let err = docker_failed(137, None);
        assert_eq!(err.to_string(), "docker compose exited with status 137");
        assert_eq!(err.chain().count(), 1);
    }

    /// A plain terminal session: no `--json`, stdin is a TTY.
    const TERM: Shell = Shell {
        json: false,
        tty: true,
    };
    /// A script or a pipe: no terminal on stdin.
    const SCRIPT: Shell = Shell {
        json: false,
        tty: false,
    };
    /// `--json` asked for, from a terminal.
    const JSON: Shell = Shell {
        json: true,
        tty: true,
    };

    fn up(attach: bool, extra: &[&str]) -> DockerCmd {
        DockerCmd::Up(UpArgs {
            attach,
            pull: false,
            no_preflight: false,
            extra: extra.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn exec(service: &str, cmd: &[&str]) -> DockerCmd {
        DockerCmd::Exec(ExecArgs {
            service: service.to_string(),
            cmd: cmd.iter().map(|s| s.to_string()).collect(),
        })
    }

    #[test]
    fn up_detaches_unless_attach_is_asked_for() {
        assert_eq!(args_for(&up(false, &[]), TERM), vec!["up", "-d"]);
        assert_eq!(args_for(&up(true, &[]), TERM), vec!["up"]);
    }

    #[test]
    fn up_pull_asks_compose_to_pull_always() {
        let cmd = DockerCmd::Up(UpArgs {
            attach: false,
            pull: true,
            no_preflight: false,
            extra: vec!["chap".to_string()],
        });
        assert_eq!(
            args_for(&cmd, TERM),
            vec!["up", "-d", "--pull", "always", "chap"]
        );
        let cmd = DockerCmd::Up(UpArgs {
            attach: true,
            pull: true,
            no_preflight: true,
            extra: vec![],
        });
        assert_eq!(args_for(&cmd, TERM), vec!["up", "--pull", "always"]);
    }

    #[test]
    fn up_passes_extra_arguments_through_after_the_flags() {
        assert_eq!(
            args_for(&up(false, &["--build", "chap"]), TERM),
            vec!["up", "-d", "--build", "chap"]
        );
        assert_eq!(
            args_for(&up(true, &["chap"]), TERM),
            vec!["up", "chap"],
            "--attach must not inject -d before the service name"
        );
    }

    #[test]
    fn down_and_pull_are_bare_commands() {
        assert_eq!(
            args_for(&DockerCmd::Down(DownArgs { extra: vec![] }), TERM),
            vec!["down"]
        );
        assert_eq!(
            args_for(
                &DockerCmd::Down(DownArgs {
                    extra: vec!["-v".to_string()],
                }),
                TERM
            ),
            vec!["down", "-v"]
        );
        assert_eq!(
            args_for(&DockerCmd::Pull(PullArgs {}), JSON),
            vec!["pull"],
            "--json does not change pull"
        );
    }

    /// The `RestartArgs` a `chaps restart` would have parsed.
    fn restart(all: bool, services: &[&str]) -> DockerCmd {
        DockerCmd::Restart(crate::cli::RestartArgs {
            all,
            services: services.iter().map(|s| s.to_string()).collect(),
        })
    }

    #[test]
    fn restart_is_an_up_that_recreates_only_what_moved() {
        // The whole project: compose decides, service by service.
        assert_eq!(args_for(&restart(false, &[]), TERM), vec!["up", "-d"]);
        // Named services are recreated on their own.
        assert_eq!(
            args_for(&restart(false, &["chap", "worker"]), TERM),
            vec!["up", "-d", "--no-deps", "chap", "worker"]
        );
        // --all is what turns a restart into an unconditional one.
        assert_eq!(
            args_for(&restart(true, &[]), TERM),
            vec!["up", "-d", "--force-recreate"]
        );
        assert_eq!(
            args_for(&restart(true, &["chapkit-ewars-model"]), TERM),
            vec![
                "up",
                "-d",
                "--force-recreate",
                "--no-deps",
                "chapkit-ewars-model"
            ]
        );
        // Nothing about it depends on --json.
        assert_eq!(args_for(&restart(false, &[]), JSON), vec!["up", "-d"]);
    }

    #[test]
    fn restart_says_which_services_it_recreated() {
        let before = vec![
            container("chap", "aaa", "t1"),
            container("worker", "bbb", "t1"),
            container("postgres", "ccc", "t1"),
        ];
        let after = vec![
            container("chap", "ddd", "t2"),
            container("worker", "eee", "t2"),
            container("postgres", "ccc", "t1"),
        ];
        assert_eq!(
            restart_summary(&Out::default(), &before, &after),
            "recreated: chap, worker; unchanged: postgres"
        );
        // Everything moved: there is no second half to print.
        assert_eq!(
            restart_summary(&Out::default(), &before, &[container("chap", "ddd", "t2")]),
            "recreated: chap"
        );
        // Nothing moved, which is an answer and not a failure.
        assert_eq!(
            restart_summary(&Out::default(), &before, &before),
            "nothing needed a restart"
        );
    }

    #[test]
    fn ps_asks_docker_for_json_under_the_global_flag() {
        let cmd = DockerCmd::Ps(PsArgs { extra: vec![] });
        assert_eq!(args_for(&cmd, TERM), vec!["ps"]);
        assert_eq!(args_for(&cmd, JSON), vec!["ps", "--format", "json"]);

        let cmd = DockerCmd::Ps(PsArgs {
            extra: vec!["-a".to_string()],
        });
        assert_eq!(args_for(&cmd, JSON), vec!["ps", "--format", "json", "-a"]);
    }

    #[test]
    fn logs_maps_follow_and_service_names() {
        let cmd = DockerCmd::Logs(LogsArgs {
            follow: true,
            services: vec!["chap".to_string(), "chapkit-ewars-model".to_string()],
        });
        assert_eq!(
            args_for(&cmd, TERM),
            vec!["logs", "-f", "chap", "chapkit-ewars-model"]
        );

        let cmd = DockerCmd::Logs(LogsArgs {
            follow: false,
            services: vec![],
        });
        assert_eq!(args_for(&cmd, TERM), vec!["logs"]);
    }

    #[test]
    fn exec_falls_back_to_a_shell() {
        assert_eq!(
            args_for(&exec("chap", &[]), TERM),
            vec!["exec", "chap", "sh"]
        );
        assert_eq!(
            args_for(&exec("chap", &["env"]), TERM),
            vec!["exec", "chap", "env"]
        );
    }

    #[test]
    fn exec_asks_for_no_tty_when_there_is_none() {
        assert_eq!(
            args_for(&exec("chap", &["echo", "hello"]), SCRIPT),
            vec!["exec", "-T", "chap", "echo", "hello"],
            "-T goes before the service name, where compose expects its flags"
        );
        assert_eq!(
            args_for(&exec("chap", &[]), SCRIPT),
            vec!["exec", "-T", "chap", "sh"]
        );
    }

    #[test]
    fn exec_hands_the_container_its_own_flags() {
        assert_eq!(
            args_for(&exec("chap", &["ls", "-la", "/app"]), TERM),
            vec!["exec", "chap", "ls", "-la", "/app"]
        );
    }

    #[test]
    fn run_is_passed_through_verbatim() {
        let cmd = DockerCmd::Run(RunArgs {
            args: vec![
                "config".to_string(),
                "--services".to_string(),
                "--quiet".to_string(),
            ],
        });
        assert_eq!(
            args_for(&cmd, JSON),
            vec!["config", "--services", "--quiet"]
        );
        assert_eq!(
            args_for(&cmd, SCRIPT),
            vec!["config", "--services", "--quiet"],
            "neither --json nor the terminal touches a raw passthrough"
        );

        let cmd = DockerCmd::Run(RunArgs {
            args: vec!["restart".to_string(), "chap".to_string()],
        });
        assert_eq!(args_for(&cmd, TERM), vec!["restart", "chap"]);

        let cmd = DockerCmd::Run(RunArgs { args: vec![] });
        assert!(args_for(&cmd, TERM).is_empty());
    }

    #[test]
    fn config_serialises_under_the_global_flag() {
        let cmd = DockerCmd::Config(ConfigArgs { extra: vec![] });
        assert_eq!(args_for(&cmd, TERM), vec!["config"]);
        assert_eq!(args_for(&cmd, JSON), vec!["config", "--format", "json"]);

        let cmd = DockerCmd::Config(ConfigArgs {
            extra: vec!["--services".to_string()],
        });
        assert_eq!(args_for(&cmd, TERM), vec!["config", "--services"]);
        assert_eq!(
            args_for(&cmd, JSON),
            vec!["config", "--format", "json", "--services"]
        );
    }

    #[test]
    fn detect_keeps_the_json_flag_it_is_given() {
        assert!(Shell::detect(true).json);
        assert!(!Shell::detect(false).json);
    }

    /// One running container of `service`, created at `created`.
    fn container(service: &str, id: &str, created: &str) -> docker::Container {
        docker::Container {
            service: service.to_string(),
            name: format!("chapx-{service}-1"),
            id: id.to_string(),
            created_at: created.to_string(),
            state: "running".to_string(),
            ..docker::Container::default()
        }
    }

    #[test]
    fn up_summarises_what_it_started_and_what_it_left_alone() {
        let before = vec![
            container("chap", "aaa", "t1"),
            container("postgres", "bbb", "t1"),
        ];
        let after = vec![
            container("chap", "ccc", "t2"),
            container("postgres", "bbb", "t1"),
            container("chapkit-ewars-model", "ddd", "t2"),
        ];
        assert_eq!(
            up_summary(&Out::default(), &before, &after),
            "started/recreated: chap, chapkit-ewars-model; unchanged: postgres\n\
             run `chaps status` to check chap-core and the models"
        );

        // A first start has nothing to leave alone.
        assert!(up_summary(&Out::default(), &[], &after).starts_with(
            "started/recreated: chap, postgres, chapkit-ewars-model\nrun `chaps status`"
        ));
        // A no-op `up` says so rather than printing an empty list.
        assert!(
            up_summary(&Out::default(), &after, &after)
                .starts_with("unchanged: chap, postgres, chapkit")
        );
        // And an `up` that left nothing running is a problem, not a summary.
        assert_eq!(
            up_summary(&Out::default(), &[], &[]),
            "nothing is running after `up`; run `chaps logs` to see why"
        );
    }

    #[test]
    fn down_reports_what_it_stopped_and_what_it_kept() {
        let stopped = vec!["chap".to_string(), "worker".to_string()];
        let name = Some("mychap-1ab2c3");
        // The volumes that stay behind are named after the compose project,
        // so the line says which prefix to look for them under.
        assert_eq!(
            down_summary(&Out::default(), &stopped, &[], name),
            "stopped CHAP: chap, worker (2 containers); volumes kept: mychap-1ab2c3_* \
             (`chaps docker run -- down -v` removes them)"
        );
        assert!(
            down_summary(&Out::default(), &stopped[..1], &[], name)
                .starts_with("stopped CHAP: chap (1 container);")
        );
        // A deployment whose name could not be worked out still gets the line.
        assert!(
            down_summary(&Out::default(), &stopped, &[], None)
                .ends_with("volumes kept (`chaps docker run -- down -v` removes them)"),
            "{}",
            down_summary(&Out::default(), &stopped, &[], None)
        );
        // `down -v` did take the volumes, so it must not claim otherwise.
        assert!(
            down_summary(&Out::default(), &stopped, &["-v".to_string()], name)
                .ends_with("volumes removed too (-v)"),
            "{}",
            down_summary(&Out::default(), &stopped, &["-v".to_string()], name)
        );
        assert_eq!(
            down_summary(&Out::default(), &[], &[], name),
            "nothing was running"
        );
    }

    #[test]
    fn only_the_wrappers_that_can_hit_a_failed_dependency_read_stderr() {
        // A detached `up` and `restart` can end on "dependency failed to
        // start", and compose says which container that was on stderr.
        assert!(reads_stderr(&up(false, &[])));
        assert!(reads_stderr(&restart(false, &[])));
        // An attached `up` is streaming logs; its streams stay docker's own.
        assert!(!reads_stderr(&up(true, &[])));
        assert!(!reads_stderr(&DockerCmd::Down(DownArgs { extra: vec![] })));
        assert!(!reads_stderr(&exec("chap", &[])));
    }

    #[test]
    fn pull_counts_the_images_it_fetched() {
        assert_eq!(pull_summary(Some(4)), "pulled 4 images");
        assert_eq!(pull_summary(Some(1)), "pulled 1 image");
        assert_eq!(pull_summary(Some(0)), "pulled 0 images");
        assert_eq!(
            pull_summary(None),
            "pulled the images this project pins",
            "the pull worked even when the count could not be read"
        );
    }

    #[test]
    fn logs_names_the_services_there_are() {
        let services = vec![
            "chap".to_string(),
            "chapkit-ewars-model".to_string(),
            "postgres".to_string(),
        ];
        assert_eq!(
            unknown_service_message(&["chap-worker".to_string()], &services),
            "no service `chap-worker` in this project; the services are \
             chap, chapkit-ewars-model, postgres"
        );
        assert!(
            unknown_service_message(&["a".to_string(), "b".to_string()], &services)
                .starts_with("no service `a`, `b` in this project;")
        );
    }

    #[test]
    fn the_empty_state_line_says_what_to_do_about_it() {
        assert_eq!(
            NOTHING_RUNNING,
            "nothing is running for this project; start CHAP with `chaps up`"
        );
    }
}
