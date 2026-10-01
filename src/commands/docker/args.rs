//! The argument list each wrapper hands to `docker compose`.

use crate::cli::DockerCmd;
use std::io::IsTerminal;

/// The command run inside the container when `chaps docker exec` is given none.
const DEFAULT_EXEC_CMD: &str = "sh";

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
