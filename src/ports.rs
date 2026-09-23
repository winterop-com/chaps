//! Host ports: is anything listening right now, and what `chaps up` says when
//! the answer is yes.
//!
//! A deployment publishes very few host ports - chap-core's API, plus the
//! model services someone asked for explicitly - but every one of them is a
//! way for `docker compose up` to fail several seconds in, with an error that
//! names a container rather than a port. Probing first turns that into one
//! line of guidance before anything is started.
//!
//! The probe is a bind, not a connect: a bind that fails with `EADDRINUSE`
//! means a listener holds the port, and it needs no crate beyond `std`. A
//! listening socket has no `TIME_WAIT`, so the socket we drop leaves nothing
//! behind.
//!
//! It takes four binds rather than one because `std` sets `SO_REUSEADDR` on
//! Unix, and on BSD (macOS included) that lets a wildcard bind succeed next to
//! a loopback-only listener and the other way round. Only the *same* address
//! reliably collides, so every address the stack could be published on is
//! tried in turn.

use crate::project::{API_PORT_ENV_VAR, Project};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};

/// A host port the stack wants, and the compose service that publishes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortClaim {
    /// Compose service name, as the rendered files spell it.
    pub service: String,
    pub port: u16,
}

/// Whether something on this machine is listening on `port`.
///
/// This is the real probe; the functions that take a `busy` callback exist so
/// tests can answer the question without binding anything.
pub fn is_busy(port: u16) -> bool {
    // Port 0 means "any free port" to the kernel, so a bind always succeeds
    // and the answer would be meaningless.
    if port == 0 {
        return false;
    }
    [
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
        SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
        SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)),
        SocketAddr::from((Ipv6Addr::LOCALHOST, port)),
    ]
    .into_iter()
    .any(taken)
}

/// Whether one address is already bound.
///
/// Only `EADDRINUSE` counts. A host with no IPv6 stack, or a privileged port
/// this process may not bind, fails for a reason that says nothing about a
/// conflict - and Docker, which does the real binding, is not this process.
/// The listener we open is dropped before returning, so the next address is
/// probed against the machine rather than against us.
fn taken(addr: SocketAddr) -> bool {
    match TcpListener::bind(addr) {
        Ok(_) => false,
        Err(e) => e.kind() == std::io::ErrorKind::AddrInUse,
    }
}

/// The lowest port in `lo..=hi` that `busy` says nothing is listening on.
pub fn first_free(lo: u16, hi: u16, busy: &dyn Fn(u16) -> bool) -> Option<u16> {
    (lo..=hi).find(|port| !busy(*port))
}

/// Every `(service, host port)` pair `chaps up` is about to ask Docker to
/// publish.
///
/// chap-core's port comes from `.chaps/project.yaml` rather than from the
/// files: `compose.chaps.yml` publishes it as `${CHAP_API_PORT:-<port>}`,
/// which only Docker expands. Everything else is read out of the files this
/// project renders, so a model with no host port contributes nothing.
pub fn claims(project: &Project) -> Vec<PortClaim> {
    let mut claims = vec![PortClaim {
        service: crate::compose::API_SERVICE.to_string(),
        port: project.state.api_port,
    }];
    let mut files: Vec<String> = project.state.compose_files.clone();
    for name in &project.state.rendered_files {
        if !files.contains(name) {
            files.push(name.clone());
        }
    }
    for (service, port) in crate::compose::ports::published_ports(&project.dir, &files) {
        // compose.chaps.yml overrides chap's own mapping, and the api_port
        // above is what that override resolves to.
        if service == crate::compose::API_SERVICE {
            continue;
        }
        let claim = PortClaim { service, port };
        if !claims.contains(&claim) {
            claims.push(claim);
        }
    }
    claims
}

/// The claims `busy` reports as taken, leaving out services `running` says
/// this project already has up: those ports are ours, and `docker compose up`
/// on a running stack is a no-op rather than a conflict.
pub fn busy_claims(
    claims: &[PortClaim],
    running: &std::collections::BTreeSet<String>,
    busy: &dyn Fn(u16) -> bool,
) -> Vec<PortClaim> {
    claims
        .iter()
        .filter(|claim| !running.contains(&claim.service))
        .filter(|claim| busy(claim.port))
        .cloned()
        .collect()
}

/// One line of guidance for a port that is taken: what wanted it, and the two
/// ways out.
///
/// `suggestion` is a port nothing is listening on, for the API message; a
/// `None` leaves the placeholder in place rather than naming a port that may
/// itself be busy.
pub fn busy_line(claim: &PortClaim, suggestion: Option<u16>) -> String {
    let head = format!(
        "port {} is already in use on this machine (needed by {})",
        claim.port, claim.service
    );
    if claim.service == crate::compose::API_SERVICE {
        let free = match suggestion {
            Some(port) => port.to_string(),
            None => "<free>".to_string(),
        };
        return format!(
            "{head}; free it, or run `chaps init --api-port {free} --force` here / \
             set {API_PORT_ENV_VAR}={free} in .env"
        );
    }
    format!(
        "{head}; free it, or run `chaps models unexpose {service}` (the model stays \
         reachable through chap-core) / `chaps models expose {service} --port auto`",
        service = claim.service
    )
}

/// The whole message `chaps up` fails with when a port it needs is taken.
pub fn preflight_message(busy: &[PortClaim], suggestion: Option<u16>) -> String {
    let mut out = format!(
        "{} host port{} CHAP needs {} already in use; nothing was started\n",
        busy.len(),
        if busy.len() == 1 { "" } else { "s" },
        if busy.len() == 1 { "is" } else { "are" },
    );
    for claim in busy {
        out.push_str(&format!("  {}\n", busy_line(claim, suggestion)));
    }
    out.push_str("  or run `chaps up --no-preflight` to hand the conflict to Docker");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{EnabledModel, ProjectState};
    use std::collections::{BTreeMap, BTreeSet};

    /// A listener on a port the kernel picked, so the test never fights
    /// another process over a fixed number.
    fn bound(addr: Ipv4Addr) -> (TcpListener, u16) {
        let listener = TcpListener::bind((addr, 0)).expect("a free port");
        let port = listener.local_addr().unwrap().port();
        (listener, port)
    }

    #[test]
    fn a_loopback_only_listener_is_busy() {
        // The case a single wildcard bind misses on macOS, because std sets
        // SO_REUSEADDR and BSD then allows the wildcard next to it.
        let (listener, port) = bound(Ipv4Addr::LOCALHOST);
        assert!(is_busy(port), "port {port} has a listener on it");
        drop(listener);
    }

    #[test]
    fn a_wildcard_listener_is_busy_too() {
        let (listener, port) = bound(Ipv4Addr::UNSPECIFIED);
        assert!(is_busy(port), "port {port} is published on every address");
        drop(listener);
    }

    #[test]
    fn a_port_nothing_holds_is_free() {
        let (listener, port) = bound(Ipv4Addr::LOCALHOST);
        drop(listener);
        let answer = is_busy(port);
        // Tests run in parallel, and the released port is back in the
        // ephemeral range, so another test in this run may have taken it
        // between the drop and the probe. Taking it ourselves is what says the
        // probe was asked about a port nobody held.
        match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
            Ok(_) => assert!(!answer, "nothing was listening on port {port}"),
            Err(_) => eprintln!("skipping: port {port} was claimed by another test"),
        }
    }

    #[test]
    fn port_zero_is_never_reported_busy() {
        // Binding port 0 always succeeds, so the question has no answer; the
        // honest one is "no conflict".
        assert!(!is_busy(0));
    }

    #[test]
    fn first_free_walks_upwards() {
        let busy = |port: u16| (8000..8003).contains(&port);
        assert_eq!(first_free(8000, 8100, &busy), Some(8003));
        assert_eq!(first_free(8004, 8100, &busy), Some(8004));
        assert_eq!(first_free(8000, 8002, &busy), None);
    }

    fn enabled(service_id: &str, port: Option<u16>) -> EnabledModel {
        EnabledModel {
            service_id: service_id.to_string(),
            image: "ghcr.io/chap-models/x".into(),
            image_tag: "sha-1111111".into(),
            version: "1.0.0".into(),
            channel: None,
            host_port: port,
            data_dir: "/app/data".into(),
            user: "chapkit:chapkit".into(),
            platform: None,
            compose_file: format!("compose.{service_id}.yml"),
        }
    }

    /// A project directory with one published and one internal overlay.
    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("compose.yml"),
            "services:\n  chap:\n    ports:\n      - \"8000:8000\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("compose.chaps.yml"),
            "services:\n  chap:\n    ports: !override\n      - \"${CHAP_API_PORT:-8123}:8000\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("compose.loud.yml"),
            "services:\n  loud:\n    expose:\n      - \"8000\"\n    ports:\n      - \"5001:8000\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("compose.quiet.yml"),
            "services:\n  quiet:\n    expose:\n      - \"8000\"\n",
        )
        .unwrap();
        // Never in the -f list, so never probed.
        std::fs::write(
            dir.path().join("compose.stranger.yml"),
            "services:\n  stranger:\n    ports:\n      - \"5999:8000\"\n",
        )
        .unwrap();
        let state = ProjectState {
            api_port: 8123,
            rendered_files: vec![
                "compose.yml".into(),
                "compose.chaps.yml".into(),
                "compose.loud.yml".into(),
                "compose.quiet.yml".into(),
                "compose.marketplace.yml".into(),
            ],
            models: BTreeMap::from([
                ("loud".to_string(), enabled("loud", Some(5001))),
                ("quiet".to_string(), enabled("quiet", None)),
            ]),
            ..ProjectState::default()
        };
        let project = Project {
            dir: dir.path().to_path_buf(),
            state,
        };
        (dir, project)
    }

    #[test]
    fn claims_are_the_api_port_plus_the_published_overlays() {
        let (_dir, project) = project();
        assert_eq!(
            claims(&project),
            vec![
                PortClaim {
                    service: "chap".into(),
                    port: 8123
                },
                PortClaim {
                    service: "loud".into(),
                    port: 5001
                },
            ],
            "an internal model publishes nothing, and a stray compose file is not ours"
        );
    }

    #[test]
    fn a_running_service_keeps_its_own_port() {
        let (_dir, project) = project();
        let claims = claims(&project);
        let all_busy = |_: u16| true;

        assert_eq!(
            busy_claims(&claims, &BTreeSet::new(), &all_busy).len(),
            2,
            "nothing of ours is up, so both conflicts are real"
        );
        let running = BTreeSet::from(["chap".to_string()]);
        let busy = busy_claims(&claims, &running, &all_busy);
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].service, "loud");

        let nothing_busy = |_: u16| false;
        assert!(busy_claims(&claims, &BTreeSet::new(), &nothing_busy).is_empty());
    }

    #[test]
    fn the_api_message_names_a_free_port_when_one_was_found() {
        let claim = PortClaim {
            service: "chap".into(),
            port: 8000,
        };
        let line = busy_line(&claim, Some(8010));
        assert!(line.starts_with(
            "port 8000 is already in use on this machine (needed by chap); free it, or run"
        ));
        assert!(line.contains("`chaps init --api-port 8010 --force`"));
        assert!(line.contains("set CHAP_API_PORT=8010 in .env"));

        // Without a suggestion the placeholder stays: naming a port we have
        // not probed would be a guess.
        assert!(busy_line(&claim, None).contains("--api-port <free>"));
    }

    #[test]
    fn a_model_message_points_at_expose_and_unexpose() {
        let claim = PortClaim {
            service: "chapkit-ewars-model".into(),
            port: 5001,
        };
        let line = busy_line(&claim, Some(8010));
        assert!(line.contains("(needed by chapkit-ewars-model)"));
        assert!(line.contains("`chaps models unexpose chapkit-ewars-model`"));
        assert!(line.contains("`chaps models expose chapkit-ewars-model --port auto`"));
        assert!(!line.contains("--api-port"), "{line}");
    }

    #[test]
    fn the_preflight_message_counts_and_offers_the_escape_hatch() {
        let one = preflight_message(
            &[PortClaim {
                service: "chap".into(),
                port: 8000,
            }],
            Some(8001),
        );
        assert!(one.starts_with("1 host port CHAP needs is already in use; nothing was started\n"));
        assert!(one.ends_with("or run `chaps up --no-preflight` to hand the conflict to Docker"));

        let two = preflight_message(
            &[
                PortClaim {
                    service: "chap".into(),
                    port: 8000,
                },
                PortClaim {
                    service: "loud".into(),
                    port: 5001,
                },
            ],
            None,
        );
        assert!(two.starts_with("2 host ports CHAP needs are already in use"));
        assert_eq!(two.lines().count(), 4);
        assert!(two.contains("\n  port 5001 is already in use"));
    }
}
