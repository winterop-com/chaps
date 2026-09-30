//! `chaps up|down|logs|restart` and the `chaps docker` group — the docker
//! compose wrappers.
//!
//! Every one of them builds an argument list and hands it to the same runner,
//! so they all get the project's explicit `-f` list and the same exit-code
//! behaviour. `up` syncs the compose files from `.chaps/` first; the others,
//! `restart` included, run against whatever is on disk.

use crate::cli::{DockerCmd, DownArgs};
use crate::commands::Ctx;
use crate::components::{Components, DHIS2_CONNECT_FORGOTTEN_WITH_VOLUME, dhis2_connect_hint};
use crate::compose::sync;
use crate::docker;
use crate::error::{ChapError, Result};
use crate::output::Out;
use crate::ports;
use crate::project::Project;
use std::io::{BufRead, IsTerminal, Write};

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
const AFTER_UP: &str = "run `chaps status` to check that everything answers";

/// What `down --volumes` says when there is nobody to confirm to.
///
/// A prompt nobody can answer is a hang, and going ahead unasked would
/// destroy the data of whatever deployment the script happened to be in, so
/// the flag that cannot be undone is the one place `chaps` refuses instead.
const NO_TERMINAL: &str = "this destroys this deployment's data and there is no terminal to \
     confirm at; pass --yes";

/// The same under `--json`, which is a run nobody is watching either.
const NO_JSON_ANSWER: &str =
    "--json cannot ask before destroying this deployment's data; pass --yes";

/// What `down --volumes` asks before it runs.
const REMOVE_THEM: &str = "remove this deployment's volumes? [y/N] ";

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
    let mut project = ctx.project()?;
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
    } else {
        docker::run_compose(&project, &args)?
    };
    if code != 0 {
        return Err(docker_failed(code, unasked));
    }
    report_what_changed(ctx, &mut project, cmd, &before, &volumes);
    Ok(())
}

/// The `-v`-shaped token an operator meant for compose's `--volumes`, if
/// `down` was given one.
///
/// Two ways in, and neither can be passed on. `chaps down -v` is taken by
/// clap as the global `--verbose` flag, so compose never sees it: the volumes
/// stay while the person who typed it believes they went, which is the whole
/// reason `--volumes` exists. Nothing in the parsed command line can tell
/// that apart from a deliberately verbose stop, so the raw arguments answer
/// it, and only the ones after `down` count - `chaps -v down` is a verbose
/// stop and stays one. Past a `--` the token reaches `EXTRA` instead, where
/// passing it through would destroy the data of an operator who only asked
/// for a louder `down`.
///
/// `chaps docker run -- down -v` is not this: it is the passthrough asking
/// for compose's own command line, where `-v` means what compose says it
/// means.
pub fn misused_volumes_flag<'a>(argv: &'a [String], extra: &'a [String]) -> Option<&'a str> {
    let typed = argv
        .iter()
        .position(|arg| arg == "down")
        .map_or(&[][..], |at| &argv[at + 1..]);
    typed
        .iter()
        .map(String::as_str)
        .find(|arg| *arg == "-v")
        .or_else(|| {
            extra
                .iter()
                .map(String::as_str)
                .find(|arg| matches!(*arg, "-v" | "--volumes" | "--volume"))
        })
}

/// What that refusal says: which flag it was, which flag does the job, and
/// how to ask for the verbose `down` the other reading would have given.
pub fn volumes_flag_message(token: &str) -> String {
    format!(
        "`{token}` after `down` is not passed on to compose (`-v` there is chaps's own \
         --verbose flag); `chaps down --volumes` removes this deployment's volumes and the \
         data in them, and `chaps -v down` is the verbose stop"
    )
}

/// Name the volumes `down --volumes` is about to destroy, and get a yes.
///
/// Returns the volumes docker holds under this deployment's prefix, which is
/// also what the closing line is measured against: compose removes the
/// volumes its files declare, so a leftover under the same prefix may well
/// outlive the run, and only docker can say which ones actually went.
///
/// `--yes` is how a script says it meant it. A run that cannot be asked - no
/// terminal on stdin, or `--json`, which nobody is watching - refuses rather
/// than assume, because there is no undoing this one.
fn confirm_volumes(ctx: &Ctx, project: &Project, args: &DownArgs) -> Result<Vec<String>> {
    let prefix = project.volume_prefix();
    let volumes = match prefix.as_deref() {
        Some(prefix) => docker::volume_names_with_prefix(prefix),
        None => Vec::new(),
    };
    // How much the one volume that is usually large holds, so the prompt says
    // what is at stake and not only how many volumes there are. Only asked
    // for when docker has already said the volume is there, and bounded like
    // every other size: a `du` that will not finish costs the size and not
    // the prompt.
    // With `--yes` there is no question for the line to inform, and the
    // closing line names every volume that went.
    if args.yes {
        return Ok(volumes);
    }
    let ocs_data = prefix
        .as_deref()
        .and_then(|prefix| ocs_data_at_stake(prefix, &volumes));
    note(
        ctx,
        &volumes_at_stake(
            &ctx.out,
            &volumes,
            prefix.as_deref(),
            ocs_data
                .as_ref()
                .map(|(name, bytes)| (name.as_str(), *bytes)),
        ),
    );
    if ctx.out.json {
        return Err(anyhow::anyhow!(NO_JSON_ANSWER));
    }
    if !std::io::stdin().is_terminal() {
        return Err(anyhow::anyhow!(NO_TERMINAL));
    }
    eprint!("\n{REMOVE_THEM}");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| anyhow::anyhow!("reading the answer: {e}"))?;
    if matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
        return Ok(volumes);
    }
    Err(anyhow::anyhow!("cancelled; nothing was stopped"))
}

/// The line that names what is at stake, before `down --volumes` is let near
/// it.
///
/// Every volume under the deployment's prefix, because that is the set at
/// risk as docker sees it - and a statement of what docker holds rather than
/// a promise about what will go, since compose removes the volumes its files
/// still declare and a leftover under the same prefix may well outlive the
/// run. What did go is the closing line's to report.
///
/// An empty list is a line of its own rather than a silence: a deployment
/// that was never started has nothing to lose, and a docker that could not be
/// asked answers the same way, which is worth seeing before saying yes.
pub fn volumes_at_stake(
    out: &Out,
    volumes: &[String],
    prefix: Option<&str>,
    ocs_data: Option<(&str, u64)>,
) -> String {
    let under = match prefix {
        Some(prefix) => format!("under {prefix}*"),
        None => "under this deployment's name".to_string(),
    };
    if volumes.is_empty() {
        return out.dim(&format!("docker holds no volumes {under}"));
    }
    let mut line = format!(
        "{} {}",
        out.warn(&format!(
            "docker holds {} {under}, with their data:",
            volume_count(volumes.len())
        )),
        out.value(&volumes.join(", "))
    );
    // The names say what would go; the size says how much of it matters.
    if let Some((name, bytes)) = ocs_data {
        line.push_str(&format!(
            " {}",
            out.dim(&format!(
                "({name} holds {})",
                crate::backup::human_size(bytes)
            ))
        ));
    }
    line
}

/// This deployment's OCS data volume and what it holds, when docker holds one.
///
/// The size is the OCS one and no other because it is the one that grows: a
/// downloaded dataset store is gigabytes where the database and the model
/// volumes are megabytes, and it is the number that decides whether a `down
/// --volumes` is a keystroke or a mistake. `None` when there is no such
/// volume, or when the `du` could not be had.
fn ocs_data_at_stake(prefix: &str, volumes: &[String]) -> Option<(String, u64)> {
    let name = format!("{prefix}{}", crate::compose::render::OCS_VOLUME);
    if !volumes.contains(&name) {
        return None;
    }
    let bytes = docker::volume_size_bytes(&name)?;
    Some((name, bytes))
}

/// `1 volume` or `N volumes`.
fn volume_count(count: usize) -> String {
    format!("{count} volume{}", if count == 1 { "" } else { "s" })
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
///
/// `volumes` are the ones docker held before a `down --volumes`, and empty
/// for every other wrapper.
fn report_what_changed(
    ctx: &Ctx,
    project: &mut Project,
    cmd: &DockerCmd,
    before: &[docker::Container],
    volumes: &[String],
) {
    match cmd {
        // An attached `up` has just streamed the logs and been interrupted;
        // there is nothing left running to summarise.
        DockerCmd::Up(args) if !args.attach => {
            let after = docker::running_containers(project).unwrap_or_default();
            note(
                ctx,
                &up_summary(&ctx.out, before, &after, &project.state.components),
            );
        }
        DockerCmd::Restart(args) => {
            let after = docker::running_containers(project).unwrap_or_default();
            note(
                ctx,
                &restart_summary(&ctx.out, before, &after, &args.services),
            );
        }
        DockerCmd::Down(args) => {
            let removed = args.volumes.then(|| removed_volumes(project, volumes));
            note(
                ctx,
                &down_summary(
                    &ctx.out,
                    &docker::service_names(before),
                    match &removed {
                        Some(names) => DownVolumes::Removed(names),
                        None => DownVolumes::Kept,
                    },
                    project.compose_project_name().as_deref(),
                ),
            );
            // Said after the line that names what went, because it is a
            // consequence of it.
            if let Some(names) = &removed
                && let Some(line) = forget_dhis2_connect(project, names)
            {
                note(ctx, &ctx.out.backticks(&line));
            }
            note(ctx, &ctx.out.backticks(down_next(removed.is_some())));
        }
        DockerCmd::Pull(_) => note(
            ctx,
            &ctx.out.ok(&pull_summary(docker::image_count(project))),
        ),
        _ => {}
    }
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

/// The line a `down` ends on: what brings the deployment back, and after a
/// `--volumes` that there is nothing left in the folder worth keeping.
pub fn down_next(volumes_removed: bool) -> &'static str {
    match volumes_removed {
        false => "run `chaps up` to start it again, with its data",
        true => {
            "run `chaps up` to start it again with empty data, or delete this folder: its containers \
             and volumes are gone"
        }
    }
}

/// Forget a recorded `chaps dhis2 connect` when this run has just removed the
/// database it was true of, and give back the line that says so.
///
/// `chaps down --volumes` is the one wrapper that destroys data, and `dhis2_db`
/// is one of the volumes it takes. The record left behind would then be a
/// record of a route in a database that no longer exists, suppressing the hint
/// on the next `chaps up` - which restores the seed dump, and that dump ships a
/// `chap` route pointing at a CHAP this deployment has nothing to do with.
///
/// Measured against what docker no longer holds rather than against the flag on
/// the command line: compose removes the volumes its own files declare, so a
/// `down --volumes` on a deployment whose `compose.dhis2.yml` has already gone
/// leaves `dhis2_db` exactly where it was. `None` when there was no record,
/// when this deployment can name no volumes, or when that volume survived - in
/// each case nothing was changed and there is nothing to say.
fn forget_dhis2_connect(project: &mut Project, removed: &[String]) -> Option<String> {
    project.state.components.dhis2.connected_at.as_ref()?;
    let volume = project.prefixed_volume(crate::compose::render::DHIS2_DB_VOLUME)?;
    if !removed.contains(&volume) {
        return None;
    }
    project.state.components.dhis2.connected_at = None;
    // A state file that could not be written is worth a line of its own: the
    // record is still there, and the next `chaps up` will still be quiet about
    // connecting. The `down` itself succeeded and is not failed for it.
    if let Err(why) = project.save() {
        crate::output::warn(&format!(
            "the record of `chaps dhis2 connect` could not be cleared from \
             `.chaps/components.yaml`: {why}; run `chaps dhis2 connect` after the next `chaps up`"
        ));
        return None;
    }
    Some(
        match project.state.components.dhis2_seed_source() {
            Some(_) => DHIS2_CONNECT_FORGOTTEN_WITH_VOLUME,
            None => crate::components::DHIS2_CONNECT_FORGOTTEN_UNSEEDED,
        }
        .to_string(),
    )
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
///
/// A deployment with a DHIS2 nothing has connected gets one more line under
/// that, because this is the run the reader is about to wait minutes for and
/// the line that named `chaps dhis2 connect` scrolled past when the component
/// was added. It comes off `.chaps/components.yaml` alone - `up` asks DHIS2
/// nothing, and there would be nothing to ask yet - so it says what chaps has
/// recorded rather than what DHIS2 is. See [`dhis2_connect_hint`].
///
/// Not on the run that started nothing at all: there is no deployment up to
/// connect, and the line above already says to go and read the logs.
pub fn up_summary(
    out: &Out,
    before: &[docker::Container],
    after: &[docker::Container],
    components: &Components,
) -> String {
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
    let mut text = format!("{summary}\n{}", out.backticks(AFTER_UP));
    if components.dhis2_needs_connecting() {
        text.push('\n');
        text.push_str(&out.backticks(&dhis2_connect_hint(false)));
    }
    text
}

/// What `restart` recreated, from the containers before and after.
///
/// Compose recreates only the containers that no longer match the files, and
/// says nothing at all about the ones it skipped; this is the line that says
/// which half each service fell in. A run that recreated nothing is the
/// answer "nothing had moved on under you", not a failure.
///
/// `named` are the services the command was given, which is what the way to
/// force it spells out; none named is the whole project.
pub fn restart_summary(
    out: &Out,
    before: &[docker::Container],
    after: &[docker::Container],
    named: &[String],
) -> String {
    let (recreated, unchanged) = docker::diff_containers(before, after);
    if recreated.is_empty() {
        let force = match named {
            [] => "`chaps restart --all` recreates every one anyway".to_string(),
            [one] => format!("`chaps restart --all {one}` recreates it anyway"),
            many => format!(
                "`chaps restart --all {}` recreates them anyway",
                many.join(" ")
            ),
        };
        return out.backticks(&format!(
            "nothing needed a restart: every container matches its files; {force}"
        ));
    }
    let head = format!(
        "{} {}",
        out.ok("recreated:"),
        out.value(&recreated.join(", "))
    );
    let head = match unchanged.is_empty() {
        true => head,
        false => format!(
            "{head}; {}",
            out.dim(&format!("unchanged: {}", unchanged.join(", ")))
        ),
    };
    format!("{head}\n{}", out.backticks(AFTER_UP))
}

/// What `down` did about the volumes, for the second half of its line.
#[derive(Debug, Clone, Copy)]
pub enum DownVolumes<'a> {
    /// A plain `down`: the data is all still there.
    Kept,
    /// `down --volumes`: the volumes docker no longer holds, which is what
    /// the run actually removed rather than what it set out to.
    Removed(&'a [String]),
}

/// The volumes of `before` that docker no longer holds.
///
/// Compose removes the volumes its own files declare, so a leftover from a
/// disabled model under the same prefix survives a `down --volumes`; asking
/// docker again is the only way to say which ones went.
fn removed_volumes(project: &Project, before: &[String]) -> Vec<String> {
    let left = match project.volume_prefix() {
        Some(prefix) => docker::volume_names_with_prefix(&prefix),
        None => Vec::new(),
    };
    before
        .iter()
        .filter(|name| !left.contains(name))
        .cloned()
        .collect()
}

/// What `down` stopped, and what became of the volumes.
///
/// The volumes are the point of the second half: `down` is the command people
/// reach for to "reset" a deployment, and it keeps the database. The compose
/// project name goes with them, because it is the prefix those volumes carry
/// and therefore what `docker volume ls` has to be asked about. A
/// `--volumes` run names what it removed instead, by name: the deployment
/// whose data is gone is not the place for a count alone.
pub fn down_summary(
    out: &Out,
    stopped: &[String],
    volumes: DownVolumes,
    project_name: Option<&str>,
) -> String {
    let head = if stopped.is_empty() {
        out.dim("nothing was running")
    } else {
        format!(
            "{} {}",
            out.warn("stopped:"),
            out.dim(&format!(
                "{} ({} container{})",
                stopped.join(", "),
                stopped.len(),
                if stopped.len() == 1 { "" } else { "s" }
            ))
        )
    };
    match volumes {
        // Nothing ran and nothing was asked about the volumes: one clause
        // says everything there is to say.
        DownVolumes::Kept if stopped.is_empty() => head,
        DownVolumes::Kept => {
            let kept = match project_name {
                Some(name) => {
                    format!("volumes kept: {name}_* (`chaps down --volumes` removes them)")
                }
                None => "volumes kept (`chaps down --volumes` removes them)".to_string(),
            };
            format!("{head}; {}", out.backticks(&kept))
        }
        DownVolumes::Removed([]) => format!("{head}; {}", out.dim("no volumes were removed")),
        DownVolumes::Removed(names) => format!(
            "{head}; {} {}",
            out.warn(&format!("removed {}", volume_count(names.len()))),
            out.value(&format!("({})", names.join(", ")))
        ),
    }
}

/// The "there is nothing here" answer `logs` and `ps` give a project whose
/// containers have never been created.
fn nothing_running(out: &Out) -> String {
    out.backticks(NOTHING_RUNNING)
}

/// The "there is nothing to recreate" answer `restart` gives a deployment
/// that is not running.
fn not_running(out: &Out) -> String {
    out.backticks(NOT_RUNNING)
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

/// `--remove-orphans`, unless the caller already passed it.
///
/// `chaps` owns the whole `-f` list, so a container of this project whose
/// service no longer appears anywhere in it is genuinely an orphan: a model or
/// a component that was disabled, whose definition `sync` then removed.
/// Without this, following the closing line of `models disable` and running
/// `chaps up` would leave that container running and its host port published,
/// which is the opposite of what disabling something means.
///
/// The list is rendered from `.chaps/`, so a service of the operator's own is
/// only at risk when they started it under this deployment's compose project
/// name; run from a project of its own it is another project's container and
/// none of ours. `docs/concepts.md` says so where people will look for it.
fn remove_orphans(extra: &[String]) -> Option<String> {
    let already = extra.iter().any(|arg| arg == "--remove-orphans");
    (!already).then(|| "--remove-orphans".to_string())
}

/// Stop and remove the containers of services that are losing their
/// definition, and say in one line what happened.
///
/// `losing` picks them out of this project's containers: the model service
/// `models disable` just took out of the state, or the services of a component
/// `components disable` turned off.
///
/// Called before the compose files are rewritten, while compose still knows
/// those services: afterwards `docker compose stop ocs` is `no such service`,
/// and the container would keep running - and keep its host port published -
/// until the next `chaps up` removed it as an orphan.
///
/// Best-effort in every direction. A docker that cannot be asked, a stop that
/// failed or a project that was never started all yield a line or nothing at
/// all, never an error: the state edit is what the command is for, and
/// `chaps up` cleans up whatever is left.
pub fn stop_and_remove(project: &Project, losing: &dyn Fn(&str) -> bool) -> Option<String> {
    let containers = docker::all_containers(project)?;
    let services: Vec<String> = docker::service_names(&containers)
        .into_iter()
        .filter(|service| losing(service))
        .collect();
    if services.is_empty() {
        return None;
    }
    let ran = |verb: &[&str]| {
        let mut args: Vec<String> = verb.iter().map(|s| s.to_string()).collect();
        args.extend(services.iter().cloned());
        matches!(docker::compose_output(project, &args), Ok((0, _, _)))
    };
    // Stop first: that is what frees the port, and `rm` refuses a running
    // container without it.
    let what = match services.len() {
        1 => format!("the {} container", services[0]),
        count => format!("{count} containers ({})", services.join(", ")),
    };
    if ran(&["stop"]) && ran(&["rm", "-f"]) {
        let ports = if services.len() == 1 {
            "the host port it published is"
        } else {
            "the host ports they published are"
        };
        return Some(format!("stopped and removed {what}; {ports} free again"));
    }
    let orphans = if services.len() == 1 {
        "it as an orphan"
    } else {
        "them as orphans"
    };
    Some(format!(
        "{what} could not be stopped; `chaps up` removes {orphans}"
    ))
}

/// What a `disable` says when the data volume cannot be named at all.
///
/// [`crate::project::Project::prefixed_volume`] answers `None` only for a
/// directory that records no compose project name and whose own name
/// normalises to nothing, which `chaps sync` fixes by writing one down.
pub const UNNAMEABLE_VOLUME: &str = "this directory has no compose project name, so its data volume \
     cannot be named; `chaps sync` records one";

/// What became of one data volume a `--purge` asked for, and the line that
/// reports it.
///
/// The name is returned only when the volume was there and is gone, which is
/// what the `purged` list of a `--json` report holds. Everything else - never
/// created, already removed, a docker that could not be asked - is a line and
/// no name, because nothing was removed.
///
/// Best-effort like [`stop_and_remove`], and for the same reason: the state
/// edit is what `disable` is for, and a volume that would not go is a thing
/// to say rather than a thing to fail over.
pub fn purge_volume(name: &str) -> (Option<String>, String) {
    let outcome = docker::remove_volume(name);
    let removed = matches!(outcome, docker::Removal::Removed).then(|| name.to_string());
    (removed, removal_line(name, &outcome))
}

/// The line one attempted volume removal is reported with.
pub fn removal_line(name: &str, outcome: &docker::Removal) -> String {
    match outcome {
        docker::Removal::Removed => format!("removed volume {name}"),
        docker::Removal::NotFound => format!("volume {name} not found"),
        docker::Removal::Refused(why) => format!("volume {name} could not be removed: {why}"),
    }
}

/// The line a `disable` without `--purge` closes with: the data volume it
/// kept, and the two ways to remove it.
///
/// `purge` is the command that would have taken it, typed as the reader would
/// type it again (`chaps models disable ewars`, `chaps components disable
/// ocs`). Both ways are given because the second works from any directory and
/// after the deployment itself is gone. `None` is for a command after which
/// no chaps command names the volume any more, such as `chaps models remove`:
/// only `docker volume rm` is left to offer.
pub fn kept_volume_line(name: &str, purge: Option<&str>) -> String {
    match purge {
        Some(purge) => format!(
            "kept volume {name}; remove it with `{purge} --purge` or `docker volume rm {name}`"
        ),
        None => format!("kept volume {name}; remove it with `docker volume rm {name}`"),
    }
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
            out.extend(remove_orphans(&args.extra));
            if args.pull {
                out.push("--pull".to_string());
                out.push("always".to_string());
            }
            out.extend(args.extra.iter().cloned());
            out
        }
        DockerCmd::Down(args) => {
            let mut out = vec!["down".to_string()];
            // With the volumes go the containers of services this project no
            // longer declares: a `down --volumes` that left an orphan running
            // would leave it holding the data it was asked to destroy.
            if args.volumes {
                out.push("-v".to_string());
                out.extend(remove_orphans(&args.extra));
            }
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
            // `restart` passes nothing through, so there is no caller's
            // `--remove-orphans` to avoid repeating.
            out.extend(remove_orphans(&[]));
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
///
/// When every taken port is published by another chaps deployment, that is
/// almost always the one to put away, so `up` offers to: at a terminal it asks,
/// with `--replace` it does it without asking, and otherwise it refuses and
/// names both ways.
fn preflight(ctx: &Ctx, project: &Project, replace: bool) -> Result<()> {
    let claims = ports::claims(project);
    if claims.is_empty() {
        return Ok(());
    }
    let running = docker::running_services(project);
    let busy = ports::busy_claims(&claims, &running, &ports::is_busy);
    if busy.is_empty() {
        return Ok(());
    }
    // Who else publishes each port, and a free port above each one, found the
    // way `init` finds them: a port another deployment publishes is as good
    // as taken when suggesting one.
    let others = ports::other_deployments(&project.dir, &docker::compose_ls_json);
    let taken = |port: u16| ports::is_busy(port) || others.iter().any(|other| other.holds(port));
    let conflicts: Vec<ports::Conflict> = busy
        .into_iter()
        .map(|claim| ports::Conflict {
            suggestion: ports::first_free(claim.port.saturating_add(1), u16::MAX, &taken),
            holders: others
                .iter()
                .filter(|other| other.holds(claim.port))
                .collect(),
            claim,
        })
        .collect();
    let message = ports::preflight_message(&conflicts);
    // Only when every conflict is another chaps deployment: a port some other
    // program holds is not chaps' to take away.
    if conflicts.iter().any(|conflict| conflict.holders.is_empty()) {
        return Err(anyhow::anyhow!(message));
    }
    let mut holders: Vec<&ports::Deployment> = Vec::new();
    for conflict in &conflicts {
        for held in &conflict.holders {
            if !holders.iter().any(|seen| seen.dir == held.dir) {
                holders.push(held);
            }
        }
    }
    let names = holders
        .iter()
        .map(|held| held.name())
        .collect::<Vec<_>>()
        .join(", ");
    if !replace {
        let interactive = !ctx.out.json && std::io::stdin().is_terminal();
        if !interactive {
            return Err(anyhow::anyhow!(
                "{message}\n  or run `chaps up --replace` to stop {names} first"
            ));
        }
        eprintln!("{message}");
        eprint!(
            "\n{names} {} using these ports. Stop {} and start this deployment instead? \
             Its data is kept. [y/N] ",
            if holders.len() == 1 { "is" } else { "are" },
            if holders.len() == 1 { "it" } else { "them" },
        );
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        std::io::stdin()
            .lock()
            .read_line(&mut answer)
            .map_err(|e| anyhow::anyhow!("reading the answer: {e}"))?;
        if !matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
            return Err(anyhow::anyhow!("cancelled; nothing was started or stopped"));
        }
    }
    for held in &holders {
        let other = Project::load(&held.dir)?;
        note(
            ctx,
            &format!(
                "stopping {} ({}) to free its ports; `chaps -C {} up` starts it again",
                held.name(),
                held.dir.display(),
                held.dir.display()
            ),
        );
        docker::run_compose(&other, &["down".to_string()])?;
    }
    // What stopped them is what is asked again: a port something else took
    // in the meantime is the refusal it always was.
    let busy = ports::busy_claims(&claims, &running, &ports::is_busy);
    if !busy.is_empty() {
        let still: Vec<String> = busy.iter().map(|claim| claim.port.to_string()).collect();
        return Err(anyhow::anyhow!(
            "stopped {names}, and port {} is still in use; run `chaps up` again to see by what",
            still.join(", ")
        ));
    }
    Ok(())
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
