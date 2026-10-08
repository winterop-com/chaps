//! Host ports: is anything listening right now, and what `varde up` says when
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
//! is down holds no socket, and the collision only shows at the `varde up`
//! that finds the other one already there. [`other_deployments`] finds the
//! ones that can be found without a registry, so `varde init` can say so while
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

/// Every `(service, host port)` pair `varde up` is about to ask Docker to
/// publish.
///
/// chap-core's port comes from `.env` or `.varde/project.yaml` rather than
/// from the files: `compose.varde.yml` publishes it as
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
    // The components are read from `.varde/components.yaml` rather than from
    // the rendered files, so a port is checked even before the first sync.
    // The same claim coming back out of the files below is deduplicated.
    for component in Component::ALL {
        if let (Some(service), Some(port)) = (
            service_of(*component),
            project.state.components.port_of(*component),
        ) {
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
        // compose.varde.yml overrides chap's own mapping, and the api_port
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

/// The compose service that publishes a component's host port.
///
/// `None` for chap-core, whose host port is the API port of a service this CLI
/// does not name for it.
fn service_of(component: Component) -> Option<&'static str> {
    match component {
        Component::Ocs => Some(crate::compose::OCS_SERVICE),
        Component::S3 => Some(crate::compose::S3_SERVICE),
        Component::Dhis2 => Some(crate::compose::DHIS2_SERVICE),
        Component::ChapCore => None,
    }
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
        // `.env` and nothing else: `init` writes an active `CHAP_API_PORT`
        // line even at the default, and compose reads it last, so an
        // `init --api-port N --force` would move `.varde/` and leave the
        // published port where it was.
        return format!("set {API_PORT_ENV_VAR}={free} in `.env`");
    }
    // A component publishes its port from `.varde/components.yaml`, so the
    // way to move it is the command that wrote it there, with the free port
    // the caller found above this claim's own.
    if let Ok(component) = Component::from_name(&claim.service)
        && component.takes_port()
    {
        return format!(
            "run `varde components enable {name} --port {free}`",
            name = component.name(),
            free = suggestion
                .map(|port| port.to_string())
                .unwrap_or_else(|| "<free>".to_string())
        );
    }
    format!(
        "run `varde models unexpose {service}` (the model stays \
         reachable through chap-core) / `varde models expose {service} --port auto`",
        service = claim.service
    )
}

/// How many other deployments one [`claimed_line`] names before it counts the
/// rest.
const NAMED_HOLDERS: usize = 3;

/// One line of guidance for a port no listener holds, but another varde
/// deployment on this machine already publishes.
///
/// Not the same problem as a busy port: nothing is wrong today, and the
/// collision only surfaces at the second `varde up`. So the first way out is
/// to keep the port and run the deployments one at a time.
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
    let verb = if holders.len() == 1 { "is" } else { "are" };
    format!(
        "port {port} is also used by {who}, which {verb} not running; run only one of them \
         at a time, or {}",
        ways_out(claim, suggestion),
        port = claim.port,
    )
}

/// One line about a host port a component is being told to publish, or `None`
/// when the port can be had.
///
/// The decision `varde components enable NAME --port PORT` makes is the same
/// decision `varde init --api-port` makes, so it gets the same two answers:
/// something on this machine is listening on that port now, or another varde
/// deployment beside this one publishes it and the two cannot be up at once.
/// Neither is a refusal - the deployment is not up yet, and the port is still
/// easy to change - so the caller reports the line as a warning and carries on,
/// exactly as `varde init` does.
///
/// Best-effort about docker, like everything else that asks it a question: no
/// docker CLI means no running services and no other deployments, which is the
/// answer that reports the conflict rather than hiding it.
pub fn component_port_warning(
    project: &Project,
    component: Component,
    port: u16,
    busy: &dyn Fn(u16) -> bool,
) -> Option<String> {
    let running = crate::docker::running_services(project);
    let others = other_deployments(&project.dir, &crate::docker::compose_ls_json);
    component_port_line(project, component, port, busy, &running, &others)
}

/// [`component_port_warning`] with the two docker answers handed in, so the
/// tests never need a daemon.
fn component_port_line(
    project: &Project,
    component: Component,
    port: u16,
    busy: &dyn Fn(u16) -> bool,
    running: &BTreeSet<String>,
    others: &[Deployment],
) -> Option<String> {
    // A component with no host port of its own has nothing to collide over,
    // and port 0 is not a port [`is_busy`] can answer about.
    if !component.takes_port() || port == 0 {
        return None;
    }
    let claim = PortClaim {
        service: service_of(component)?.to_string(),
        port,
    };
    // A port one of this deployment's own running services already publishes is
    // ours: `docker compose up` on a service that is already up is a no-op
    // rather than a conflict. The same exemption [`busy_claims`] makes, but by
    // port as well as by service - a component being *moved* onto some other
    // process's port is a conflict however much of this deployment is up.
    if claims(project)
        .iter()
        .any(|held| held.port == port && running.contains(&held.service))
    {
        return None;
    }
    // A port another deployment publishes is as good as taken when suggesting
    // one: moving onto it would trade one collision for another. So is one
    // this deployment already gives to another of its own services.
    let own: Vec<PortClaim> = claims(project)
        .into_iter()
        .filter(|held| held.service != claim.service)
        .collect();
    let taken = |port: u16| {
        busy(port)
            || others.iter().any(|other| other.holds(port))
            || own.iter().any(|held| held.port == port)
    };
    let suggestion = first_free(port.saturating_add(1), u16::MAX, &taken);
    // Another service of this very deployment on the same port: whether or
    // not it is up, the next `varde up` cannot start both.
    if let Some(held) = own.iter().find(|held| held.port == port) {
        return Some(format!(
            "port {port} is also published by {} in this deployment, so `varde up` cannot start \
             both; pick another with `--port {}`",
            held.service,
            suggestion.unwrap_or(port.saturating_add(1))
        ));
    }
    // One line either way, and the listener is the half that is in the way
    // today.
    if busy(port) {
        return Some(busy_line(&claim, suggestion));
    }
    let holders: Vec<&Deployment> = others.iter().filter(|other| other.holds(port)).collect();
    if holders.is_empty() {
        return None;
    }
    Some(claimed_line(&claim, &holders, suggestion))
}

/// Another varde deployment on this machine: where it is, and the host ports
/// it would publish were it up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployment {
    pub dir: PathBuf,
    pub claims: Vec<PortClaim>,
}

impl Deployment {
    /// What this deployment goes by: its directory name, which is what
    /// `varde init` was pointed at.
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

    /// Whether this deployment holds `port` now: a service that publishes it
    /// is in `running`, the services of this deployment that run.
    pub fn holds_now(&self, port: u16, running: &BTreeSet<String>) -> bool {
        self.claims
            .iter()
            .any(|claim| claim.port == port && running.contains(&claim.service))
    }
}

/// Every other varde deployment on this machine that can be found without
/// anyone keeping a registry, given the directory one is about to be written
/// in.
///
/// Two cheap searches, because a deployment is a directory and nothing else
/// records where they are: the directories beside `dir`, which is where
/// `varde init a && varde init b` puts them, and the compose projects docker
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
    let mut seen = BTreeSet::from([real_path(dir)]);
    let mut found = Vec::new();
    for path in dirs {
        // A sibling can be docker-known as well, and is one deployment either
        // way.
        if !seen.insert(real_path(&path)) {
            continue;
        }
        let Ok(project) = Project::load(&path) else {
            continue;
        };
        found.push(Deployment {
            claims: claims(&project),
            // Read from the path that was walked, but named by the resolved
            // one: the two are the same directory, and only one of them is a
            // directory the operator will recognise.
            dir: resolved(&path),
        });
    }
    found
}

/// The directories next to `dir` that hold a `.varde/project.yaml`, in name
/// order. `dir` itself is never one of them.
pub(crate) fn sibling_dirs(dir: &Path) -> Vec<PathBuf> {
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
/// the same directory on macOS, and the short names and the casing that do
/// the same on Windows, but it needs the path to exist - and the directory
/// `init` is about to write does not yet. So the parent is resolved and the
/// name put back on, which is enough for the two searches and the new
/// deployment to agree on which directory is which. What it answers is put
/// back in the spelling the fallback uses, so all three branches agree too.
///
/// It is also the spelling a path is printed in: `chapx/../archives` reads
/// as `archives`. The fallback removes `.` and `..` by their text, because
/// there is nothing on disk to resolve them against.
pub(crate) fn real_path(path: &Path) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return plain(real);
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name())
        && let Ok(real) = parent.canonicalize()
    {
        return plain(real).join(name);
    }
    lexical(&std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// `path` without its `.` parts, and with each `..` taking the part before it.
pub(crate) fn lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(part);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// A canonical path without the verbatim prefix Windows gives one.
///
/// There `canonicalize` answers `\\?\C:\...`, which no other spelling of the
/// same directory equals: not `std::path::absolute`, and not what docker
/// printed either, so a canonicalized sibling would never be the docker-known
/// deployment beside it. The prefix comes off before anything is compared.
/// No path on Unix carries one, where this hands back what it was given.
fn plain(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    let Some(rest) = text.strip_prefix(r"\\?\") else {
        return path;
    };
    // A drive path only: the UNC spelling is the line above, and a device
    // path (`\\?\Volume{...}`) has no plainer spelling to put it in.
    match rest.as_bytes() {
        [drive, b':', ..] if drive.is_ascii_alphabetic() => PathBuf::from(rest),
        _ => path,
    }
}

/// The directory a deployment is named by in a warning: the resolved one.
///
/// Both searches hand back the path they walked, and on Windows that can
/// still be the 8.3 short name a temp or profile directory is reached by,
/// which no operator recognises and no second spelling of the same directory
/// matches. `canonicalize` settles it, in the plain spelling like everything
/// else here; a path it cannot resolve is named as it was found, since a name
/// is better than none.
fn resolved(path: &Path) -> PathBuf {
    match path.canonicalize() {
        Ok(real) => plain(real),
        Err(_) => path.to_path_buf(),
    }
}

/// One busy port, as `varde up` reports it: the claim, a free port above it,
/// and the other varde deployments that publish it too.
pub struct Conflict<'a> {
    pub claim: PortClaim,
    pub suggestion: Option<u16>,
    pub holders: Vec<&'a Deployment>,
}

/// The line for a busy port that another running varde deployment
/// publishes, named with the command that stops it.
pub fn held_line(claim: &PortClaim, holders: &[&Deployment], suggestion: Option<u16>) -> String {
    let named: Vec<String> = holders
        .iter()
        .take(NAMED_HOLDERS)
        .map(|held| format!("{} ({})", held.name(), held.dir.display()))
        .collect();
    let stop = holders
        .first()
        .map(|held| format!("stop it with `varde -C {} down`", held.dir.display()))
        .unwrap_or_default();
    let verb = if holders.len() == 1 {
        "publishes"
    } else {
        "publish"
    };
    format!(
        "port {port} (needed by {service}) is in use, and {who} {verb} it too; {stop}, or move \
         this one: {}",
        ways_out(claim, suggestion),
        port = claim.port,
        service = claim.service,
        who = named.join(", "),
    )
}

/// The whole message `varde up` fails with when a port it needs is taken.
pub fn preflight_message(conflicts: &[Conflict]) -> String {
    let n = conflicts.len();
    let mut out = format!(
        "{n} host port{} this deployment needs {} already in use; nothing was started\n",
        if n == 1 { "" } else { "s" },
        if n == 1 { "is" } else { "are" },
    );
    for conflict in conflicts {
        let line = match conflict.holders.is_empty() {
            true => busy_line(&conflict.claim, conflict.suggestion),
            false => held_line(&conflict.claim, &conflict.holders, conflict.suggestion),
        };
        out.push_str(&format!("  {line}\n"));
    }
    out.push_str("  or run `varde up --no-preflight` to hand the conflict to Docker");
    out
}

#[cfg(test)]
mod tests;
