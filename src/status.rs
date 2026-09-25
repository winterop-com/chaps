//! `chaps status`: chap-core health plus the services it has registered.
//!
//! Owned by agent C.
//!
//! Lenient about the fields chap-core sends - it may add fields, rename
//! optional ones or leave one empty, and none of that should turn
//! `chaps status` into a crash - and strict about who is answering. A 200
//! from something that is not chap-core is reported as down: `up` has to mean
//! that the deployment works, not that the port is taken.

use crate::output;
use crate::project::{ApiPortSource, Project};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

/// Path of the chap-core health endpoint.
pub const HEALTH_PATH: &str = "/health";
/// Path of the service registry chapkit models register themselves with.
pub const SERVICES_PATH: &str = "/v2/services";
/// Paths that may carry chap-core's own version, in the order they are tried.
/// Neither is required: a chap-core that answers neither is still up, and the
/// version falls back to the tag the project pins.
pub const INFO_PATHS: &[&str] = &["/system/info", "/v2/info"];

/// Path of the OCS dataset list. `f=json` because the same path serves the
/// landing page as HTML, and the landing page is not something to count.
pub const DATASETS_PATH: &str = "/datasets?f=json";

/// How long the OCS dataset count may take.
///
/// Its own bound rather than `--timeout`: this is one extra fact on a line
/// that is already complete without it, so it gets a short leash and gives up
/// silently.
pub const OCS_TIMEOUT: Duration = Duration::from_secs(3);

/// What `chaps status` reports.
#[derive(Debug, Serialize)]
pub struct StatusReport {
    /// The compose project name: the prefix every container and every named
    /// volume of this deployment carries. `None` only for a deployment whose
    /// directory name compose can make no project name out of.
    pub project: Option<String>,
    pub api_url: String,
    /// The host port this deployment publishes chap-core's API on. `--url`
    /// does not change it: it says which port the deployment uses, not which
    /// one this run happened to ask.
    pub api_port: u16,
    /// Which file [`StatusReport::api_port`] came from. `.env` wins, as it
    /// does for compose, so a deployment whose `.env` moved the port reports
    /// `env` and the port `.chaps/project.yaml` records is not the one in use.
    pub api_port_source: ApiPortSource,
    pub api: ApiHealth,
    /// Which chap-core this is, and whether the API said so itself.
    pub version: ApiVersion,
    /// The chap-core tag this deployment pins, and whether it is one that can
    /// point at a different image tomorrow.
    pub chap_tag: String,
    pub chap_tag_moving: bool,
    /// The build chap-core's container is running, as a short digest.
    ///
    /// Filled in by the caller, which is the half that has docker, and only
    /// asked for when the tag is a moving one: a release tag names its image
    /// already. `None` is "not asked" or "docker could not say".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chap_build: Option<String>,
    /// Services chap-core currently knows about, from `/v2/services`.
    pub registered: Vec<RegisteredService>,
    /// Service ids the project expects to be registered.
    pub expected: Vec<String>,
    /// Expected ids that are not registered.
    pub missing: Vec<String>,
    /// How a human reaches each service this project enabled, by service id:
    /// its own host port, or chap-core's proxy for a service that publishes
    /// none. The URL chap-core reports in `registered` is the internal one and
    /// only resolves inside the compose network.
    pub reach: BTreeMap<String, String>,
    /// One row per model: everything this project enables, plus every
    /// registered service it does not know about.
    pub models: Vec<ModelStatus>,
    /// Registered service ids the project does not enable.
    pub unmanaged: Vec<String>,
    /// Whether `.env` sets an API token, which is also whether these requests
    /// carried one.
    pub auth: bool,
    /// One row per enabled component other than chap-core, which has the
    /// chap-core line of its own. Empty on a deployment that has none.
    pub components: Vec<ComponentStatus>,
    /// Containers of this deployment that are failing, with the lines of their
    /// logs that say why.
    ///
    /// Filled in by the caller, which is the half that has docker: a container
    /// that is unhealthy is why the API is not answering, and the reason is in
    /// its log rather than anywhere this probe can reach.
    pub unhealthy: Vec<crate::diagnose::Unhealthy>,
}

impl StatusReport {
    /// Whether the API answered its health check as chap-core.
    #[cfg(test)]
    pub fn is_up(&self) -> bool {
        matches!(self.api, ApiHealth::Up { .. })
    }

    /// Whether everything the project enabled has registered.
    #[cfg(test)]
    pub fn is_complete(&self) -> bool {
        self.is_up() && self.missing.is_empty()
    }

    /// Whether chap-core's own container is up and failing its healthcheck.
    ///
    /// This is the difference between "nothing is listening on that port" and
    /// "chap-core is there and cannot start", which is the question an
    /// operator staring at a down API is actually asking.
    pub fn api_container_unhealthy(&self) -> bool {
        self.unhealthy
            .iter()
            .any(|entry| entry.service == crate::compose::API_SERVICE && entry.unhealthy)
    }
}

/// Result of `GET /health`.
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ApiHealth {
    Up {
        status: String,
        message: String,
    },
    Down {
        error: String,
    },
    /// chap-core is not a component of this deployment, so there is no API to
    /// ask about. Not a failure: `chaps components disable chap-core` is how a
    /// deployment becomes, say, OCS on its own.
    Off,
}

/// Where one component other than chap-core stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentState {
    /// Answering its health endpoint, or - for a service this CLI does not
    /// probe over HTTP - simply running.
    Up,
    /// Its container is up but it is not answering yet.
    Starting,
    /// No container, so nothing to answer.
    NotRunning,
}

impl ComponentState {
    /// The STATE cell.
    pub fn label(self) -> &'static str {
        match self {
            ComponentState::Up => "up",
            ComponentState::Starting => "starting",
            ComponentState::NotRunning => "not running",
        }
    }

    /// Whether this row is something to do about. A component that was never
    /// started is not: `chaps up` is the answer, and the report says so once.
    pub fn is_problem(self) -> bool {
        self == ComponentState::Starting
    }
}

/// One row of the component table.
#[derive(Debug, Clone, Serialize)]
pub struct ComponentStatus {
    /// Component name, which is also the compose service name.
    pub name: String,
    pub state: ComponentState,
    /// Where a human reaches it from this machine, or `internal` for a
    /// component that publishes no host port - with the proxy named when the
    /// component records one, since that is the address that does work.
    pub reach: String,
    /// The health URL that was probed, when one was.
    pub health_url: Option<String>,
    /// Whether this OCS instance refuses ingestion over HTTP. Always false for
    /// every other component, none of which has the setting.
    pub read_only: bool,
    /// How many datasets this OCS instance holds, from its own JSON API.
    /// `None` for every other component, and for an instance that was not
    /// asked or did not answer.
    pub datasets: Option<u32>,
    /// How much its data directory holds, in bytes.
    ///
    /// Filled in by the caller, which is the half that has docker: it is read
    /// from inside the running container, so it is `None` here and `None`
    /// altogether for an instance that is not running.
    pub data_bytes: Option<u64>,
}

/// The version `chaps status` puts next to the API URL.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ApiVersion {
    /// What the API reported, or the tag `.chaps/project.yaml` pins when it
    /// reported nothing.
    pub value: String,
    /// True when `value` is that pin rather than the API's own answer.
    pub pinned: bool,
    /// The commit that build came from, where `/system/info` said. `None`
    /// when it did not, which is every chap-core built without `GIT_REVISION`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

impl ApiVersion {
    /// The cell for the chap-core line, empty when nothing is known at all.
    pub fn label(&self) -> String {
        match (self.value.is_empty(), self.pinned) {
            (true, _) => String::new(),
            (false, true) => format!("{} (pinned)", self.value),
            (false, false) => self.value.clone(),
        }
    }
}

/// A commit as the line prints it: the first twelve characters of a full SHA,
/// and anything else as it came.
///
/// chap-core reports the whole forty-character commit, which is thirty more
/// than anyone reads off a status line and the same length the image digest
/// is already shortened to.
pub fn short_revision(revision: &str) -> String {
    let revision = revision.trim();
    let sha = revision.len() > 12 && revision.chars().all(|c| c.is_ascii_hexdigit());
    match sha {
        true => revision.chars().take(12).collect(),
        false => revision.to_string(),
    }
}

/// Which build a moving chap-core tag is actually on.
///
/// `dev` is the same name today and tomorrow, so the tag says nothing about
/// what is running. These two do: the digest the image was pulled at, and the
/// commit chap-core reports for itself. Both are best effort - docker may not
/// be there and the build may carry no revision - and an empty one is left
/// off the line rather than printed as a hole.
pub fn moving_build_cell(tag: &str, digest: Option<&str>, revision: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(digest) = digest.filter(|d| !d.is_empty()) {
        parts.push(format!("running {digest}"));
    }
    if let Some(revision) = revision.filter(|r| !r.is_empty()) {
        parts.push(format!("revision {}", short_revision(revision)));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("{tag}: {}", parts.join(", "))
}

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
}

impl ModelState {
    /// The STATE cell.
    pub fn label(self) -> &'static str {
        match self {
            ModelState::Registered => "registered",
            ModelState::RunningNotRegistered => "running, not registered",
            ModelState::NotRunning => "not running",
            ModelState::Unmanaged => "unmanaged",
        }
    }

    /// Whether this row is something to do about.
    pub fn is_problem(self) -> bool {
        matches!(
            self,
            ModelState::RunningNotRegistered | ModelState::NotRunning
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

/// Probe chap-core and diff the registered services against the project.
///
/// Never fails: an unreachable API - or one that answers with something that
/// is not chap-core - is reported as [`ApiHealth::Down`]. `running` is the
/// set of compose services with a container that is up, which is what tells a
/// model that never started from one that started and did not register; an
/// empty set is a safe answer when docker cannot be asked.
///
/// `token` is the API token this deployment's `.env` sets, sent as
/// `Authorization: Bearer` on every request. It is needed for `/v2/services`
/// on a protected deployment; the health and info paths are open either way,
/// and a token they do not need does them no harm.
pub fn status(
    project: &Project,
    api_url: &str,
    timeout: Duration,
    running: &BTreeSet<String>,
    token: Option<&str>,
) -> StatusReport {
    let base = api_url.trim_end_matches('/').to_string();
    let mut expected: Vec<String> = project
        .state
        .models
        .values()
        .map(|m| m.service_id.clone())
        .collect();
    expected.sort();
    expected.dedup();

    let agent = agent(timeout);
    let chap_core = project.state.components.chap_core.enabled;
    let mut api = if !chap_core {
        // Nothing to ask, and nothing to wait out: a deployment without
        // chap-core has no API on this port, and probing one would only spend
        // a timeout to say so.
        ApiHealth::Off
    } else {
        match get(&agent, &base, HEALTH_PATH, token) {
            Ok(answer) => parse_health(&base, &answer.content_type, &answer.body),
            // A 401 is about the token, not about who answered: saying "not
            // chap-core" here would send an operator hunting for a dev server
            // that is not there.
            Err(Failure::Unauthorized) => ApiHealth::Down {
                error: token_rejected(&base, HEALTH_PATH, token.is_some()),
            },
            Err(Failure::Other(error)) => ApiHealth::Down { error },
        }
    };

    // Only ask for the service list when health already looked like
    // chap-core: otherwise we would wait out a second timeout to learn the
    // same thing. The list is the second half of the identity check - a
    // service registry that does not parse means whatever answered is not
    // chap-core, whatever `/health` said.
    let mut registered = Vec::new();
    if matches!(api, ApiHealth::Up { .. }) {
        let wrong = match get(&agent, &base, SERVICES_PATH, token) {
            Ok(answer) => match parse_services(&answer.body) {
                Ok(services) => {
                    registered = services;
                    None
                }
                Err(_) => Some(services_are_not_chap_core(
                    &base,
                    &body_description(&answer.content_type, &answer.body),
                )),
            },
            // The registry is not an open path, so a 401 here is the one place
            // a wrong token usually shows up: `/health` answered happily a
            // moment ago.
            Err(Failure::Unauthorized) => {
                Some(token_rejected(&base, SERVICES_PATH, token.is_some()))
            }
            Err(Failure::Other(error)) => Some(services_are_not_chap_core(&base, &error)),
        };
        if let Some(error) = wrong {
            api = ApiHealth::Down { error };
        }
    }

    let version = version_of(&agent, &base, &api, &project.state.chap_image_tag, token);
    let missing = missing_ids(&expected, &registered);
    let reach = project
        .state
        .models
        .values()
        .map(|m| {
            (
                m.service_id.clone(),
                reach(project, m.host_port, &m.service_id),
            )
        })
        .collect();
    let models = model_rows(&enabled_models(project), &registered, running, now());
    let unmanaged = models
        .iter()
        .filter(|m| m.state == ModelState::Unmanaged)
        .map(|m| m.id.clone())
        .collect();
    let components = component_rows(project, &agent, running);
    let (api_port, api_port_source) = project.api_port_in_effect();
    StatusReport {
        project: project.compose_project_name(),
        api_url: base,
        api_port,
        api_port_source,
        api,
        version,
        chap_tag_moving: crate::chapcore::is_moving_tag(&project.state.chap_image_tag),
        chap_tag: project.state.chap_image_tag.clone(),
        chap_build: None,
        registered,
        expected,
        missing,
        reach,
        models,
        unmanaged,
        auth: token.is_some(),
        components,
        unhealthy: Vec::new(),
    }
}

/// One row per enabled component other than chap-core.
///
/// OCS is asked over HTTP, because it publishes a host port and a `/health`
/// endpoint of its own; the object store publishes nothing by default, so the
/// only thing that can be said about it from out here is whether its container
/// is up. A component with no container at all is `not running` rather than
/// down: there is nothing wrong with a deployment that has not been started.
fn component_rows(
    project: &Project,
    agent: &ureq::Agent,
    running: &BTreeSet<String>,
) -> Vec<ComponentStatus> {
    let components = &project.state.components;
    let mut rows = Vec::new();
    if components.ocs.enabled {
        // An instance with no host port cannot be asked from out here at all:
        // the only way in is the compose network or whatever proxy sits in
        // front of it, and neither is something this probe can assume. So it
        // is judged by its container, exactly as the object store is.
        let probe = components
            .ocs_url()
            .map(|url| (get(agent, &url, HEALTH_PATH, None).is_ok(), url));
        let up = running.contains(crate::compose::OCS_SERVICE);
        // What the instance holds is only asked for when it has just
        // answered: a second request to an instance that is down would spend
        // another timeout to learn the same thing.
        let datasets = probe
            .as_ref()
            .filter(|(answered, _)| *answered)
            .and_then(|(_, url)| ocs_datasets(url));
        rows.push(ComponentStatus {
            name: crate::compose::OCS_SERVICE.to_string(),
            state: match &probe {
                Some((answered, _)) => component_state(*answered, up),
                None => component_state(up, up),
            },
            reach: components.ocs_reach(),
            health_url: probe.map(|(_, url)| format!("{url}{HEALTH_PATH}")),
            read_only: components.ocs.read_only,
            datasets,
            data_bytes: None,
        });
    }
    if components.s3.enabled {
        let up = running.contains(crate::compose::S3_SERVICE);
        rows.push(ComponentStatus {
            name: crate::compose::S3_SERVICE.to_string(),
            state: component_state(up, up),
            reach: match components.s3.port {
                Some(port) => format!("http://localhost:{port}"),
                None => "internal".to_string(),
            },
            health_url: None,
            read_only: false,
            datasets: None,
            data_bytes: None,
        });
    }
    rows
}

/// How many datasets an OCS instance holds, from `GET /datasets?f=json`.
///
/// Best-effort and bounded by [`OCS_TIMEOUT`]: an instance that does not
/// answer, answers something else, or is an older OCS without the JSON list
/// yields `None`, and the line it would have decorated is printed unchanged.
pub fn ocs_datasets(url: &str) -> Option<u32> {
    let agent = agent(OCS_TIMEOUT);
    let answer = get(&agent, url.trim_end_matches('/'), DATASETS_PATH, None).ok()?;
    parse_dataset_count(&answer.body)
}

/// The number of entries in an OCS `DatasetList`.
///
/// Strict about the envelope, for the same reason [`parse_services`] is: the
/// count goes on a line that says the instance is up, so a JSON body from
/// something else on that port must not become a number next to its name.
pub fn parse_dataset_count(body: &str) -> Option<u32> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    if value.get("kind")?.as_str()? != DATASET_LIST_KIND {
        return None;
    }
    u32::try_from(value.get("items")?.as_array()?.len()).ok()
}

/// The `kind` OCS puts on its dataset list.
const DATASET_LIST_KIND: &str = "DatasetList";

/// Where one component stands, from whether it answered and whether its
/// container is up.
pub fn component_state(answered: bool, container_up: bool) -> ComponentState {
    match (answered, container_up) {
        (true, _) => ComponentState::Up,
        (false, true) => ComponentState::Starting,
        (false, false) => ComponentState::NotRunning,
    }
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
    }));
    rows
}

/// The one line the table adds up to.
///
/// Rows the project does not manage are left out of the count: an unmanaged
/// registration is not a model this deployment has to get running.
pub fn closing_line(rows: &[ModelStatus]) -> String {
    let mine: Vec<&ModelStatus> = rows
        .iter()
        .filter(|r| r.state != ModelState::Unmanaged)
        .collect();
    let total = mine.len();
    if total == 0 {
        return "no models enabled; run `chaps models enable ID` to add one".to_string();
    }
    let problems = mine.iter().filter(|r| r.state.is_problem()).count();
    let noun = if total == 1 { "model" } else { "models" };
    if problems == 0 {
        return format!("all {total} {noun} registered");
    }
    let verb = if problems == 1 { "is" } else { "are" };
    format!("{problems} of {total} {noun} {verb} not registered.")
}

/// One hint per row that needs doing something about, in table order.
///
/// A model whose container is up but which chap-core does not know about is
/// not a crash to read the logs for: chapkit tries to register five times
/// while it starts and then gives up for good, so a model that came up before
/// chap-core was healthy stays invisible until it is restarted.
///
/// `--all` because nothing about the service has changed: it is running the
/// image and the configuration it should be, and the restart is only there to
/// make it introduce itself again. A plain `chaps restart` would recreate
/// what moved, which here is nothing.
///
/// `auth` adds the other reason a model never appears on a protected
/// deployment: chap-core rejects a registration that carries no key, and a
/// chap-core created before `compose.chaps.yml` passed the key through never
/// had one to check against.
/// The line under a clean status: registration is a heartbeat, and the only
/// way to know a model can work is to make it work.
pub const TEST_HINT: &str = "run `chaps models test --all` to check they can run";

pub fn hints(rows: &[ModelStatus], auth: bool) -> Vec<String> {
    let registration_key = if auth {
        concat!(
            "; if its log shows 401, chap-core is missing the registration key: ",
            "run `chaps sync`, then `chaps restart`"
        )
    } else {
        ""
    };
    let mine: Vec<&ModelStatus> = rows
        .iter()
        .filter(|row| row.state != ModelState::Unmanaged)
        .collect();
    let hints: Vec<String> = mine
        .iter()
        .filter_map(|row| match row.state {
            ModelState::RunningNotRegistered => Some(format!(
                "{}: restart it with `chaps restart --all {}`{registration_key}",
                row.id, row.id
            )),
            ModelState::NotRunning => Some(format!(
                "{}: start CHAP with `chaps up`, then `chaps logs {}`",
                row.id, row.id
            )),
            ModelState::Registered | ModelState::Unmanaged => None,
        })
        .collect();
    // Nothing to fix is not nothing to do: every model answered its
    // heartbeat, which is as far as `chaps status` can see.
    if hints.is_empty() && !mine.is_empty() {
        return vec![TEST_HINT.to_string()];
    }
    hints
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

/// Parse a `/health` body, strictly.
///
/// chap-core answers `{"status":"success","message":"healthy"}`. Anything
/// that is not JSON with a `status` field is somebody else holding the port -
/// a dev server, a proxy, an old deployment - and reporting that as `up`
/// would be worse than reporting nothing at all.
pub fn parse_health(api_url: &str, content_type: &str, body: &str) -> ApiHealth {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => {
            return ApiHealth::Down {
                error: is_not_chap_core(api_url, &body_description(content_type, body)),
            };
        }
    };
    let Some(status) = value.get("status").map(json_text) else {
        return ApiHealth::Down {
            error: is_not_chap_core(
                api_url,
                &format!(
                    "{} without a `status` field",
                    body_description(content_type, body)
                ),
            ),
        };
    };
    ApiHealth::Up {
        status,
        message: value.get("message").map(json_text).unwrap_or_default(),
    }
}

/// `port 8000 answers but it is not chap-core (got text/html)`.
pub fn is_not_chap_core(api_url: &str, what: &str) -> String {
    format!(
        "{} answers but it is not chap-core (got {what})",
        endpoint_phrase(api_url)
    )
}

/// What an HTTP 401 means: the token, never the identity of the server.
///
/// `sent` says whether `.env` had a token to send. Both halves matter: a
/// deployment whose `.env` is out of step with its running chap-core, and a
/// chap-core that was given a token while `.env` was not. The path is named
/// because the two ends of it differ - [`crate::auth::OPEN_PATHS`] answer
/// without a token at all, so a 401 from one of those means something in front
/// of chap-core is asking as well.
pub fn token_rejected(api_url: &str, path: &str, sent: bool) -> String {
    let endpoint = endpoint_phrase(api_url);
    let mut text = if sent {
        format!(
            "{endpoint} answers {path} with HTTP 401: the API token in .env is not accepted; \
             `chaps auth show --reveal` prints it, and `chaps up` hands a rotated one to \
             chap-core"
        )
    } else {
        format!(
            "{endpoint} answers {path} with HTTP 401: it requires an API token and .env sets \
             none; `chaps auth enable` writes one, or add CHAP_API_TOKEN to .env to match the \
             chap-core that is running"
        )
    };
    if crate::auth::OPEN_PATHS.contains(&path) {
        text.push_str(&format!(
            " ({path} is open even when CHAP_API_TOKEN is set, so something in front of \
             chap-core may be asking for credentials too)"
        ));
    }
    text
}

/// The same verdict, reached through the service registry instead.
pub fn services_are_not_chap_core(api_url: &str, what: &str) -> String {
    format!(
        "{} answers but it is not chap-core: {SERVICES_PATH} returned {what}, not {{count, services}}",
        endpoint_phrase(api_url)
    )
}

/// How to name the thing that answered: its port, which is what the operator
/// has to free, or the whole URL when there is no port in it.
fn endpoint_phrase(api_url: &str) -> String {
    match port_of(api_url) {
        Some(port) => format!("port {port}"),
        None => api_url.to_string(),
    }
}

/// The port of `http://host:PORT/path`, when it has one.
fn port_of(url: &str) -> Option<u16> {
    let authority = url.rsplit("://").next()?.split('/').next()?;
    authority.rsplit_once(':')?.1.parse().ok()
}

/// What to call a body that is not chap-core's JSON: the content type the
/// server sent, or the first bytes when it sent none.
pub fn body_description(content_type: &str, body: &str) -> String {
    // `text/html; charset=utf-8` says nothing more than `text/html` here.
    let kind = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if !kind.is_empty() {
        return kind;
    }
    let head: String = body
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(FIRST_BYTES)
        .collect();
    if head.is_empty() {
        return "an empty body".to_string();
    }
    let ellipsis = if body.trim().chars().count() > FIRST_BYTES {
        "..."
    } else {
        ""
    };
    format!("`{head}{ellipsis}`")
}

/// How much of an unrecognised body to quote back.
const FIRST_BYTES: usize = 40;

/// Parse a `/v2/services` body into the flattened report entries.
///
/// Strict about the envelope: chap-core's registry is `{count, services}`, so
/// a body without both is not chap-core's, however well-formed it is.
pub fn parse_services(body: &str) -> Result<Vec<RegisteredService>, serde_json::Error> {
    let wire: ServicesBody = serde_json::from_str(body)?;
    Ok(wire.services.into_iter().map(flatten).collect())
}

/// chap-core's own version, from whichever info endpoint answers.
///
/// Best-effort: neither path is required, and a chap-core that publishes
/// neither still gets a version in the report - the tag the project pins,
/// marked as such so nobody reads it as the running build.
fn version_of(
    agent: &ureq::Agent,
    base: &str,
    api: &ApiHealth,
    pinned: &str,
    token: Option<&str>,
) -> ApiVersion {
    if matches!(api, ApiHealth::Up { .. }) {
        for path in INFO_PATHS {
            if let Ok(answer) = get(agent, base, path, token)
                && let Some(version) = parse_version(&answer.body)
            {
                return ApiVersion {
                    value: version,
                    pinned: false,
                    revision: parse_revision(&answer.body),
                };
            }
        }
    }
    ApiVersion {
        value: pinned.to_string(),
        pinned: true,
        revision: None,
    }
}

/// The commit an info body says the build came from.
///
/// chap-core fills `revision` from `GIT_REVISION`, which is empty in a build
/// that was not given one; empty is no answer, not an answer of "".
pub fn parse_revision(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    const KEYS: &[&str] = &["revision", "git_revision", "commit"];
    for key in KEYS {
        if let Some(found) = string_at(&value, key) {
            return Some(found);
        }
    }
    None
}

/// A version out of an info body, wherever it keeps it.
///
/// chap-core has moved this field around between releases, so the obvious
/// spellings are all accepted and an unknown shape is simply no answer.
pub fn parse_version(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    const KEYS: &[&str] = &["version", "chap_core_version", "app_version"];
    const NESTED: &[&str] = &["info", "system", "chap_core"];
    for key in KEYS {
        if let Some(found) = string_at(&value, key) {
            return Some(found);
        }
    }
    for outer in NESTED {
        let inner = value.get(outer)?;
        for key in KEYS {
            if let Some(found) = string_at(inner, key) {
                return Some(found);
            }
        }
    }
    None
}

fn string_at(value: &serde_json::Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A JSON value as the text to print: a string as itself, anything else as
/// its JSON spelling, so a numeric `status` is still reported rather than
/// dropped.
fn json_text(value: &serde_json::Value) -> String {
    match value.as_str() {
        Some(text) => text.to_string(),
        None => value.to_string(),
    }
}

/// How long ago a wire timestamp was, as a table cell.
fn ago(now: u64, at: &str) -> Option<String> {
    let then = parse_rfc3339(at)?;
    Some(output::ago(Duration::from_secs(now.saturating_sub(then))))
}

/// Seconds since the Unix epoch for an RFC 3339 timestamp, as chap-core
/// writes `last_ping_at`.
///
/// Accepts `2026-09-22T09:04:30Z`, a fractional second, a numeric offset and
/// a space in place of the `T`. Anything else is `None`, which the table
/// shows as `-` rather than inventing an age.
pub fn parse_rfc3339(text: &str) -> Option<u64> {
    let text = text.trim();
    let (date, rest) = text.split_once(['T', 't', ' '])?;
    let mut fields = date.split('-');
    let year: i64 = fields.next()?.parse().ok()?;
    let month: u32 = fields.next()?.parse().ok()?;
    let day: u32 = fields.next()?.parse().ok()?;
    if fields.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let (clock, offset) = split_offset(rest);
    let mut fields = clock.split(':');
    let hour: i64 = fields.next()?.trim().parse().ok()?;
    let minute: i64 = fields.next()?.parse().ok()?;
    // The fractional part is below the resolution of anything this prints.
    let second: i64 = match fields.next() {
        Some(text) => text.split('.').next()?.parse().ok()?,
        None => 0,
    };
    if fields.next().is_some() {
        return None;
    }

    let seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    u64::try_from(seconds).ok()
}

/// Split a time off its UTC offset, in seconds. A time with no offset at all
/// is read as UTC, which is what every chap-core timestamp is.
fn split_offset(rest: &str) -> (&str, i64) {
    if let Some(clock) = rest.strip_suffix(['Z', 'z']) {
        return (clock, 0);
    }
    // A clock holds no sign, so the last one can only start the offset.
    let Some(at) = rest.rfind(['+', '-']) else {
        return (rest, 0);
    };
    let (clock, offset) = rest.split_at(at);
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let digits: String = offset.chars().filter(char::is_ascii_digit).collect();
    let (hours, minutes) = match digits.len() {
        4 => (digits[..2].parse().unwrap_or(0), digits[2..].parse().ok()),
        2 => (digits.parse().unwrap_or(0), Some(0)),
        _ => (0, Some(0)),
    };
    (clock, sign * (hours * 3600 + minutes.unwrap_or(0) * 60))
}

/// Days since the Unix epoch for a civil date.
///
/// Howard Hinnant's `days_from_civil`, the inverse of the conversion
/// [`crate::backup::utc_parts`] uses, and exact for every date a deployment
/// will ever report.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Seconds since the Unix epoch, now.
fn now() -> u64 {
    crate::backup::now()
}

fn flatten(w: WireService) -> RegisteredService {
    let id = if w.id.is_empty() {
        w.info.id.clone()
    } else {
        w.id
    };
    let display_name = if w.info.display_name.is_empty() {
        id.clone()
    } else {
        w.info.display_name
    };
    RegisteredService {
        id,
        url: w.url,
        display_name,
        version: w.info.version,
        last_ping_at: w.last_ping_at,
        expires_at: w.expires_at,
    }
}

/// One answer from the API: the body plus the content type, which is how an
/// unrecognised body is named in the report.
struct Answer {
    content_type: String,
    body: String,
}

/// Why a request yielded no usable answer.
enum Failure {
    /// HTTP 401. Reported separately because it is the one status code that
    /// says something about the deployment's own configuration rather than
    /// about whatever is on the port.
    Unauthorized,
    /// Anything else, already described for a human.
    Other(String),
}

/// `GET base+path`, with the API token when there is one.
fn get(
    agent: &ureq::Agent,
    base: &str,
    path: &str,
    token: Option<&str>,
) -> Result<Answer, Failure> {
    let mut request = agent.get(format!("{base}{path}"));
    if let Some(token) = token {
        // The header chap-core documents in its OpenAPI spec and enforces in
        // its middleware. It also accepts the token in `X-Service-Key`, but
        // that spelling exists for servicekit, which can send no other.
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let url = format!("{base}{path}");
    let started = std::time::Instant::now();
    let mut response = request.call().map_err(|e| {
        crate::output::verbose(&format!(
            "GET {url} -> failed in {}ms",
            started.elapsed().as_millis()
        ));
        Failure::Other(e.to_string())
    })?;
    let status = response.status();
    crate::output::verbose(&format!(
        "GET {url} -> {} in {}ms",
        status.as_u16(),
        started.elapsed().as_millis()
    ));
    if status.as_u16() == 401 {
        return Err(Failure::Unauthorized);
    }
    if !status.is_success() {
        return Err(Failure::Other(format!("HTTP {}", status.as_u16())));
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| Failure::Other(e.to_string()))?;
    crate::output::debug(&format!("  {}", crate::output::trace_body(&body)));
    Ok(Answer { content_type, body })
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        // Status codes are reported by the caller, not raised as errors.
        .http_status_as_error(false)
        .user_agent(concat!("chaps-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

/// `GET /v2/services`. The envelope is required (see [`parse_services`]);
/// fields the report does not use - `registered_at`, everything chap-core
/// adds later - are ignored.
#[derive(Debug, Deserialize)]
struct ServicesBody {
    #[allow(dead_code)]
    count: u64,
    services: Vec<WireService>,
}

#[derive(Debug, Default, Deserialize)]
struct WireService {
    #[serde(default)]
    id: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    info: WireInfo,
    #[serde(default)]
    last_ping_at: String,
    #[serde(default)]
    expires_at: String,
}

#[derive(Debug, Default, Deserialize)]
struct WireInfo {
    #[serde(default)]
    id: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://localhost:8000";

    const SERVICES: &str = r#"{
      "count": 2,
      "services": [
        {
          "id": "chapkit-ewars-model",
          "url": "http://chapkit-ewars-model:8000",
          "info": {
            "id": "chapkit-ewars-model",
            "display_name": "CHAP-EWARS",
            "version": "1.0.0",
            "author": "someone",
            "unknown_future_field": 42
          },
          "registered_at": "2026-09-22T09:00:00Z",
          "last_ping_at": "2026-09-22T09:04:30Z",
          "expires_at": "2026-09-22T09:09:30Z"
        },
        {
          "id": "auto-arima-chapkit",
          "url": "http://auto-arima-chapkit:8000",
          "info": { "id": "auto-arima-chapkit", "display_name": "Auto ARIMA", "version": "1.2.0" },
          "registered_at": "2026-09-22T09:01:00Z",
          "last_ping_at": "2026-09-22T09:04:31Z",
          "expires_at": "2026-09-22T09:09:31Z"
        }
      ]
    }"#;

    #[test]
    fn services_payload_is_flattened() {
        let services = parse_services(SERVICES).expect("sample payload parses");
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].id, "chapkit-ewars-model");
        assert_eq!(services[0].display_name, "CHAP-EWARS");
        assert_eq!(services[0].version, "1.0.0");
        assert_eq!(services[0].url, "http://chapkit-ewars-model:8000");
        assert_eq!(services[0].last_ping_at, "2026-09-22T09:04:30Z");
        assert_eq!(services[0].expires_at, "2026-09-22T09:09:30Z");
        assert_eq!(services[1].version, "1.2.0");
    }

    #[test]
    fn services_payload_tolerates_missing_fields() {
        let services = parse_services(r#"{"count":1,"services":[{"id":"x"}]}"#).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].id, "x");
        // Without an info block the id doubles as the display name.
        assert_eq!(services[0].display_name, "x");
        assert!(services[0].version.is_empty());
        assert!(services[0].url.is_empty());
    }

    #[test]
    fn services_payload_falls_back_to_the_info_id() {
        let services = parse_services(r#"{"count":1,"services":[{"info":{"id":"y"}}]}"#).unwrap();
        assert_eq!(services[0].id, "y");
        assert_eq!(services[0].display_name, "y");
    }

    #[test]
    fn an_empty_registry_parses_to_nothing() {
        assert!(
            parse_services(r#"{"count":0,"services":[]}"#)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_services_body_without_the_envelope_is_not_chap_cores() {
        // Well-formed JSON is not enough: the registry is `{count, services}`,
        // and anything else means something else is on the port.
        for body in [
            "<html>nope</html>",
            "{}",
            r#"{"services":[]}"#,
            r#"{"count":0}"#,
            r#"[{"id":"x"}]"#,
            r#"{"detail":"Not Found"}"#,
        ] {
            assert!(parse_services(body).is_err(), "{body}");
        }
    }

    /// OCS's dataset list, as `GET /datasets?f=json` answers it.
    const DATASETS: &str = r#"{
      "kind": "DatasetList",
      "items": [
        { "dataset_id": "worldpop", "dataset_name": "WorldPop", "period_type": "year" },
        { "dataset_id": "chirps3", "dataset_name": "CHIRPS3", "period_type": "day" },
        { "dataset_id": "era5-land", "dataset_name": "ERA5-Land", "period_type": "day",
          "unknown_future_field": 42 }
      ],
      "links": []
    }"#;

    #[test]
    fn the_dataset_list_is_counted_and_nothing_else_is() {
        assert_eq!(parse_dataset_count(DATASETS), Some(3));
        assert_eq!(
            parse_dataset_count(r#"{"kind":"DatasetList","items":[]}"#),
            Some(0)
        );
        assert_eq!(
            parse_dataset_count(r#"{"kind":"DatasetList","items":[{"dataset_id":"a"}]}"#),
            Some(1)
        );

        // Anything that is not OCS's own envelope is no count at all: the
        // number sits on a line that says the instance is up.
        for body in [
            "",
            "<html>nope</html>",
            "{}",
            r#"{"items":[]}"#,
            r#"{"kind":"Dataset","items":[]}"#,
            r#"{"kind":"DatasetList"}"#,
            r#"{"kind":"DatasetList","items":{}}"#,
            r#"[{"dataset_id":"a"}]"#,
            r#"{"count":2,"services":[]}"#,
        ] {
            assert_eq!(parse_dataset_count(body), None, "{body}");
        }
    }

    #[test]
    fn health_is_up_only_for_chap_cores_own_json() {
        let ApiHealth::Up { status, message } = parse_health(
            URL,
            "application/json",
            r#"{"status":"success","message":"healthy"}"#,
        ) else {
            panic!("chap-core's own body is up");
        };
        assert_eq!(status, "success");
        assert_eq!(message, "healthy");

        // A message chap-core did not send is not a reason to call it down.
        let ApiHealth::Up { status, message } =
            parse_health(URL, "application/json", r#"{"status":"ok"}"#)
        else {
            panic!("expected up");
        };
        assert_eq!(status, "ok");
        assert!(message.is_empty());
    }

    #[test]
    fn an_html_200_is_down_and_names_the_content_type() {
        let ApiHealth::Down { error } = parse_health(
            URL,
            "text/html; charset=utf-8",
            "<!doctype html><html><body>hello</body></html>",
        ) else {
            panic!("HTML is not chap-core");
        };
        assert_eq!(
            error,
            "port 8000 answers but it is not chap-core (got text/html)"
        );
    }

    #[test]
    fn a_non_json_200_without_a_content_type_is_quoted_back() {
        let ApiHealth::Down { error } = parse_health(URL, "", "not json at all") else {
            panic!("plain text is not chap-core");
        };
        assert_eq!(
            error,
            "port 8000 answers but it is not chap-core (got `not json at all`)"
        );

        // A long body is cut, so the line stays one line.
        let ApiHealth::Down { error } = parse_health(URL, "", &"x".repeat(200)) else {
            panic!("expected down");
        };
        assert!(error.ends_with("...`)"), "{error}");
        assert!(error.len() < 120, "{error}");

        // An empty 200 says so rather than quoting nothing.
        let ApiHealth::Down { error } = parse_health(URL, "", "   ") else {
            panic!("expected down");
        };
        assert!(error.contains("an empty body"), "{error}");
    }

    #[test]
    fn json_without_a_status_field_is_down() {
        // The shape a reverse proxy or another API answers with.
        let ApiHealth::Down { error } =
            parse_health(URL, "application/json", r#"{"detail":"Not Found"}"#)
        else {
            panic!("JSON without a status is not chap-core");
        };
        assert!(error.contains("not chap-core"), "{error}");
        assert!(error.contains("without a `status` field"), "{error}");

        // An empty JSON object used to count as up; it no longer does.
        assert!(matches!(
            parse_health(URL, "application/json", "{}"),
            ApiHealth::Down { .. }
        ));
    }

    #[test]
    fn the_endpoint_is_named_by_its_port_when_it_has_one() {
        assert_eq!(
            is_not_chap_core("http://127.0.0.1:54321", "text/html"),
            "port 54321 answers but it is not chap-core (got text/html)"
        );
        // No port to name: the URL itself is the next best thing.
        assert_eq!(
            is_not_chap_core("https://chap.example.test", "text/html"),
            "https://chap.example.test answers but it is not chap-core (got text/html)"
        );
        assert_eq!(port_of("http://localhost:8000/health"), Some(8000));
        assert_eq!(port_of("http://localhost"), None);
    }

    #[test]
    fn a_401_is_about_the_token_and_not_about_who_answered() {
        // A token we sent and chap-core refused.
        let error = token_rejected(URL, SERVICES_PATH, true);
        assert!(
            error.starts_with("port 8000 answers /v2/services with HTTP 401:"),
            "{error}"
        );
        assert!(
            error.contains("the API token in .env is not accepted"),
            "{error}"
        );
        assert!(error.contains("chaps auth show --reveal"), "{error}");
        // Never the other verdict: the port is chap-core's, the token is wrong.
        assert!(!error.contains("not chap-core"), "{error}");

        // A path chap-core leaves open says so as well: the 401 cannot have
        // come from its own middleware.
        assert!(
            !error.contains("is open even when"),
            "/v2/services is not open: {error}"
        );

        // A protected chap-core and a `.env` that has no token for it.
        let error = token_rejected(URL, HEALTH_PATH, false);
        assert!(
            error.contains("it requires an API token and .env sets none"),
            "{error}"
        );
        assert!(error.contains("chaps auth enable"), "{error}");
        assert!(!error.contains("not chap-core"), "{error}");
        assert!(
            error.contains("/health is open even when CHAP_API_TOKEN is set"),
            "{error}"
        );

        // The endpoint is named the same way the other verdicts name it.
        assert!(
            token_rejected("https://chap.example.test", HEALTH_PATH, true)
                .starts_with("https://chap.example.test answers /health")
        );
    }

    #[test]
    fn a_registry_that_is_not_chap_cores_says_what_it_expected() {
        let error = services_are_not_chap_core(URL, "HTTP 404");
        assert_eq!(
            error,
            "port 8000 answers but it is not chap-core: /v2/services returned HTTP 404, \
             not {count, services}"
        );
        assert!(services_are_not_chap_core(URL, "text/html").contains("not chap-core"));
    }

    #[test]
    fn the_version_is_read_wherever_the_info_body_keeps_it() {
        assert_eq!(
            parse_version(r#"{"version":"2.3.1","name":"chap-core"}"#).as_deref(),
            Some("2.3.1")
        );
        assert_eq!(
            parse_version(r#"{"chap_core_version":"v2.3.1"}"#).as_deref(),
            Some("v2.3.1")
        );
        assert_eq!(
            parse_version(r#"{"info":{"version":"2.4.0"}}"#).as_deref(),
            Some("2.4.0")
        );
        for body in [
            "{}",
            "<html>",
            r#"{"version":""}"#,
            r#"{"version":2}"#,
            r#"{"other":{"version":"1"}}"#,
        ] {
            assert_eq!(parse_version(body), None, "{body}");
        }
    }

    #[test]
    fn the_version_label_marks_the_pin() {
        assert_eq!(
            ApiVersion {
                value: "2.3.1".into(),
                pinned: false,
                revision: None,
            }
            .label(),
            "2.3.1"
        );
        assert_eq!(
            ApiVersion {
                value: "v2.3.1".into(),
                pinned: true,
                revision: None,
            }
            .label(),
            "v2.3.1 (pinned)"
        );
        assert!(
            ApiVersion {
                value: String::new(),
                pinned: true,
                revision: None,
            }
            .label()
            .is_empty()
        );
    }

    #[test]
    fn a_moving_tag_says_which_build_it_is_running() {
        let sha = "7bf2a98739f46b57487c8cb05e9ddd29778080e9";
        assert_eq!(
            moving_build_cell("master", Some("cc09e3654ff2"), Some(sha)),
            "master: running cc09e3654ff2, revision 7bf2a98739f4"
        );
        // Either half on its own, and nothing at all when neither could be had.
        assert_eq!(
            moving_build_cell("dev", Some("cc09e3654ff2"), None),
            "dev: running cc09e3654ff2"
        );
        assert_eq!(
            moving_build_cell("dev", None, Some("a1b2c3d")),
            "dev: revision a1b2c3d"
        );
        assert_eq!(moving_build_cell("dev", None, None), "");
        assert_eq!(moving_build_cell("dev", Some(""), Some("")), "");
    }

    #[test]
    fn a_full_commit_is_shortened_and_anything_else_is_not() {
        assert_eq!(
            short_revision("7bf2a98739f46b57487c8cb05e9ddd29778080e9"),
            "7bf2a98739f4"
        );
        assert_eq!(short_revision("a1b2c3d"), "a1b2c3d");
        assert_eq!(short_revision("2.4.0.dev0+g7bf2a98"), "2.4.0.dev0+g7bf2a98");
        assert_eq!(short_revision(""), "");
    }

    #[test]
    fn the_revision_comes_off_the_info_body_when_there_is_one() {
        let body = r#"{"chap_core_version":"2.4.0.dev0","revision":"7bf2a98739f4"}"#;
        assert_eq!(parse_revision(body).as_deref(), Some("7bf2a98739f4"));
        assert_eq!(parse_version(body).as_deref(), Some("2.4.0.dev0"));
        // A build with no GIT_REVISION reports an empty one, which is no answer.
        assert_eq!(parse_revision(r#"{"revision":""}"#), None);
        assert_eq!(parse_revision(r#"{"chap_core_version":"2.3.1"}"#), None);
        assert_eq!(parse_revision("<html>"), None);
    }

    #[test]
    fn missing_is_expected_minus_registered() {
        let registered = parse_services(SERVICES).unwrap();
        let expected = vec![
            "auto-arima-chapkit".to_string(),
            "chapkit-ewars-model".to_string(),
            "chapkit-simple-multistep-model".to_string(),
        ];
        assert_eq!(
            missing_ids(&expected, &registered),
            vec!["chapkit-simple-multistep-model".to_string()]
        );
        assert!(missing_ids(&[], &registered).is_empty());
        assert_eq!(missing_ids(&expected, &[]), expected);
    }

    /// A set of running compose services, as `docker compose ps` would give.
    fn running(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    /// One registered service, pinged `age` seconds before [`NOW`].
    fn registered(id: &str, age: u64) -> RegisteredService {
        RegisteredService {
            id: id.to_string(),
            url: format!("http://{id}:8000"),
            display_name: id.to_string(),
            version: "1.0.0".to_string(),
            last_ping_at: crate::backup::timestamp(NOW - age),
            expires_at: crate::backup::timestamp(NOW + 300 - age),
        }
    }

    /// A fixed "now", so the relative times in these tests do not move.
    const NOW: u64 = 1_790_147_400;

    /// The models of the sample deployment: one published, two internal.
    fn enabled() -> Vec<(String, Option<u16>)> {
        vec![
            ("chapkit-ewars-model".to_string(), Some(5001)),
            ("chapkit-rwanda-malaria-bym-model".to_string(), None),
            ("auto-arima-chapkit".to_string(), None),
        ]
    }

    #[test]
    fn a_row_state_comes_from_registration_and_the_running_set() {
        let rows = model_rows(
            &enabled(),
            &[registered("chapkit-ewars-model", 12)],
            &running(&[
                "chap",
                "chapkit-ewars-model",
                "chapkit-rwanda-malaria-bym-model",
            ]),
            NOW,
        );
        assert_eq!(rows.len(), 3, "one row per enabled model");

        assert_eq!(rows[0].id, "chapkit-ewars-model");
        assert_eq!(rows[0].state, ModelState::Registered);
        assert_eq!(
            rows[0].reach, "http://localhost:5001",
            "`--json` keeps the URL it has always carried"
        );
        assert_eq!(rows[0].host_port, Some(5001), "and the table gets the port");
        assert_eq!(rows[0].last_ping.as_deref(), Some("12s ago"));

        // Its container is up, so the registration is what failed.
        assert_eq!(rows[1].state, ModelState::RunningNotRegistered);
        assert_eq!(rows[1].state.label(), "running, not registered");
        assert_eq!(rows[1].reach, "internal");
        assert_eq!(rows[1].host_port, None);
        assert_eq!(rows[1].last_ping, None);

        // Nothing is running it at all.
        assert_eq!(rows[2].state, ModelState::NotRunning);
        assert_eq!(rows[2].reach, "internal");
    }

    #[test]
    fn a_service_the_project_does_not_enable_is_unmanaged() {
        let rows = model_rows(
            &enabled(),
            &[
                registered("chapkit-ewars-model", 12),
                registered("some-other-service", 3),
            ],
            &running(&["chapkit-ewars-model"]),
            NOW,
        );
        assert_eq!(rows.len(), 4, "the stranger gets a row of its own");
        let stranger = rows.last().unwrap();
        assert_eq!(stranger.id, "some-other-service");
        assert_eq!(stranger.state, ModelState::Unmanaged);
        assert_eq!(stranger.state.label(), "unmanaged");
        // chap-core's own URL is the only handle there is on it.
        assert_eq!(stranger.reach, "http://some-other-service:8000");
        assert_eq!(stranger.last_ping.as_deref(), Some("3s ago"));

        // And it is not counted as one of this deployment's models.
        assert_eq!(closing_line(&rows), "2 of 3 models are not registered.");
    }

    #[test]
    fn the_closing_line_counts_what_the_project_enables() {
        let all_good = model_rows(
            &enabled(),
            &[
                registered("chapkit-ewars-model", 12),
                registered("chapkit-rwanda-malaria-bym-model", 12),
                registered("auto-arima-chapkit", 12),
            ],
            &BTreeSet::new(),
            NOW,
        );
        assert_eq!(closing_line(&all_good), "all 3 models registered");

        let one_missing = model_rows(
            &enabled(),
            &[
                registered("chapkit-ewars-model", 12),
                registered("auto-arima-chapkit", 12),
            ],
            &running(&["chapkit-rwanda-malaria-bym-model"]),
            NOW,
        );
        assert_eq!(
            closing_line(&one_missing),
            "1 of 3 models is not registered."
        );

        // One model, and nothing at all, both read as English.
        let one = model_rows(&enabled()[..1], &[], &BTreeSet::new(), NOW);
        assert_eq!(closing_line(&one), "1 of 1 model is not registered.");
        let one = model_rows(
            &enabled()[..1],
            &[registered("chapkit-ewars-model", 1)],
            &BTreeSet::new(),
            NOW,
        );
        assert_eq!(closing_line(&one), "all 1 model registered");
        assert_eq!(
            closing_line(&[]),
            "no models enabled; run `chaps models enable ID` to add one"
        );
    }

    #[test]
    fn every_problem_row_gets_its_own_hint() {
        let rows = model_rows(
            &enabled(),
            &[registered("chapkit-ewars-model", 12)],
            &running(&["chapkit-rwanda-malaria-bym-model"]),
            NOW,
        );
        assert_eq!(
            hints(&rows, false),
            vec![
                "chapkit-rwanda-malaria-bym-model: restart it with \
                 `chaps restart --all chapkit-rwanda-malaria-bym-model`"
                    .to_string(),
                "auto-arima-chapkit: start CHAP with `chaps up`, \
                 then `chaps logs auto-arima-chapkit`"
                    .to_string(),
            ]
        );

        // Nothing wrong: the one line left is the check `status` cannot make
        // itself, and it does not depend on whether the API is protected.
        let rows = model_rows(
            &enabled()[..1],
            &[registered("chapkit-ewars-model", 12)],
            &BTreeSet::new(),
            NOW,
        );
        assert_eq!(hints(&rows, false), vec![TEST_HINT.to_string()]);
        assert_eq!(hints(&rows, true), vec![TEST_HINT.to_string()]);
        assert!(TEST_HINT.contains("chaps models test --all"));

        // Nothing enabled at all has nothing to test either, and a
        // registration this project does not manage is not a model of ours.
        assert!(hints(&[], false).is_empty());
        let stranger = model_rows(
            &[],
            &[registered("some-other-service", 3)],
            &BTreeSet::new(),
            NOW,
        );
        assert!(hints(&stranger, false).is_empty());
    }

    #[test]
    fn a_protected_deployment_names_the_other_reason_a_model_never_registers() {
        let rows = model_rows(
            &enabled()[..1],
            &[],
            &running(&["chapkit-ewars-model"]),
            NOW,
        );
        let hint = &hints(&rows, true)[0];
        assert!(
            hint.starts_with("chapkit-ewars-model: restart it with "),
            "{hint}"
        );
        assert!(
            hint.ends_with(
                "; if its log shows 401, chap-core is missing the registration key: \
                 run `chaps sync`, then `chaps restart`"
            ),
            "{hint}"
        );
        // Without authentication there is no 401 to explain.
        assert!(!hints(&rows, false)[0].contains("401"));
    }

    #[test]
    fn relative_times_come_from_the_wire_timestamp() {
        assert_eq!(
            ago(NOW, &crate::backup::timestamp(NOW)).as_deref(),
            Some("0s ago")
        );
        assert_eq!(
            ago(NOW, &crate::backup::timestamp(NOW - 12)).as_deref(),
            Some("12s ago")
        );
        assert_eq!(
            ago(NOW, &crate::backup::timestamp(NOW - 180)).as_deref(),
            Some("3m ago")
        );
        assert_eq!(
            ago(NOW, &crate::backup::timestamp(NOW - 7200)).as_deref(),
            Some("2h ago")
        );
        // A clock that is ahead of ours is not a negative age.
        assert_eq!(
            ago(NOW, &crate::backup::timestamp(NOW + 60)).as_deref(),
            Some("0s ago")
        );
        // Nothing to go on: the table prints a dash instead.
        for text in ["", "-", "soon", "2026-09-22"] {
            assert_eq!(ago(NOW, text), None, "{text:?}");
        }
    }

    #[test]
    fn rfc3339_parsing_round_trips_through_the_formatter() {
        for unix in [0, 1_709_164_800, 1_735_689_599, NOW] {
            let text = crate::backup::timestamp(unix);
            assert_eq!(parse_rfc3339(&text), Some(unix), "{text}");
        }
        // The spellings a JSON API may use.
        assert_eq!(
            parse_rfc3339("2026-09-22T09:04:30.123456Z"),
            parse_rfc3339("2026-09-22T09:04:30Z")
        );
        assert_eq!(
            parse_rfc3339("2026-09-22 09:04:30"),
            parse_rfc3339("2026-09-22T09:04:30Z")
        );
        assert_eq!(
            parse_rfc3339("2026-09-22T11:04:30+02:00"),
            parse_rfc3339("2026-09-22T09:04:30Z")
        );
        assert_eq!(
            parse_rfc3339("2026-09-22T07:04:30-0200"),
            parse_rfc3339("2026-09-22T09:04:30Z")
        );
        for bad in [
            "",
            "not a time",
            "2026-09-22",
            "2026-13-01T00:00:00Z",
            "1969-12-31T23:59:59Z",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }

    #[test]
    fn reach_is_the_host_port_or_the_proxy() {
        let project = Project {
            dir: std::path::PathBuf::from("/tmp/chapx"),
            state: crate::project::ProjectState {
                api_port: 8123,
                ..Default::default()
            },
        };
        assert_eq!(
            reach(&project, Some(5001), "chapkit-ewars-model"),
            "http://localhost:5001"
        );
        assert_eq!(
            reach(&project, None, "chapkit-ewars-model"),
            "internal (proxy: http://localhost:8123/v2/services/chapkit-ewars-model/run/)"
        );
    }

    #[test]
    fn report_helpers_describe_the_state() {
        let report = StatusReport {
            project: Some("chapx-1ab2c3".into()),
            api_url: URL.into(),
            api_port: 8000,
            api_port_source: ApiPortSource::Project,
            api: ApiHealth::Down {
                error: "connection refused".into(),
            },
            version: ApiVersion {
                value: "v2.3.1".into(),
                pinned: true,
                revision: None,
            },
            chap_tag: "v2.3.1".into(),
            chap_tag_moving: false,
            chap_build: None,
            registered: Vec::new(),
            expected: vec!["a".into()],
            missing: vec!["a".into()],
            reach: BTreeMap::new(),
            models: Vec::new(),
            unmanaged: Vec::new(),
            auth: false,
            components: Vec::new(),
            unhealthy: Vec::new(),
        };
        assert!(!report.is_up());
        assert!(!report.api_container_unhealthy());
        assert!(!report.is_complete());

        let report = StatusReport {
            api: ApiHealth::Up {
                status: "success".into(),
                message: String::new(),
            },
            missing: Vec::new(),
            ..report
        };
        assert!(report.is_up());
        assert!(report.is_complete());
    }

    #[test]
    fn a_down_api_serialises_with_its_state_tag() {
        let value = serde_json::to_value(ApiHealth::Down {
            error: "connection refused".into(),
        })
        .unwrap();
        assert_eq!(value["state"], "down");
        assert_eq!(value["error"], "connection refused");
    }

    #[test]
    fn a_row_serialises_with_its_state_as_a_string() {
        let rows = model_rows(
            &enabled()[..1],
            &[registered("chapkit-ewars-model", 12)],
            &BTreeSet::new(),
            NOW,
        );
        let value = serde_json::to_value(&rows).unwrap();
        assert_eq!(value[0]["id"], "chapkit-ewars-model");
        assert_eq!(value[0]["state"], "registered");
        assert_eq!(value[0]["reach"], "http://localhost:5001");
        assert_eq!(value[0]["last_ping"], "12s ago");

        let rows = model_rows(&enabled()[1..2], &[], &running(&["x"]), NOW);
        assert_eq!(
            serde_json::to_value(&rows).unwrap()[0]["state"],
            "not-running"
        );
    }
}
