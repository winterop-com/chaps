//! `varde up --wait`: hold the command until chap-core and every model answer.

use crate::commands::Ctx;
use crate::docker;
use crate::project::Project;
use crate::status::{ApiHealth, ModelState, StatusReport, status};
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
                ready: matches!(state, ModelState::Registered | ModelState::Up),
                url: match model.host_port {
                    Some(port) => format!("http://localhost:{port}"),
                    None => project.proxy_url(&model.service_id),
                },
            }
        })
        .collect();
    Readiness {
        ready: api_up && models.iter().all(|m| m.ready),
        waited_s: waited.as_secs(),
        api_url: has_api.then(|| report.api_url.clone()),
        api_up,
        models,
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
    out
}
