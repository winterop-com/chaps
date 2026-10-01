//! Host port allocation for model overlays.
//!
//! Model services publish no host port by default, so the allocator is only
//! asked for one when someone said `--port`: `chaps models enable X --port N`,
//! `chaps models expose X`, or `--port auto`. A port has to be free twice
//! over - unclaimed by any compose file in the directory, and with nothing
//! listening on it - which is why every hand-out takes a probe.

use crate::error::{ChapError, PortHolder, Result};
use crate::project::Project;
use serde_yaml_ng::Value;
use std::collections::BTreeSet;
use std::path::Path;

/// Hands out free host ports inside a range, never reusing a claimed one.
#[derive(Debug, Clone)]
pub struct PortAllocator {
    used: BTreeSet<u16>,
    lo: u16,
    hi: u16,
}

impl PortAllocator {
    /// Seed an allocator with the ports already in use.
    pub fn new(range: (u16, u16), used: impl IntoIterator<Item = u16>) -> Self {
        PortAllocator {
            used: used.into_iter().collect(),
            lo: range.0,
            hi: range.1,
        }
    }

    /// Host ports published by any `compose*.yml` in `dir`.
    ///
    /// Overlays this CLI did not write count too: the point is to avoid
    /// handing out a port some hand-written file already publishes. A file
    /// that does not parse is skipped with a warning rather than failing the
    /// command, since it may be unrelated to the deployment.
    pub fn scan_compose_dir(dir: &Path) -> Result<BTreeSet<u16>> {
        let mut ports = BTreeSet::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ports),
            Err(e) => return Err(anyhow::anyhow!("reading {}: {e}", dir.display())),
        };
        let mut files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_compose_file(p))
            .collect();
        files.sort();

        for path in files {
            let body = match std::fs::read_to_string(&path) {
                Ok(body) => body,
                Err(e) => {
                    eprintln!("warning: skipping {}: {e}", path.display());
                    continue;
                }
            };
            let doc: Value = match serde_yaml_ng::from_str(&body) {
                Ok(doc) => doc,
                Err(e) => {
                    eprintln!("warning: skipping {}: not valid YAML: {e}", path.display());
                    continue;
                }
            };
            ports.extend(host_ports(&doc));
        }
        Ok(ports)
    }

    /// Take the lowest port in the range that no compose file claims and that
    /// `busy` says nothing is listening on.
    pub fn allocate(&mut self, busy: &dyn Fn(u16) -> bool) -> Result<u16> {
        for port in self.lo..=self.hi {
            if self.used.contains(&port) || busy(port) {
                continue;
            }
            self.used.insert(port);
            return Ok(port);
        }
        Err(anyhow::anyhow!(
            "no free host port left in {}-{} (checked the compose files and this machine); \
             free one or widen port_range in .chaps/project.yaml",
            self.lo,
            self.hi
        ))
    }

    /// Reserve a specific port.
    ///
    /// Errors with [`crate::error::ChapError::PortOutOfRange`] outside the
    /// range and [`crate::error::ChapError::PortInUse`] when a compose file
    /// already publishes it or `busy` finds a listener on it - the message
    /// says which, because the two are fixed in different places.
    pub fn claim(&mut self, p: u16, busy: &dyn Fn(u16) -> bool) -> Result<()> {
        if p < self.lo || p > self.hi {
            return Err(ChapError::PortOutOfRange {
                port: p,
                lo: self.lo,
                hi: self.hi,
            }
            .into());
        }
        if self.used.contains(&p) {
            return Err(ChapError::PortInUse {
                port: p,
                holder: PortHolder::ComposeFile,
            }
            .into());
        }
        if busy(p) {
            return Err(ChapError::PortInUse {
                port: p,
                holder: PortHolder::Host,
            }
            .into());
        }
        self.used.insert(p);
        Ok(())
    }

    /// Mark a port as taken without range or conflict checks.
    ///
    /// Used when replaying a port a project already recorded: it may sit
    /// outside the current range because the range was narrowed later, and
    /// re-enabling a model must not fail over its own port.
    pub fn reserve(&mut self, p: u16) {
        self.used.insert(p);
    }

    /// Whether a port is already taken.
    #[cfg(test)]
    pub fn is_used(&self, p: u16) -> bool {
        self.used.contains(&p)
    }

    /// The ports currently considered taken.
    #[cfg(test)]
    pub fn used(&self) -> &BTreeSet<u16> {
        &self.used
    }

    /// The inclusive range ports are handed out from.
    #[cfg(test)]
    pub fn range(&self) -> (u16, u16) {
        (self.lo, self.hi)
    }
}

/// A [`PortAllocator`] seeded with every host port this project records and
/// every one a `compose*.yml` in its directory publishes, minus `freed`.
///
/// `freed` is for the ports of services the caller is about to rewrite or
/// remove: a model keeping or re-claiming its own port is not a conflict.
pub fn allocator_for(project: &Project, freed: &BTreeSet<u16>) -> Result<PortAllocator> {
    let mut used: BTreeSet<u16> = project.used_ports();
    used.extend(PortAllocator::scan_compose_dir(&project.dir)?);
    for port in freed {
        used.remove(port);
    }
    Ok(PortAllocator::new(project.state.port_range, used))
}

/// Every `(service, host port)` pair the named compose files publish, in file
/// then service order.
///
/// Unlike [`PortAllocator::scan_compose_dir`] this reads only the files it is
/// given - the ones this project actually hands to `docker compose -f` - and
/// keeps the service names, which is what the `chaps up` preflight needs to
/// say who wanted a port. A file that is missing or does not parse contributes
/// nothing; `sync` and the allocator already report on those.
pub fn published_ports(dir: &Path, files: &[String]) -> Vec<(String, u16)> {
    let mut out = Vec::new();
    for name in files {
        let Ok(body) = std::fs::read_to_string(dir.join(name)) else {
            continue;
        };
        let Ok(doc) = serde_yaml_ng::from_str::<Value>(&body) else {
            continue;
        };
        out.extend(service_host_ports(&doc));
    }
    out
}

/// `compose.yml`, `compose.marketplace.yml`, `compose.<service>.yml`, ...
fn is_compose_file(path: &Path) -> bool {
    match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name.starts_with("compose") && name.ends_with(".yml"),
        None => false,
    }
}

/// The value behind a YAML tag, so compose's own `!override` and `!reset`
/// merge directives do not hide the list they carry.
fn untagged(value: &Value) -> &Value {
    match value {
        Value::Tagged(tagged) => &tagged.value,
        other => other,
    }
}

/// Host ports from every `services.*.ports[]` entry of one compose document.
fn host_ports(doc: &Value) -> BTreeSet<u16> {
    service_host_ports(doc)
        .into_iter()
        .map(|(_, port)| port)
        .collect()
}

/// The same, keeping the service each port belongs to.
fn service_host_ports(doc: &Value) -> Vec<(String, u16)> {
    let mut out = Vec::new();
    let Some(services) = untagged(doc).get("services").map(untagged) else {
        return out;
    };
    let Some(services) = services.as_mapping() else {
        return out;
    };
    for (name, service) in services {
        let Some(entries) = untagged(service).get("ports").map(untagged) else {
            continue;
        };
        let Some(entries) = entries.as_sequence() else {
            continue;
        };
        let name = name.as_str().unwrap_or_default().to_string();
        for entry in entries {
            let entry = untagged(entry);
            let port = if let Some(text) = entry.as_str() {
                host_port_of_short_form(text)
            } else if let Some(published) = entry.get("published") {
                // Long form: published may be a number or a string.
                match published.as_u64() {
                    Some(n) => u16::try_from(n).ok(),
                    None => published.as_str().and_then(|t| t.parse::<u16>().ok()),
                }
            } else {
                None
            };
            if let Some(port) = port {
                out.push((name.clone(), port));
            }
        }
    }
    out
}

/// The host side of `"8000"`, `"5002:8000"`, `"5002:8000/tcp"` or
/// `"127.0.0.1:5002:8000"`.
///
/// A bare container port publishes on an ephemeral host port, and a range
/// (`"5000-5010:8000"`) is not a single number; both yield `None` rather than
/// a wrong guess.
fn host_port_of_short_form(entry: &str) -> Option<u16> {
    let entry = entry.split('/').next().unwrap_or(entry);
    let parts: Vec<&str> = entry.split(':').collect();
    let host = match parts.len() {
        // "8000": container port only, no host port published.
        1 => return None,
        // "5002:8000".
        2 => parts[0],
        // "127.0.0.1:5002:8000".
        3 => parts[1],
        _ => return None,
    };
    host.parse().ok()
}

#[cfg(test)]
mod tests;
