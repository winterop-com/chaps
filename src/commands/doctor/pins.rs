//! Checks that what the deployment pins is current: marketplace and manual model builds, and chap-core's tag.

use super::*;

/// One line per model this deployment added itself that follows a branch:
/// is the build it runs still the newest one that branch has published.
///
/// Nothing here is a failure. A model added from an image reference is pinned
/// on purpose and only reported as such, and a repository that will not
/// answer is a check that was not made rather than a problem with the
/// deployment.
pub(super) fn manual_checks(ctx: &Ctx, project: &Project) -> Vec<Check> {
    let endpoints = crate::manual::Endpoints {
        timeout: NET_TIMEOUT,
        ..crate::manual::Endpoints::from_env(ctx.registry.offline)
    };
    project
        .state
        .manual
        .iter()
        .map(|(id, entry)| {
            // The lookup is only made for an entry that has a branch to
            // compare against, and only when this run may use the network.
            let newest = match (&entry.repository, &entry.follow, endpoints.offline) {
                (Some(repository), Some(branch), false) => {
                    crate::manual::newest_published(repository, branch, &endpoints)
                        .ok()
                        .flatten()
                        .map(|(_, tag)| tag)
                }
                _ => None,
            };
            manual_check(id, entry, newest.as_deref(), endpoints.offline)
        })
        .collect()
}

/// The verdict for one manually added model, given what its branch publishes
/// now.
fn manual_check(
    id: &str,
    entry: &crate::project::ManualModel,
    newest: Option<&str>,
    offline: bool,
) -> Check {
    let check_id = format!("manual-{id}");
    let name = format!("manual {id}");
    let Some(branch) = &entry.follow else {
        return Check::skip(
            check_id,
            name,
            format!("pinned to {}, so nothing moves it", entry.tag),
        );
    };
    if offline {
        return Check::skip(
            check_id,
            name,
            format!("--offline, so {branch} was not asked about a newer build"),
        );
    }
    match newest {
        None => Check::skip_with(
            check_id,
            name,
            format!(
                "could not ask {} for the newest build on {branch}",
                entry
                    .repository
                    .clone()
                    .unwrap_or_else(|| entry.image.clone())
            ),
            "run `varde update --dry-run` to see the error in full",
        ),
        Some(tag) if tag == entry.tag => Check::ok(
            check_id,
            name,
            format!("{tag}, the newest published build on {branch}"),
        ),
        Some(tag) => Check::warn(
            check_id,
            name,
            format!("running {}, and {branch} has published {tag}", entry.tag),
            "run `varde update` to move the pin",
        ),
    }
}

/// How many commits of a model's default branch a marketplace pin is placed
/// against. Twenty is several weeks of a model repository.
pub const PIN_COMMITS: u32 = 20;

/// How many of those commits are asked about on ghcr. The publish workflow
/// builds every push, so the newest commit is normally the first and only tag
/// asked for, and no model is allowed to turn one line into a page of
/// registry round trips.
pub const PIN_PROBES: usize = 5;

/// What `--offline` leaves every marketplace pin line saying.
pub const PIN_OFFLINE: &str =
    "--offline, so the model repositories were not asked about newer builds";

/// Where the commit the catalogue pins sits against the newest build the
/// model's default branch has published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinPlace {
    /// The pin is that build.
    Newest,
    /// The branch has built something newer, so the catalogue is behind.
    Behind,
    /// The pin is newer than anything the branch has built, which is what a
    /// pin taken from a branch or a tag of its own looks like.
    Ahead,
}

/// What one model's repository says, next to the commit the catalogue pins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinFacts {
    /// The repository's default branch, e.g. `main`.
    pub branch: String,
    /// The `sha-` tag of the newest commit on it with a published image.
    pub newest_tag: String,
    /// The day that commit was made, empty where the listing carried none.
    pub newest_day: String,
    /// The day the pinned commit was made, on the same terms.
    pub pinned_day: String,
    pub place: PinPlace,
}

/// The `registry pin <id>` line: whether the commit the catalogue pins is
/// still the newest build the model's own repository publishes.
///
/// Nothing here is a failure. A pin that lags is a marketplace to nudge, not
/// a deployment that is broken, and a repository that will not answer is a
/// check that was not made.
pub fn registry_pin_verdict(
    pinned_tag: &str,
    marketplace: &str,
    facts: &std::result::Result<PinFacts, String>,
) -> (Status, String, Option<String>) {
    let facts = match facts {
        Ok(facts) => facts,
        Err(why) => return (Status::Skip, why.clone(), None),
    };
    let newest = dated(&facts.newest_tag, &facts.newest_day);
    let pinned = dated(pinned_tag, &facts.pinned_day);
    match facts.place {
        PinPlace::Newest => (
            Status::Ok,
            format!("{pinned_tag} is the newest build on {}", facts.branch),
            None,
        ),
        PinPlace::Ahead => (
            Status::Ok,
            format!(
                "{pinned} is pinned ahead of {}'s newest build {newest}",
                facts.branch
            ),
            None,
        ),
        PinPlace::Behind => (
            Status::Warn,
            format!(
                "the registry pins {pinned} but {}'s newest build is {newest}",
                facts.branch
            ),
            Some(format!(
                "ask the marketplace maintainers for a new pin: {marketplace}"
            )),
        ),
    }
}

/// `sha-70c07a9 (2026-09-08)`, or the tag alone where no day was found.
fn dated(tag: &str, day: &str) -> String {
    match day.is_empty() {
        true => tag.to_string(),
        false => format!("{tag} ({day})"),
    }
}

/// The stable id of one marketplace pin line, and the name it is printed
/// under.
fn pin_id(id: &str) -> String {
    format!("registry-pin-{id}")
}

fn pin_name(id: &str) -> String {
    format!("registry pin {id}")
}

/// One line per enabled marketplace model, looked up in parallel.
///
/// Each lookup is two GitHub reads and a registry round trip, and a
/// deployment with five models would otherwise wait out five of them in a
/// row.
pub(super) fn registry_pin_checks(
    ctx: &Ctx,
    project: &Project,
    catalogue: &Result<registry::Registry>,
) -> Vec<Check> {
    // A model this deployment defines itself is left out: its `manual <id>`
    // line already asks the same question of the branch it follows.
    let models: Vec<(&str, &crate::project::EnabledModel)> = project
        .state
        .models
        .iter()
        .filter(|(id, _)| !project.state.manual.contains_key(*id))
        .map(|(id, entry)| (id.as_str(), entry))
        .collect();
    if models.is_empty() {
        return Vec::new();
    }

    let skipped = |reason: String| -> Vec<Check> {
        models
            .iter()
            .map(|(id, _)| Check::skip(pin_id(id), pin_name(id), reason.clone()))
            .collect()
    };

    let endpoints = crate::manual::Endpoints {
        timeout: NET_TIMEOUT,
        ..crate::manual::Endpoints::from_env(ctx.registry.offline)
    };
    if endpoints.offline {
        return skipped(PIN_OFFLINE.to_string());
    }
    let registry = match catalogue {
        Ok(registry) => registry,
        Err(err) => return skipped(format!("no catalogue to read the pin from: {err}")),
    };
    let marketplace = registry.index.marketplace.repository.as_str();

    std::thread::scope(|scope| {
        let handles: Vec<_> = models
            .iter()
            .map(|(id, entry)| {
                let endpoints = &endpoints;
                scope.spawn(move || registry_pin_check(id, entry, registry, marketplace, endpoints))
            })
            .collect();
        handles
            .into_iter()
            .zip(models.iter())
            .map(|(handle, (id, _))| {
                handle.join().unwrap_or_else(|_| {
                    Check::skip(pin_id(id), pin_name(id), "the lookup did not finish")
                })
            })
            .collect()
    })
}

/// The verdict for one marketplace model: what the catalogue pins for the
/// channel this deployment follows, against what its repository publishes.
fn registry_pin_check(
    id: &str,
    entry: &crate::project::EnabledModel,
    registry: &registry::Registry,
    marketplace: &str,
    endpoints: &crate::manual::Endpoints,
) -> Check {
    let (check_id, name) = (pin_id(id), pin_name(id));
    let Some(model) = registry.get(id) else {
        return Check::skip(check_id, name, "the catalogue no longer lists this model");
    };
    // The pin this deployment follows: the channel's version where it follows
    // one, and the version it was pinned to where it does not.
    let selector = match entry.channel {
        Some(channel) => registry::VersionSelector::Channel(channel),
        None => registry::VersionSelector::Exact(entry.version.clone()),
    };
    let version = match model.resolve(&selector) {
        Ok(version) => version,
        Err(err) => return Check::skip(check_id, name, format!("{err}")),
    };
    Check::from_verdict(
        check_id,
        name,
        registry_pin_verdict(
            &version.image_tag,
            marketplace,
            &pin_facts(&model.source.repository, &version.commit, endpoints),
        ),
    )
}

/// The day of a commit the branch listing did not carry: `(commit) -> day`,
/// or the sentence the line is skipped with.
pub type PinDayFn<'a> = &'a dyn Fn(&str) -> std::result::Result<String, String>;

/// What the model's repository publishes, and where the pinned commit sits
/// against it.
///
/// The error is the sentence the line is skipped with: offline, a rate limit,
/// a private or renamed repository and a branch that has published nothing
/// are all "this was not asked", never a fault in the deployment.
fn pin_facts(
    repository: &str,
    pinned_commit: &str,
    endpoints: &crate::manual::Endpoints,
) -> std::result::Result<PinFacts, String> {
    let branch = crate::manual::default_branch_of(repository, endpoints)
        .map_err(|err| format!("could not ask {repository} for its default branch: {err:#}"))?;
    let builds =
        crate::manual::branch_builds(repository, &branch, PIN_COMMITS, PIN_PROBES, endpoints)
            .map_err(|err| format!("could not ask {repository} about {branch}: {err:#}"))?;
    pin_facts_of(&branch, &builds, pinned_commit, &|sha| {
        crate::manual::commit_day(repository, sha, endpoints).map_err(|err| {
            format!(
                "could not ask {repository} about {}: {err:#}",
                crate::manual::github::sha_tag(sha)
            )
        })
    })
}

/// [`pin_facts`] once the branch has answered: the half that decides, with
/// the one lookup it may still need handed in.
fn pin_facts_of(
    branch: &str,
    builds: &crate::manual::BranchBuilds,
    pinned_commit: &str,
    day_of_pin: PinDayFn,
) -> std::result::Result<PinFacts, String> {
    let pinned_commit = pinned_commit.trim();
    if pinned_commit.is_empty() {
        return Err("the catalogue entry names no commit".to_string());
    }
    let Some((newest_sha, newest_tag)) = builds.newest.clone() else {
        return Err(format!(
            "none of the newest {PIN_PROBES} commits on {branch} has a published `sha-` build"
        ));
    };
    let newest_day = day_in(&builds.log, &newest_sha);
    let pinned_tag = crate::manual::github::sha_tag(pinned_commit);
    let facts = |place, pinned_day| PinFacts {
        branch: branch.to_string(),
        newest_tag: newest_tag.clone(),
        newest_day: newest_day.clone(),
        pinned_day,
        place,
    };

    if pinned_tag == newest_tag {
        return Ok(facts(PinPlace::Newest, newest_day.clone()));
    }

    let short = |sha: &str| sha.chars().take(7).collect::<String>();
    let pinned_at = builds
        .log
        .iter()
        .position(|commit| short(&commit.sha) == short(pinned_commit));
    let newest_at = builds
        .log
        .iter()
        .position(|commit| commit.sha == newest_sha);
    match (pinned_at, newest_at) {
        // Both are on the branch, so their order on it settles which is
        // newer, whatever the dates say.
        (Some(pinned), Some(newest)) => Ok(facts(
            match pinned < newest {
                true => PinPlace::Ahead,
                false => PinPlace::Behind,
            },
            builds.log[pinned].day.clone(),
        )),
        // A commit the branch does not carry: one from a branch or a tag of
        // its own, or one older than the page that was read. The day it was
        // made is what tells those apart, and it costs one more request.
        _ => {
            let day = day_of_pin(pinned_commit)?;
            if newest_day.is_empty() {
                return Err(format!(
                    "{branch} gave no commit dates to place {pinned_tag} against"
                ));
            }
            Ok(facts(
                match day > newest_day {
                    true => PinPlace::Ahead,
                    false => PinPlace::Behind,
                },
                day,
            ))
        }
    }
}

/// The day one commit of a listing was made, empty when it is not in it.
fn day_in(log: &[crate::manual::github::Commit], sha: &str) -> String {
    log.iter()
        .find(|commit| commit.sha == sha)
        .map(|commit| commit.day.clone())
        .unwrap_or_default()
}

/// Whether the rendered compose files still match `.varde/`.
///
/// This is `varde sync --check` with its own exit code taken away: the same
/// dry run, reported as one line.
pub(super) fn sync_check(project: &Project, catalogue: &Result<registry::Registry>) -> Check {
    const ID: &str = "sync";
    const NAME: &str = "compose files";
    // The same catalogue `varde sync` would load, with the deployment's own
    // definitions folded in - without them a manually added model would look
    // like one the catalogue dropped. A collision is reported by every
    // command that writes; this one only reads.
    let registry = match catalogue {
        Ok(registry) => {
            let mut registry = registry.clone();
            let _ = registry.with_manual(&project.state.manual);
            registry
        }
        Err(err) => {
            return Check::skip(ID, NAME, format!("no catalogue to compare against: {err}"));
        }
    };
    // A clone because `sync` takes the project by value to update its
    // rendered-file list; in check mode it writes nothing, and this copy is
    // thrown away either way.
    let mut project = project.clone();
    match sync(&mut project, &registry, true) {
        Err(err) => Check::fail(
            ID,
            NAME,
            format!("{err:#}"),
            "run `varde sync` to see it in full",
        ),
        Ok(report) if report.drift => Check::warn(
            ID,
            NAME,
            format!("out of date with .varde/ ({})", report.summary()),
            "run `varde sync`, or `varde up`, which syncs first",
        ),
        Ok(report) => Check::ok(ID, NAME, format!("in sync ({})", report.summary())),
    }
}

/// Whether the chap-core tag this deployment pins is still the newest one.
///
/// A moving tag is answered before the release list is looked at: there is no
/// comparison to make, so that verdict is the same online and offline. What
/// it says instead is which build the tag is on - `running` is the digest the
/// image chap-core's container runs was pulled at, or `None` when it is not
/// up and docker has nothing to say.
pub fn pin_verdict(
    tag: &str,
    latest: ReleaseList<'_>,
    running: Option<&str>,
) -> (Status, String, Option<String>) {
    if chapcore::is_moving_tag(tag) {
        let build = match running.filter(|digest| !digest.is_empty()) {
            Some(digest) => format!("{tag} (moving tag, running {digest})"),
            None => format!("{tag} (moving tag)"),
        };
        return (
            Status::Ok,
            format!(
                "{build}; `varde update` re-pulls it, `varde update --pin-chap-core` pins a release"
            ),
            None,
        );
    }
    let latest = match latest {
        ReleaseList::Newest(latest) => latest,
        ReleaseList::Offline => {
            return (
                Status::Skip,
                format!("{tag} pinned; {}", ReleaseList::OFFLINE_WHY),
                None,
            );
        }
        ReleaseList::Unreachable => {
            return (
                Status::Skip,
                format!("{tag} pinned; the release list was not reachable"),
                None,
            );
        }
    };
    if chapcore::is_newer(latest, tag) {
        return (
            Status::Warn,
            format!("{tag} pinned, {latest} released"),
            Some("run `varde update --dry-run` to see what would move".to_string()),
        );
    }
    (Status::Ok, format!("{tag} is the newest release"), None)
}

/// The `chap-core-pin` line.
pub fn pin_check(tag: &str, latest: ReleaseList<'_>, running: Option<&str>) -> Check {
    Check::from_verdict(
        "chap-core-pin",
        "chap-core pin",
        pin_verdict(tag, latest, running),
    )
}

/// Whether a manifest document offers a linux/amd64 image.
///
/// `None` when the document says nothing about platforms at all: a single
/// image manifest carries no list, and "the tag exists" is then the whole
/// answer this can give.
pub fn manifest_has_amd64(body: &str) -> Option<bool> {
    let doc: serde_json::Value = serde_json::from_str(body).ok()?;
    let manifests = doc.get("manifests")?.as_array()?;
    Some(manifests.iter().any(|entry| {
        let platform = entry.get("platform");
        let field = |key: &str| {
            platform
                .and_then(|p| p.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
        };
        field("os") == "linux" && field("architecture") == "amd64"
    }))
}

#[cfg(test)]
mod tests;
