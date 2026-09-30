//! Fixtures the doctor tests of more than one area share.

use super::*;

/// A finished report, so the rendering and the counting can be driven
/// without anything having been probed.
pub(super) fn report(checks: Vec<Check>, project: Option<&str>) -> Report {
    Report {
        summary: summary(&checks),
        checks,
        project: project.map(PathBuf::from),
    }
}

/// What `docker version` prints when it worked.
pub(super) fn done(ok: bool, stdout: &str, stderr: &str) -> Outcome {
    Outcome::Done {
        ok,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    }
}

/// The facts of a deployment that has its credentials and no plugins: the
/// quiet case, where the line says nothing beyond the config file.
pub(super) fn facts(config: Option<&str>) -> OcsFacts<'_> {
    OcsFacts {
        config,
        credentials: true,
        plugins: None,
        datasets: None,
        data_bytes: None,
    }
}

/// [`components_verdict`] for the OCS half of it, whose deployments have no
/// DHIS2 to have facts about.
pub(super) fn ocs_verdict(
    components: &Components,
    facts: &OcsFacts,
) -> (Status, String, Option<String>) {
    components_verdict(components, facts, &Dhis2Facts::default())
}

/// The volumes of a deployment, as `docker volume ls` reports them.
pub(super) fn volumes(names: &[(&str, u64)]) -> Vec<(String, Option<u64>)> {
    names
        .iter()
        .map(|(name, at)| (name.to_string(), Some(*at)))
        .collect()
}

/// The claims a two-model deployment would make.
pub(super) fn claim(service: &str, port: u16) -> PortClaim {
    PortClaim {
        service: service.to_string(),
        port,
    }
}
