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
//!
//! A port nothing is listening on can still be spoken for: a deployment that
//! is down holds no socket, and the collision only shows at the `chaps up`
//! that finds the other one already there. [`other_deployments`] finds the
//! ones that can be found without a registry, so `chaps init` can say so while
//! the port is still easy to change.

use crate::components::Component;
use crate::project::{API_PORT_ENV_VAR, Project};
use std::collections::BTreeSet;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};

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
/// chap-core's port comes from `.env` or `.chaps/project.yaml` rather than
/// from the files: `compose.chaps.yml` publishes it as
/// `${CHAP_API_PORT:-<port>}`, which only Docker expands, so reading the files
/// would give the recorded default and not the port the stack will actually
/// ask for. [`Project::api_port_in_effect`] is what Docker will resolve that
/// to. Everything else is read out of the files this project renders, so a
/// model with no host port contributes nothing.
pub fn claims(project: &Project) -> Vec<PortClaim> {
    let mut claims = Vec::new();
    if project.state.components.chap_core.enabled {
        claims.push(PortClaim {
            service: crate::compose::API_SERVICE.to_string(),
            port: project.effective_api_port(),
        });
    }
    // The components are read from `.chaps/components.yaml` rather than from
    // the rendered files, so a port is checked even before the first sync.
    // The same claim coming back out of the files below is deduplicated.
    for (component, service) in [
        (Component::Ocs, crate::compose::OCS_SERVICE),
        (Component::S3, crate::compose::S3_SERVICE),
    ] {
        if let Some(port) = project.state.components.port_of(component) {
            claims.push(PortClaim {
                service: service.to_string(),
                port,
            });
        }
    }
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
    format!(
        "port {port} is already in use on this machine (needed by {service}); free it, or {}",
        ways_out(claim, suggestion),
        port = claim.port,
        service = claim.service,
    )
}

/// Where to move this deployment's port to, for the service that wants it.
///
/// Shared by [`busy_line`] and [`claimed_line`] so the two warnings `init` can
/// print about one port end the same way.
fn ways_out(claim: &PortClaim, suggestion: Option<u16>) -> String {
    if claim.service == crate::compose::API_SERVICE {
        let free = match suggestion {
            Some(port) => port.to_string(),
            None => "<free>".to_string(),
        };
        return format!(
            "run `chaps init --api-port {free} --force` here / \
             set {API_PORT_ENV_VAR}={free} in .env"
        );
    }
    // A component publishes its port from `.chaps/components.yaml`, so the
    // way to move it is the command that wrote it there. The port is left as
    // a placeholder: the caller's suggestion is the one free above the *first*
    // conflict, which for a component is not always its own.
    if let Ok(component) = Component::from_name(&claim.service)
        && component.takes_port()
    {
        return format!(
            "run `chaps components enable {name} --port <free>`",
            name = component.name()
        );
    }
    format!(
        "run `chaps models unexpose {service}` (the model stays \
         reachable through chap-core) / `chaps models expose {service} --port auto`",
        service = claim.service
    )
}

/// How many other deployments one [`claimed_line`] names before it counts the
/// rest.
const NAMED_HOLDERS: usize = 3;

/// One line of guidance for a port no listener holds, but another chaps
/// deployment on this machine already publishes.
///
/// Not the same problem as a busy port: nothing is wrong today, and the
/// collision only surfaces at the second `chaps up`. So the first way out is
/// to keep the port and live with the two deployments taking turns.
pub fn claimed_line(claim: &PortClaim, holders: &[&Deployment], suggestion: Option<u16>) -> String {
    let named: Vec<String> = holders
        .iter()
        .take(NAMED_HOLDERS)
        .map(|held| format!("{} ({})", held.name(), held.dir.display()))
        .collect();
    let mut who = named.join(", ");
    if holders.len() > named.len() {
        who.push_str(&format!(" and {} more", holders.len() - named.len()));
    }
    let (verb, clash) = if holders.len() == 1 {
        ("is", "both cannot be up at once")
    } else {
        ("are", "they cannot all be up at once")
    };
    format!(
        "port {port} is also used by {who}, which {verb} not running; {clash}. Keep it, or {}",
        ways_out(claim, suggestion),
        port = claim.port,
    )
}

/// Another chaps deployment on this machine: where it is, and the host ports
/// it would publish were it up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployment {
    pub dir: PathBuf,
    pub claims: Vec<PortClaim>,
}

impl Deployment {
    /// What this deployment goes by: its directory name, which is what
    /// `chaps init` was pointed at.
    pub fn name(&self) -> String {
        self.dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.dir.display().to_string())
    }

    /// Whether this deployment publishes `port`.
    pub fn holds(&self, port: u16) -> bool {
        self.claims.iter().any(|claim| claim.port == port)
    }
}

/// Every other chaps deployment on this machine that can be found without
/// anyone keeping a registry, given the directory one is about to be written
/// in.
///
/// Two cheap searches, because a deployment is a directory and nothing else
/// records where they are: the directories beside `dir`, which is where
/// `chaps init a && chaps init b` puts them, and the compose projects docker
/// remembers, which covers the ones that have been started at least once from
/// anywhere. A deployment created somewhere else and never started is in
/// neither, and is the case this cannot see.
///
/// `compose_ls` hands back the stdout of `docker compose ls -a --format json`,
/// or `None` when docker is not there to ask; it is a parameter so the tests
/// never need a daemon. Best-effort throughout: an unreadable parent, an
/// unparseable `project.yaml` or no docker at all each contribute nothing
/// rather than failing the command that asked.
pub fn other_deployments(dir: &Path, compose_ls: &dyn Fn() -> Option<String>) -> Vec<Deployment> {
    let mut dirs = sibling_dirs(dir);
    if let Some(text) = compose_ls() {
        dirs.extend(crate::docker::compose_ls_dirs(&text));
    }
    // The deployment being written is not another deployment - `--force` over
    // one that is already there included, where its own old files are on disk
    // and docker may well remember it.
    let mut seen = BTreeSet::from([identity(dir)]);
    let mut found = Vec::new();
    for path in dirs {
        // A sibling can be docker-known as well, and is one deployment either
        // way.
        if !seen.insert(identity(&path)) {
            continue;
        }
        let Ok(project) = Project::load(&path) else {
            continue;
        };
        found.push(Deployment {
            claims: claims(&project),
            dir: path,
        });
    }
    found
}

/// The directories next to `dir` that hold a `.chaps/project.yaml`, in name
/// order. `dir` itself is never one of them.
fn sibling_dirs(dir: &Path) -> Vec<PathBuf> {
    let Some(parent) = dir.parent() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path != dir && Project::exists(path))
        .collect();
    dirs.sort();
    dirs
}

/// A path in the one spelling two of them can be compared in.
///
/// `canonicalize` resolves the symlinks that make `/tmp` and `/private/tmp`
/// the same directory on macOS, but it needs the path to exist - and the
/// directory `init` is about to write does not yet. So the parent is resolved
/// and the name put back on, which is enough for the two searches and the new
/// deployment to agree on which directory is which.
fn identity(path: &Path) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return real;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(real) = parent.canonicalize()
    {
        return real.join(name);
    }
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
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

    /// The preflight has to reserve the port the stack will really publish,
    /// which is the one `.env` names: checking the recorded port instead lets
    /// `chaps up` sail past a conflict and hand it to Docker.
    #[test]
    fn the_api_claim_follows_the_env_override() {
        let (dir, project) = project();
        std::fs::write(dir.path().join(".env"), "CHAP_API_PORT=18000\n").unwrap();
        assert!(claims(&project).contains(&PortClaim {
            service: "chap".into(),
            port: 18000
        }));
        assert!(
            !claims(&project).iter().any(|c| c.port == 8123),
            "the recorded port is not published any more"
        );

        // A commented line is not an override, and the recorded port stands.
        std::fs::write(dir.path().join(".env"), "# CHAP_API_PORT=18000\n").unwrap();
        assert!(claims(&project).contains(&PortClaim {
            service: "chap".into(),
            port: 8123
        }));
    }

    #[test]
    fn a_component_claims_its_port_before_the_first_sync() {
        let (_dir, mut project) = project();
        project.state.components.ocs.enabled = true;
        project.state.components.ocs.port = Some(9010);
        project.state.components.s3.enabled = true;

        let found = claims(&project);
        assert!(found.contains(&PortClaim {
            service: "ocs".into(),
            port: 9010
        }));
        assert!(
            !found.iter().any(|c| c.service == "s3"),
            "the store publishes nothing by default"
        );

        project.state.components.s3.port = Some(9002);
        assert!(claims(&project).contains(&PortClaim {
            service: "s3".into(),
            port: 9002
        }));

        // With chap-core off there is no API port to claim at all.
        project.state.components.chap_core.enabled = false;
        let found = claims(&project);
        assert!(!found.iter().any(|c| c.service == "chap"), "{found:?}");
    }

    #[test]
    fn a_component_port_is_claimed_once_however_many_files_publish_it() {
        let (dir, mut project) = project();
        project.state.components.ocs.enabled = true;
        project.state.components.ocs.port = Some(9010);
        std::fs::write(
            dir.path().join("compose.ocs.yml"),
            "services:\n  ocs:\n    ports:\n      - \"9010:9000\"\n",
        )
        .unwrap();
        project.state.rendered_files.push("compose.ocs.yml".into());
        let ocs: Vec<PortClaim> = claims(&project)
            .into_iter()
            .filter(|c| c.service == "ocs")
            .collect();
        assert_eq!(ocs.len(), 1, "{ocs:?}");
        assert_eq!(ocs[0].port, 9010);
    }

    #[test]
    fn a_component_message_points_at_the_command_that_moves_its_port() {
        let claim = PortClaim {
            service: "ocs".into(),
            port: 9000,
        };
        let line = busy_line(&claim, Some(9001));
        assert!(line.contains("(needed by ocs)"), "{line}");
        assert!(
            line.contains("`chaps components enable ocs --port <free>`"),
            "{line}"
        );
        assert!(!line.contains("--api-port"), "{line}");
        assert!(!line.contains("models unexpose"), "{line}");
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

    /// A deployment directory with nothing in it but the state files that
    /// make it one: `api_port` recorded, and an optional `.env` line over it.
    fn deployment(parent: &Path, name: &str, recorded: u16, env: Option<u16>) -> PathBuf {
        let dir = parent.join(name);
        std::fs::create_dir_all(dir.join(crate::project::CHAPS_DIR)).unwrap();
        let state = ProjectState {
            api_port: recorded,
            ..ProjectState::default()
        };
        std::fs::write(
            dir.join(crate::project::CHAPS_DIR)
                .join(crate::project::PROJECT_FILE),
            serde_yaml_ng::to_string(&state).unwrap(),
        )
        .unwrap();
        if let Some(port) = env {
            std::fs::write(
                dir.join(crate::project::ENV_FILE),
                format!("POSTGRES_DB=chap_core\n{API_PORT_ENV_VAR}={port}\n"),
            )
            .unwrap();
        }
        dir
    }

    /// No docker to ask.
    fn no_docker() -> Option<String> {
        None
    }

    #[test]
    fn the_deployments_beside_a_new_one_are_found_and_read_like_any_project() {
        let home = tempfile::tempdir().unwrap();
        // One claims the port in `.chaps/project.yaml`, the other overrides a
        // different recorded port from `.env` - which is the port that would
        // really be published, so it is the one that has to be found.
        let recorded = deployment(home.path(), "hello1", 8000, None);
        let overridden = deployment(home.path(), "hello2", 9999, Some(8000));
        // Not a deployment, and not a directory: neither is ours.
        std::fs::create_dir_all(home.path().join("notes")).unwrap();
        std::fs::write(home.path().join("README"), "hi").unwrap();

        let found = other_deployments(&home.path().join("hello3"), &no_docker);
        let dirs: Vec<&PathBuf> = found.iter().map(|d| &d.dir).collect();
        assert_eq!(dirs, vec![&recorded, &overridden], "{found:?}");
        assert!(found.iter().all(|d| d.holds(8000)), "{found:?}");
        assert_eq!(found[0].name(), "hello1");
        assert!(!found[1].holds(9999), "the recorded port is not published");
    }

    #[test]
    fn a_deployment_never_counts_as_another_one_of_itself() {
        let home = tempfile::tempdir().unwrap();
        let one = deployment(home.path(), "hello1", 8000, None);
        // `init --force` over a deployment that is already there: its own old
        // files are on disk, and docker remembers it too.
        let json = format!(
            r#"[{{"Name":"hello1","Status":"exited(0)","ConfigFiles":"{}/compose.yml,{}/compose.chaps.yml"}}]"#,
            one.display(),
            one.display()
        );
        let docker = || Some(json.clone());
        assert!(
            other_deployments(&one, &docker).is_empty(),
            "the deployment being written is not another deployment"
        );
    }

    #[test]
    fn docker_known_deployments_are_found_and_deduplicated() {
        let home = tempfile::tempdir().unwrap();
        // Beside the new directory, so both searches find it.
        let sibling = deployment(home.path(), "hello1", 8000, None);
        // Somewhere else entirely: only docker knows about this one.
        let elsewhere = tempfile::tempdir().unwrap();
        let far = deployment(elsewhere.path(), "hello2", 8000, None);
        let json = format!(
            r#"[
              {{"Name":"hello1","ConfigFiles":"{sibling}/compose.yml,{sibling}/compose.chaps.yml"}},
              {{"Name":"hello2","ConfigFiles":"{far}/compose.chaps.yml"}},
              {{"Name":"something-else","ConfigFiles":"{home}/other/docker-compose.yml"}},
              {{"Name":"deleted","ConfigFiles":"{home}/gone/compose.chaps.yml"}}
            ]"#,
            sibling = sibling.display(),
            far = far.display(),
            home = home.path().display(),
        );
        let found = other_deployments(&home.path().join("hello3"), &|| Some(json.clone()));
        let names: Vec<String> = found.iter().map(Deployment::name).collect();
        assert_eq!(
            names,
            vec!["hello1", "hello2"],
            "a sibling docker also knows is one deployment; a compose project \
             with no compose.chaps.yml is not ours, and a directory that is gone \
             has nothing left to warn about"
        );
    }

    #[test]
    fn a_directory_that_is_not_a_project_contributes_nothing() {
        let home = tempfile::tempdir().unwrap();
        let empty = home.path().join("hello1");
        std::fs::create_dir_all(empty.join(crate::project::CHAPS_DIR)).unwrap();
        std::fs::write(
            empty
                .join(crate::project::CHAPS_DIR)
                .join(crate::project::PROJECT_FILE),
            "nonsense: [\n",
        )
        .unwrap();
        assert!(
            other_deployments(&home.path().join("hello2"), &no_docker).is_empty(),
            "an unreadable project.yaml is not a claim on anything"
        );
    }

    #[test]
    fn a_claimed_port_names_the_deployment_and_the_way_out() {
        let claim = PortClaim {
            service: "chap".into(),
            port: 8000,
        };
        let hello1 = Deployment {
            dir: PathBuf::from("/Users/x/t/hello1"),
            claims: vec![claim.clone()],
        };
        let line = claimed_line(&claim, &[&hello1], Some(8001));
        assert_eq!(
            line,
            "port 8000 is also used by hello1 (/Users/x/t/hello1), which is not running; \
             both cannot be up at once. Keep it, or run `chaps init --api-port 8001 --force` \
             here / set CHAP_API_PORT=8001 in .env"
        );
        // The same ways out as the live-listener line, so the two read alike.
        assert!(busy_line(&claim, Some(8001)).ends_with(
            "run `chaps init --api-port 8001 --force` here / set CHAP_API_PORT=8001 in .env"
        ));
    }

    #[test]
    fn several_deployments_are_named_three_at_a_time_and_then_counted() {
        let claim = PortClaim {
            service: "ocs".into(),
            port: 9000,
        };
        let held: Vec<Deployment> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|name| Deployment {
                dir: PathBuf::from("/t").join(name),
                claims: vec![claim.clone()],
            })
            .collect();

        let two: Vec<&Deployment> = held.iter().take(2).collect();
        let line = claimed_line(&claim, &two, Some(9001));
        assert!(
            line.starts_with(
                "port 9000 is also used by a (/t/a), b (/t/b), which are not \
                 running; they cannot all be up at once."
            ),
            "{line}"
        );
        // A component's port moves with the command that set it, and the
        // placeholder stays for the reason busy_line keeps it.
        assert!(
            line.ends_with("Keep it, or run `chaps components enable ocs --port <free>`"),
            "{line}"
        );

        let all: Vec<&Deployment> = held.iter().collect();
        let line = claimed_line(&claim, &all, None);
        assert!(
            line.contains("a (/t/a), b (/t/b), c (/t/c) and 2 more, which are not running"),
            "{line}"
        );
    }
}
