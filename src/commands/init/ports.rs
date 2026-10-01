//! The host ports the new deployment publishes, and the warnings about the
//! ones already spoken for.

use crate::components::{Component, Components};
use crate::compose::API_SERVICE;

/// What `init` found out about the host ports the new deployment wants, and
/// warned about.
#[derive(Debug, Default)]
pub(super) struct PortWarnings {
    /// Ports something on this machine is listening on right now.
    pub(super) busy: Vec<crate::ports::PortClaim>,
    /// Ports another deployment on this machine already publishes, and nothing
    /// is listening on.
    pub(super) claimed: Vec<crate::ports::PortClaim>,
    /// Every line that was printed, in the order it went out, for `--json`.
    pub(super) lines: Vec<String>,
}

/// The host ports the new deployment is going to publish.
pub(super) fn wanted_claims(
    components: &Components,
    api_port: u16,
) -> Vec<crate::ports::PortClaim> {
    let mut claims = Vec::new();
    if components.chap_core.enabled {
        claims.push(crate::ports::PortClaim {
            service: API_SERVICE.to_string(),
            port: api_port,
        });
    }
    for (component, service) in [
        (Component::Ocs, crate::compose::OCS_SERVICE),
        (Component::S3, crate::compose::S3_SERVICE),
        (Component::Dhis2, crate::compose::DHIS2_SERVICE),
    ] {
        if let Some(port) = components.port_of(component) {
            claims.push(crate::ports::PortClaim {
                service: service.to_string(),
                port,
            });
        }
    }
    claims
}

/// Warn about every host port the new deployment publishes that is already
/// spoken for, and say what to do about each.
///
/// Two ways a port can be spoken for, and one line either way: something is
/// listening on it now, or one of `others` - a deployment that is down, so
/// holding no socket at all - publishes it too. The second is the one nothing
/// else catches until the `chaps up` that finds the other deployment there
/// first.
///
/// Neither is an error. The listener may be a stack this deployment is meant
/// to replace, two deployments on one port is a perfectly good way to take
/// turns, and the directory is worth writing either way.
pub(super) fn warn_about_ports(
    components: &Components,
    api_port: u16,
    busy: &dyn Fn(u16) -> bool,
    others: &[crate::ports::Deployment],
) -> PortWarnings {
    // A port another deployment publishes is as good as taken when suggesting
    // one: moving onto it would trade one collision for another.
    let taken = |port: u16| busy(port) || others.iter().any(|other| other.holds(port));
    let mut found = PortWarnings::default();
    for claim in wanted_claims(components, api_port) {
        // Upwards from the requested port: the next free number is the one
        // least likely to collide with something else the operator has in
        // mind.
        let suggestion = crate::ports::first_free(claim.port.saturating_add(1), u16::MAX, &taken);
        // A port that is both listening and claimed gets one line, and the
        // listener is the half that is in the way today.
        if busy(claim.port) {
            let line = crate::ports::busy_line(&claim, suggestion);
            crate::output::warn(&line);
            found.lines.push(line);
            found.busy.push(claim);
            continue;
        }
        let holders: Vec<&crate::ports::Deployment> = others
            .iter()
            .filter(|other| other.holds(claim.port))
            .collect();
        if holders.is_empty() {
            continue;
        }
        let line = crate::ports::claimed_line(&claim, &holders, suggestion);
        crate::output::warn(&line);
        found.lines.push(line);
        found.claimed.push(claim);
    }
    found
}
