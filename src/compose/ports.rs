//! Host port allocation for model overlays.
//!
//! Owned by agent B.

use crate::error::{ChapError, Result};
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
    ///
    /// Owned by agent B.
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

    /// Take the lowest free port in the range.
    ///
    /// Owned by agent B.
    pub fn allocate(&mut self) -> Result<u16> {
        for port in self.lo..=self.hi {
            if !self.used.contains(&port) {
                self.used.insert(port);
                return Ok(port);
            }
        }
        Err(anyhow::anyhow!(
            "no free host port left in {}-{}; free one or widen port_range in .chaps/project.yaml",
            self.lo,
            self.hi
        ))
    }

    /// Reserve a specific port.
    ///
    /// Errors with [`crate::error::ChapError::PortOutOfRange`] outside the
    /// range and [`crate::error::ChapError::PortInUse`] when already taken.
    ///
    /// Owned by agent B.
    pub fn claim(&mut self, p: u16) -> Result<()> {
        if p < self.lo || p > self.hi {
            return Err(ChapError::PortOutOfRange(p).into());
        }
        if !self.used.insert(p) {
            return Err(ChapError::PortInUse(p).into());
        }
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

    /// Release a port, e.g. because the service holding it is being removed.
    pub fn release(&mut self, p: u16) {
        self.used.remove(&p);
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

/// `compose.yml`, `compose.marketplace.yml`, `compose.<service>.yml`, ...
fn is_compose_file(path: &Path) -> bool {
    match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => name.starts_with("compose") && name.ends_with(".yml"),
        None => false,
    }
}

/// Host ports from every `services.*.ports[]` entry of one compose document.
fn host_ports(doc: &Value) -> BTreeSet<u16> {
    let mut ports = BTreeSet::new();
    let Some(services) = doc.get("services").and_then(Value::as_mapping) else {
        return ports;
    };
    for (_, service) in services {
        let Some(entries) = service.get("ports").and_then(Value::as_sequence) else {
            continue;
        };
        for entry in entries {
            if let Some(text) = entry.as_str() {
                ports.extend(host_port_of_short_form(text));
            } else if let Some(published) = entry.get("published") {
                // Long form: published may be a number or a string.
                if let Some(n) = published.as_u64() {
                    ports.extend(u16::try_from(n).ok());
                } else if let Some(text) = published.as_str() {
                    ports.extend(text.parse::<u16>().ok());
                }
            }
        }
    }
    ports
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
mod tests {
    use super::*;
    use crate::project::DEFAULT_PORT_RANGE;

    #[test]
    fn allocate_hands_out_the_lowest_free_port() {
        let mut alloc = PortAllocator::new((5001, 5999), [5001, 5002, 5004]);
        assert_eq!(alloc.allocate().unwrap(), 5003);
        assert_eq!(alloc.allocate().unwrap(), 5005);
        assert_eq!(alloc.allocate().unwrap(), 5006);
        assert!(alloc.used().contains(&5003));
        assert_eq!(alloc.range(), (5001, 5999));
    }

    #[test]
    fn allocate_runs_out_at_the_top_of_the_range() {
        let mut alloc = PortAllocator::new((5001, 5002), []);
        assert_eq!(alloc.allocate().unwrap(), 5001);
        assert_eq!(alloc.allocate().unwrap(), 5002);
        let err = alloc.allocate().expect_err("the range is exhausted");
        assert!(err.to_string().contains("5001-5002"));
    }

    #[test]
    fn claim_rejects_a_taken_port_and_one_outside_the_range() {
        let mut alloc = PortAllocator::new(DEFAULT_PORT_RANGE, [5002]);
        alloc.claim(5005).unwrap();
        assert!(alloc.is_used(5005));

        let err = alloc.claim(5002).expect_err("5002 is taken");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::PortInUse(5002))
        ));
        let err = alloc.claim(5005).expect_err("just claimed");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::PortInUse(5005))
        ));

        for out in [80, 5000, 6000] {
            let err = alloc.claim(out).expect_err("outside the range");
            assert!(
                matches!(err.downcast_ref::<ChapError>(), Some(ChapError::PortOutOfRange(p)) if *p == out),
                "port {out}"
            );
        }
    }

    #[test]
    fn reserve_and_release_ignore_the_range() {
        let mut alloc = PortAllocator::new((5001, 5999), []);
        alloc.reserve(9090);
        assert!(alloc.is_used(9090));
        alloc.reserve(5001);
        assert_eq!(alloc.allocate().unwrap(), 5002);
        alloc.release(5001);
        assert_eq!(alloc.allocate().unwrap(), 5001);
    }

    #[test]
    fn short_form_entries_yield_the_host_side() {
        assert_eq!(host_port_of_short_form("5002:8000"), Some(5002));
        assert_eq!(host_port_of_short_form("5002:8000/tcp"), Some(5002));
        assert_eq!(host_port_of_short_form("127.0.0.1:5003:8000"), Some(5003));
        assert_eq!(host_port_of_short_form("0.0.0.0:5004:8000/udp"), Some(5004));
        assert_eq!(host_port_of_short_form("8000"), None);
        assert_eq!(host_port_of_short_form("5000-5010:8000"), None);
    }

    #[test]
    fn scan_compose_dir_reads_every_compose_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("compose.yml"),
            concat!(
                "services:\n",
                "  chap:\n",
                "    image: chap\n",
                "    ports:\n",
                "      - \"8000:8000\"\n",
                "  quiet:\n",
                "    image: quiet\n",
            ),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("compose.models.yml"),
            concat!(
                "services:\n",
                "  short:\n",
                "    ports:\n",
                "      - \"5002:8000\"\n",
                "      - \"5003:8000/tcp\"\n",
                "      - \"127.0.0.1:5004:8000\"\n",
                "      - \"8000\"\n",
                "  long:\n",
                "    ports:\n",
                "      - target: 8000\n",
                "        published: \"5005\"\n",
                "        protocol: tcp\n",
                "      - target: 9000\n",
                "        published: 5006\n",
            ),
        )
        .unwrap();
        // Not a compose file: must not be read.
        std::fs::write(
            dir.path().join("notes.yml"),
            "services:\n  x:\n    ports:\n      - \"5111:8000\"\n",
        )
        .unwrap();
        // A compose file that does not parse is skipped, not fatal.
        std::fs::write(dir.path().join("compose.broken.yml"), "services: [\n").unwrap();
        // An empty umbrella parses to null.
        std::fs::write(dir.path().join("compose.marketplace.yml"), "services: {}\n").unwrap();

        let ports = PortAllocator::scan_compose_dir(dir.path()).unwrap();
        assert_eq!(
            ports,
            BTreeSet::from([8000, 5002, 5003, 5004, 5005, 5006]),
            "unexpected scan result"
        );
    }

    #[test]
    fn scan_compose_dir_on_a_missing_directory_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let ports = PortAllocator::scan_compose_dir(&dir.path().join("nope")).unwrap();
        assert!(ports.is_empty());
    }
}
