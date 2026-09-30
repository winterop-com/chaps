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
            "run `chaps update --dry-run` to see the error in full",
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
            "run `chaps update` to move the pin",
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

/// Whether the rendered compose files still match `.chaps/`.
///
/// This is `chaps sync --check` with its own exit code taken away: the same
/// dry run, reported as one line.
pub(super) fn sync_check(project: &Project, catalogue: &Result<registry::Registry>) -> Check {
    const ID: &str = "sync";
    const NAME: &str = "compose files";
    // The same catalogue `chaps sync` would load, with the deployment's own
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
            "run `chaps sync` to see it in full",
        ),
        Ok(report) if report.drift => Check::warn(
            ID,
            NAME,
            format!("out of date with .chaps/ ({})", report.summary()),
            "run `chaps sync`, or `chaps up`, which syncs first",
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
                "{build}; `chaps update` re-pulls it, `chaps update --pin-chap-core` pins a release"
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
            Some("run `chaps update --dry-run` to see what would move".to_string()),
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
mod tests {
    use super::*;

    /// A model this deployment added itself, following `main` at `tag`.
    fn manual_entry(tag: &str, follow: Option<&str>) -> crate::project::ManualModel {
        crate::project::ManualModel {
            service_id: "chapkit-ghr-model".into(),
            display_name: "chapkit_ghr_model".into(),
            repository: Some("https://github.com/chap-models/chapkit_ghr_model".into()),
            image: "ghcr.io/chap-models/chapkit_ghr_model".into(),
            tag: tag.to_string(),
            commit: None,
            follow: follow.map(str::to_string),
            data_dir: Some("/work/data".into()),
            user: Some("10001:10001".into()),
            runtime_amd64: true,
            added: "2026-09-24".into(),
        }
    }

    /// The `manual <id>` line: a warning only when the branch has moved on,
    /// and never a failure.
    #[test]
    fn a_manual_model_is_checked_against_the_branch_it_follows() {
        let entry = manual_entry("sha-1eb8cf1", Some("main"));

        let ok = manual_check("chapkit_ghr_model", &entry, Some("sha-1eb8cf1"), false);
        assert_eq!(ok.status, Status::Ok);
        assert_eq!(ok.id, "manual-chapkit_ghr_model");
        assert_eq!(ok.name, "manual chapkit_ghr_model");
        assert!(ok.detail.contains("the newest published build on main"));
        assert_eq!(ok.fix, None);

        let behind = manual_check("chapkit_ghr_model", &entry, Some("sha-b1d6c31"), false);
        assert_eq!(behind.status, Status::Warn);
        assert!(behind.detail.contains("running sha-1eb8cf1"), "{behind:?}");
        assert!(
            behind.detail.contains("published sha-b1d6c31"),
            "{behind:?}"
        );
        assert_eq!(
            behind.fix.as_deref(),
            Some("run `chaps update` to move the pin")
        );

        // A repository that would not answer is a check that was not made.
        let unknown = manual_check("chapkit_ghr_model", &entry, None, false);
        assert_eq!(unknown.status, Status::Skip);
        assert!(unknown.detail.contains("could not ask"), "{unknown:?}");
        assert!(unknown.fix.is_some());

        // `--offline` asks nothing, and an entry with no branch has nothing
        // to be behind.
        let offline = manual_check("chapkit_ghr_model", &entry, None, true);
        assert_eq!(offline.status, Status::Skip);
        assert!(offline.detail.contains("--offline"), "{offline:?}");

        let pinned = manual_check(
            "chapkit_ghr_model",
            &manual_entry("sha-1eb8cf1", None),
            Some("sha-b1d6c31"),
            false,
        );
        assert_eq!(pinned.status, Status::Skip);
        assert!(
            pinned.detail.contains("pinned to sha-1eb8cf1"),
            "{pinned:?}"
        );
    }

    /// Where the marketplace is asked to move a pin.
    const MARKETPLACE: &str = "https://github.com/dhis2-chap/model-marketplace";

    /// The newest commit of the example branch, and the one before it.
    const PIN_NEW: &str = "a6a05ef0000000000000000000000000000000aa";

    const PIN_OLD: &str = "70c07a90000000000000000000000000000000bb";

    /// A commit that is on no branch this listing carries: a tag of its own.
    const PIN_OFF: &str = "5adf3a80000000000000000000000000000000cc";

    /// A branch listing, newest first, with the newest commit that has a
    /// published image named separately.
    fn branch(log: &[(&str, &str)], newest: Option<&str>) -> crate::manual::BranchBuilds {
        crate::manual::BranchBuilds {
            log: log
                .iter()
                .map(|(sha, day)| crate::manual::github::Commit {
                    sha: (*sha).to_string(),
                    day: (*day).to_string(),
                })
                .collect(),
            newest: newest.map(|sha| (sha.to_string(), crate::manual::github::sha_tag(sha))),
        }
    }

    /// The main branch of the example model: two commits, both built.
    fn main_log() -> [(&'static str, &'static str); 2] {
        [(PIN_NEW, "2026-09-23"), (PIN_OLD, "2026-09-08")]
    }

    /// The `registry pin <id>` line: the marketplace's own pin against the
    /// newest build the model's repository has published.
    #[test]
    fn a_marketplace_pin_is_checked_against_the_branch_the_model_builds_from() {
        let unasked = |sha: &str| -> std::result::Result<String, String> {
            Err(format!("could not ask about {sha}"))
        };
        let verdict = |pinned: &str, builds: &crate::manual::BranchBuilds, day: PinDayFn| {
            let facts = pin_facts_of("main", builds, pinned, day);
            registry_pin_verdict(&crate::manual::github::sha_tag(pinned), MARKETPLACE, &facts)
        };

        // Same: the registry pins the newest build there is.
        let built = branch(&main_log(), Some(PIN_NEW));
        let same = verdict(PIN_NEW, &built, &unasked);
        assert_eq!(same.0, Status::Ok);
        assert_eq!(same.1, "sha-a6a05ef is the newest build on main");
        assert_eq!(same.2, None);
        // The line it is printed as.
        let check = Check::from_verdict(
            pin_id("chapkit_ewars_model"),
            pin_name("chapkit_ewars_model"),
            same,
        );
        assert_eq!(check.id, "registry-pin-chapkit_ewars_model");
        assert_eq!(check.name, "registry pin chapkit_ewars_model");

        // Behind: the registry still pins the commit before it. This is the
        // case the check exists for.
        let behind = verdict(PIN_OLD, &built, &unasked);
        assert_eq!(behind.0, Status::Warn);
        assert_eq!(
            behind.1,
            "the registry pins sha-70c07a9 (2026-09-08) but main's newest build is \
             sha-a6a05ef (2026-09-23)"
        );
        assert_eq!(
            behind.2.as_deref(),
            Some(
                "ask the marketplace maintainers for a new pin: https://github.com/dhis2-chap/model-marketplace"
            )
        );

        // Ahead: the pinned commit is on the branch and newer than anything
        // the publish workflow has built, so there is nothing to move to.
        let ahead = verdict(PIN_NEW, &branch(&main_log(), Some(PIN_OLD)), &unasked);
        assert_eq!(ahead.0, Status::Ok);
        assert_eq!(
            ahead.1,
            "sha-a6a05ef (2026-09-23) is pinned ahead of main's newest build \
             sha-70c07a9 (2026-09-08)"
        );
        assert_eq!(ahead.2, None);

        // A pin the branch does not carry at all is placed by its day, which
        // is one more request. Newer than the newest build: ahead.
        let dated = |day: &'static str| move |_: &str| Ok(day.to_string());
        let off_ahead = dated("2026-09-24");
        let off_ahead = verdict(PIN_OFF, &built, &off_ahead);
        assert_eq!(off_ahead.0, Status::Ok);
        assert!(
            off_ahead.1.contains("sha-5adf3a8 (2026-09-24)"),
            "{off_ahead:?}"
        );
        assert!(off_ahead.1.contains("pinned ahead"), "{off_ahead:?}");

        // Older than it: behind, whatever branch it came from.
        let off_behind = dated("2026-08-01");
        let off_behind = verdict(PIN_OFF, &built, &off_behind);
        assert_eq!(off_behind.0, Status::Warn);
        assert!(
            off_behind
                .1
                .contains("the registry pins sha-5adf3a8 (2026-08-01)"),
            "{off_behind:?}"
        );
    }

    /// Nothing the lookup cannot answer is a failure: every one of them is a
    /// line that was not checked, with the reason on it.
    #[test]
    fn a_marketplace_pin_nobody_could_place_is_skipped_with_the_reason() {
        let unasked = |sha: &str| -> std::result::Result<String, String> {
            Err(format!("could not ask about {sha}"))
        };
        let skipped = |facts: std::result::Result<PinFacts, String>| {
            registry_pin_verdict("sha-5adf3a8", MARKETPLACE, &facts)
        };

        // The one extra request a pin off the branch needs, refused.
        let off = skipped(pin_facts_of(
            "main",
            &branch(&main_log(), Some(PIN_NEW)),
            PIN_OFF,
            &unasked,
        ));
        assert_eq!(off.0, Status::Skip);
        assert!(off.1.contains("could not ask about"), "{off:?}");
        assert_eq!(off.2, None, "a skip has nothing to do about it");

        // A branch whose newest commits have no published build yet.
        let unbuilt = skipped(pin_facts_of(
            "main",
            &branch(&main_log(), None),
            PIN_OFF,
            &unasked,
        ));
        assert_eq!(unbuilt.0, Status::Skip);
        assert!(
            unbuilt.1.contains("has a published `sha-` build"),
            "{unbuilt:?}"
        );

        // A catalogue entry with no commit to compare.
        let bare = skipped(pin_facts_of(
            "main",
            &branch(&main_log(), Some(PIN_NEW)),
            "  ",
            &unasked,
        ));
        assert_eq!(bare.0, Status::Skip);
        assert!(bare.1.contains("names no commit"), "{bare:?}");

        // A listing with no dates cannot place a commit that is not on it.
        let undated = skipped(pin_facts_of(
            "main",
            &branch(&[(PIN_NEW, ""), (PIN_OLD, "")], Some(PIN_NEW)),
            PIN_OFF,
            &|_| Ok("2026-09-24".to_string()),
        ));
        assert_eq!(undated.0, Status::Skip);
        assert!(undated.1.contains("no commit dates"), "{undated:?}");

        // And the whole lookup ruled out by the flag the user gave.
        assert!(PIN_OFFLINE.contains("--offline"));
    }

    #[test]
    fn a_manifest_list_is_searched_for_linux_amd64() {
        let list = r#"{"manifests":[
            {"platform":{"architecture":"amd64","os":"linux"}},
            {"platform":{"architecture":"arm64","os":"linux"}}
        ]}"#;
        assert_eq!(manifest_has_amd64(list), Some(true));

        let arm_only = r#"{"manifests":[{"platform":{"architecture":"arm64","os":"linux"}}]}"#;
        assert_eq!(manifest_has_amd64(arm_only), Some(false));

        // A single image manifest carries no list, so the platform is simply
        // not something this can answer.
        let single =
            r#"{"schemaVersion":2,"mediaType":"application/vnd.oci.image.manifest.v1+json"}"#;
        assert_eq!(manifest_has_amd64(single), None);
        assert_eq!(manifest_has_amd64("not json"), None);
    }
}
