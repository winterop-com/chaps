//! The containers of a deployment, as `docker compose ps` reports them.

use super::{QueryFailure, compose_capture, compose_query};
use crate::project::Project;
use std::collections::BTreeSet;

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
    /// Docker's own summary, such as `Up 12 seconds` or `Exited (0) 3 hours ago`.
    pub status: String,
}

impl Container {
    /// Whether this container is up. A `ps` that reported no state at all is
    /// treated as running: plain `ps` lists what is up.
    pub fn is_running(&self) -> bool {
        self.state.is_empty() || self.state.eq_ignore_ascii_case("running")
    }

    /// When compose created it, in seconds since the Unix epoch.
    ///
    /// Docker writes `2026-09-30 13:37:27 +0200 CEST`; the zone name after the
    /// offset says nothing the offset does not.
    pub fn created_unix(&self) -> Option<u64> {
        let mut parts = self.created_at.split_whitespace();
        let (date, clock, offset) = (parts.next()?, parts.next()?, parts.next()?);
        crate::status::parse_rfc3339(&format!("{date}T{clock}{offset}"))
    }

    /// Whether it has been up for less than two minutes.
    ///
    /// Read from docker's `Status`, which humanises the uptime: `Less than a
    /// second`, `N seconds`, `About a minute` for the second minute, and
    /// minutes from there on. Two minutes is how long a model is given to
    /// register before `chaps status` calls it stuck: chapkit retries while it
    /// starts, and a model on an emulated amd64 image can take most of that.
    pub fn is_young(&self) -> bool {
        let Some(uptime) = self.status.strip_prefix("Up ") else {
            return false;
        };
        uptime.starts_with("Less than a second")
            || uptime.starts_with("About a minute")
            || uptime
                .split_whitespace()
                .nth(1)
                .is_some_and(|unit| unit.starts_with("second"))
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
                status: string("Status"),
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
