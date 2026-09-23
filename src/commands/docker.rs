//! `chaps up|down|logs` and the `chaps docker` group — the docker compose
//! wrappers.
//!
//! Every one of them builds an argument list and hands it to the same runner,
//! so they all get the project's explicit `-f` list and the same exit-code
//! behaviour. `up` syncs the compose files from `.chaps/` first; the others
//! run against whatever is on disk.

use crate::cli::DockerCmd;
use crate::commands::Ctx;
use crate::compose::sync;
use crate::docker;
use crate::error::{ChapError, Result};
use crate::ports;
use crate::project::Project;
use crate::registry;
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

/// Run one docker compose wrapper against the project's explicit `-f` list.
///
/// A non-zero child exit status surfaces as [`ChapError::DockerFailed`] so
/// `main` can mirror the code: `chaps up` is then a drop-in for
/// `docker compose up` in scripts.
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

    let args = args_for(cmd, Shell::detect(ctx.out.json));
    let code = docker::run_compose(&project, &args)?;
    if code != 0 {
        return Err(ChapError::DockerFailed(code).into());
    }
    Ok(())
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
        eprintln!("warning: {warning}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{ConfigArgs, DownArgs, ExecArgs, LogsArgs, PsArgs, PullArgs, RunArgs, UpArgs};

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
}
