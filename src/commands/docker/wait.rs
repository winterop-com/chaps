//! `varde up --wait`: hold the command until chap-core, every model and every
//! component with a health check answer.

use crate::commands::Ctx;
use crate::components::Component;
use crate::docker;
use crate::project::Project;
use crate::status::{ApiHealth, ComponentState, ModelState, StatusReport, status};
use serde::Serialize;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// How long one round of probes may take per request.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// The pause between two rounds.
const INTERVAL: Duration = Duration::from_secs(2);

/// What `--wait` found when it stopped waiting.
#[derive(Debug, Clone, Serialize)]
pub struct Readiness {
    /// Everything answered before the deadline.
    pub ready: bool,
    /// Seconds spent waiting.
    pub waited_s: u64,
    /// chap-core's API, when this deployment has one.
    pub api_url: Option<String>,
    /// Whether that API answered; `true` when there is none to ask.
    pub api_up: bool,
    pub models: Vec<ModelReadiness>,
    /// The components with a health check (OCS and DHIS2 on a host port).
    /// Empty when the wait is for one model only.
    pub components: Vec<ComponentReadiness>,
}

/// One component with a health check as the last round saw it.
#[derive(Debug, Clone, Serialize)]
pub struct ComponentReadiness {
    /// The component name, which is also its compose service.
    pub name: String,
    /// The `varde status` STATE word.
    pub state: &'static str,
    pub ready: bool,
    /// Where it answers from this machine.
    pub url: String,
}

/// The components `up --wait` waits for: the ones with a health endpoint
/// on a host port, which are OCS and DHIS2. The object store has no health
/// check that can be asked from this machine.
pub fn health_checked(project: &Project) -> Vec<&'static str> {
    let components = &project.state.components;
    let mut out = Vec::new();
    if components.port_of(Component::Ocs).is_some() {
        out.push(crate::compose::OCS_SERVICE);
    }
    if components.port_of(Component::Dhis2).is_some() {
        out.push(crate::compose::DHIS2_SERVICE);
    }
    out
}

/// One enabled model as the last round saw it.
#[derive(Debug, Clone, Serialize)]
pub struct ModelReadiness {
    /// Marketplace id.
    pub id: String,
    pub service_id: String,
    /// The `varde status` STATE word.
    pub state: &'static str,
    pub ready: bool,
    /// Where the model answers from this machine.
    pub url: String,
}

/// Probe until chap-core answers and every model this project enables is
/// registered (or, without chap-core, answers on its own port), or until
/// `timeout` runs out. `only` narrows the models to one service.
pub fn wait_until_ready(
    ctx: &Ctx,
    project: &Project,
    timeout: Duration,
    only: Option<&str>,
) -> Readiness {
    let start = Instant::now();
    let token = crate::api::token_for(Some(&project.dir));
    let url = project.api_url();
    loop {
        let running: BTreeSet<String> = docker::running_containers(project)
            .as_deref()
            .map(docker::running_of)
            .unwrap_or_default();
        let report = status(
            project,
            &url,
            PROBE_TIMEOUT,
            &running,
            token.as_deref(),
            true,
        );
        let readiness = readiness_of(project, &report, start.elapsed(), only);
        // Ctrl-C ends the wait at once: the caller takes back out what it
        // started, and does not wait out the timeout first.
        if readiness.ready || start.elapsed() >= timeout || crate::interrupt::requested() {
            return readiness;
        }
        ctx.out
            .verbose(&format!("waiting: {}", pending(&readiness).join(", ")));
        std::thread::sleep(INTERVAL);
    }
}

/// Read one status report as ready or not.
fn readiness_of(
    project: &Project,
    report: &StatusReport,
    waited: Duration,
    only: Option<&str>,
) -> Readiness {
    let has_api = project.state.components.has_chap_core_api();
    let api_up = !has_api || matches!(report.api, ApiHealth::Up { .. });
    let models: Vec<ModelReadiness> = project
        .state
        .models
        .iter()
        .filter(|(_, model)| only.is_none_or(|service| model.service_id == service))
        .map(|(id, model)| {
            let row = report.models.iter().find(|r| r.id == model.service_id);
            let state = row.map(|r| r.state).unwrap_or(ModelState::NotRunning);
            ModelReadiness {
                id: id.clone(),
                service_id: model.service_id.clone(),
                state: state.label(),
                // Not configured is registered: `up --wait` makes the
                // configured models once everything is ready.
                ready: matches!(
                    state,
                    ModelState::Registered | ModelState::Up | ModelState::NotConfigured
                ),
                url: match model.host_port {
                    Some(port) => format!("http://localhost:{port}"),
                    None => project.proxy_url(&model.service_id),
                },
            }
        })
        .collect();
    // A wait for one model is about that model, not about the rest.
    let checked = match only {
        Some(_) => Vec::new(),
        None => health_checked(project),
    };
    let components: Vec<ComponentReadiness> = checked
        .iter()
        .map(|name| {
            let row = report.components.iter().find(|row| row.name == *name);
            let state = row.map(|r| r.state).unwrap_or(ComponentState::NotRunning);
            ComponentReadiness {
                name: name.to_string(),
                state: state.label(),
                ready: state == ComponentState::Up,
                url: row.map(|r| r.reach.clone()).unwrap_or_default(),
            }
        })
        .collect();
    Readiness {
        ready: api_up && models.iter().all(|m| m.ready) && components.iter().all(|c| c.ready),
        waited_s: waited.as_secs(),
        api_url: has_api.then(|| report.api_url.clone()),
        api_up,
        models,
        components,
    }
}

/// Show the models the configure step just gave configured models to as
/// registered, which is what `varde status` now says of them.
pub fn mark_configured(readiness: &mut Readiness, outcomes: &[crate::configs::sync::ModelOutcome]) {
    for model in readiness.models.iter_mut() {
        let created = outcomes.iter().any(|outcome| {
            outcome.service_id == model.service_id
                && matches!(
                    outcome.outcome,
                    crate::configs::sync::Outcome::Created { .. }
                )
        });
        if created {
            model.state = ModelState::Registered.label();
        }
    }
}

/// What `up --wait` waits for in this deployment, by name: chap-core only
/// when it has one, the components with a health check, and the models only
/// when it has some. `None` when there is nothing to wait for.
pub fn waited_for(has_api: bool, components: &[&str], models: usize) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if has_api {
        parts.push("chap-core".to_string());
    }
    parts.extend(components.iter().map(|name| name.to_string()));
    match models {
        0 => {}
        1 => parts.push("the model".to_string()),
        count => parts.push(format!("the {count} models")),
    }
    match parts.as_slice() {
        [] => None,
        [one] => Some(one.clone()),
        [rest @ .., last] => Some(format!("{} and {last}", rest.join(", "))),
    }
}

/// What is still not answering, by name.
pub fn pending(readiness: &Readiness) -> Vec<String> {
    let mut out = Vec::new();
    if !readiness.api_up {
        out.push("chap-core".to_string());
    }
    out.extend(
        readiness
            .models
            .iter()
            .filter(|m| !m.ready)
            .map(|m| format!("{} ({})", m.service_id, m.state)),
    );
    out.extend(
        readiness
            .components
            .iter()
            .filter(|c| !c.ready)
            .map(|c| format!("{} ({})", c.name, c.state)),
    );
    out
}
