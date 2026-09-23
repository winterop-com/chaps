//! Host port allocation for model overlays.
//!
//! Owned by agent B.
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

    /// Take the lowest port in the range that no compose file claims and that
    /// `busy` says nothing is listening on.
    ///
    /// Owned by agent B.
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
    ///
    /// Owned by agent B.
    pub fn claim(&mut self, p: u16, busy: &dyn Fn(u16) -> bool) -> Result<()> {
        if p < self.lo || p > self.hi {
            return Err(ChapError::PortOutOfRange(p).into());
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
mod tests {
    use super::*;
    use crate::project::DEFAULT_PORT_RANGE;

    /// Nothing is listening on anything.
    fn all_free(_: u16) -> bool {
        false
    }

    #[test]
    fn allocate_hands_out_the_lowest_free_port() {
        let mut alloc = PortAllocator::new((5001, 5999), [5001, 5002, 5004]);
        assert_eq!(alloc.allocate(&all_free).unwrap(), 5003);
        assert_eq!(alloc.allocate(&all_free).unwrap(), 5005);
        assert_eq!(alloc.allocate(&all_free).unwrap(), 5006);
        assert!(alloc.used().contains(&5003));
        assert_eq!(alloc.range(), (5001, 5999));
    }

    #[test]
    fn allocate_skips_a_port_something_is_listening_on() {
        // 5001 is free in every compose file and still unusable.
        let busy = |port: u16| (5001..=5002).contains(&port);
        let mut alloc = PortAllocator::new((5001, 5999), []);
        assert_eq!(alloc.allocate(&busy).unwrap(), 5003);
        assert!(
            !alloc.is_used(5001),
            "a port the host holds is not ours to record"
        );
    }

    #[test]
    fn allocate_runs_out_at_the_top_of_the_range() {
        let mut alloc = PortAllocator::new((5001, 5002), []);
        assert_eq!(alloc.allocate(&all_free).unwrap(), 5001);
        assert_eq!(alloc.allocate(&all_free).unwrap(), 5002);
        let err = alloc
            .allocate(&all_free)
            .expect_err("the range is exhausted");
        assert!(err.to_string().contains("5001-5002"));

        // A range that is entirely busy on the host runs out the same way.
        let mut alloc = PortAllocator::new((5001, 5002), []);
        let err = alloc
            .allocate(&|_| true)
            .expect_err("the whole range is listening");
        assert!(err.to_string().contains("this machine"));
    }

    #[test]
    fn claim_rejects_a_taken_port_and_one_outside_the_range() {
        let mut alloc = PortAllocator::new(DEFAULT_PORT_RANGE, [5002]);
        alloc.claim(5005, &all_free).unwrap();
        assert!(alloc.is_used(5005));

        let err = alloc.claim(5002, &all_free).expect_err("5002 is taken");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::PortInUse {
                port: 5002,
                holder: PortHolder::ComposeFile
            })
        ));
        let err = alloc.claim(5005, &all_free).expect_err("just claimed");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::PortInUse {
                port: 5005,
                holder: PortHolder::ComposeFile
            })
        ));

        for out in [80, 5000, 6000] {
            let err = alloc.claim(out, &all_free).expect_err("outside the range");
            assert!(
                matches!(err.downcast_ref::<ChapError>(), Some(ChapError::PortOutOfRange(p)) if *p == out),
                "port {out}"
            );
        }
    }

    #[test]
    fn claim_says_when_the_host_is_the_one_holding_the_port() {
        let mut alloc = PortAllocator::new(DEFAULT_PORT_RANGE, []);
        let err = alloc
            .claim(5010, &|port| port == 5010)
            .expect_err("something is listening on 5010");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::PortInUse {
                port: 5010,
                holder: PortHolder::Host
            })
        ));
        assert!(
            err.to_string().contains("something is listening"),
            "{err:#}"
        );
        assert!(!alloc.is_used(5010), "a rejected claim records nothing");
    }

    #[test]
    fn reserve_ignores_the_range() {
        // A port a project recorded before its range was narrowed is still
        // taken, and re-recording it must not fail over the range.
        let mut alloc = PortAllocator::new((5001, 5999), []);
        alloc.reserve(9090);
        assert!(alloc.is_used(9090));
        alloc.reserve(5001);
        assert_eq!(alloc.allocate(&all_free).unwrap(), 5002);
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
    fn scan_compose_dir_sees_nothing_in_an_internal_only_overlay() {
        // The golden overlay publishes no host port at all: it only exposes
        // 8000 on the compose network, and its volume init service publishes
        // nothing either.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("compose.chapkit-ewars-model.yml"),
            crate::compose::render::normalize_newlines(include_str!(
                "../../tests/fixtures/compose.chapkit-ewars-model.yml"
            )),
        )
        .unwrap();
        assert!(
            PortAllocator::scan_compose_dir(dir.path())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn scan_compose_dir_on_a_missing_directory_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let ports = PortAllocator::scan_compose_dir(&dir.path().join("nope")).unwrap();
        assert!(ports.is_empty());
    }

    #[test]
    fn a_compose_override_tag_does_not_hide_its_port_list() {
        // `ports: !override` is the only way to replace a mapping across -f
        // files, and compose.chaps.yml uses it; the scanner has to see through
        // the tag rather than skip the file.
        let doc: Value = serde_yaml_ng::from_str(
            "services:\n  chap:\n    ports: !override\n      - \"8010:8000\"\n",
        )
        .unwrap();
        assert_eq!(service_host_ports(&doc), vec![("chap".to_string(), 8010)]);

        // A published port that is a compose variable is no number, so it is
        // left out rather than guessed at.
        let doc: Value = serde_yaml_ng::from_str(
            "services:\n  chap:\n    ports: !override\n      - \"${CHAP_API_PORT:-8000}:8000\"\n",
        )
        .unwrap();
        assert!(service_host_ports(&doc).is_empty());
    }

    #[test]
    fn published_ports_reads_only_the_files_it_is_given() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("compose.yml"),
            "services:\n  chap:\n    ports:\n      - \"8000:8000\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("compose.model.yml"),
            concat!(
                "services:\n",
                "  model:\n",
                "    expose:\n      - \"8000\"\n",
                "    ports:\n      - \"5001:8000\"\n",
                "  model-init:\n",
                "    image: busybox\n",
            ),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("compose.stranger.yml"),
            "services:\n  stranger:\n    ports:\n      - \"5999:8000\"\n",
        )
        .unwrap();

        let files = vec![
            "compose.yml".to_string(),
            "compose.model.yml".to_string(),
            "compose.gone.yml".to_string(),
        ];
        assert_eq!(
            published_ports(dir.path(), &files),
            vec![("chap".to_string(), 8000), ("model".to_string(), 5001)],
            "a missing file is skipped and an unlisted one is never read"
        );
        assert!(published_ports(dir.path(), &[]).is_empty());
    }

    #[test]
    fn allocator_for_seeds_from_the_state_and_the_directory() {
        use crate::project::{EnabledModel, ProjectState};
        use std::collections::BTreeMap;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("compose.mine.yml"),
            "services:\n  mine:\n    ports:\n      - \"5002:8000\"\n",
        )
        .unwrap();
        let model = EnabledModel {
            service_id: "m".into(),
            image: "ghcr.io/x".into(),
            image_tag: "sha-1111111".into(),
            version: "1.0.0".into(),
            channel: None,
            host_port: Some(5001),
            data_dir: "/app/data".into(),
            user: "chapkit:chapkit".into(),
            platform: None,
            compose_file: "compose.m.yml".into(),
        };
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState {
                models: BTreeMap::from([("m".to_string(), model)]),
                ..ProjectState::default()
            },
        };

        let mut alloc = allocator_for(&project, &BTreeSet::new()).unwrap();
        assert_eq!(alloc.allocate(&all_free).unwrap(), 5003);

        // Freeing the model's own port makes it available again, which is how
        // a model keeps its port across a rewrite.
        let mut alloc = allocator_for(&project, &BTreeSet::from([5001])).unwrap();
        assert_eq!(alloc.allocate(&all_free).unwrap(), 5001);
    }
}
