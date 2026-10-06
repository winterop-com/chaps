//! Why a container is unhealthy, in the words of its own log.
//!
//! Compose says `dependency failed to start: container demo-chap-1 is
//! unhealthy` and stops there. The container it names has been printing the
//! reason all along - a refused connection, a rejected password - so the
//! diagnosis is to read the last of its log and put the handful of lines that
//! look like a failure under a heading. Everything that decides what to say is
//! a pure function of text docker printed, so the whole of it is testable
//! without docker.

use crate::docker::{self, Container};
use crate::output::Out;
use crate::project::Project;
use serde::Serialize;

/// How many lines of a service's log are fetched.
pub const TAIL: usize = 40;

/// How many of them are ever printed. Five is enough for a stack trace's last
/// frame plus the exception under it, and few enough that the heading above
/// them is still the thing the eye lands on.
pub const MAX_LINES: usize = 5;

/// How long one printed line may be before it is cut.
const MAX_WIDTH: usize = 160;

/// The same for [`headline`], which goes into a `doctor` check with a
/// sentence in front of it. Narrower than [`MAX_WIDTH`], so the check does not
/// cut a second time from the other end and leave nothing of the cause.
const HEADLINE_WIDTH: usize = 110;

/// Lines that carry one of [`ERROR_WORDS`] and say nothing anyway.
///
/// SQLAlchemy appends a documentation link under every `OperationalError`,
/// which holds the word "error" and none of the cause. Keeping it would spend
/// half of a five-line block on the same URL repeated.
const NOISE: &[&str] = &["sqlalche.me/e/"];

/// The words that make a log line worth quoting back, matched without regard
/// to case.
///
/// Deliberately broad: a line wrongly kept costs a line of screen, while a
/// line wrongly dropped costs the whole diagnosis. The ordering is only for
/// reading - every one of them counts the same.
const ERROR_WORDS: &[&str] = &[
    "error",
    "failed",
    "refused",
    "denied",
    "operationalerror",
    "password authentication",
    "unhealthy",
    "invalid command",
];

/// One service that is failing, and what its own log says about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unhealthy {
    /// The compose service name, which is what `chaps logs SERVICE` takes.
    pub service: String,
    /// Whether its healthcheck is what said so.
    ///
    /// A container that crash-loops is only `unhealthy` between restarts -
    /// docker resets the health state each time it starts the container - so
    /// "the API does not answer and this is its container" is the other way
    /// to arrive here, and the heading says which.
    pub unhealthy: bool,
    /// The lines of its log that look like the failure, oldest first.
    pub why: Vec<String>,
    /// What to do about it, when the lines name a cause this CLI knows.
    pub hint: Option<String>,
}

impl Unhealthy {
    /// The diagnosis for one service, from the tail of its log.
    ///
    /// The lines are kept as the container printed them, whole: the sentence
    /// that names the cause is often the tail of a very long one, and cutting
    /// is for the screen rather than for the reasoning. [`block`] and
    /// [`lines`] cut as they print.
    pub fn of(service: &str, logs: &str) -> Unhealthy {
        let why = why_lines(logs);
        Unhealthy {
            service: service.to_string(),
            unhealthy: true,
            hint: hint_for(&why),
            why,
        }
    }

    /// The same, for a container whose healthcheck has not (yet) failed: the
    /// API is not answering and this is the container behind it.
    pub fn of_down(service: &str, logs: &str) -> Unhealthy {
        Unhealthy {
            unhealthy: false,
            ..Unhealthy::of(service, logs)
        }
    }

    /// The line the lines below it are printed under.
    pub fn heading(&self) -> String {
        match self.unhealthy {
            true => format!("why {} is unhealthy:", self.service),
            false => format!("why {} is down:", self.service),
        }
    }

    /// Whether anything at all was found to say.
    pub fn is_empty(&self) -> bool {
        self.why.is_empty() && self.hint.is_none()
    }
}

/// The services a failed `docker compose up` blamed, as compose named them on
/// stderr.
///
/// Two spellings matter: `container <name> is unhealthy`, which names the
/// container, and `dependency failed to start`, which precedes it on the same
/// line. `containers` is what `docker compose ps` last said, so a container
/// name can be turned into the service name the rest of the CLI speaks;
/// without it the name is taken apart by the pattern compose builds it from.
pub fn blamed_services(
    stderr: &str,
    containers: &[Container],
    project_name: Option<&str>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in unhealthy_container_names(stderr) {
        let service = containers
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.service.clone())
            .unwrap_or_else(|| service_of_container(&name, project_name));
        if !service.is_empty() && !out.contains(&service) {
            out.push(service);
        }
    }
    out
}

/// Whether docker's stderr is one of the failures this module can explain.
///
/// `dependency failed to start` on its own counts: compose prints it for a
/// container that exited as well as for one that is unhealthy, and the
/// containers themselves say which.
pub fn is_health_failure(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    lower.contains("is unhealthy") || lower.contains("dependency failed to start")
}

/// The container names in `container <name> is unhealthy` lines.
fn unhealthy_container_names(stderr: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        let mut rest = line;
        while let Some(at) = rest.find("container ") {
            rest = &rest[at + "container ".len()..];
            let Some(end) = rest.find(" is unhealthy") else {
                continue;
            };
            let name = rest[..end].trim().trim_matches('"');
            if !name.is_empty() && !out.contains(&name.to_string()) {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// The compose service a container name belongs to, taken apart the way
/// compose puts it together: `<project><sep><service><sep><index>`.
///
/// The separator is `-` on compose v2 and `_` on the v1 names some deployments
/// still carry, and a service name may hold either, so the index and the
/// project prefix are stripped rather than the name being split on a
/// separator.
pub fn service_of_container(name: &str, project_name: Option<&str>) -> String {
    let mut rest = name.trim();
    if let Some(project) = project_name {
        for sep in ['-', '_'] {
            if let Some(stripped) = rest.strip_prefix(&format!("{project}{sep}")) {
                rest = stripped;
                break;
            }
        }
    }
    // The trailing replica index compose appends to every container name.
    if let Some(at) = rest.rfind(['-', '_'])
        && rest[at + 1..].chars().all(|c| c.is_ascii_digit())
        && at + 1 < rest.len()
    {
        rest = &rest[..at];
    }
    rest.to_string()
}

/// The lines of a log that look like the failure: the last [`MAX_LINES`] of
/// them, oldest first and as the container printed them.
///
/// Compose prefixes each line with the service it came from
/// (`chap-1  | text`); that prefix is dropped, because the heading above these
/// lines already names the service. Repeated lines collapse: a container that
/// retried the same connection forty times has one thing to say, not forty.
pub fn why_lines(logs: &str) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for line in logs.lines() {
        let line = strip_compose_prefix(line).trim().to_string();
        if line.is_empty() || !looks_like_an_error(&line) {
            continue;
        }
        // A line that has been printed before moves to the end rather than
        // being printed twice: a retry loop alternates two or three lines for
        // as long as it runs, and all a reader needs is one of each, newest.
        if let Some(at) = kept.iter().position(|seen| seen == &line) {
            kept.remove(at);
        }
        kept.push(line);
    }
    if kept.len() > MAX_LINES {
        kept.drain(..kept.len() - MAX_LINES);
    }
    kept
}

/// Whether a log line says something failed, and says it in words.
fn looks_like_an_error(line: &str) -> bool {
    let lower = line.to_lowercase();
    ERROR_WORDS.iter().any(|word| lower.contains(word))
        && !NOISE.iter().any(|noise| lower.contains(noise))
}

/// Drop the `<container>  | ` prefix `docker compose logs` writes in front of
/// every line, leaving a log that was not prefixed alone.
///
/// Compose pads the container name to a column and follows it with `| `, so
/// what precedes the pipe is a short run with no spaces in it but the padding.
/// A pipe anywhere else - a table drawn in a log, a shell command echoed back -
/// belongs to the message.
fn strip_compose_prefix(line: &str) -> &str {
    match line.split_once('|') {
        Some((head, tail))
            if head.len() <= MAX_PREFIX && (!head.contains(' ') || head.ends_with("  ")) =>
        {
            tail
        }
        _ => line,
    }
}

/// The longest container name compose could be padding out in front of a log
/// line. Docker's own limit on a container name is well below it.
const MAX_PREFIX: usize = 64;

/// One line, short enough to print.
fn cut(line: &str) -> String {
    if line.chars().count() <= MAX_WIDTH {
        return line.to_string();
    }
    format!(
        "{}...",
        line.chars().take(MAX_WIDTH - 3).collect::<String>()
    )
}

/// What to do about the failure these lines describe, when it is one this CLI
/// recognises.
///
/// Two causes account for almost every unhealthy chap-core: a database volume
/// left over from another deployment of the same name, whose role password is
/// not the one in `.env`, and a database that never came up at all.
pub fn hint_for(lines: &[String]) -> Option<String> {
    let text = lines.join("\n").to_lowercase();
    if text.contains("password authentication failed for user") {
        return Some(
            "the database volume holds a different password than .env (a previous deployment \
             with the same name, or --fresh-env); the `volumes` line of `chaps doctor` says \
             which, and `chaps down --volumes` removes the volume if this deployment's data can go"
                .to_string(),
        );
    }
    // The postgres entrypoint names the init script that failed. After it,
    // docker starts the database again and postgres skips the init, so the
    // database stays incomplete until its volume goes.
    // DHIS2 migrates an older database on its first start, and a jump of
    // several versions can fail half way. Flyway then rolls back, and DHIS2
    // answers 404 on every path.
    if text.contains("migration of schema") && text.contains("failed") {
        return Some(
            "DHIS2 could not migrate the database to its version; a database from an older \
             DHIS2 must go up one version at a time, so set `image_tag:` under `dhis2:` in \
             `.chaps/components.yaml` to the next version, run `chaps sync` and `chaps up`, \
             and repeat"
                .to_string(),
        );
    }
    if text.contains("/docker-entrypoint-initdb.d/") {
        return Some(SEED_RESTORE_HINT.to_string());
    }
    if text.contains("connection refused") || text.contains("could not connect") {
        let names = ["postgres", "5432", "redis", "valkey", "6379", "database"];
        if names.iter().any(|name| text.contains(name)) {
            return Some(
                "the database or valkey did not come up; run `chaps logs postgres`".to_string(),
            );
        }
    }
    None
}

/// The `why <service> is unhealthy:` block, as it is printed.
///
/// Empty when there is nothing to say, so a caller can append it to whatever
/// it was going to print without checking first.
pub fn block(unhealthy: &[Unhealthy], out: &Out) -> String {
    let mut text = String::new();
    for entry in unhealthy {
        if entry.is_empty() {
            continue;
        }
        text.push_str(&format!("{}\n", out.warn(&entry.heading())));
        for line in &entry.why {
            text.push_str(&format!("  {}\n", out.dim(&cut(line))));
        }
        if let Some(hint) = &entry.hint {
            text.push_str(&format!("  {}\n", out.backticks(hint)));
        }
    }
    text
}

/// The same block as plain lines, for an error message rather than the screen.
pub fn lines(unhealthy: &[Unhealthy]) -> Vec<String> {
    let mut out = Vec::new();
    for entry in unhealthy {
        if entry.is_empty() {
            continue;
        }
        out.push(entry.heading());
        out.extend(entry.why.iter().map(|line| format!("  {}", cut(line))));
        if let Some(hint) = &entry.hint {
            out.push(format!("  {hint}"));
        }
    }
    out
}

/// The one-line version, for a `doctor` check: the line that names the cause,
/// or the newest one when none of them does.
pub fn headline(unhealthy: &[Unhealthy]) -> Option<String> {
    let why = &unhealthy.iter().find(|entry| !entry.why.is_empty())?.why;
    why.iter()
        .rev()
        .find(|line| names_a_cause(line))
        .or_else(|| why.last())
        .map(|line| headline_of(line))
}

/// One line, short enough for a `doctor` check.
///
/// A line that names a cause is cut from the front rather than the back: the
/// words that matter are at the end of it, after the connection string and
/// the address and the port.
fn headline_of(line: &str) -> String {
    let length = line.chars().count();
    if length <= HEADLINE_WIDTH {
        return line.to_string();
    }
    if !names_a_cause(line) {
        return cut(line);
    }
    let tail: String = line.chars().skip(length - (HEADLINE_WIDTH - 3)).collect();
    format!("...{tail}")
}

/// Whether one line holds one of the causes [`hint_for`] knows.
fn names_a_cause(line: &str) -> bool {
    hint_for(std::slice::from_ref(&line.to_string())).is_some()
}

// ---------------------------------------------------------------------------
// Asking docker
// ---------------------------------------------------------------------------

/// Diagnose every container of this project whose healthcheck is failing, plus
/// `also` - the service behind an API that is not answering, whatever docker
/// says about its health.
///
/// That second half is what catches a container in a crash loop: it exits, it
/// is restarted, and its health reads `starting` again every time, so the
/// moment `chaps status` looks is as likely as not to be one where nothing is
/// marked unhealthy at all. The log says the same thing either way.
///
/// Best-effort throughout: a service whose log cannot be read yields nothing,
/// which is what the callers already handle.
pub fn failing_containers(
    project: &Project,
    containers: &[Container],
    also: Option<&str>,
) -> Vec<Unhealthy> {
    let mut out: Vec<Unhealthy> = Vec::new();
    for container in containers.iter().filter(|c| c.is_unhealthy()) {
        push(&mut out, diagnose_one(project, &container.service, true));
    }
    if let Some(service) = also
        && !out.iter().any(|entry| entry.service == service)
        && containers.iter().any(|c| c.service == service)
    {
        push(&mut out, diagnose_one(project, service, false));
    }
    out
}

/// Keep a verdict that has something to say.
fn push(out: &mut Vec<Unhealthy>, entry: Option<Unhealthy>) {
    if let Some(entry) = entry.filter(|entry| !entry.is_empty()) {
        out.push(entry);
    }
}

/// Diagnose the services a failed `up` or `restart` blamed on stderr.
pub fn from_stderr(project: &Project, stderr: &str) -> Vec<Unhealthy> {
    if !is_health_failure(stderr) {
        return Vec::new();
    }
    let containers = docker::all_containers(project).unwrap_or_default();
    let project_name = project.compose_project_name();
    let mut services = blamed_services(stderr, &containers, project_name.as_deref());
    // `dependency failed to start` without a container name of its own: the
    // containers themselves say which one is failing.
    if services.is_empty() {
        services = containers
            .iter()
            .filter(|c| c.is_running() && c.is_unhealthy())
            .map(|c| c.service.clone())
            .collect();
    }
    diagnose(project, &services)
}

/// Read each service's log and turn it into a verdict.
fn diagnose(project: &Project, services: &[String]) -> Vec<Unhealthy> {
    let mut out: Vec<Unhealthy> = Vec::new();
    for service in services {
        if out.iter().any(|entry| &entry.service == service) {
            continue;
        }
        push(&mut out, diagnose_one(project, service, true));
    }
    out
}

/// Read one service's log and turn it into a verdict, or `None` when docker
/// would not hand the log over.
///
/// A seeded `dhis2-db` whose log says nothing is asked for the mark of a
/// complete restore: without it, the health check fails and the log is quiet.
fn diagnose_one(project: &Project, service: &str, unhealthy: bool) -> Option<Unhealthy> {
    let logs = docker::service_logs(project, service, TAIL)?;
    let mut entry = match unhealthy {
        true => Unhealthy::of(service, &logs),
        false => Unhealthy::of_down(service, &logs),
    };
    let seeded = project.state.components.dhis2_seed_source().is_some();
    if service == "dhis2-db" && seeded && entry.hint.is_none() {
        let mark = docker::dhis2_seed_mark(project);
        if let Some((why, hint)) = seed_mark_verdict(mark.as_deref()) {
            entry.why.push(why);
            entry.hint = Some(hint);
        }
    }
    Some(entry)
}

/// What to say about the mark of a seed restore, from the comment the
/// database holds: nothing when the mark is there or the database did not
/// answer, and a line and a hint when the mark is not there.
pub fn seed_mark_verdict(comment: Option<&str>) -> Option<(String, String)> {
    let comment = comment?;
    if comment == crate::compose::render::DHIS2_SEED_MARK {
        return None;
    }
    Some((
        format!(
            "the database has no mark `{}`, so its seed restore did not finish",
            crate::compose::render::DHIS2_SEED_MARK
        ),
        SEED_RESTORE_HINT.to_string(),
    ))
}

/// The way out of a seed restore that did not finish. Only the DHIS2
/// volumes: `chaps down --volumes` would also take the chap-core database.
const SEED_RESTORE_HINT: &str = "the restore of the DHIS2 seed stopped before the end, so \
     `dhis2_db` is incomplete; fix the cause above, then run `chaps components disable dhis2 \
     --purge`, `chaps components enable dhis2` and `chaps up`";

#[cfg(test)]
mod tests;
