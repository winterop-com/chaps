//! The model rows: what each enabled model is to chap-core, or to its own
//! `/health` on a deployment without chap-core.

use super::time::ago;
use crate::project::Project;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Where one model row stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelState {
    /// chap-core knows it: the model works.
    Registered,
    /// Its container is up but chap-core never heard from it. chapkit gives
    /// up registering five attempts into its startup, so a model that came up
    /// before chap-core was healthy stays invisible until it is restarted.
    RunningNotRegistered,
    /// No container, so nothing could have registered.
    NotRunning,
    /// Registered with chap-core, but not a model this project enables.
    Unmanaged,
    /// Without chap-core: its container is up and its own `/health` answered
    /// on its host port.
    Up,
    /// Without chap-core: its container is up and its `/health` did not
    /// answer, which is a service still starting or one that failed to.
    RunningNotAnswering,
    /// chap-core knows it, and cannot reach it: its proxy to the model's own
    /// `/health` failed. The usual cause is an address that works from where
    /// the model registered and not from where chap-core runs, such as
    /// `localhost` seen from inside chap-core's container.
    Unreachable,
}

impl ModelState {
    /// The STATE cell.
    pub fn label(self) -> &'static str {
        match self {
            ModelState::Registered => "registered",
            ModelState::RunningNotRegistered => "running, not registered",
            ModelState::NotRunning => "not running",
            ModelState::Unmanaged => "unmanaged",
            ModelState::Up => "up",
            ModelState::RunningNotAnswering => "running, not answering",
            ModelState::Unreachable => "registered, unreachable",
        }
    }

    /// Whether this row is something to do about.
    pub fn is_problem(self) -> bool {
        matches!(
            self,
            ModelState::RunningNotRegistered
                | ModelState::NotRunning
                | ModelState::RunningNotAnswering
                | ModelState::Unreachable
        )
    }
}

/// One row of the model table.
#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    /// Compose service name, which is also the id the model registers with.
    pub id: String,
    pub state: ModelState,
    /// Where a human reaches it: a host port, `internal`, or - for an
    /// unmanaged service - the URL chap-core has for it.
    pub reach: String,
    /// The host port itself, for the table, which says `port 5010` where
    /// `--json` says the whole URL. Not serialised: [`ModelStatus::reach`] is
    /// what a script reads, and it has not changed.
    #[serde(skip)]
    pub host_port: Option<u16>,
    /// How long ago chap-core last heard from it, `None` when never.
    pub last_ping: Option<String>,
    /// For a model that is running and not registered: the id its own
    /// container did register under, when an unmanaged row turned out to be
    /// it. That is a service id that does not match, not a model that failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registered_as: Option<String>,
    /// Whether its container has been up for under two minutes, which makes
    /// "not registered yet" a wait rather than a fault. Filled in by the
    /// caller, which has docker.
    #[serde(skip)]
    pub young: bool,
    /// For a model added with `varde models add`: its model id and the source
    /// it was added from, so a hint can spell out removing and adding it again.
    #[serde(skip)]
    pub added_from: Option<(String, String)>,
    /// For an [`ModelState::Unreachable`] row: the URL it registered under
    /// and what chap-core's proxy answered, for the hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreachable: Option<Unreachable>,
}

/// Why chap-core could not reach a model it has registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unreachable {
    /// The URL the model registered under, which is where chap-core calls it.
    pub registered_url: String,
    /// What the proxy request came back with: `HTTP 502`, or the error.
    pub answer: String,
}

/// The path, under chap-core, that proxies to a registered model's `/health`.
pub fn proxied_health_path(service_id: &str) -> String {
    format!("/v2/services/{service_id}/run{MODEL_HEALTH_PATH}")
}

/// Turn every registered row chap-core cannot reach into
/// [`ModelState::Unreachable`].
///
/// A registration is a heartbeat the model sends; it says nothing about the
/// way back. `probe` asks chap-core's proxy for the model's own `/health` and
/// answers `Some(what came back)` when that failed in a way that means
/// chap-core could not get to the model (a 5xx, or no answer), and `None`
/// otherwise - a 404 included, since a chap-core without the proxy route
/// cannot say either way.
pub fn mark_unreachable(
    rows: &mut [ModelStatus],
    registered: &[RegisteredService],
    probe: &dyn Fn(&str) -> Option<String>,
) {
    for row in rows
        .iter_mut()
        .filter(|row| row.state == ModelState::Registered)
    {
        if let Some(answer) = probe(&row.id) {
            row.state = ModelState::Unreachable;
            row.unreachable = Some(Unreachable {
                registered_url: registered
                    .iter()
                    .find(|s| s.id == row.id)
                    .map(|s| s.url.clone())
                    .unwrap_or_default(),
                answer,
            });
        }
    }
}

/// Tie each unmanaged row to the model whose container it is, when it is one.
///
/// chap-core records the URL a service registered from, and a container's
/// hostname is the start of its id - so `http://ee5e63ab53bd:8000` is the
/// container `ee5e63ab53bd...`. When that container is one of this
/// deployment's models and that model is `running, not registered`, the
/// model did register, under an id of its own: the fix is `--service-id`, and
/// a restart would change nothing. `containers` is `(service, id)`.
pub fn link_strays(rows: &mut [ModelStatus], containers: &[(String, String)]) {
    let strays: Vec<(String, String)> = rows
        .iter()
        .filter(|row| row.state == ModelState::Unmanaged)
        .filter_map(|row| Some((row.id.clone(), url_host(&row.reach)?)))
        .collect();
    for (stray, host) in strays {
        // Twelve hex digits is how docker names a container's host; anything
        // shorter would match too much.
        if host.len() < 12 || !host.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let Some((service, _)) = containers.iter().find(|(_, id)| id.starts_with(&host)) else {
            continue;
        };
        if let Some(row) = rows
            .iter_mut()
            .find(|row| &row.id == service && row.state == ModelState::RunningNotRegistered)
        {
            row.registered_as = Some(stray);
        }
    }
}

/// `ee5e63ab53bd` out of `http://ee5e63ab53bd:8000/`.
fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.split(['/', ':']).next()?;
    (!host.is_empty()).then(|| host.to_string())
}

/// One entry of `GET /v2/services`.
#[derive(Debug, Serialize, Deserialize)]
pub struct RegisteredService {
    pub id: String,
    pub url: String,
    pub display_name: String,
    pub version: String,
    pub last_ping_at: String,
    pub expires_at: String,
}
/// Every model this project enables, as `(service id, host port)`, in the
/// order `models.yaml` records them.
pub fn enabled_models(project: &Project) -> Vec<(String, Option<u16>)> {
    project
        .state
        .models
        .values()
        .map(|m| (m.service_id.clone(), m.host_port))
        .collect()
}

/// Build the table: one row per enabled model, then every registered service
/// the project does not know as `unmanaged`.
///
/// `now` is Unix seconds, passed in so the relative times are testable.
pub fn model_rows(
    enabled: &[(String, Option<u16>)],
    registered: &[RegisteredService],
    running: &BTreeSet<String>,
    now: u64,
) -> Vec<ModelStatus> {
    let mut rows: Vec<ModelStatus> = enabled
        .iter()
        .map(|(id, host_port)| {
            let found = registered.iter().find(|s| &s.id == id);
            let state = match (found.is_some(), running.contains(id)) {
                (true, _) => ModelState::Registered,
                (false, true) => ModelState::RunningNotRegistered,
                (false, false) => ModelState::NotRunning,
            };
            ModelStatus {
                id: id.clone(),
                state,
                reach: match host_port {
                    Some(port) => format!("http://localhost:{port}"),
                    None => "internal".to_string(),
                },
                host_port: *host_port,
                last_ping: found.and_then(|s| ago(now, &s.last_ping_at)),
                registered_as: None,
                young: false,
                added_from: None,
                unreachable: None,
            }
        })
        .collect();

    let mut strangers: Vec<&RegisteredService> = registered
        .iter()
        .filter(|s| !enabled.iter().any(|(id, _)| id == &s.id))
        .collect();
    strangers.sort_by(|a, b| a.id.cmp(&b.id));
    rows.extend(strangers.into_iter().map(|s| ModelStatus {
        id: s.id.clone(),
        state: ModelState::Unmanaged,
        // Whatever chap-core has for it: an unmanaged service is not ours to
        // describe, and its URL is the only handle anyone has on it.
        reach: s.url.clone(),
        // Not this deployment's port to know.
        host_port: None,
        last_ping: ago(now, &s.last_ping_at),
        registered_as: None,
        young: false,
        added_from: None,
        unreachable: None,
    }));
    rows
}

/// The path a chapkit model service answers its own health on.
pub const MODEL_HEALTH_PATH: &str = "/health";

/// The model table for a deployment without chap-core: one row per enabled
/// model, judged by its container and then by its own `/health`.
///
/// `answers` is asked only about a model whose container is up and that has a
/// host port, so a stopped deployment costs no timeouts. A model with no host
/// port (one enabled with `--port none`) cannot be asked from out here, so its
/// container is the whole answer.
pub fn standalone_model_rows(
    enabled: &[(String, Option<u16>)],
    running: &BTreeSet<String>,
    answers: &dyn Fn(u16) -> bool,
) -> Vec<ModelStatus> {
    enabled
        .iter()
        .map(|(id, host_port)| {
            let state = match (running.contains(id), host_port) {
                (false, _) => ModelState::NotRunning,
                (true, Some(port)) if !answers(*port) => ModelState::RunningNotAnswering,
                (true, _) => ModelState::Up,
            };
            ModelStatus {
                id: id.clone(),
                state,
                reach: match host_port {
                    Some(port) => format!("http://localhost:{port}"),
                    None => "internal".to_string(),
                },
                host_port: *host_port,
                last_ping: None,
                registered_as: None,
                young: false,
                added_from: None,
                unreachable: None,
            }
        })
        .collect()
}
/// Where a human reaches one model service from this machine.
///
/// A published host port is the direct answer; without one the way in is
/// chap-core's read-only proxy, which reaches a registered service over the
/// compose network without any port of its own.
pub fn reach(project: &Project, host_port: Option<u16>, service_id: &str) -> String {
    match host_port {
        Some(port) => format!("http://localhost:{port}"),
        None => format!("internal (proxy: {})", project.proxy_url(service_id)),
    }
}

/// Expected service ids that nothing has registered.
pub fn missing_ids(expected: &[String], registered: &[RegisteredService]) -> Vec<String> {
    expected
        .iter()
        .filter(|id| !registered.iter().any(|s| &&s.id == id))
        .cloned()
        .collect()
}
