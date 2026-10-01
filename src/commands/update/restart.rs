//! Which running services are out of date once the pull has finished.

use crate::docker;
use std::collections::{BTreeMap, BTreeSet};

/// One running service, and what it would be made of if it were created now.
///
/// The two halves come from opposite sides of the run: the `running_` fields
/// from the container docker has, the other two from the files on disk and
/// the images the pull left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceCheck {
    pub service: String,
    /// The id of the image the container is running.
    pub running_image: String,
    /// The id that image reference points at now, after the pull.
    pub pulled_image: String,
    /// The configuration hash the container was created with.
    pub running_hash: String,
    /// The hash compose computes for that service today.
    pub current_hash: String,
}

/// Whether this service is running something the project no longer describes.
///
/// A value that could not be read is not a difference: docker being unable to
/// say what an image is must never turn into "restart everything". Both sides
/// of a comparison have to be there for it to count.
pub fn needs_restart(check: &ServiceCheck) -> bool {
    differs(&check.running_image, &check.pulled_image)
        || differs(&check.running_hash, &check.current_hash)
}

/// Whether two answers disagree, as opposed to one of them being missing.
fn differs(running: &str, current: &str) -> bool {
    !running.is_empty() && !current.is_empty() && running != current
}

/// The services of `checks` that need recreating, in the order given.
pub fn restart_needed(checks: &[ServiceCheck]) -> Vec<String> {
    checks
        .iter()
        .filter(|check| needs_restart(check))
        .map(|check| check.service.clone())
        .collect()
}

/// Pair every running service with what it would be made of today.
///
/// A container whose service the files no longer mention, or that docker
/// would not talk about, yields a check with empty halves, which
/// [`needs_restart`] reads as "not known to have moved".
pub fn service_checks(
    running: &[docker::Container],
    builds: &BTreeMap<String, docker::ContainerBuild>,
    images: &BTreeMap<String, String>,
    ids: &BTreeMap<String, String>,
    hashes: &BTreeMap<String, String>,
) -> Vec<ServiceCheck> {
    let mut seen = BTreeSet::new();
    running
        .iter()
        .filter(|container| container.is_running())
        .filter(|container| seen.insert(container.service.clone()))
        .map(|container| {
            let build = builds.get(&container.id).cloned().unwrap_or_default();
            // The reference the files pin, not the one the container was made
            // from: they are the same until a pin moves, and when they are
            // not, the hashes have already said so.
            let reference = images.get(&container.service);
            ServiceCheck {
                service: container.service.clone(),
                running_image: build.image_id,
                pulled_image: reference
                    .and_then(|r| ids.get(r))
                    .cloned()
                    .unwrap_or_default(),
                running_hash: build.config_hash,
                current_hash: hashes.get(&container.service).cloned().unwrap_or_default(),
            }
        })
        .collect()
}

/// The services whose image reference points somewhere else than it did
/// before the pull.
///
/// A reference docker still does not have afterwards is left out: the pull
/// having failed to fetch something is not the same as it having fetched
/// something new.
pub fn pulled_new(
    images: &BTreeMap<String, String>,
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<String> {
    images
        .iter()
        .filter(|(_, reference)| match after.get(*reference) {
            Some(new) => before.get(*reference) != Some(new),
            None => false,
        })
        .map(|(service, _)| service.clone())
        .collect()
}
