//! Thin wrappers around `docker compose`.
//!
//! Every invocation passes the project's explicit `-f` list, so the wrappers
//! work from any working directory and never depend on compose's own file
//! discovery.
//!
//! Owned by agent C.

use crate::error::{ChapError, Result};
use crate::project::Project;
use std::process::{Command, ExitStatus, Stdio};

/// Minimum compose version that understands `include:`.
pub const MIN_COMPOSE_VERSION: (u32, u32, u32) = (2, 20, 0);

/// Exit code used when the `docker` binary itself cannot be run, mirroring the
/// shell's "command not found".
pub const DOCKER_NOT_FOUND: i32 = 127;

/// The leading `docker` arguments for a project: `compose -f ... -f ...`.
///
/// The paths are absolute so the child process does not depend on its working
/// directory, and they are in the order recorded in `chaps.json`: later files
/// override earlier ones.
pub fn compose_args(project: &Project) -> Vec<String> {
    let mut args = Vec::with_capacity(1 + project.state.compose_files.len() * 2);
    args.push("compose".to_string());
    for path in project.compose_file_paths() {
        args.push("-f".to_string());
        args.push(path.to_string_lossy().into_owned());
    }
    args
}

/// Run `docker compose <args> <extra>` with inherited stdio, returning the
/// child's exit code.
///
/// The child runs in the project directory so compose picks up `.env` and
/// names the project after the directory, exactly as a hand-written
/// `docker compose` would.
pub fn run_compose(project: &Project, extra: &[String]) -> Result<i32> {
    let mut args = compose_args(project);
    args.extend(extra.iter().cloned());

    let status = Command::new("docker")
        .args(&args)
        .current_dir(&project.dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| spawn_error(&e))?;

    Ok(exit_code(status))
}

/// The installed `docker compose` version, from `docker compose version --short`.
pub fn compose_version() -> Result<(u32, u32, u32)> {
    let out = Command::new("docker")
        .args(["compose", "version", "--short"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| spawn_error(&e))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(anyhow::anyhow!(
            "`docker compose version --short` failed{}",
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        ));
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

/// Warn when the installed compose predates [`MIN_COMPOSE_VERSION`].
///
/// Returns `Ok(None)` when compose is new enough, and an error when the version
/// cannot be determined at all (docker missing, for instance) so callers can
/// decide whether that is fatal.
pub fn check_compose_version() -> Result<Option<String>> {
    let found = compose_version()?;
    if found >= MIN_COMPOSE_VERSION {
        return Ok(None);
    }
    let (fa, fb, fc) = found;
    let (wa, wb, wc) = MIN_COMPOSE_VERSION;
    Ok(Some(format!(
        "docker compose {fa}.{fb}.{fc} is older than {wa}.{wb}.{wc}; \
         compose.marketplace.yml uses `include:`, which needs {wa}.{wb}.{wc} or newer"
    )))
}

/// Parse `docker compose version --short` output into `(major, minor, patch)`.
///
/// Accepts `v5.5.1`, `2.24.0`, `2.24.0-desktop.1` and the like: a leading `v`
/// is dropped and everything after the numeric dotted prefix is ignored.
/// Missing components default to zero, so `2.24` parses as `(2, 24, 0)`.
pub fn parse_version(text: &str) -> Result<(u32, u32, u32)> {
    let raw = text.trim();
    let trimmed = raw.strip_prefix('v').unwrap_or(raw).trim_start();
    let numeric: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();

    let mut parts = numeric
        .split('.')
        .filter(|p| !p.is_empty())
        .map(str::parse::<u32>);

    let major = match parts.next() {
        Some(Ok(n)) => n,
        _ => return Err(anyhow::anyhow!("unrecognised compose version `{raw}`")),
    };
    let minor = match parts.next() {
        Some(Ok(n)) => n,
        Some(Err(_)) => return Err(anyhow::anyhow!("unrecognised compose version `{raw}`")),
        None => 0,
    };
    let patch = match parts.next() {
        Some(Ok(n)) => n,
        Some(Err(_)) => return Err(anyhow::anyhow!("unrecognised compose version `{raw}`")),
        None => 0,
    };
    Ok((major, minor, patch))
}

/// Turn a spawn failure into an error that keeps the shell's "not found" exit
/// code, so `chaps up` behaves like the command it wraps.
fn spawn_error(err: &std::io::Error) -> anyhow::Error {
    if err.kind() == std::io::ErrorKind::NotFound {
        anyhow::Error::new(ChapError::DockerFailed(DOCKER_NOT_FOUND)).context(
            "`docker` was not found on PATH; install Docker Desktop or the docker CLI \
             (https://docs.docker.com/get-docker/)",
        )
    } else {
        anyhow::anyhow!("could not run `docker`: {err}")
    }
}

/// The child's exit code, mapping death by signal to `128 + signal` the way a
/// shell does.
fn exit_code(status: ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{MARKETPLACE_COMPOSE, ProjectState};
    use std::path::PathBuf;

    fn project(dir: &str) -> Project {
        Project {
            dir: PathBuf::from(dir),
            state: ProjectState::default(),
        }
    }

    #[test]
    fn compose_args_lists_every_file_absolutely_and_in_order() {
        let p = project("/tmp/chapx");
        assert_eq!(
            compose_args(&p),
            vec![
                "compose".to_string(),
                "-f".to_string(),
                PathBuf::from("/tmp/chapx/compose.yml")
                    .to_string_lossy()
                    .into_owned(),
                "-f".to_string(),
                PathBuf::from("/tmp/chapx/compose.marketplace.yml")
                    .to_string_lossy()
                    .into_owned(),
            ]
        );
    }

    #[test]
    fn compose_args_follows_the_state_file_list() {
        let mut p = project("/tmp/chapx");
        p.state.compose_files = vec![
            "compose.yml".to_string(),
            MARKETPLACE_COMPOSE.to_string(),
            "compose.override.yml".to_string(),
        ];
        let args = compose_args(&p);
        assert_eq!(args.iter().filter(|a| *a == "-f").count(), 3);
        assert!(args.last().unwrap().ends_with("compose.override.yml"));
    }

    #[test]
    fn compose_args_of_a_project_without_files_is_just_compose() {
        let mut p = project("/tmp/chapx");
        p.state.compose_files.clear();
        assert_eq!(compose_args(&p), vec!["compose".to_string()]);
    }

    #[test]
    fn version_parsing_handles_the_shapes_compose_prints() {
        assert_eq!(parse_version("v5.5.1").unwrap(), (5, 5, 1));
        assert_eq!(parse_version("2.24.0\n").unwrap(), (2, 24, 0));
        assert_eq!(parse_version("  v2.20.0  ").unwrap(), (2, 20, 0));
        assert_eq!(parse_version("2.24.0-desktop.1").unwrap(), (2, 24, 0));
        assert_eq!(parse_version("2.24").unwrap(), (2, 24, 0));
        assert_eq!(parse_version("3").unwrap(), (3, 0, 0));
    }

    #[test]
    fn version_parsing_rejects_nonsense() {
        for bad in ["", "unknown", "vx.y.z", "Docker Compose"] {
            assert!(parse_version(bad).is_err(), "{bad} should not parse");
        }
    }

    #[test]
    fn min_version_ordering_is_what_the_warning_uses() {
        assert!(parse_version("2.19.1").unwrap() < MIN_COMPOSE_VERSION);
        assert!(parse_version("2.20.0").unwrap() >= MIN_COMPOSE_VERSION);
        assert!(parse_version("v5.5.1").unwrap() >= MIN_COMPOSE_VERSION);
    }

    #[test]
    fn missing_docker_keeps_the_shell_not_found_code() {
        let err = spawn_error(&std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no such file",
        ));
        assert!(err.to_string().contains("not found on PATH"));
        match err.downcast_ref::<ChapError>() {
            Some(ChapError::DockerFailed(code)) => assert_eq!(*code, DOCKER_NOT_FOUND),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn other_spawn_failures_are_plain_errors() {
        let err = spawn_error(&std::io::Error::other("boom"));
        assert!(err.downcast_ref::<ChapError>().is_none());
        assert!(err.to_string().contains("could not run"));
    }
}
