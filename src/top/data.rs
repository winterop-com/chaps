//! What `chaps top` shows: every chaps deployment on the machine as a node,
//! with its services under it.

use crate::docker::{self, Labeled};
use crate::project::Project;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// How a deployment came to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// `chaps init`: a directory of the operator's own.
    Init,
    /// `chaps run`: a group chaps keeps under its data directory.
    Run,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Init => "init",
            Kind::Run => "run",
        }
    }
}

/// One deployment.
#[derive(Debug, Clone, Serialize)]
pub struct Node {
    /// The compose project name.
    pub project: String,
    /// What a person calls it: the directory name, or `run/<group>`.
    pub name: String,
    pub kind: Kind,
    pub group: Option<String>,
    pub dir: Option<PathBuf>,
    pub services: Vec<Service>,
}

impl Node {
    pub fn running(&self) -> usize {
        self.services
            .iter()
            .filter(|s| s.state == "running")
            .count()
    }
}

/// One compose service of a deployment.
#[derive(Debug, Clone, Serialize)]
pub struct Service {
    pub service: String,
    /// `chap-core`, `model`, `ocs`, `s3` or `dhis2`.
    pub role: String,
    /// The marketplace id, for a model.
    pub model: Option<String>,
    /// docker's state: `running`, `exited`, ...; `not running` when there is
    /// no container at all.
    pub state: String,
    pub health: Option<String>,
    pub cpu: Option<String>,
    pub memory: Option<String>,
    pub url: Option<String>,
}

/// Every deployment there is to show, from the containers docker knows and
/// from the deployments on disk that have none running.
///
/// `known` are deployment directories to show even without containers: the
/// one the command was run in, and every `chaps run` group.
pub fn collect(known: &[PathBuf]) -> Vec<Node> {
    build(
        &docker::chaps_containers(),
        &docker::container_usage(),
        known,
        &|dir| Project::load(dir).ok(),
    )
}

/// [`collect`] with docker's answers and the project loader handed in.
pub fn build(
    containers: &[Labeled],
    usage: &BTreeMap<String, docker::Usage>,
    known: &[PathBuf],
    load: &dyn Fn(&std::path::Path) -> Option<Project>,
) -> Vec<Node> {
    let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
    let mut projects: BTreeMap<String, Project> = BTreeMap::new();

    for container in containers {
        let Some(project_name) = container.label(docker::COMPOSE_PROJECT_LABEL) else {
            continue;
        };
        let service = container
            .label(docker::COMPOSE_SERVICE_LABEL)
            .unwrap_or(&container.name)
            .to_string();
        // A one-shot init container that did its job is not something to watch.
        if service.ends_with("-init") && container.state == "exited" {
            continue;
        }
        let dir = container
            .label(docker::COMPOSE_DIR_LABEL)
            .map(PathBuf::from);
        if let Some(dir) = &dir
            && !projects.contains_key(project_name)
            && let Some(project) = load(dir)
        {
            projects.insert(project_name.to_string(), project);
        }
        let node = nodes.entry(project_name.to_string()).or_insert_with(|| {
            let group = container.label(docker::GROUP_LABEL).map(str::to_string);
            let kind = match container.label(docker::KIND_LABEL) {
                Some("run") => Kind::Run,
                _ => Kind::Init,
            };
            Node {
                project: project_name.to_string(),
                name: display_name(kind, group.as_deref(), dir.as_deref(), project_name),
                kind,
                group,
                dir: dir.clone(),
                services: Vec::new(),
            }
        });
        let used = usage.get(&container.name);
        node.services.push(Service {
            service,
            role: container
                .label(docker::ROLE_LABEL)
                .unwrap_or_default()
                .to_string(),
            model: container.label(docker::MODEL_LABEL).map(str::to_string),
            state: container.state.clone(),
            health: container.health().map(str::to_string),
            cpu: used.map(|u| u.cpu.clone()).filter(|c| !c.is_empty()),
            memory: used.map(|u| u.memory.clone()).filter(|m| !m.is_empty()),
            url: None,
        });
    }

    // The deployments asked about by path: shown even with nothing running,
    // and their enabled models listed as not running.
    for dir in known {
        let Some(project) = load(dir) else { continue };
        let Some(name) = project.compose_project_name() else {
            continue;
        };
        let node = nodes.entry(name.clone()).or_insert_with(|| {
            // A directory under the groups directory is a `chaps run` group.
            let group = (dir.parent() == Some(crate::paths::run_groups_dir().as_path()))
                .then(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()))
                .flatten();
            let kind = match group {
                Some(_) => Kind::Run,
                None => Kind::Init,
            };
            Node {
                project: name.clone(),
                name: display_name(kind, group.as_deref(), Some(dir), &name),
                kind,
                group,
                dir: Some(dir.clone()),
                services: Vec::new(),
            }
        });
        for (id, model) in &project.state.models {
            if !node.services.iter().any(|s| s.service == model.service_id) {
                node.services.push(Service {
                    service: model.service_id.clone(),
                    role: "model".to_string(),
                    model: Some(id.clone()),
                    state: "not running".to_string(),
                    health: None,
                    cpu: None,
                    memory: None,
                    url: None,
                });
            }
        }
        projects.entry(name).or_insert(project);
    }

    let mut out: Vec<Node> = nodes.into_values().collect();
    for node in &mut out {
        if let Some(project) = projects.get(&node.project) {
            for service in &mut node.services {
                service.url = url_of(project, service);
                if service.model.is_none() {
                    service.model = project
                        .state
                        .models
                        .iter()
                        .find(|(_, m)| m.service_id == service.service)
                        .map(|(id, _)| id.clone());
                }
            }
        }
        node.services
            .sort_by_key(|s| (role_order(&s.role), s.service.clone()));
    }
    out.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
    out
}

/// What a person calls a deployment.
fn display_name(
    kind: Kind,
    group: Option<&str>,
    dir: Option<&std::path::Path>,
    project: &str,
) -> String {
    match (kind, group) {
        (Kind::Run, Some(group)) => format!("run/{group}"),
        _ => dir
            .and_then(|d| d.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| project.to_string()),
    }
}

/// chap-core first, then the components, then the models.
fn role_order(role: &str) -> u8 {
    match role {
        "chap-core" => 0,
        "dhis2" => 1,
        "ocs" => 2,
        "s3" => 3,
        "model" => 4,
        _ => 5,
    }
}

/// Where a service answers from this machine, when it has a web address.
fn url_of(project: &Project, service: &Service) -> Option<String> {
    use crate::components::Component;
    use crate::open::{Openable, resolve};
    if let Some((id, model)) = project
        .state
        .models
        .iter()
        .find(|(_, m)| m.service_id == service.service)
    {
        return crate::commands::enable::ModelRef::of(id, model, project).url;
    }
    let component = match service.service.as_str() {
        // The API's own address, which is what every client is pointed at,
        // rather than the documentation page `chaps open` lands on.
        crate::compose::API_SERVICE if project.state.components.chap_core.enabled => {
            return Some(project.api_url());
        }
        crate::compose::OCS_SERVICE => Component::Ocs,
        crate::compose::DHIS2_SERVICE => Component::Dhis2,
        _ => return None,
    };
    match resolve(component, &project.state.components, &project.api_url()) {
        Openable::Url { url, .. } => Some(url),
        _ => None,
    }
}
