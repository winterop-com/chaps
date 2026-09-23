//! Thin wrappers around `docker compose`.
//!
//! Every invocation passes the project's explicit `-f` list, so the wrappers
//! work from any working directory and never depend on compose's own file
//! discovery.
//!
//! Owned by agent C.

use crate::error::{ChapError, Result};
use crate::project::Project;
use std::collections::BTreeSet;
use std::process::{Command, ExitStatus, Stdio};

/// Minimum compose version that understands `include:`.
pub const MIN_COMPOSE_VERSION: (u32, u32, u32) = (2, 20, 0);

/// Exit code used when the `docker` binary itself cannot be run, mirroring the
/// shell's "command not found".
pub const DOCKER_NOT_FOUND: i32 = 127;

/// The leading `docker` arguments for a project: `compose -f ... -f ...`.
///
/// The paths are absolute so the child process does not depend on its working
/// directory, and they are in the order recorded in `.chaps/project.yaml`: later files
/// override earlier ones.
/// Echo a `docker` invocation under `-v`, the way a person would have typed it.
///
/// Every spawn in this module goes through it, so `-v` is a complete record of
/// what `chaps` asked Docker to do.
fn trace_command(args: &[String]) {
    crate::output::verbose(&format!("$ docker {}", args.join(" ")));
}

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
    trace_command(&args);

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

/// Compose services of this project that have a running container.
///
/// Best-effort by design: this only sharpens a hint in `chaps status`, so no
/// docker, no daemon, or an output shape we do not recognise all yield an
/// empty set rather than an error. Nothing is printed either - `status` has
/// already said what it knows about the API.
pub fn running_services(project: &Project) -> BTreeSet<String> {
    match compose_capture(project, &["ps", "--format", "json"]) {
        Some(text) => parse_ps_json(&text),
        None => BTreeSet::new(),
    }
}

/// The services named by `docker compose ps --format json`.
///
/// Compose 2.21 and newer print one JSON object per line; older versions print
/// a single array. Both are accepted, and an entry whose `State` is something
/// other than `running` is left out (`ps` without `-a` mostly does that
/// already, but a restarting container shows up there).
pub fn parse_ps_json(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for value in ps_entries(text) {
        let Some(service) = value.get("Service").and_then(|s| s.as_str()) else {
            continue;
        };
        let state = value.get("State").and_then(|s| s.as_str()).unwrap_or("");
        if state.is_empty() || state.eq_ignore_ascii_case("running") {
            out.insert(service.to_string());
        }
    }
    out
}

/// One container of this project, as `docker compose ps` reports it.
///
/// Only the fields the wrappers reason about: which service it belongs to,
/// whether it is up, and the pair that says whether it is the same container
/// as before (`id` plus `created_at`, because a recreated service keeps its
/// name but gets both anew).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Container {
    pub service: String,
    pub name: String,
    pub id: String,
    pub created_at: String,
    pub state: String,
}

impl Container {
    /// Whether this container is up. A `ps` that reported no state at all is
    /// treated as running: plain `ps` lists what is up.
    pub fn is_running(&self) -> bool {
        self.state.is_empty() || self.state.eq_ignore_ascii_case("running")
    }

    /// The identity `up` compares before and after: a restarted service keeps
    /// its service name but gets a new container id and creation time.
    fn identity(&self) -> (&str, &str, &str) {
        (
            self.service.as_str(),
            self.id.as_str(),
            self.created_at.as_str(),
        )
    }
}

/// Parse `docker compose ps [--all] --format json` into containers.
///
/// Both shapes compose prints are accepted (see [`ps_entries`]); an entry
/// without a `Service` is skipped, because nothing can be said about it.
pub fn containers(text: &str) -> Vec<Container> {
    ps_entries(text)
        .iter()
        .filter_map(|value| {
            let string = |key: &str| {
                value
                    .get(key)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            let service = string("Service");
            if service.is_empty() {
                return None;
            }
            Some(Container {
                service,
                name: string("Name"),
                id: string("ID"),
                created_at: string("CreatedAt"),
                state: string("State"),
            })
        })
        .collect()
}

/// The services with a running container, out of a [`containers`] list.
pub fn running_of(containers: &[Container]) -> BTreeSet<String> {
    containers
        .iter()
        .filter(|c| c.is_running())
        .map(|c| c.service.clone())
        .collect()
}

/// Service names of `containers`, deduplicated, in the order compose listed
/// them.
pub fn service_names(containers: &[Container]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    containers
        .iter()
        .filter(|c| seen.insert(c.service.clone()))
        .map(|c| c.service.clone())
        .collect()
}

/// Which services of `after` are new or were recreated, and which are the
/// containers that were already there, as two lists of service names.
///
/// The comparison is by container id and creation time, which is what tells a
/// service compose left alone from one it replaced: the container name is the
/// same either way.
pub fn diff_containers(before: &[Container], after: &[Container]) -> (Vec<String>, Vec<String>) {
    let kept: BTreeSet<(&str, &str, &str)> = before.iter().map(Container::identity).collect();
    let mut started = Vec::new();
    let mut unchanged = Vec::new();
    for container in after {
        let list = if kept.contains(&container.identity()) {
            &mut unchanged
        } else {
            &mut started
        };
        if !list.contains(&container.service) {
            list.push(container.service.clone());
        }
    }
    (started, unchanged)
}

/// Every container of this project, stopped ones included
/// (`docker compose ps -a`).
///
/// `None` means docker could not be asked at all - no binary, no daemon, or
/// output in a shape we do not recognise - which callers have to tell apart
/// from `Some(vec![])`, "this project has no containers".
pub fn all_containers(project: &Project) -> Option<Vec<Container>> {
    let text = compose_capture(project, &["ps", "-a", "--format", "json"])?;
    Some(containers(&text))
}

/// The containers of this project that are up (`docker compose ps`).
///
/// `None` when docker could not be asked, as in [`all_containers`].
pub fn running_containers(project: &Project) -> Option<Vec<Container>> {
    let text = compose_capture(project, &["ps", "--format", "json"])?;
    Some(containers(&text))
}

/// The services this project's compose files define
/// (`docker compose config --services`), or `None` when docker could not be
/// asked.
pub fn config_services(project: &Project) -> Option<Vec<String>> {
    let text = compose_capture(project, &["config", "--services"])?;
    let mut names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    Some(names)
}

/// How many distinct images this project's stack pins
/// (`docker compose config --images`), or `None` when docker could not be
/// asked. chap and its worker share one image, so the list is deduplicated.
pub fn image_count(project: &Project) -> Option<usize> {
    let text = compose_capture(project, &["config", "--images"])?;
    let images: BTreeSet<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    Some(images.len())
}

/// Run a short `docker compose` query and return its stdout, or `None` when
/// docker could not be run or answered non-zero.
///
/// Nothing is printed on failure: every caller has a better thing to say than
/// docker's own noise, and stderr is discarded for that reason.
fn compose_capture(project: &Project, extra: &[&str]) -> Option<String> {
    let mut args = compose_args(project);
    args.extend(extra.iter().map(|a| a.to_string()));
    trace_command(&args);
    let out = Command::new("docker")
        .args(&args)
        .current_dir(&project.dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        crate::output::verbose("  docker answered non-zero; ignoring its output");
        return None;
    }
    let body = String::from_utf8_lossy(&out.stdout).into_owned();
    crate::output::debug(&format!(
        "  docker said:\n{}",
        crate::output::trace_body(body.trim_end())
    ));
    Some(body)
}

/// The container objects in `docker compose ps --format json` output.
///
/// Compose 2.21 and newer print one JSON object per line; older versions print
/// a single array. Both are accepted, and anything that is not JSON at all
/// yields nothing rather than an error.
pub fn ps_entries(text: &str) -> Vec<serde_json::Value> {
    let trimmed = text.trim();
    if trimmed.starts_with('[')
        && let Ok(serde_json::Value::Array(items)) =
            serde_json::from_str::<serde_json::Value>(trimmed)
    {
        return items;
    }
    trimmed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Whether `docker compose ps --format json` shows `service` ready to be used:
/// running, and healthy when it declares a healthcheck at all.
///
/// This is what the wait before `pg_restore` polls; a container that is up but
/// still in `starting` would refuse the connection.
pub fn service_is_healthy(text: &str, service: &str) -> bool {
    ps_entries(text).iter().any(|value| {
        if value.get("Service").and_then(|s| s.as_str()) != Some(service) {
            return false;
        }
        let state = value.get("State").and_then(|s| s.as_str()).unwrap_or("");
        if !(state.is_empty() || state.eq_ignore_ascii_case("running")) {
            return false;
        }
        let health = value.get("Health").and_then(|s| s.as_str()).unwrap_or("");
        health.is_empty() || health.eq_ignore_ascii_case("healthy")
    })
}

/// Run `docker compose <args>` and capture everything it prints.
///
/// For the short, machine-read commands (`ps --format json`, a `psql -tAc`
/// one-liner): both streams are pipes, so this must stay away from anything
/// that produces real volume. Returns the exit code, stdout and stderr; a
/// non-zero exit is the caller's to interpret.
pub fn compose_output(project: &Project, extra: &[String]) -> Result<(i32, String, String)> {
    let mut args = compose_args(project);
    args.extend(extra.iter().cloned());
    trace_command(&args);
    let out = Command::new("docker")
        .args(&args)
        .current_dir(&project.dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| spawn_error(&e))?;
    Ok((
        exit_code(out.status),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    ))
}

/// What a redirected compose command did.
#[derive(Debug, Clone)]
pub struct Piped {
    pub code: i32,
    /// Everything the command wrote to stderr.
    pub stderr: String,
}

/// Run `docker compose <args>` with its payload streams wired to files.
///
/// This is how the dumps travel: `pg_dump` writes gigabytes to `stdout`, and a
/// model tar arrives on `stdin`, so neither may be a pipe this process has to
/// drain - a full pipe buffer with nobody reading it is a deadlock. Only
/// stderr is a pipe, and it is read while the child runs.
pub fn run_compose_piped(
    project: &Project,
    extra: &[String],
    stdin: Option<&std::path::Path>,
    stdout: Option<&std::path::Path>,
) -> Result<Piped> {
    use std::io::Read;

    let mut args = compose_args(project);
    args.extend(extra.iter().cloned());

    trace_command(&args);
    let mut cmd = Command::new("docker");
    cmd.args(&args).current_dir(&project.dir);
    match stdin {
        Some(path) => {
            let file = std::fs::File::open(path)
                .map_err(|e| anyhow::anyhow!("opening {}: {e}", path.display()))?;
            cmd.stdin(Stdio::from(file));
        }
        None => {
            cmd.stdin(Stdio::null());
        }
    }
    match stdout {
        Some(path) => {
            let file = std::fs::File::create(path)
                .map_err(|e| anyhow::anyhow!("creating {}: {e}", path.display()))?;
            cmd.stdout(Stdio::from(file));
        }
        None => {
            cmd.stdout(Stdio::null());
        }
    }
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| spawn_error(&e))?;
    let mut stderr = String::new();
    if let Some(mut stream) = child.stderr.take() {
        let _ = stream.read_to_string(&mut stderr);
    }
    let status = child
        .wait()
        .map_err(|e| anyhow::anyhow!("waiting for docker: {e}"))?;
    Ok(Piped {
        code: exit_code(status),
        stderr,
    })
}

/// The compose project name, the prefix every container and named volume of
/// this deployment carries.
///
/// Read from `docker compose config`, which applies the same rules compose
/// itself does (the directory name, normalised, unless a `name:` key or
/// `COMPOSE_PROJECT_NAME` says otherwise). Best-effort: `None` when docker
/// cannot be reached, so callers degrade rather than fail.
pub fn compose_project_name(project: &Project) -> Option<String> {
    let args = [
        "config".to_string(),
        "--format".to_string(),
        "json".to_string(),
    ];
    let (code, stdout, _) = compose_output(project, &args).ok()?;
    if code != 0 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&stdout).ok()?;
    value
        .get("name")
        .and_then(|n| n.as_str())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
}

/// Whether a named docker volume exists.
pub fn volume_exists(name: &str) -> bool {
    trace_command(&[
        "volume".to_string(),
        "inspect".to_string(),
        name.to_string(),
    ]);
    Command::new("docker")
        .args(["volume", "inspect", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The installed `docker compose` version, from `docker compose version --short`.
pub fn compose_version() -> Result<(u32, u32, u32)> {
    trace_command(&[
        "compose".to_string(),
        "version".to_string(),
        "--short".to_string(),
    ]);
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
                // The chaps-owned overrides sit between the base file and the
                // umbrella: later files win, and this one overrides chap.
                "-f".to_string(),
                PathBuf::from("/tmp/chapx/compose.chaps.yml")
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
    fn ps_json_is_read_in_both_shapes_compose_prints() {
        // Compose 2.21+: one object per line.
        let lines = concat!(
            r#"{"Name":"x-chap-1","Service":"chap","State":"running","Health":"healthy"}"#,
            "\n",
            r#"{"Name":"x-ewars-1","Service":"chapkit-ewars-model","State":"running"}"#,
            "\n",
            r#"{"Name":"x-init-1","Service":"chapkit-ewars-model-init","State":"exited"}"#,
            "\n"
        );
        let running = parse_ps_json(lines);
        assert_eq!(
            running,
            BTreeSet::from(["chap".to_string(), "chapkit-ewars-model".to_string()]),
            "an exited container is not running"
        );

        // Older compose: a single array.
        let array =
            r#"[{"Service":"chap","State":"running"},{"Service":"worker","State":"restarting"}]"#;
        assert_eq!(parse_ps_json(array), BTreeSet::from(["chap".to_string()]));

        // A state compose did not report still counts: `ps` without -a lists
        // what is up.
        assert_eq!(
            parse_ps_json(r#"{"Service":"chap"}"#),
            BTreeSet::from(["chap".to_string()])
        );
    }

    #[test]
    fn unreadable_ps_output_is_an_empty_set_not_a_panic() {
        for text in [
            "",
            "   \n\n",
            "no containers",
            "<html>",
            r#"{"Name":"x-chap-1"}"#,
            "[]",
            "{}",
        ] {
            assert!(parse_ps_json(text).is_empty(), "{text:?}");
        }
    }

    /// Two containers, as `ps -a --format json` prints them.
    const PS_A: &str = concat!(
        r#"{"ID":"aaa","Name":"x-chap-1","Service":"chap","State":"running","#,
        r#""CreatedAt":"2026-09-23 12:58:52 +0200 CEST"}"#,
        "\n",
        r#"{"ID":"bbb","Name":"x-init-1","Service":"ewars-init","State":"exited","#,
        r#""CreatedAt":"2026-09-23 12:58:52 +0200 CEST"}"#,
        "\n"
    );

    #[test]
    fn containers_are_read_with_the_fields_the_wrappers_use() {
        let found = containers(PS_A);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].service, "chap");
        assert_eq!(found[0].name, "x-chap-1");
        assert_eq!(found[0].id, "aaa");
        assert_eq!(found[0].created_at, "2026-09-23 12:58:52 +0200 CEST");
        assert!(found[0].is_running());
        assert!(!found[1].is_running(), "an exited container is not running");

        assert_eq!(running_of(&found), BTreeSet::from(["chap".to_string()]));
        assert_eq!(service_names(&found), vec!["chap", "ewars-init"]);

        // Nothing usable in, nothing out.
        for text in ["", "   ", "no containers", "<html>", "[]", r#"{"ID":"a"}"#] {
            assert!(containers(text).is_empty(), "{text:?}");
            assert!(service_names(&containers(text)).is_empty(), "{text:?}");
        }
    }

    /// One container, spelled out so a test can vary a single field.
    fn container(service: &str, id: &str, created: &str) -> Container {
        Container {
            service: service.to_string(),
            name: format!("x-{service}-1"),
            id: id.to_string(),
            created_at: created.to_string(),
            state: "running".to_string(),
        }
    }

    #[test]
    fn the_diff_splits_recreated_services_from_untouched_ones() {
        let before = vec![
            container("chap", "aaa", "t1"),
            container("postgres", "ccc", "t1"),
        ];
        let after = vec![
            // Same container: compose left it alone.
            container("postgres", "ccc", "t1"),
            // Same service, new container: recreated.
            container("chap", "ddd", "t2"),
            // Not there before at all: started.
            container("worker", "eee", "t2"),
        ];
        let (started, unchanged) = diff_containers(&before, &after);
        assert_eq!(started, vec!["chap", "worker"]);
        assert_eq!(unchanged, vec!["postgres"]);

        // A first start: everything is new.
        let (started, unchanged) = diff_containers(&[], &after);
        assert_eq!(started, vec!["postgres", "chap", "worker"]);
        assert!(unchanged.is_empty());

        // A no-op `up`: nothing moved.
        let (started, unchanged) = diff_containers(&after, &after);
        assert!(started.is_empty());
        assert_eq!(unchanged, vec!["postgres", "chap", "worker"]);

        // A container recreated with the same id (never seen in practice, but
        // the creation time still says it is a different one).
        let (started, _) = diff_containers(
            &[container("chap", "aaa", "t1")],
            &[container("chap", "aaa", "t2")],
        );
        assert_eq!(started, vec!["chap"]);
    }

    #[test]
    fn health_is_read_from_the_same_ps_output() {
        let lines = concat!(
            r#"{"Service":"postgres","State":"running","Health":"starting"}"#,
            "\n",
            r#"{"Service":"chap","State":"running","Health":"healthy"}"#,
            "\n",
            r#"{"Service":"redis","State":"exited","Health":"healthy"}"#,
            "\n"
        );
        assert_eq!(ps_entries(lines).len(), 3);
        assert!(service_is_healthy(lines, "chap"));
        assert!(!service_is_healthy(lines, "postgres"), "still starting");
        assert!(!service_is_healthy(lines, "redis"), "not running");
        assert!(!service_is_healthy(lines, "worker"), "not there at all");

        // No healthcheck at all: running is as good as it gets.
        assert!(service_is_healthy(
            r#"{"Service":"postgres","State":"running"}"#,
            "postgres"
        ));
        // The array shape older compose prints.
        assert!(service_is_healthy(
            r#"[{"Service":"postgres","State":"running","Health":"healthy"}]"#,
            "postgres"
        ));
        for text in ["", "not json", "[]"] {
            assert!(!service_is_healthy(text, "postgres"), "{text:?}");
            assert!(ps_entries(text).is_empty(), "{text:?}");
        }
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
