//! Thin wrappers around `docker compose`.
//!
//! Every invocation passes the project's explicit `-f` list, so the wrappers
//! work from any working directory and never depend on compose's own file
//! discovery.
//!
//! Owned by agent C.

use crate::error::{ChapError, Result};
use crate::project::Project;
use std::collections::{BTreeMap, BTreeSet};
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

/// Run `docker compose <args> <extra>` with stdout inherited and stderr teed:
/// everything docker writes there goes to this process's stderr as it
/// arrives, and is kept.
///
/// Compose says why a service would not start on stderr and nowhere else -
/// `dependency failed to start: container x-chap-1 is unhealthy` is the whole
/// of it - so a wrapper that wants to explain that failure has to have read
/// it. What is read is passed on byte for byte as it arrives, so the command
/// still reads as it runs and nothing docker wrote is reformatted, reordered
/// or held back.
///
/// The one visible difference from [`run_compose`] is compose's own doing: a
/// pipe is not a terminal, so it prints its progress as one line per step
/// rather than redrawing a block in place.
pub fn run_compose_teed(project: &Project, extra: &[String]) -> Result<Piped> {
    use std::io::{Read, Write};

    let mut args = compose_args(project);
    args.extend(extra.iter().cloned());
    trace_command(&args);

    let mut child = Command::new("docker")
        .args(&args)
        .current_dir(&project.dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| spawn_error(&e))?;

    let mut raw: Vec<u8> = Vec::new();
    if let Some(mut stream) = child.stderr.take() {
        let mut buffer = [0u8; 4096];
        loop {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let chunk = &buffer[..read];
                    let mut err = std::io::stderr();
                    let _ = err.write_all(chunk);
                    let _ = err.flush();
                    raw.extend_from_slice(chunk);
                }
            }
        }
    }
    let status = child
        .wait()
        .map_err(|e| anyhow::anyhow!("waiting for docker: {e}"))?;
    Ok(Piped {
        code: exit_code(status),
        stderr: String::from_utf8_lossy(&raw).into_owned(),
    })
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
/// whether it is up, the image reference it was created from, and the pair
/// that says whether it is the same container as before (`id` plus
/// `created_at`, because a recreated service keeps its name but gets both
/// anew).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Container {
    pub service: String,
    pub name: String,
    pub id: String,
    pub created_at: String,
    pub state: String,
    /// What the container's healthcheck last said: `healthy`, `unhealthy`,
    /// `starting`, or empty for a service that declares none.
    pub health: String,
    /// The image as compose names it, such as `ghcr.io/dhis2-chap/chap:v2.3.1`.
    /// It is the reference the container was created from, which is not the
    /// same question as which image that reference points at today.
    pub image: String,
}

impl Container {
    /// Whether this container is up. A `ps` that reported no state at all is
    /// treated as running: plain `ps` lists what is up.
    pub fn is_running(&self) -> bool {
        self.state.is_empty() || self.state.eq_ignore_ascii_case("running")
    }

    /// Whether its healthcheck is failing.
    ///
    /// This is the state `chaps up` ends on when compose says
    /// `dependency failed to start`: the container is up, so `ps` lists it,
    /// and nothing that depends on it will start.
    pub fn is_unhealthy(&self) -> bool {
        self.health.eq_ignore_ascii_case("unhealthy")
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
                health: string("Health"),
                image: string("Image"),
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
/// (`docker compose ps -a`), or why docker did not say.
///
/// An `Err` means docker could not be asked at all - no binary, no daemon, a
/// daemon in a mode that cannot serve this stack - which callers have to tell
/// apart from `Ok(vec![])`, "this project has no containers". It carries
/// docker's own words, because an exit code on its own explains none of that.
pub fn all_containers_or_why(
    project: &Project,
) -> std::result::Result<Vec<Container>, QueryFailure> {
    compose_query(project, &["ps", "-a", "--format", "json"]).map(|text| containers(&text))
}

/// [`all_containers_or_why`] for the callers that only act on an answer.
pub fn all_containers(project: &Project) -> Option<Vec<Container>> {
    all_containers_or_why(project).ok()
}

/// The containers of this project that are up (`docker compose ps`), or why
/// docker did not say, as in [`all_containers_or_why`].
pub fn running_containers_or_why(
    project: &Project,
) -> std::result::Result<Vec<Container>, QueryFailure> {
    compose_query(project, &["ps", "--format", "json"]).map(|text| containers(&text))
}

/// [`running_containers_or_why`] for the callers that only act on an answer.
pub fn running_containers(project: &Project) -> Option<Vec<Container>> {
    running_containers_or_why(project).ok()
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

/// The image reference each service of this project pins, from
/// `docker compose config --format json`.
///
/// This is the stack as the files describe it now, which is the half
/// `docker compose ps` cannot answer: `ps` reports the reference a container
/// was created from, and the point of asking both is to find where they have
/// come apart. Best-effort: an empty map when docker could not be asked.
pub fn service_images(project: &Project) -> BTreeMap<String, String> {
    match compose_capture(project, &["config", "--format", "json"]) {
        Some(text) => parse_service_images(&text),
        None => BTreeMap::new(),
    }
}

/// The `services.<name>.image` of a `docker compose config --format json`
/// document. A service built from a Dockerfile has no image and is left out.
pub fn parse_service_images(text: &str) -> BTreeMap<String, String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return BTreeMap::new();
    };
    let Some(services) = value.get("services").and_then(|s| s.as_object()) else {
        return BTreeMap::new();
    };
    services
        .iter()
        .filter_map(|(name, service)| {
            let image = service.get("image")?.as_str()?;
            (!image.is_empty()).then(|| (name.clone(), image.to_string()))
        })
        .collect()
}

/// The configuration hash compose computes for each service of this project
/// right now (`docker compose config --hash='*'`).
///
/// It is the same value compose stamps on a container as
/// `com.docker.compose.config-hash` when it creates it, so the two together
/// say whether a running container was made from the files as they are today.
/// Best-effort: an empty map when docker could not be asked.
pub fn config_hashes(project: &Project) -> BTreeMap<String, String> {
    match compose_capture(project, &["config", "--hash=*"]) {
        Some(text) => parse_config_hashes(&text),
        None => BTreeMap::new(),
    }
}

/// Parse the `<service> <hash>` lines of `docker compose config --hash='*'`.
pub fn parse_config_hashes(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (service, hash) = line.trim().split_once(char::is_whitespace)?;
            let hash = hash.trim();
            (!service.is_empty() && !hash.is_empty())
                .then(|| (service.to_string(), hash.to_string()))
        })
        .collect()
}

/// What one running container is actually made of.
///
/// The pair that decides whether it is stale: the id of the image it runs
/// (not the reference, which can point somewhere else after a pull) and the
/// configuration hash it was created with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainerBuild {
    pub image_id: String,
    pub config_hash: String,
}

/// What each of these containers is made of, keyed by the id it was asked
/// about (`docker inspect`).
///
/// One call for the whole list, matched back by id rather than by position,
/// so a container that vanished between the `ps` and the `inspect` costs its
/// own row and not the rows after it.
pub fn container_builds(ids: &[String]) -> BTreeMap<String, ContainerBuild> {
    if ids.is_empty() {
        return BTreeMap::new();
    }
    let mut args = vec![
        "inspect".to_string(),
        "--format".to_string(),
        INSPECT_FORMAT.to_string(),
    ];
    args.extend(ids.iter().cloned());
    let Some(text) = docker_capture(&args) else {
        return BTreeMap::new();
    };
    let found = parse_container_builds(&text);
    // `ps` prints short ids and `inspect` prints full ones, so the answer is
    // keyed back to the id the caller knows.
    ids.iter()
        .filter_map(|id| {
            let build = found
                .iter()
                .find(|(full, _)| full == id || full.starts_with(id.as_str()))?;
            Some((id.clone(), build.1.clone()))
        })
        .collect()
}

/// The `--format` [`container_builds`] asks for: full id, image id and
/// configuration hash, one tab-separated line per container.
///
/// `{{json .Config.Labels}}` rather than `{{index .Config.Labels "..."}}`
/// because the quotes that second spelling needs are one more thing to get
/// right on a Windows command line, and the whole label set costs nothing.
const INSPECT_FORMAT: &str = "{{.Id}}\t{{.Image}}\t{{json .Config.Labels}}";

/// The label compose stamps a container with, so a later run can tell whether
/// the files it was made from have changed.
const CONFIG_HASH_LABEL: &str = "com.docker.compose.config-hash";

/// Parse the [`INSPECT_FORMAT`] lines into `(full id, build)` pairs.
pub fn parse_container_builds(text: &str) -> Vec<(String, ContainerBuild)> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.trim_end().splitn(3, '\t');
            let id = fields.next()?.trim();
            let image_id = fields.next()?.trim();
            if id.is_empty() {
                return None;
            }
            let labels = fields.next().unwrap_or_default();
            let config_hash = serde_json::from_str::<serde_json::Value>(labels)
                .ok()
                .and_then(|v| {
                    v.get(CONFIG_HASH_LABEL)
                        .and_then(|h| h.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_default();
            Some((
                id.to_string(),
                ContainerBuild {
                    image_id: image_id.to_string(),
                    config_hash,
                },
            ))
        })
        .collect()
}

/// The local image id each of these references points at right now
/// (`docker image inspect`), leaving out the ones this machine does not have.
///
/// A reference is asked about once however many services share it, and a
/// reference docker has never pulled is simply absent: "we do not know" and
/// "it changed" must not be the same answer.
pub fn image_ids(refs: &[String]) -> BTreeMap<String, String> {
    let unique: BTreeSet<&String> = refs.iter().filter(|r| !r.is_empty()).collect();
    unique
        .into_iter()
        .filter_map(|reference| Some((reference.clone(), image_id(reference)?)))
        .collect()
}

/// The local image id one reference points at, or `None` when this machine
/// does not have it.
fn image_id(reference: &str) -> Option<String> {
    let args = vec![
        "image".to_string(),
        "inspect".to_string(),
        "--format".to_string(),
        "{{.Id}}".to_string(),
        reference.to_string(),
    ];
    let id = docker_capture(&args)?.trim().to_string();
    (!id.is_empty()).then_some(id)
}

/// Run a plain `docker` command (no `compose`, no project) and return its
/// stdout, or `None` when it could not be run or answered non-zero.
///
/// Best-effort like [`compose_capture`]: every caller is sharpening an answer
/// it can give without docker's help.
fn docker_capture(args: &[String]) -> Option<String> {
    trace_command(args);
    let out = Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        crate::output::verbose(&format!(
            "  `docker {}` exited with status {}",
            args.first().map(String::as_str).unwrap_or_default(),
            exit_code(out.status)
        ));
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Why a short `docker compose` query went unanswered.
///
/// "exited with status 1" is no use on its own: a daemon that is not running,
/// a daemon in Windows-containers mode and a stack file compose will not read
/// all look the same from the exit code. Docker says which it is on stderr,
/// and its first line is what this carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryFailure {
    /// The query as it was typed after `docker compose`, such as `ps -a`.
    pub query: String,
    /// The status docker exited with, or `None` when it could not be run.
    pub code: Option<i32>,
    /// Docker's own first words: the first line of its stderr with anything
    /// on it, or the reason it could not be started.
    pub detail: String,
}

impl QueryFailure {
    /// Docker ran and refused to answer.
    fn refused(query: &str, code: i32, stderr: &str) -> QueryFailure {
        QueryFailure {
            query: query.to_string(),
            code: Some(code),
            detail: first_stderr_line(stderr).unwrap_or_default(),
        }
    }

    /// Docker could not be started at all.
    fn unusable(query: &str, err: &std::io::Error) -> QueryFailure {
        QueryFailure {
            query: query.to_string(),
            code: None,
            detail: err.to_string(),
        }
    }

    /// Whether docker ran at all, as opposed to not being there to run.
    ///
    /// A missing `docker` is not worth a word from a command that only asked
    /// it in passing; a docker that answered and refused is.
    pub fn ran(&self) -> bool {
        self.code.is_some()
    }
}

impl std::fmt::Display for QueryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let detail = if self.detail.is_empty() {
            String::new()
        } else {
            format!(": {}", self.detail)
        };
        match self.code {
            Some(code) => write!(
                f,
                "`docker compose {}` exited with status {code}{detail}",
                self.query
            ),
            None => write!(f, "`docker` could not be run{detail}"),
        }
    }
}

/// The first line of docker's stderr with anything on it.
///
/// Compose pads its diagnostics with blank lines and with progress it has
/// already erased; the first line with words on it names the problem.
fn first_stderr_line(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// Run a short `docker compose` query and return its stdout, or why it went
/// unanswered.
///
/// Nothing is printed on failure: every caller has a better thing to say than
/// docker's own noise, and the [`QueryFailure`] is how the callers that want
/// a word of it get one.
fn compose_query(project: &Project, extra: &[&str]) -> std::result::Result<String, QueryFailure> {
    let mut args = compose_args(project);
    args.extend(extra.iter().map(|a| a.to_string()));
    trace_command(&args);
    let query = extra.join(" ");
    let out = Command::new("docker")
        .args(&args)
        .current_dir(&project.dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| QueryFailure::unusable(&query, &e))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let failure = QueryFailure::refused(&query, exit_code(out.status), &stderr);
        crate::output::verbose(&format!("  {failure}"));
        return Err(failure);
    }
    let body = String::from_utf8_lossy(&out.stdout).into_owned();
    crate::output::debug(&format!(
        "  docker said:\n{}",
        crate::output::trace_body(body.trim_end())
    ));
    Ok(body)
}

/// Run a short `docker compose` query and return its stdout, or `None` when
/// docker could not be run or answered non-zero.
///
/// The best-effort half of [`compose_query`], for the callers that have
/// nothing to say about a failure beyond not having an answer.
fn compose_capture(project: &Project, extra: &[&str]) -> Option<String> {
    compose_query(project, extra).ok()
}

/// The last `tail` lines one service has logged
/// (`docker compose logs --tail N --no-color SERVICE`).
///
/// `--no-color` because this is read rather than shown: the escape codes
/// compose adds to colour the service column would end up inside the lines
/// that are quoted back. Best-effort: `None` when docker could not be asked.
pub fn service_logs(project: &Project, service: &str, tail: usize) -> Option<String> {
    let tail = tail.to_string();
    compose_capture(
        project,
        &[
            "logs",
            "--tail",
            &tail,
            "--no-color",
            "--no-log-prefix",
            service,
        ],
    )
    // `--no-log-prefix` arrived in compose 2.x but not in every 2.x; a
    // compose that rejects it still answers the same question without it.
    .or_else(|| compose_capture(project, &["logs", "--tail", &tail, "--no-color", service]))
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
/// The recorded name when `.chaps/project.yaml` has one - it is the `name:`
/// key `chaps sync` renders into the compose files, so compose reaches the
/// same answer without being asked. Otherwise read from
/// `docker compose config`, which applies the same rules compose itself does
/// (the directory name, normalised, unless a `name:` key or
/// `COMPOSE_PROJECT_NAME` says otherwise). Best-effort: `None` when docker
/// cannot be reached, so callers degrade rather than fail.
pub fn compose_project_name(project: &Project) -> Option<String> {
    if let Some(name) = project.compose_project() {
        return Some(name.to_string());
    }
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

/// One named volume of a deployment, and when docker created it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Volume {
    pub name: String,
    /// `CreatedAt` as `docker volume inspect` prints it (RFC 3339), empty when
    /// this docker did not say.
    pub created_at: String,
}

/// The named volumes whose names start with `prefix`
/// (`docker volume ls --filter name=<prefix>`), each with its creation time.
///
/// Best-effort like every other query here: an empty list when docker could
/// not be asked. The filter is a substring match, so the names are checked
/// again out here - `demo_` must not answer for `olddemo_chap-db`.
pub fn volumes_with_prefix(prefix: &str) -> Vec<Volume> {
    let args = vec![
        "volume".to_string(),
        "ls".to_string(),
        "--filter".to_string(),
        format!("name={prefix}"),
        "--format".to_string(),
        "{{.Name}}".to_string(),
    ];
    let Some(text) = docker_capture(&args) else {
        return Vec::new();
    };
    let names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with(prefix))
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return Vec::new();
    }
    let created = volume_creation_times(&names);
    names
        .into_iter()
        .map(|name| Volume {
            created_at: created.get(&name).cloned().unwrap_or_default(),
            name,
        })
        .collect()
}

/// When docker created each of these volumes, keyed by name
/// (`docker volume inspect`). One call for the whole list.
fn volume_creation_times(names: &[String]) -> BTreeMap<String, String> {
    let mut args = vec![
        "volume".to_string(),
        "inspect".to_string(),
        "--format".to_string(),
        "{{.Name}}\t{{.CreatedAt}}".to_string(),
    ];
    args.extend(names.iter().cloned());
    match docker_capture(&args) {
        Some(text) => parse_volume_times(&text),
        None => BTreeMap::new(),
    }
}

/// Parse the `<name>\t<created at>` lines of the inspect above.
pub fn parse_volume_times(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (name, created) = line.trim_end().split_once('\t')?;
            let (name, created) = (name.trim(), created.trim());
            (!name.is_empty() && !created.is_empty())
                .then(|| (name.to_string(), created.to_string()))
        })
        .collect()
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
        let dir = "/tmp/chapx";
        let p = project(dir);
        // Each `-f` is the project directory joined with the file name, in
        // the separator of whatever platform the CLI runs on.
        let file = |name: &str| PathBuf::from(dir).join(name).to_string_lossy().into_owned();
        assert_eq!(
            compose_args(&p),
            vec![
                "compose".to_string(),
                "-f".to_string(),
                file("compose.yml"),
                // The chaps-owned overrides sit between the base file and the
                // umbrella: later files win, and this one overrides chap.
                "-f".to_string(),
                file("compose.chaps.yml"),
                "-f".to_string(),
                file("compose.marketplace.yml"),
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
            ..Container::default()
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
    fn a_refused_query_keeps_dockers_first_words() {
        let failure = QueryFailure::refused(
            "ps -a --format json",
            1,
            "\n  error during connect: open //./pipe/dockerDesktopLinuxEngine: \
             the system cannot find the file specified.\nrun `docker context ls`\n",
        );
        assert!(failure.ran());
        assert_eq!(
            failure.to_string(),
            "`docker compose ps -a --format json` exited with status 1: error during connect: \
             open //./pipe/dockerDesktopLinuxEngine: the system cannot find the file specified."
        );
    }

    #[test]
    fn a_query_refused_without_a_word_is_still_the_status() {
        let failure = QueryFailure::refused("ps", 1, "  \n\n");
        assert_eq!(
            failure.to_string(),
            "`docker compose ps` exited with status 1"
        );
    }

    #[test]
    fn a_docker_that_is_not_there_never_answered() {
        let failure = QueryFailure::unusable(
            "ps",
            &std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
        );
        assert!(!failure.ran());
        assert!(
            failure
                .to_string()
                .starts_with("`docker` could not be run: "),
            "{failure}"
        );
    }

    #[test]
    fn other_spawn_failures_are_plain_errors() {
        let err = spawn_error(&std::io::Error::other("boom"));
        assert!(err.downcast_ref::<ChapError>().is_none());
        assert!(err.to_string().contains("could not run"));
    }

    #[test]
    fn ps_carries_the_image_reference_the_container_was_made_from() {
        let text = r#"{"Service":"chap","Name":"x-chap-1","ID":"ca78","CreatedAt":"now","State":"running","Image":"ghcr.io/dhis2-chap/chap:v2.3.1"}"#;
        let found = containers(text);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].image, "ghcr.io/dhis2-chap/chap:v2.3.1");
        // A `ps` from a compose that does not print it is not an error.
        assert_eq!(containers(r#"{"Service":"chap"}"#)[0].image, "");
    }

    #[test]
    fn the_service_images_come_from_the_merged_configuration() {
        let text = r#"{
          "name": "chapx",
          "services": {
            "chap":   {"image": "ghcr.io/dhis2-chap/chap:v2.3.1"},
            "worker": {"image": "ghcr.io/dhis2-chap/chap:v2.3.1"},
            "local":  {"build": {"context": "."}}
          }
        }"#;
        let images = parse_service_images(text);
        assert_eq!(images.len(), 2, "a built service pins no image");
        assert_eq!(
            images.get("chap").map(String::as_str),
            Some("ghcr.io/dhis2-chap/chap:v2.3.1")
        );
        assert_eq!(images.get("worker"), images.get("chap"));
        assert!(parse_service_images("not json").is_empty());
        assert!(parse_service_images("{}").is_empty());
    }

    #[test]
    fn the_config_hashes_are_one_service_per_line() {
        let text = "chap 8b08690bc131\nworker 2aaae2717d12\n";
        let hashes = parse_config_hashes(text);
        assert_eq!(hashes.get("chap").map(String::as_str), Some("8b08690bc131"));
        assert_eq!(
            hashes.get("worker").map(String::as_str),
            Some("2aaae2717d12")
        );
        // Anything that is not a pair is not a hash.
        assert!(parse_config_hashes("chap\n\n   \n").is_empty());
    }

    #[test]
    fn an_inspected_container_yields_its_image_and_its_config_hash() {
        let text = "abc123def456\tsha256:aaa\t{\"com.docker.compose.config-hash\":\"h1\",\
                    \"com.docker.compose.project\":\"chapx\"}\n\
                    fff000\tsha256:bbb\t{}\n";
        let found = parse_container_builds(text);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, "abc123def456");
        assert_eq!(found[0].1.image_id, "sha256:aaa");
        assert_eq!(found[0].1.config_hash, "h1");
        // No label, no hash: unknown, not empty-and-therefore-changed.
        assert_eq!(found[1].1.config_hash, "");
        // Labels that are not JSON at all cost the hash and nothing else.
        let odd = parse_container_builds("abc\tsha256:ccc\tnot json\n");
        assert_eq!(odd[0].1.image_id, "sha256:ccc");
        assert_eq!(odd[0].1.config_hash, "");
        assert!(parse_container_builds("").is_empty());
    }
}
