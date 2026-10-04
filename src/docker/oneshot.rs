//! One container that is not part of any deployment: `chaps chap` runs the
//! chap CLI in one, and asks docker a few questions before it does.

use super::{docker_capture, exit_code, image_ids, spawn_error, trace_command};
use crate::error::Result;
use std::process::{Command, Stdio};

/// The path of the docker socket, on the host and in a container.
///
/// Docker Desktop, Colima and OrbStack all answer a bind mount of this path
/// with their own socket, so it is the one path that works on every machine.
pub const DOCKER_SOCKET: &str = "/var/run/docker.sock";

/// Run `docker <args>` with this process's stdin, stdout and stderr, and give
/// back the exit code.
pub fn run_plain(args: &[String]) -> Result<i32> {
    trace_command(args);
    let status = Command::new("docker")
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| spawn_error(&e))?;
    Ok(exit_code(status))
}

/// Whether this docker has `reference` already, so a run does not pull it.
pub fn image_is_local(reference: &str) -> bool {
    !image_ids(&[reference.to_string()]).is_empty()
}

/// The tags of `repository` that this docker has, as `docker image ls` lists
/// them. Empty when docker could not be asked.
pub fn local_tags(repository: &str) -> Vec<String> {
    let args = ["image", "ls", repository, "--format", "{{.Tag}}"].map(str::to_string);
    docker_capture(&args)
        .map(|text| parse_tags(&text))
        .unwrap_or_default()
}

/// The tags in `docker image ls --format {{.Tag}}` output, without `<none>`.
pub fn parse_tags(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|tag| !tag.is_empty() && *tag != "<none>")
        .map(str::to_string)
        .collect()
}

/// The group that owns the docker socket, as a container sees it.
///
/// The host's own answer can differ: Docker Desktop's socket is
/// `root:root` inside its VM whatever it is on the Mac. So the question goes
/// to a container that mounts the socket, the way the real run will.
pub fn socket_gid(probe_image: &str) -> Option<u32> {
    let mount = format!("{DOCKER_SOCKET}:{DOCKER_SOCKET}");
    let args = [
        "run",
        "--rm",
        "-v",
        &mount,
        probe_image,
        "stat",
        "-c",
        "%g",
        DOCKER_SOCKET,
    ]
    .map(str::to_string);
    docker_capture(&args)?.trim().parse().ok()
}

#[cfg(test)]
mod tests;
