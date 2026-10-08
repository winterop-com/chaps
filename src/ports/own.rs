//! The host ports this deployment's own running containers publish.

use super::PortClaim;
use crate::docker::Container;
use crate::project::Project;
use std::collections::BTreeMap;

/// The host ports this deployment's own running containers publish, by
/// service.
///
/// A port one of them publishes is ours, not a conflict: `docker compose up`
/// on a service that is already up leaves it where it is. A running service is
/// not exempt for a port it does not publish now, though. A restored
/// `.varde/components.yaml` can move a running DHIS2 from 8781 to 8780, and
/// compose then recreates it on 8780, so 8780 must be free.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnPorts(BTreeMap<String, Vec<u16>>);

impl OwnPorts {
    /// The running containers of a `docker compose ps` answer.
    pub fn of(containers: &[Container]) -> Self {
        let mut own: BTreeMap<String, Vec<u16>> = BTreeMap::new();
        for container in containers.iter().filter(|c| c.is_running()) {
            own.entry(container.service.clone())
                .or_default()
                .extend(container.published.iter().copied());
        }
        Self(own)
    }

    /// What docker says about this project now; nothing when docker does not
    /// answer, which reports a conflict rather than hide it.
    pub fn of_project(project: &Project) -> Self {
        crate::docker::running_containers(project)
            .map(|containers| Self::of(&containers))
            .unwrap_or_default()
    }

    /// Whether a running container of this deployment already holds `claim`.
    ///
    /// A container that `ps` lists without any published port (a compose
    /// older than the `Publishers` field) is trusted for the claim of its
    /// service, because there is nothing to compare.
    pub fn holds(&self, claim: &PortClaim) -> bool {
        match self.0.get(&claim.service) {
            None => false,
            Some(ports) if ports.is_empty() => true,
            Some(ports) => ports.contains(&claim.port),
        }
    }

    /// Whether the running container of `service` publishes `port` now.
    pub fn publishes(&self, service: &str, port: u16) -> bool {
        self.0
            .get(service)
            .is_some_and(|ports| ports.contains(&port))
    }

    /// Running services without the ports they publish, for the tests that
    /// only say what is up.
    #[cfg(test)]
    pub fn services(names: &[&str]) -> Self {
        Self(
            names
                .iter()
                .map(|name| (name.to_string(), Vec::new()))
                .collect(),
        )
    }

    /// Running services and the ports they publish.
    #[cfg(test)]
    pub fn with(entries: &[(&str, &[u16])]) -> Self {
        Self(
            entries
                .iter()
                .map(|(name, ports)| (name.to_string(), ports.to_vec()))
                .collect(),
        )
    }
}
