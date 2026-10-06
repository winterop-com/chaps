//! `varde run --chap-core URL`: the models of a group register with a
//! chap-core that runs elsewhere, such as one from its own checkout.
//!
//! A group is a deployment, so this is the same record that
//! `varde components enable chap-core --url` writes there, and `varde sync`
//! renders the overlays from it as for any deployment.

use crate::api::Api;
use crate::components::{ExternalChapCore, detect_models_host, external_chap_core};
use crate::error::Result;
use crate::project::Project;
use std::time::{Duration, Instant};

/// How long to ask chap-core whether the model registered. servicekit
/// registers once its app answers, which is before `varde run` returns.
const REGISTRATION_WAIT: Duration = Duration::from_secs(30);

/// The pause between two asks.
const POLL: Duration = Duration::from_secs(2);

/// Record `url` as the chap-core of the group in `project`, and give the
/// notes about it: where chap-core calls the models back, and that the
/// models already running register with the old one until they restart.
pub(super) fn point_group_at(
    project: &mut Project,
    url: &str,
    models_host: Option<&str>,
) -> Result<Vec<String>> {
    let mut external = external_chap_core(url)?;
    let mut notes = Vec::new();
    match models_host.map(str::trim).filter(|host| !host.is_empty()) {
        Some(host) => external.models_host = host.to_string(),
        None => notes.extend(detect_models_host(
            &mut external,
            &crate::docker::container_publishing,
        )),
    }
    let previous = project.state.components.chap_core_external.clone();
    if let Some(note) = changed_note(previous.as_ref(), &external, &project.dir) {
        notes.push(note);
    }
    project.state.components.chap_core_external = Some(external);
    Ok(notes)
}

/// The note for a group whose chap-core moves to another URL, or `None`
/// when it does not move.
pub(super) fn changed_note(
    previous: Option<&ExternalChapCore>,
    next: &ExternalChapCore,
    dir: &std::path::Path,
) -> Option<String> {
    let previous = previous?;
    (previous.url != next.url).then(|| {
        format!(
            "this group's models registered with {}; the ones that run now move to {} on \
             their next start (`varde -C {} restart --all`)",
            previous.url,
            next.url,
            dir.display()
        )
    })
}

/// Ask the group's chap-core until it lists `service_id`, or the wait runs
/// out. `true` when it registered.
pub(super) fn registered(project: &Project, service_id: &str) -> bool {
    let token = crate::api::token_for(Some(&project.dir));
    let api = Api::new(&project.api_url(), token, Duration::from_secs(5));
    let path = format!("/v2/services/{}", crate::api::encode(service_id));
    let deadline = Instant::now() + REGISTRATION_WAIT;
    loop {
        if let Ok(answer) = api.send("GET", &path, None)
            && answer.is_success()
        {
            return true;
        }
        if Instant::now() >= deadline || crate::interrupt::requested() {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests;
