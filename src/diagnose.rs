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
             with the same name, or --fresh-env); run `chaps doctor`, or remove the volume with \
             `chaps docker run -- down -v` if this deployment's data can go"
                .to_string(),
        );
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
fn diagnose_one(project: &Project, service: &str, unhealthy: bool) -> Option<Unhealthy> {
    let logs = docker::service_logs(project, service, TAIL)?;
    Some(match unhealthy {
        true => Unhealthy::of(service, &logs),
        false => Unhealthy::of_down(service, &logs),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tail of a chap-core log after it was started against a database
    /// volume another deployment of the same name had created.
    const CHAP_CORE_LOG: &str = concat!(
        "chap-1  | INFO:     Started server process [1]\n",
        "chap-1  | INFO:     Waiting for application startup.\n",
        "chap-1  |   File \"/usr/local/lib/python3.11/site-packages/sqlalchemy/engine/base.py\", line 145, in __init__\n",
        "chap-1  |     self._dbapi_connection = engine.raw_connection()\n",
        "chap-1  | sqlalchemy.exc.OperationalError: (psycopg2.OperationalError) connection to server at \"postgres\" (192.168.32.2), port 5432 failed: FATAL:  password authentication failed for user \"chap\"\n",
        "chap-1  | (Background on this error at: https://sqlalche.me/e/20/e3q8)\n",
        "chap-1  | ERROR:    Application startup failed. Exiting.\n",
    );

    #[test]
    fn the_why_lines_are_the_last_error_looking_ones() {
        let why = why_lines(CHAP_CORE_LOG);
        assert_eq!(why.len(), 2, "{why:#?}");
        assert!(
            why[0].starts_with("sqlalchemy.exc.OperationalError:"),
            "{why:#?}"
        );
        assert!(
            why[0].contains("password authentication failed for user \"chap\""),
            "the whole line is kept, however long it is: {why:#?}"
        );
        assert_eq!(why[1], "ERROR:    Application startup failed. Exiting.");
        // The traceback frames and the startup chatter are not the failure.
        assert!(
            why.iter()
                .all(|line| !line.contains("Started server process")),
            "{why:#?}"
        );
        // The compose column is dropped: the heading names the service.
        assert!(
            why.iter().all(|line| !line.contains("chap-1  |")),
            "{why:#?}"
        );
        // It is cut for the screen, and only there.
        let printed = block(&[Unhealthy::of("chap", CHAP_CORE_LOG)], &Out::default());
        assert!(printed.contains("password authentica..."), "{printed}");
    }

    #[test]
    fn a_password_failure_maps_to_the_volume_it_came_from() {
        let entry = Unhealthy::of("chap", CHAP_CORE_LOG);
        assert_eq!(entry.service, "chap");
        let hint = entry.hint.expect("a password failure has a hint");
        assert!(
            hint.contains("the database volume holds a different password"),
            "{hint}"
        );
        assert!(hint.contains("chaps docker run -- down -v"), "{hint}");
        assert!(hint.contains("chaps doctor"), "{hint}");

        // The bare line chap-core prints, without the traceback around it.
        let bare = Unhealthy::of(
            "chap",
            "FATAL:  password authentication failed for user \"chap\"\n",
        );
        assert_eq!(bare.why.len(), 1);
        assert!(bare.hint.is_some());
    }

    #[test]
    fn a_refused_database_maps_to_the_database() {
        let logs = concat!(
            "chap-1  | INFO:     Waiting for application startup.\n",
            "chap-1  | psycopg2.OperationalError: connection to server at \"postgres\" \
             (192.168.32.2), port 5432 failed: Connection refused\n",
            "chap-1  | \tIs the server running on that host and accepting TCP/IP connections?\n",
        );
        let entry = Unhealthy::of("chap", logs);
        assert_eq!(
            entry.hint.as_deref(),
            Some("the database or valkey did not come up; run `chaps logs postgres`")
        );
    }

    #[test]
    fn only_composes_own_column_is_stripped_from_a_line() {
        // What `docker compose logs` writes in front of every line.
        assert_eq!(
            why_lines("chap-1  | ERROR: it broke\n"),
            vec!["ERROR: it broke".to_string()]
        );
        // A pipe inside the message is part of the message.
        let piped = "ERROR: running `psql -tAc 'select 1' | grep -q 1` failed\n";
        assert_eq!(why_lines(piped)[0], piped.trim());
        // And so is one in a table a log drew for itself.
        let table = "ERROR: expected | got | diff\n";
        assert_eq!(why_lines(table)[0], table.trim());
    }

    #[test]
    fn a_log_with_nothing_wrong_in_it_says_nothing() {
        let logs = "chap-1  | INFO:     Application startup complete.\n\
                    chap-1  | INFO:     Uvicorn running on http://0.0.0.0:8000\n";
        let entry = Unhealthy::of("chap", logs);
        assert!(entry.is_empty());
        assert!(why_lines("").is_empty());
        assert!(hint_for(&[]).is_none());
        assert!(block(&[entry], &Out::default()).is_empty());
    }

    #[test]
    fn at_most_five_lines_survive_and_repeats_collapse() {
        let repeated = "connection refused\n".repeat(40);
        assert_eq!(why_lines(&repeated).len(), 1, "a retry loop says one thing");

        // A loop that alternates two lines says two things, not forty.
        let alternating =
            "ERROR: could not connect to postgres\nretrying: connection refused\n".repeat(20);
        assert_eq!(why_lines(&alternating).len(), 2, "{alternating}");

        // The documentation link SQLAlchemy prints under every error holds
        // the word "error" and none of the cause.
        let noisy = "OperationalError: password authentication failed for user \"chap\"\n\
                     (Background on this error at: https://sqlalche.me/e/20/e3q8)\n";
        let why = why_lines(noisy);
        assert_eq!(why.len(), 1, "{why:#?}");
        assert!(why[0].starts_with("OperationalError:"), "{why:#?}");

        let many: String = (0..12)
            .map(|n| format!("error number {n}\n"))
            .collect::<String>();
        let why = why_lines(&many);
        assert_eq!(why.len(), MAX_LINES);
        assert_eq!(
            why[0], "error number 7",
            "the newest lines are the ones kept"
        );
        assert_eq!(why[MAX_LINES - 1], "error number 11");

        // A line nobody can read is cut rather than printed in full.
        let long = format!("error {}\n", "x".repeat(400));
        let printed = lines(&[Unhealthy::of("chap", &long)]);
        assert!(printed[1].ends_with("..."), "{printed:#?}");
        assert!(printed[1].chars().count() <= MAX_WIDTH + 2);
    }

    /// What compose writes to stderr when chap-core will not become healthy.
    const UP_STDERR: &str = concat!(
        " Container trap-a-1ab2c3-postgres-1  Healthy\n",
        " Container trap-a-1ab2c3-chap-1  Error\n",
        "dependency failed to start: container trap-a-1ab2c3-chap-1 is unhealthy\n",
    );

    fn container(service: &str, name: &str, health: &str) -> Container {
        Container {
            service: service.to_string(),
            name: name.to_string(),
            state: "running".to_string(),
            health: health.to_string(),
            ..Container::default()
        }
    }

    #[test]
    fn the_failed_up_names_the_service_behind_the_container() {
        assert!(is_health_failure(UP_STDERR));
        let containers = vec![
            container("postgres", "trap-a-1ab2c3-postgres-1", "healthy"),
            container("chap", "trap-a-1ab2c3-chap-1", "unhealthy"),
        ];
        assert_eq!(
            blamed_services(UP_STDERR, &containers, Some("trap-a-1ab2c3")),
            vec!["chap".to_string()]
        );
        // Without a `ps` to match against, the name is taken apart instead.
        assert_eq!(
            blamed_services(UP_STDERR, &[], Some("trap-a-1ab2c3")),
            vec!["chap".to_string()]
        );
        // And without even the project name.
        assert_eq!(
            blamed_services(UP_STDERR, &[], None),
            vec!["trap-a-1ab2c3-chap".to_string()]
        );
    }

    #[test]
    fn stderr_that_is_not_a_health_failure_is_left_alone() {
        for text in [
            "",
            "Error response from daemon: pull access denied",
            "port is already allocated",
        ] {
            assert!(!is_health_failure(text), "{text:?}");
            assert!(blamed_services(text, &[], None).is_empty(), "{text:?}");
        }
    }

    #[test]
    fn a_container_name_comes_apart_the_way_compose_put_it_together() {
        assert_eq!(
            service_of_container("demo-1ab2c3-chap-1", Some("demo-1ab2c3")),
            "chap"
        );
        // The v1 spelling some deployments still carry.
        assert_eq!(service_of_container("demo_chap_1", Some("demo")), "chap");
        // A service name with a separator of its own survives.
        assert_eq!(
            service_of_container("demo-chapkit-ewars-model-1", Some("demo")),
            "chapkit-ewars-model"
        );
        // A name that does not start with the project is not cut into.
        assert_eq!(service_of_container("chap-1", Some("other")), "chap");
    }

    #[test]
    fn the_block_names_the_service_and_indents_what_it_said() {
        let entry = Unhealthy::of("chap", CHAP_CORE_LOG);
        let text = block(std::slice::from_ref(&entry), &Out::default());
        assert!(text.starts_with("why chap is unhealthy:\n"), "{text}");
        assert!(
            text.contains("  sqlalchemy.exc.OperationalError:"),
            "{text}"
        );
        assert!(
            text.contains("password authentica..."),
            "cut for the screen: {text}"
        );
        assert!(
            text.contains("\n  the database volume holds a different password"),
            "{text}"
        );
        // The same thing as lines, for an error message.
        let lines = lines(std::slice::from_ref(&entry));
        assert_eq!(lines[0], "why chap is unhealthy:");
        assert!(lines.len() >= 3);
        // The headline is the line that names the cause, not just the newest
        // one: what a `doctor` check has room for is the reason.
        assert!(
            headline(std::slice::from_ref(&entry))
                .unwrap()
                .contains("password authentication failed"),
            "{entry:#?}"
        );
        assert!(
            headline(std::slice::from_ref(&entry))
                .unwrap()
                .chars()
                .count()
                <= HEADLINE_WIDTH,
            "a headline fits the one line a doctor check gives it"
        );
        // With nothing recognisable in them, the newest line is the headline.
        let plain = Unhealthy::of("chap", "ERROR: it broke\nERROR: it broke again\n");
        assert_eq!(
            headline(std::slice::from_ref(&plain)).as_deref(),
            Some("ERROR: it broke again")
        );
        assert_eq!(headline(&[]), None);

        // A container whose healthcheck has not failed - because docker keeps
        // resetting it while the container crash-loops - says the other thing.
        let down = Unhealthy::of_down("chap", CHAP_CORE_LOG);
        assert_eq!(down.heading(), "why chap is down:");
        assert!(!down.unhealthy);
        assert_eq!(down.why, entry.why, "the log is read the same way");
        assert!(block(&[down], &Out::default()).starts_with("why chap is down:\n"));
    }
}
