//! Taking a service or its data away on behalf of the `disable` and `remove`
//! commands: the containers, the data volume, and the lines that report both.

use crate::docker;
use crate::project::Project;

/// Stop and remove the containers of services that are losing their
/// definition, and say in one line what happened.
///
/// `losing` picks them out of this project's containers: the model service
/// `models disable` just took out of the state, or the services of a component
/// `components disable` turned off.
///
/// Called before the compose files are rewritten, while compose still knows
/// those services: afterwards `docker compose stop ocs` is `no such service`,
/// and the container would keep running - and keep its host port published -
/// until the next `chaps up` removed it as an orphan.
///
/// Best-effort in every direction. A docker that cannot be asked, a stop that
/// failed or a project that was never started all yield a line or nothing at
/// all, never an error: the state edit is what the command is for, and
/// `chaps up` cleans up whatever is left.
pub fn stop_and_remove(project: &Project, losing: &dyn Fn(&str) -> bool) -> Option<String> {
    let containers = docker::all_containers(project)?;
    let services: Vec<String> = docker::service_names(&containers)
        .into_iter()
        .filter(|service| losing(service))
        .collect();
    if services.is_empty() {
        return None;
    }
    let ran = |verb: &[&str]| {
        let mut args: Vec<String> = verb.iter().map(|s| s.to_string()).collect();
        args.extend(services.iter().cloned());
        matches!(docker::compose_output(project, &args), Ok((0, _, _)))
    };
    // Stop first: that is what frees the port, and `rm` refuses a running
    // container without it.
    let what = match services.len() {
        1 => format!("the {} container", services[0]),
        count => format!("{count} containers ({})", services.join(", ")),
    };
    if ran(&["stop"]) && ran(&["rm", "-f"]) {
        let ports = if services.len() == 1 {
            "the host port it published is"
        } else {
            "the host ports they published are"
        };
        return Some(format!("stopped and removed {what}; {ports} free again"));
    }
    let orphans = if services.len() == 1 {
        "it as an orphan"
    } else {
        "them as orphans"
    };
    Some(format!(
        "{what} could not be stopped; `chaps up` removes {orphans}"
    ))
}

/// What a `disable` says when the data volume cannot be named at all.
///
/// [`crate::project::Project::prefixed_volume`] answers `None` only for a
/// directory that records no compose project name and whose own name
/// normalises to nothing, which `chaps sync` fixes by writing one down.
pub const UNNAMEABLE_VOLUME: &str = "this directory has no compose project name, so its data volume \
     cannot be named; `chaps sync` records one";

/// What became of one data volume a `--purge` asked for, and the line that
/// reports it.
///
/// The name is returned only when the volume was there and is gone, which is
/// what the `purged` list of a `--json` report holds. Everything else - never
/// created, already removed, a docker that could not be asked - is a line and
/// no name, because nothing was removed.
///
/// Best-effort like [`stop_and_remove`], and for the same reason: the state
/// edit is what `disable` is for, and a volume that would not go is a thing
/// to say rather than a thing to fail over.
pub fn purge_volume(name: &str) -> (Option<String>, String) {
    let outcome = docker::remove_volume(name);
    let removed = matches!(outcome, docker::Removal::Removed).then(|| name.to_string());
    (removed, removal_line(name, &outcome))
}

/// The line one attempted volume removal is reported with.
pub fn removal_line(name: &str, outcome: &docker::Removal) -> String {
    match outcome {
        docker::Removal::Removed => format!("removed volume {name}"),
        docker::Removal::NotFound => format!("volume {name} not found"),
        docker::Removal::Refused(why) => format!("volume {name} could not be removed: {why}"),
    }
}

/// The line a `disable` without `--purge` closes with: the data volume it
/// kept, and the two ways to remove it.
///
/// `purge` is the command that would have taken it, typed as the reader would
/// type it again (`chaps models disable ewars`, `chaps components disable
/// ocs`). Both ways are given because the second works from any directory and
/// after the deployment itself is gone. `None` is for a command after which
/// no chaps command names the volume any more, such as `chaps models remove`:
/// only `docker volume rm` is left to offer.
pub fn kept_volume_line(name: &str, purge: Option<&str>) -> String {
    match purge {
        Some(purge) => format!(
            "kept volume {name}; remove it with `{purge} --purge` or `docker volume rm {name}`"
        ),
        None => format!("kept volume {name}; remove it with `docker volume rm {name}`"),
    }
}
