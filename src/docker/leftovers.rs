//! What a deployment leaves in docker after its directory is gone: whether
//! anything still uses it, for `varde cleanup` to ask before it removes.

use super::docker_capture;

/// Whether any container, running or not, belongs to the compose project
/// `name`. `None` when docker could not be asked.
pub fn project_has_containers(name: &str) -> Option<bool> {
    any_container(&format!("label=com.docker.compose.project={name}"))
}

/// Whether any container, running or not, mounts the volume `name`. `None`
/// when docker could not be asked.
pub fn volume_in_use(name: &str) -> Option<bool> {
    any_container(&format!("volume={name}"))
}

/// Whether the network compose made for the project `name` is still there.
pub fn default_network_exists(name: &str) -> bool {
    let args = ["network", "inspect", &format!("{name}_default")].map(str::to_string);
    docker_capture(&args).is_some()
}

fn any_container(filter: &str) -> Option<bool> {
    let args = ["ps", "-a", "-q", "--filter", filter].map(str::to_string);
    docker_capture(&args).map(|text| text.lines().any(|line| !line.trim().is_empty()))
}
