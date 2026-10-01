//! `chaps status`: chap-core health plus the services it has registered.
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

/// The one route DHIS2 answers without credentials: `PingController`, which
/// `DhisWebApiWebSecurityConfig` permits. So it can be asked of an instance
/// this CLI holds no login for, which is every instance.
///
/// It is also the route `compose.dhis2.yml` builds the container's own
/// healthcheck out of, and judged the same way: the status code, not the body.
pub const DHIS2_PING_PATH: &str = "/api/ping";

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
    /// Whether this deployment has a DHIS2 that no `chaps dhis2 connect` has
    /// been recorded for, from [`Components::dhis2_needs_connecting`].
    ///
    /// Read off `.chaps/components.yaml` and nothing else. It says what chaps
    /// has recorded, never what DHIS2 has: the route can have been deleted,
    /// repointed or disabled since, and `false` here is not evidence that the
    /// Modeling App can reach CHAP. `chaps dhis2 show` is the command that
    /// asks DHIS2.
    ///
    /// [`Components::dhis2_needs_connecting`]: crate::components::Components::dhis2_needs_connecting
    pub dhis2_needs_connecting: bool,
    /// Whether chap-core is one this deployment does not run, recorded with
    /// `--chap-core-url` or `chaps components enable chap-core --url`.
    pub chap_core_elsewhere: bool,
    /// Whether chap-core's container started moments ago or reports its
    /// healthcheck as still starting, which makes an API that does not answer
    /// `starting` rather than `down`. Filled in by the caller, which has docker.
    pub api_starting: bool,
    /// The URL of the DHIS2 recorded with `chaps dhis2 use`, which has no row
    /// of its own: chaps does not run it, and asking it would need its
    /// credentials. `chaps dhis2 show` is the command that does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dhis2_external: Option<String>,
    /// Containers of this deployment that are failing, with the lines of their
    /// logs that say why.
    ///
    /// Filled in by the caller, which is the half that has docker: a container
    /// that is unhealthy is why the API is not answering, and the reason is in
    /// its log rather than anywhere this probe can reach.
    pub unhealthy: Vec<crate::diagnose::Unhealthy>,
    /// Whose chap-core answered on this deployment's port while this
    /// deployment's own was not running, as the sentence that says so.
    ///
    /// Two deployments made with the same ports take turns on them, and the
    /// one that is up answers for both. Its health and its registry say
    /// nothing about this deployment, so [`StatusReport::api`] is reported
    /// down and this names the deployment that did answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_elsewhere: Option<String>,
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
    /// chap-core answered and refused the API token: it is up, and nothing it
    /// guards - the service registry among it - could be read. Kept apart from
    /// [`ApiHealth::Down`] because the cure is the token, not the container,
    /// and "down" would send an operator to logs that say it started fine.
    Rejected {
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

    /// Whether this row is something to do about: anything that is not `up`.
    ///
    /// A component that is not running is as much a problem as a model that is
    /// not registered, and it is read the same way - by a script polling
    /// `chaps status` for a deployment that has stopped being what it should
    /// be. This is the predicate that exit code is made of, and nothing else:
    /// the STATE cell is coloured from the three states directly, because
    /// `starting` is amber and `not running` is red while both are failures.
    pub fn is_problem(self) -> bool {
        matches!(self, ComponentState::Starting | ComponentState::NotRunning)
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
    /// Where this component answers its health endpoint: its own host port
    /// plus the path. `None` for an instance that publishes no host port,
    /// which is one that cannot be asked from out here at all.
    ///
    /// Derived from the address rather than from a request, so it says the same
    /// thing whether the instance is running or not: a consumer keying off it
    /// must not watch it appear and disappear with the container.
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
    /// For a model added with `chaps models add`: its model id and the source
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
///
/// `own_api` says `api_url` is this deployment's own port and docker was asked
/// which of its containers run, so an answer there while its `chap` container
/// is not running can be recognised as another deployment's.
pub fn status(
    project: &Project,
    api_url: &str,
    timeout: Duration,
    running: &BTreeSet<String>,
    token: Option<&str>,
    own_api: bool,
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
    // A chap-core elsewhere is asked exactly like this deployment's own: its
    // API answers health, the registry and the version the same way.
    let chap_core = project.state.components.has_chap_core_api();
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
            Err(Failure::Unauthorized) => ApiHealth::Rejected {
                error: token_rejected(&base, HEALTH_PATH, token.is_some()),
            },
            Err(Failure::Other(error)) => ApiHealth::Down { error },
        }
    };

    // This deployment's chap-core cannot answer with its container stopped,
    // so whatever did is another deployment on the same port, and nothing it
    // says - health, version, registry - is about this one.
    let mut api_elsewhere = None;
    if own_api
        && project.state.components.chap_core.enabled
        && project.state.components.chap_core_external.is_none()
        && !running.contains(crate::compose::API_SERVICE)
        && matches!(api, ApiHealth::Up { .. })
    {
        // Which deployment it is takes docker, which is the caller's half:
        // [`name_elsewhere`] puts the name in.
        let line = answered_elsewhere(&base, project.api_port_in_effect().0, None);
        api = ApiHealth::Down {
            error: line.clone(),
        };
        api_elsewhere = Some(line);
    }

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
                Err(_) => Some(ApiHealth::Down {
                    error: services_are_not_chap_core(
                        &base,
                        &body_description(&answer.content_type, &answer.body),
                    ),
                }),
            },
            // The registry is not an open path, so a 401 here is the one place
            // a wrong token usually shows up: `/health` answered happily a
            // moment ago, so this is chap-core, up, refusing the token.
            Err(Failure::Unauthorized) => Some(ApiHealth::Rejected {
                error: token_rejected(&base, SERVICES_PATH, token.is_some()),
            }),
            Err(Failure::Other(error)) => Some(ApiHealth::Down {
                error: services_are_not_chap_core(&base, &error),
            }),
        };
        if let Some(wrong) = wrong {
            api = wrong;
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
    let models = if chap_core {
        let mut rows = model_rows(&enabled_models(project), &registered, running, now());
        mark_unreachable(&mut rows, &registered, &|id| match get(
            &agent,
            &base,
            &proxied_health_path(id),
            token,
        ) {
            Err(Failure::Other(error))
                if error.starts_with("HTTP 5") || !error.starts_with("HTTP ") =>
            {
                Some(error)
            }
            _ => None,
        });
        rows
    } else {
        // Nothing registers anywhere, so each model is asked itself, on the
        // host port a model without chap-core always publishes.
        standalone_model_rows(&enabled_models(project), running, &|port| {
            get(
                &agent,
                &format!("http://localhost:{port}"),
                MODEL_HEALTH_PATH,
                None,
            )
            .is_ok()
        })
    };
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
        dhis2_needs_connecting: project.state.components.dhis2_needs_connecting(),
        chap_core_elsewhere: project.state.components.chap_core_external.is_some(),
        api_starting: false,
        dhis2_external: project
            .state
            .components
            .dhis2_external
            .as_ref()
            .map(|external| external.url.clone()),
        unhealthy: Vec::new(),
        api_elsewhere,
    }
}

/// Put the name of the deployment that answered into a report whose API
/// answered from elsewhere, when another deployment on this machine publishes
/// that port. `holder` is that deployment, as [`crate::ports::other_deployments`]
/// finds it.
pub fn name_elsewhere(report: &mut StatusReport, holder: Option<&crate::ports::Deployment>) {
    if report.api_elsewhere.is_none() || holder.is_none() {
        return;
    }
    let line = answered_elsewhere(&report.api_url, report.api_port, holder);
    report.api = ApiHealth::Down {
        error: line.clone(),
    };
    report.api_elsewhere = Some(line);
}

/// The sentence for an API that answered while this deployment's chap-core
/// was not running: whose it is, when another deployment on this machine
/// publishes the port, and the two ways to put this one there instead.
pub fn answered_elsewhere(
    base: &str,
    port: u16,
    holder: Option<&crate::ports::Deployment>,
) -> String {
    match holder {
        Some(other) => format!(
            "this deployment's chap-core is not running; {base} is {name} ({dir}) answering on \
             the same port; stop it with `chaps -C {dir} down`, or run `chaps up --replace` here",
            name = other.name(),
            dir = other.dir.display(),
        ),
        None => format!(
            "this deployment's chap-core is not running; something else answers on port {port}, \
             and `chaps up` names it"
        ),
    }
}

/// One row per enabled component other than chap-core.
///
/// OCS and DHIS2 are asked over HTTP, because each publishes a host port and an
/// endpoint of its own that answers without credentials; the object store
/// publishes nothing by default, so the only thing that can be said about it
/// from out here is whether its container is up. A component with no container
/// at all is `not running` rather than down: there is nothing wrong with a
/// deployment that has not been started.
///
/// Nothing is asked of a component whose container is not running. `chaps
/// status` is the command run most often, and an instance that is not there
/// would cost it a timeout to say what the container already said.
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
        let url = components.ocs_url();
        let up = running.contains(crate::compose::OCS_SERVICE);
        let state = match (&url, up) {
            // The same reasoning gates the request on the container: with
            // nothing running there is no answer to wait for, only a timeout to
            // spend on a port this deployment has nobody on - and whatever else
            // is holding it would answer in its place, which is how a stopped
            // OCS came to be reported `up` on a machine running a second
            // deployment's OCS on the default 9000.
            (_, false) => ComponentState::NotRunning,
            (Some(url), true) => component_state(get(agent, url, HEALTH_PATH, None).is_ok(), up),
            (None, true) => ComponentState::Up,
        };
        // What the instance holds is only asked for when it has just
        // answered: a second request to an instance that is down would spend
        // another timeout to learn the same thing.
        let datasets = url
            .as_deref()
            .filter(|_| state == ComponentState::Up)
            .and_then(ocs_datasets);
        rows.push(ComponentStatus {
            name: crate::compose::OCS_SERVICE.to_string(),
            state,
            reach: components.ocs_reach(),
            // The address, not the answer: the field says where this instance
            // answers, and that is as true of one that is switched off.
            health_url: url.map(|url| format!("{url}{HEALTH_PATH}")),
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
    if components.dhis2.enabled {
        // Judged by its container first and only then asked anything, exactly
        // as OCS is - and then asked, for a reason OCS does not have.
        //
        // A DHIS2 container can report healthy while the instance is entirely
        // broken. A Spring context that failed to come up - an unreadable
        // `dhis2/dhis.conf`, a PostgreSQL extension the image cannot find, a
        // Flyway checksum that does not match the database - leaves Tomcat
        // running and serving pages, and every `/api/*` request then answers
        // 404. So a 200 from `/api/ping` is the only honest evidence that this
        // instance works, and a container that is up while `/api/ping` does not
        // answer is `starting` at best.
        //
        // Nothing beyond that is asked, because nothing more can honestly be
        // had: what is worth knowing past up or down is the version the
        // instance is running, and `/api/system/info` answers that only to a
        // session, which this CLI holds no credentials for. Its 401 would prove
        // the API layer is alive, which is what a 404 disproves - and
        // `/api/ping` has established exactly that already, for one request.
        let url = components
            .port_of(crate::components::Component::Dhis2)
            .map(|port| format!("http://localhost:{port}"));
        let up = running.contains(crate::compose::DHIS2_SERVICE);
        let state = match (&url, up) {
            // The same reasoning that gates the OCS request on its container:
            // with nothing running there is no answer to wait for, only a
            // timeout to spend on a port this deployment has nobody on - and
            // whatever else holds it would answer in its place.
            (_, false) => ComponentState::NotRunning,
            (Some(url), true) => {
                component_state(get(agent, url, DHIS2_PING_PATH, None).is_ok(), up)
            }
            (None, true) => ComponentState::Up,
        };
        rows.push(ComponentStatus {
            name: crate::compose::DHIS2_SERVICE.to_string(),
            state,
            reach: components.dhis2_reach(),
            // The address, not the answer, as the OCS row reports it: the field
            // says where this instance answers, and that is as true of one that
            // is switched off.
            health_url: url.map(|url| format!("{url}{DHIS2_PING_PATH}")),
            // None of the three is a DHIS2 fact, and each is reported on the
            // same terms it is for the object store: absent, whatever the
            // container is doing, rather than a field that comes and goes.
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

/// The closing lines of a deployment without chap-core: the models, when it
/// has any, then the components, when it has any. Each names what to run.
pub fn standalone_closing_lines(
    models: &[ModelStatus],
    components: &[ComponentStatus],
) -> Vec<String> {
    let mut lines = Vec::new();
    if !models.is_empty() {
        let total = models.len();
        let noun = if total == 1 { "model" } else { "models" };
        let stopped = models
            .iter()
            .filter(|m| m.state == ModelState::NotRunning)
            .count();
        let silent = models
            .iter()
            .filter(|m| m.state == ModelState::RunningNotAnswering)
            .count();
        lines.push(if stopped == total {
            NOTHING_RUNNING.to_string()
        } else if stopped > 0 {
            let verb = if stopped == 1 { "is" } else { "are" };
            format!("{stopped} of {total} {noun} {verb} not running; start them with `chaps up`")
        } else if let Some(first) = models
            .iter()
            .find(|m| m.state == ModelState::RunningNotAnswering)
        {
            let verb = if silent == 1 { "is" } else { "are" };
            format!(
                "{silent} of {total} {noun} {verb} running and not answering on /health; a model \
                 that just started answers in a minute, so run `chaps status` again, or read \
                 `chaps logs {}`",
                first.id
            )
        } else if total == 1 {
            "1 model up, answering on its own host port; `chaps models test --all` checks it \
             can run"
                .to_string()
        } else {
            format!(
                "all {total} models up, each answering on its own host port; `chaps models test \
                 --all` checks they can run"
            )
        });
    }
    if !components.is_empty() || models.is_empty() {
        let line = components_closing_line(components);
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    lines
}

/// What a deployment with nothing running at all is told, when chap-core is not
/// one of its components.
///
/// The mirror of `chaps status`'s `CHAP is not running`: there is no CHAP here
/// to be running or not, only the components the deployment is made of, so the
/// line names the deployment rather than a product it does not contain.
pub const NOTHING_RUNNING: &str = "nothing in this deployment is running; start it with `chaps up`";

/// The one line the component rows add up to, for a deployment chap-core is not
/// a component of.
///
/// [`closing_line`] cannot answer for one: it counts models, such a deployment
/// can have none, and the line it gives for none names `chaps models enable`,
/// which is refused there. The components are the whole of the deployment, so
/// they are the whole of its verdict.
///
/// A component that is not running is what the reader has to do something
/// about, so it is what the line counts and `chaps up` is what it names - the
/// same rows [`ComponentState::is_problem`] makes the exit code out of.
pub fn components_closing_line(rows: &[ComponentStatus]) -> String {
    let total = rows.len();
    if total == 0 {
        return EMPTY.to_string();
    }
    let down = rows
        .iter()
        .filter(|row| row.state == ComponentState::NotRunning)
        .count();
    if down == total {
        return NOTHING_RUNNING.to_string();
    }
    let noun = if total == 1 {
        "component"
    } else {
        "components"
    };
    if down > 0 {
        let verb = if down == 1 { "is" } else { "are" };
        let them = if down == 1 { "it" } else { "them" };
        return format!(
            "{down} of {total} {noun} {verb} not running; start {them} with `chaps up`"
        );
    }
    let starting = rows
        .iter()
        .filter(|row| row.state == ComponentState::Starting)
        .count();
    if starting > 0 {
        let verb = if starting == 1 { "is" } else { "are" };
        return format!(
            "{starting} of {total} {noun} {verb} still starting; \
             run `chaps status` again in a moment"
        );
    }
    // An instance with a page of its own is what a person opens next; the
    // object store has none.
    let openable: Vec<&str> = rows
        .iter()
        .map(|row| row.name.as_str())
        .filter(|name| *name != crate::compose::S3_SERVICE)
        .collect();
    let opens = match openable.as_slice() {
        [] => String::new(),
        [one] => format!("; `chaps open {one}` opens it"),
        many => format!(
            "; {} open them",
            many.iter()
                .map(|name| format!("`chaps open {name}`"))
                .collect::<Vec<_>>()
                .join(" and ")
        ),
    };
    match rows {
        [only] => format!("{} is up{opens}", only.name),
        [_, _] => format!("both components are up{opens}"),
        _ => format!("all {total} {noun} are up{opens}"),
    }
}

/// What a deployment with nothing in it at all is told, by `chaps status` and
/// by `chaps up`: there is nothing to start, and these are the ways to add
/// something.
pub const EMPTY: &str = "this deployment has no components and no models; add one with \
     `chaps models add URL`, `chaps models enable ID` or `chaps components enable NAME`";

/// Whether anything this deployment declares is not where it should be, which
/// is what a non-zero exit from `chaps status` means.
///
/// One rule for every deployment shape, because the command is a health gate
/// for a script and a script cannot know which shape it is polling. Before
/// this, a component was only ever consulted on a deployment without chap-core,
/// and even there only while it was `starting` - so an `ocs` container that
/// died left `chaps status` exiting 0, which is the one answer a monitor must
/// never get wrong.
///
/// The exit stays silent. The rows have already named what is wrong - the model
/// table which ones did not register, the component lines which one is not up -
/// and an error line on top of that would be the third telling. Only an API
/// that is not answering gets one, because nothing else on the screen says why.
pub fn exit_failure(report: &StatusReport) -> bool {
    let components_failing = report
        .components
        .iter()
        .any(|component| component.state.is_problem());
    match report.api {
        ApiHealth::Down { .. } | ApiHealth::Rejected { .. } => true,
        ApiHealth::Up { .. } => {
            !report.missing.is_empty()
                || components_failing
                || report
                    .models
                    .iter()
                    .any(|m| m.state == ModelState::Unreachable)
        }
        // chap-core is not part of this deployment, so its API not answering is
        // the expected state rather than a failure.
        ApiHealth::Off => components_failing || report.models.iter().any(|m| m.state.is_problem()),
    }
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
        // Registrations from outside - a model run from its checkout - are
        // models chap-core has, so "no models" would be the wrong answer.
        let strangers = rows.len();
        return match strangers {
            0 => "no models enabled; run `chaps models enable ID` to add one".to_string(),
            1 => "no models enabled here; the unmanaged one above registered from outside this \
                  deployment"
                .to_string(),
            n => format!(
                "no models enabled here; the {n} unmanaged above registered from outside this \
                 deployment"
            ),
        };
    }
    let unreachable = mine
        .iter()
        .filter(|r| r.state == ModelState::Unreachable)
        .count();
    let problems = mine.iter().filter(|r| r.state.is_problem()).count() - unreachable;
    let noun = if total == 1 { "model" } else { "models" };
    let verb = |n: usize| if n == 1 { "is" } else { "are" };
    match (problems, unreachable) {
        (0, 0) => {
            // "all" is for more than one; a lone model is simply registered.
            let all = if total == 1 { "" } else { "all " };
            format!("{all}{total} {noun} registered")
        }
        (0, u) => format!(
            "{u} of {total} {noun} {} registered and unreachable from chap-core.",
            verb(u)
        ),
        (p, 0) => format!("{p} of {total} {noun} {} not registered.", verb(p)),
        (p, u) => format!(
            "{p} of {total} {noun} {} not registered, and {u} {} unreachable from chap-core.",
            verb(p),
            verb(u)
        ),
    }
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

/// [`TEST_HINT`] for a deployment with one model.
pub const TEST_HINT_ONE: &str = "run `chaps models test --all` to check it can run";

/// The extra hint for a model that has not registered with a chap-core
/// elsewhere: registering there needs the image to listen on the port it
/// advertises, and an image that ignores `PORT` never gets past servicekit's
/// readiness check.
pub fn external_registration_hints(rows: &[ModelStatus]) -> Vec<String> {
    rows.iter()
        .filter(|row| row.state == ModelState::RunningNotRegistered && !row.young)
        .map(|row| {
            format!(
                "{id}: with a chap-core elsewhere the image has to listen on `PORT`; if \
                 `chaps logs {id}` shows `App never became ready`, it does not, so run it \
                 with chaps' own chap-core (`chaps components enable chap-core`)",
                id = row.id
            )
        })
        .collect()
}

/// `elsewhere` is the URL of a chap-core this deployment does not run, which
/// changes what an unreachable model most likely means.
pub fn hints(rows: &[ModelStatus], auth: bool, elsewhere: Option<&str>) -> Vec<String> {
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
            ModelState::RunningNotRegistered if row.registered_as.is_some() => {
                let actual = row.registered_as.as_deref().unwrap_or_default();
                Some(match &row.added_from {
                    Some((model, source)) => format!(
                        "{}: its container registered as `{actual}`, the unmanaged row above; \
                         run `chaps models remove {model}`, then `chaps models add {source} \
                         --service-id {actual}`",
                        row.id
                    ),
                    None => format!(
                        "{}: its container registered as `{actual}`, the unmanaged row above; \
                         `chaps models remove` the model and add it again with `--service-id \
                         {actual}`",
                        row.id
                    ),
                })
            }
            // Started moments ago: registering is part of starting, and the
            // restart below would only start the wait over.
            ModelState::RunningNotRegistered if row.young => Some(format!(
                "{}: started under two minutes ago and registers once it is ready; run `chaps \
                 status` again in a minute",
                row.id
            )),
            ModelState::RunningNotRegistered => Some(format!(
                "{}: restart it with `chaps restart --all {}`{registration_key}",
                row.id, row.id
            )),
            ModelState::NotRunning => Some(format!(
                "{}: start CHAP with `chaps up`, then `chaps logs {}`",
                row.id, row.id
            )),
            ModelState::RunningNotAnswering => Some(format!(
                "{}: read `chaps logs {}`; a model still starting answers in a moment",
                row.id, row.id
            )),
            ModelState::Unreachable => Some(unreachable_hint(row, elsewhere)),
            ModelState::Registered | ModelState::Unmanaged | ModelState::Up => None,
        })
        .collect();
    // Nothing to fix is not nothing to do: every model answered its
    // heartbeat, which is as far as `chaps status` can see.
    if hints.is_empty() && !mine.is_empty() {
        let hint = if mine.len() == 1 {
            TEST_HINT_ONE
        } else {
            TEST_HINT
        };
        return vec![hint.to_string()];
    }
    hints
}

/// The hint for a model chap-core has registered and cannot reach.
///
/// With a chap-core elsewhere the cause is nearly always the address: models
/// register as `localhost:<port>` by default, which is this machine for a
/// chap-core running as a process here and the chap-core container itself for
/// one running in Docker. With chaps' own chap-core both sit on the compose
/// network, so the model's log is where the answer is.
fn unreachable_hint(row: &ModelStatus, elsewhere: Option<&str>) -> String {
    let (url, answer) = row
        .unreachable
        .as_ref()
        .map(|u| (u.registered_url.as_str(), u.answer.as_str()))
        .unwrap_or_default();
    match elsewhere {
        Some(api) => format!(
            "{id}: chap-core cannot reach it at {url} ({answer}); if your chap-core runs in a \
             container, run `chaps components enable chap-core --url {api} --models-host \
             host.docker.internal`, then `chaps up`",
            id = row.id
        ),
        None => format!(
            "{id}: chap-core cannot reach it at {url} ({answer}); read `chaps logs {id}` and \
             `chaps logs chap`",
            id = row.id
        ),
    }
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
        .user_agent(concat!("chaps/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

/// `GET /v2/services`. The envelope is required (see [`parse_services`]);
/// fields the report does not use - `registered_at`, everything chap-core
/// adds later - are ignored.
#[derive(Debug, Deserialize)]
struct ServicesBody {
    #[expect(dead_code, reason = "required in the envelope, never read")]
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
mod tests;
