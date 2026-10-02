//! Named volumes: listing a deployment's, measuring and removing them.

use super::{compose_args, docker_capture, first_stderr_line, trace_command};
use crate::project::Project;
use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::time::Duration;

/// One named volume of a deployment, and when docker created it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Volume {
    pub name: String,
    /// `CreatedAt` as `docker volume inspect` prints it (RFC 3339), empty when
    /// this docker did not say.
    pub created_at: String,
}

/// The names of the volumes docker holds whose names start with `prefix`
/// (`docker volume ls --filter name=<prefix>`).
///
/// Best-effort like every other query here: an empty list when docker could
/// not be asked. The filter is a substring match, so the names are checked
/// again out here - `demo_` must not answer for `olddemo_chap-db`.
pub fn volume_names_with_prefix(prefix: &str) -> Vec<String> {
    let args = vec![
        "volume".to_string(),
        "ls".to_string(),
        "--filter".to_string(),
        format!("name={prefix}"),
        "--format".to_string(),
        "{{.Name}}".to_string(),
    ];
    let Some(text) = docker_capture(&args) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|name| name.starts_with(prefix))
        .map(str::to_string)
        .collect()
}

/// The names of the volumes compose created for the project `name`
/// (`docker volume ls --filter label=com.docker.compose.project=<name>`),
/// whether or not a compose file still declares them: a model stopped
/// earlier took its overlay, and so its volume's only trace, with it.
///
/// Best-effort: an empty list when docker could not be asked.
pub fn volume_names_of_project(name: &str) -> Vec<String> {
    let args = vec![
        "volume".to_string(),
        "ls".to_string(),
        "--filter".to_string(),
        format!("label=com.docker.compose.project={name}"),
        "--format".to_string(),
        "{{.Name}}".to_string(),
    ];
    let Some(text) = docker_capture(&args) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// The same volumes, each with the time docker created it.
pub fn volumes_with_prefix(prefix: &str) -> Vec<Volume> {
    let names = volume_names_with_prefix(prefix);
    if names.is_empty() {
        return Vec::new();
    }
    let created = volume_creation_times(&names);
    names
        .into_iter()
        .map(|name| Volume {
            created_at: created.get(&name).cloned().unwrap_or_default(),
            name,
        })
        .collect()
}

/// When docker created each of these volumes, keyed by name
/// (`docker volume inspect`). One call for the whole list.
fn volume_creation_times(names: &[String]) -> BTreeMap<String, String> {
    let mut args = vec![
        "volume".to_string(),
        "inspect".to_string(),
        "--format".to_string(),
        "{{.Name}}\t{{.CreatedAt}}".to_string(),
    ];
    args.extend(names.iter().cloned());
    match docker_capture(&args) {
        Some(text) => parse_volume_times(&text),
        None => BTreeMap::new(),
    }
}

/// Parse the `<name>\t<created at>` lines of the inspect above.
pub fn parse_volume_times(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let (name, created) = line.trim_end().split_once('\t')?;
            let (name, created) = (name.trim(), created.trim());
            (!name.is_empty() && !created.is_empty())
                .then(|| (name.to_string(), created.to_string()))
        })
        .collect()
}

/// How long one size measurement may take before it is given up on.
///
/// `du` walks the whole tree, and an OCS data volume with a few thousand
/// icechunk chunks in it takes a moment; ten seconds is long enough for that
/// and short enough that a wedged daemon cannot hold `chaps status` open.
pub const SIZE_TIMEOUT: Duration = Duration::from_secs(10);

/// How much a named volume holds, in bytes.
///
/// `docker volume inspect` reports no size at all and `docker system df -v`
/// does not report one reliably, so the volume is mounted read-only into a
/// throwaway busybox and measured with `du -sk`. The image is the one the init
/// containers and `chaps backup` already use, so nothing new is pulled.
///
/// Best-effort and bounded, like every other query here: `None` when docker
/// could not be run, refused, or overran [`SIZE_TIMEOUT`].
///
/// Existence is asked first, as it is before a removal: `docker run -v` on a
/// name docker does not hold creates that volume rather than refusing, and a
/// question about how much a volume holds must not leave an empty one behind.
pub fn volume_size_bytes(name: &str) -> Option<u64> {
    if !volume_exists(name) {
        return None;
    }
    let mount = format!("{name}:/v:ro");
    let args = [
        "run",
        "--rm",
        "-v",
        &mount,
        crate::backup::BUSYBOX_IMAGE,
        "du",
        "-sk",
        "/v",
    ];
    du_bytes(&args, &format!("volume {name}"))
}

/// How much `dir` holds inside a running service's container, in bytes
/// (`docker compose exec -T SERVICE du -sk DIR`).
///
/// The cheaper half of [`volume_size_bytes`] for a component that is up:
/// nothing has to be started to look inside a container that is already
/// there. Same bound, and the same `None` for every failure - a service that
/// is not running is one of them.
pub fn exec_dir_size_bytes(project: &Project, service: &str, dir: &str) -> Option<u64> {
    let mut args = compose_args(project);
    args.extend(
        ["exec", "-T", service, "du", "-sk", dir]
            .iter()
            .map(|a| a.to_string()),
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    du_bytes(&args, &format!("{service}:{dir}"))
}

/// How much data this deployment's OCS instance holds, in bytes, read from
/// inside its running container.
///
/// The mount point comes from [`crate::backup::COMPONENT_VOLUMES`], which is
/// where the data directory of every component is already written down, so
/// this and the backup can never disagree about where OCS keeps its data.
/// Only ever called while the container runs; a caller that has nothing
/// running reads the volume with [`volume_size_bytes`] instead.
pub fn ocs_data_bytes(project: &Project) -> Option<u64> {
    let part = crate::backup::COMPONENT_VOLUMES
        .iter()
        .find(|part| part.service == crate::compose::OCS_SERVICE)?;
    exec_dir_size_bytes(project, part.service, part.data_dir)
}

/// Run one bounded `docker ... du -sk ...` and read the size out of it.
///
/// `what` names the thing being measured, for the `-v` line a failure leaves
/// behind: a size nobody can get is not worth a word on stdout, and it is
/// worth one under `-v`.
fn du_bytes(args: &[&str], what: &str) -> Option<u64> {
    let outcome = crate::commands::doctor::run_bounded("docker", args, SIZE_TIMEOUT);
    if !outcome.succeeded() {
        crate::output::verbose(&format!("  no size for {what}: {}", outcome.stderr()));
        return None;
    }
    parse_du_kilobytes(outcome.stdout())
}

/// The byte count in a `du -sk` line: `217088\t/v` is 212 MB.
///
/// `-k` because busybox `du` counts 512-byte blocks by default where GNU `du`
/// counts kilobytes, and a size that is wrong by a factor of two is worse than
/// no size at all. The last line with a number on it is the total, so a `du`
/// that also complained about a directory it could not read still answers.
pub fn parse_du_kilobytes(text: &str) -> Option<u64> {
    text.lines()
        .rev()
        .find_map(|line| line.split_whitespace().next()?.parse::<u64>().ok())
        .map(|kb| kb.saturating_mul(1024))
}

/// Whether a named docker volume exists.
pub fn volume_exists(name: &str) -> bool {
    trace_command(&[
        "volume".to_string(),
        "inspect".to_string(),
        name.to_string(),
    ]);
    Command::new("docker")
        .args(["volume", "inspect", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// What became of one `docker volume rm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    /// The volume was there, and is gone.
    Removed,
    /// No volume of that name existed: either it was never created, or it has
    /// already been removed.
    NotFound,
    /// Docker refused, with its own first words. A volume still attached to a
    /// container is the one that happens.
    Refused(String),
}

/// Remove a named docker volume, best-effort.
///
/// Existence is asked first rather than read out of the failure: docker says
/// "no such volume" in whatever language it was started in, and the
/// difference between "there was nothing to remove" and "it would not go" is
/// the whole of what the caller reports.
///
/// [`Removal::Refused`] where docker could not be run at all, so a machine
/// with no daemon is told why rather than told the volume is gone.
pub fn remove_volume(name: &str) -> Removal {
    if !volume_exists(name) {
        return Removal::NotFound;
    }
    let args = ["volume".to_string(), "rm".to_string(), name.to_string()];
    trace_command(&args);
    let out = Command::new("docker")
        .args(&args)
        .stdin(Stdio::null())
        .output();
    let out = match out {
        Ok(out) => out,
        Err(e) => return Removal::Refused(e.to_string()),
    };
    if out.status.success() {
        return Removal::Removed;
    }
    Removal::Refused(
        first_stderr_line(&String::from_utf8_lossy(&out.stderr))
            .unwrap_or_else(|| "docker said nothing about why".to_string()),
    )
}
