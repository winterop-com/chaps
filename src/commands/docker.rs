//! `chaps up|down|ps|logs|pull|compose` — the docker compose wrappers.
//!
//! Owned by agent C.

use crate::cli::DockerCmd;
use crate::commands::Ctx;
use crate::docker;
use crate::error::{ChapError, Result};

/// Run one docker compose wrapper against the project's explicit `-f` list.
///
/// A non-zero child exit status surfaces as [`ChapError::DockerFailed`] so
/// `main` can mirror the code: `chaps up` is then a drop-in for
/// `docker compose up` in scripts.
pub fn run(ctx: &Ctx, cmd: &DockerCmd) -> Result<()> {
    let project = ctx.project()?;
    warn_about_old_compose();

    let args = args_for(cmd, ctx.out.json);
    let code = docker::run_compose(&project, &args)?;
    if code != 0 {
        return Err(ChapError::DockerFailed(code).into());
    }
    Ok(())
}

/// The arguments appended after `compose -f ... -f ...`.
///
/// `json` is the global `--json` flag: it only changes `ps`, the one wrapper
/// whose output docker itself can serialise.
pub fn args_for(cmd: &DockerCmd, json: bool) -> Vec<String> {
    match cmd {
        DockerCmd::Up(args) => {
            let mut out = vec!["up".to_string()];
            if !args.attach {
                out.push("-d".to_string());
            }
            out.extend(args.extra.iter().cloned());
            out
        }
        DockerCmd::Down(args) => {
            let mut out = vec!["down".to_string()];
            out.extend(args.extra.iter().cloned());
            out
        }
        DockerCmd::Ps(args) => {
            let mut out = vec!["ps".to_string()];
            if json {
                out.push("--format".to_string());
                out.push("json".to_string());
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
        DockerCmd::Pull(_) => vec!["pull".to_string()],
        DockerCmd::Compose(args) => args.args.clone(),
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
        eprintln!("warning: {warning}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{ComposeArgs, DownArgs, LogsArgs, PsArgs, PullArgs, UpArgs};

    fn up(attach: bool, extra: &[&str]) -> DockerCmd {
        DockerCmd::Up(UpArgs {
            attach,
            extra: extra.iter().map(|s| s.to_string()).collect(),
        })
    }

    #[test]
    fn up_detaches_unless_attach_is_asked_for() {
        assert_eq!(args_for(&up(false, &[]), false), vec!["up", "-d"]);
        assert_eq!(args_for(&up(true, &[]), false), vec!["up"]);
    }

    #[test]
    fn up_passes_extra_arguments_through_after_the_flags() {
        assert_eq!(
            args_for(&up(false, &["--build", "chap"]), false),
            vec!["up", "-d", "--build", "chap"]
        );
        assert_eq!(
            args_for(&up(true, &["chap"]), false),
            vec!["up", "chap"],
            "--attach must not inject -d before the service name"
        );
    }

    #[test]
    fn down_and_pull_are_bare_commands() {
        assert_eq!(
            args_for(&DockerCmd::Down(DownArgs { extra: vec![] }), false),
            vec!["down"]
        );
        assert_eq!(
            args_for(
                &DockerCmd::Down(DownArgs {
                    extra: vec!["-v".to_string()],
                }),
                false
            ),
            vec!["down", "-v"]
        );
        assert_eq!(
            args_for(&DockerCmd::Pull(PullArgs {}), true),
            vec!["pull"],
            "--json does not change pull"
        );
    }

    #[test]
    fn ps_asks_docker_for_json_under_the_global_flag() {
        let cmd = DockerCmd::Ps(PsArgs { extra: vec![] });
        assert_eq!(args_for(&cmd, false), vec!["ps"]);
        assert_eq!(args_for(&cmd, true), vec!["ps", "--format", "json"]);

        let cmd = DockerCmd::Ps(PsArgs {
            extra: vec!["-a".to_string()],
        });
        assert_eq!(args_for(&cmd, true), vec!["ps", "--format", "json", "-a"]);
    }

    #[test]
    fn logs_maps_follow_and_service_names() {
        let cmd = DockerCmd::Logs(LogsArgs {
            follow: true,
            services: vec!["chap".to_string(), "chapkit-ewars-model".to_string()],
        });
        assert_eq!(
            args_for(&cmd, false),
            vec!["logs", "-f", "chap", "chapkit-ewars-model"]
        );

        let cmd = DockerCmd::Logs(LogsArgs {
            follow: false,
            services: vec![],
        });
        assert_eq!(args_for(&cmd, false), vec!["logs"]);
    }

    #[test]
    fn compose_is_passed_through_verbatim() {
        let cmd = DockerCmd::Compose(ComposeArgs {
            args: vec![
                "config".to_string(),
                "--services".to_string(),
                "--quiet".to_string(),
            ],
        });
        assert_eq!(
            args_for(&cmd, true),
            vec!["config", "--services", "--quiet"]
        );

        let cmd = DockerCmd::Compose(ComposeArgs { args: vec![] });
        assert!(args_for(&cmd, false).is_empty());
    }
}
