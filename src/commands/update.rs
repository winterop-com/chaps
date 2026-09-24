//! `chaps update [--dry-run]` — move the pins forward and pull.
//!
//! Two kinds of pin move here. A model that follows a channel is re-resolved
//! against the marketplace, and the only other thing that moves those is
//! `chaps models enable`; exact pins (`--version`) never move. chap-core's own
//! pin moves too when it is a release tag and a newer release exists, which
//! also re-fetches the `compose.ghcr.yml` that release publishes. A moving
//! chap-core tag (`latest`, `master`, `dev`) is only refreshed by the pull,
//! unless `--pin-chap-core` turns it into a release pin.
//!
//! What it does not do is touch a container. The run reads: the plan, then
//! the pull, then one line saying what moved and which running services are
//! now out of date. Applying that is `chaps restart`, and starting a
//! deployment that is down is `chaps up`; an update that did either would be
//! a deployment nobody asked for.
//!
//! "Out of date" is two questions asked of every running container. Is the
//! image it runs still the image its reference points at, now that the pull
//! has finished? And is the configuration hash compose stamped on it still
//! the hash compose computes for the files as they are? Either answer being
//! no means the service is running something the project no longer describes.

use crate::chapcore;
use crate::cli::UpdateArgs;
use crate::commands::Ctx;
use crate::components;
use crate::compose::sync::{EnvTag, refresh_env_pin, set_env_chap_tag};
use crate::compose::{sync, tag_env_var};
use crate::docker;
use crate::error::{ChapError, Result};
use crate::manual;
use crate::output::{self, Out};
use crate::project::{CHAP_TAG_ENV_VAR, ComposeSource, ManualModel, Project, cached_compose_file};
use crate::registry::{Provenance, Registry, VersionSelector};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The shape of `update --json`, and what the human output is built from.
#[derive(Debug, Serialize)]
pub struct UpdateReport {
    pub registry: RegistryInfo,
    pub models: Vec<ModelUpdate>,
    pub chap_core: ChapCoreUpdate,
    /// One row per enabled component other than chap-core. They follow moving
    /// tags, so there is no pin to move - only a pull to report.
    pub components: Vec<ComponentUpdate>,
    pub pulled: bool,
    /// Services the pull brought a different image for. A moving tag that
    /// still points at the image this machine already had is not one of them.
    pub pulled_new: Vec<String>,
    /// Running services whose image or configuration no longer matches what
    /// this project describes: what `chaps restart` would recreate.
    pub restart_needed: Vec<String>,
    /// Whether any of this project's containers was up, `null` when docker
    /// could not be asked.
    pub stack_running: Option<bool>,
    pub dry_run: bool,
}

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

/// chap-core's own before and after.
#[derive(Debug, Clone, Serialize)]
pub struct ChapCoreUpdate {
    pub old_tag: String,
    pub new_tag: String,
    pub changed: bool,
    /// Whether the old tag is one that can point at a different image
    /// tomorrow, so the pull refreshes the images even when nothing moves.
    pub moving: bool,
    /// The newest release GitHub reports, when it was looked up at all.
    pub latest_release: Option<String>,
    /// Where `compose.yml` is rendered from after this run.
    pub compose_source: ComposeSource,
}

#[derive(Debug, Serialize)]
pub struct RegistryInfo {
    pub url: String,
    pub provenance: Provenance,
}

/// One enabled component's image, and where its tag comes from.
///
/// Neither OCS nor the object store publishes a release feed this CLI can
/// consult, so both follow a moving tag: `docker compose pull` takes whatever
/// it points at today, and there is nothing in `.chaps/` to move. An active
/// `*_IMAGE_TAG` line in `.env` is the operator's own pin, and is reported as
/// such rather than silently overridden.
#[derive(Debug, Clone, Serialize)]
pub struct ComponentUpdate {
    pub name: String,
    pub image: String,
    /// The tag the pull will take, whichever of the two it is.
    pub tag: String,
    /// Whether that tag came from an active `.env` line.
    pub pinned: bool,
}

/// One row per enabled component, read from `.chaps/components.yaml` and the
/// `.env` beside it.
pub fn plan_components(project: &Project) -> Vec<ComponentUpdate> {
    let env =
        std::fs::read_to_string(project.dir.join(crate::project::ENV_FILE)).unwrap_or_default();
    let components = &project.state.components;
    let mut out = Vec::new();
    if components.ocs.enabled {
        out.push(component_update(
            crate::compose::OCS_SERVICE,
            components::OCS_IMAGE,
            components::OCS_TAG_ENV_VAR,
            &components.ocs.image_tag,
            &env,
        ));
    }
    if components.s3.enabled {
        out.push(component_update(
            crate::compose::S3_SERVICE,
            components::S3_IMAGE,
            components::S3_TAG_ENV_VAR,
            components::S3_DEFAULT_TAG,
            &env,
        ));
    }
    out
}

fn component_update(
    name: &str,
    image: &str,
    var: &str,
    default_tag: &str,
    env: &str,
) -> ComponentUpdate {
    match crate::auth::active_value(env, var) {
        Some(tag) => ComponentUpdate {
            name: name.to_string(),
            image: image.to_string(),
            tag,
            pinned: true,
        },
        None => ComponentUpdate {
            name: name.to_string(),
            image: image.to_string(),
            tag: default_tag.to_string(),
            pinned: false,
        },
    }
}

/// The row one component gets in the human list.
pub fn component_line(c: &ComponentUpdate) -> String {
    if c.pinned {
        return format!(
            "{}  {}  pinned in .env, re-pulled at that tag",
            c.name, c.tag
        );
    }
    format!("{}  {}  moving tag, re-pulled", c.name, c.tag)
}

/// One enabled model's before and after.
#[derive(Debug, Clone, Serialize)]
pub struct ModelUpdate {
    pub id: String,
    pub old_version: String,
    pub old_tag: String,
    pub new_version: String,
    pub new_tag: String,
    pub changed: bool,
    /// Pinned to an exact version, so nothing was consulted.
    pub pinned: bool,
    /// A model this deployment added itself, whose pin comes from a
    /// repository rather than from the marketplace.
    pub manual: bool,
    /// The branch a manual entry follows, `null` for every other row.
    pub follow: Option<String>,
    /// Whether the lookup behind this row could be made at all. False is a
    /// manual entry whose repository or registry would not answer, which is
    /// "unchanged" without the claim that nothing has moved.
    pub checked: bool,
    /// The commit the new tag was built from, where the lookup said.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_commit: Option<String>,
}

impl ModelUpdate {
    /// The row this entry starts as: unchanged, and checked.
    fn unchanged(id: &str, entry: &crate::project::EnabledModel) -> ModelUpdate {
        ModelUpdate {
            id: id.to_string(),
            old_version: entry.version.clone(),
            old_tag: entry.image_tag.clone(),
            new_version: entry.version.clone(),
            new_tag: entry.image_tag.clone(),
            changed: false,
            pinned: entry.channel.is_none(),
            manual: false,
            follow: None,
            checked: true,
            new_commit: None,
        }
    }
}

impl UpdateReport {
    pub fn changed(&self) -> impl Iterator<Item = &ModelUpdate> {
        self.models.iter().filter(|m| m.changed)
    }
}

/// Refresh the registry, re-resolve every channel-following model, sync and
/// pull, then say what needs restarting.
pub fn run(ctx: &Ctx, args: &UpdateArgs) -> Result<()> {
    let mut project = ctx.project()?;
    // No fallback on purpose: an update from a stale catalogue is not an update.
    let registry = super::refreshed_registry_for(ctx, Some(&project))?;

    // The manual entries are resolved against their own repositories, which
    // is the one lookup the registry refresh above cannot make.
    let endpoints = manual::Endpoints::from_env(ctx.registry.offline);
    let models = plan(&project, &registry, &|id, entry| {
        newest_published(id, entry, &endpoints)
    })?;
    let latest = lookup_latest(
        &project.state.chap_image_tag,
        args.pin_chap_core,
        ctx.registry.timeout,
    );
    // Nothing here starts or stops a container, so this answer holds for the
    // whole run and the dry run answers the same question.
    let running = docker::running_containers(&project);
    let mut report = UpdateReport {
        registry: RegistryInfo {
            url: registry.url.clone(),
            provenance: registry.provenance.clone(),
        },
        models,
        chap_core: plan_chap_core(&project, latest, args.pin_chap_core),
        components: plan_components(&project),
        pulled: false,
        pulled_new: Vec::new(),
        restart_needed: Vec::new(),
        stack_running: running.as_ref().map(|c| !c.is_empty()),
        dry_run: args.dry_run,
    };

    // The plan first, before any of docker's own noise: it is what the pull
    // is about to act on, and it is the half a person reads.
    say(ctx, &plan_text(&report, &ctx.out));
    if args.dry_run {
        return ctx
            .out
            .emit(&report, || dry_run_line(updated(&report).as_deref()));
    }

    if report.chap_core.changed {
        apply_chap_core(&mut project, &mut report.chap_core, ctx.registry.timeout)?;
    }
    for change in report.models.iter().filter(|m| m.changed) {
        // A manual entry's definition carries the pin as well: the recorded
        // model says what runs, and `models-manual.yaml` says what the next
        // `models enable` would bring back.
        if let Some(manual) = project.state.manual.get_mut(&change.id) {
            manual.tag = change.new_tag.clone();
            if let Some(commit) = &change.new_commit {
                manual.commit = Some(commit.clone());
            }
        }
        let entry = project
            .state
            .models
            .get_mut(&change.id)
            .expect("plan only lists enabled models");
        entry.version = change.new_version.clone();
        entry.image_tag = change.new_tag.clone();
        refresh_env_pin(&project.dir, &tag_env_var(&change.id), &change.new_tag)?;
    }
    let synced = sync(&mut project, &registry, ctx.cli_version, false)?;
    for warning in &synced.warnings {
        output::warn(warning);
    }

    // After the sync, so these are the images the files pin now, and before
    // the pull, so the difference the pull makes can be seen at all.
    let images = docker::service_images(&project);
    let refs: Vec<String> = images.values().cloned().collect();
    let before = docker::image_ids(&refs);

    run_compose(&project, &["pull".to_string()])?;
    report.pulled = true;

    let after = docker::image_ids(&refs);
    report.pulled_new = pulled_new(&images, &before, &after);
    let running = running.unwrap_or_default();
    let builds =
        docker::container_builds(&running.iter().map(|c| c.id.clone()).collect::<Vec<_>>());
    let checks = service_checks(
        &running,
        &builds,
        &images,
        &after,
        &docker::config_hashes(&project),
    );
    report.restart_needed = restart_needed(&checks);

    let line = closing_line(
        updated(&report).as_deref(),
        &report.restart_needed,
        report.stack_running,
    );
    ctx.out.emit(&report, || ctx.out.backticks(&line))
}

/// Print one of the command's own sections, unless a parser is reading.
///
/// Under `--json` stdout carries exactly one document, printed at the end;
/// everything the human rendering would have said is left out rather than
/// mixed into it.
fn say(ctx: &Ctx, text: &str) {
    if !ctx.out.json && !text.is_empty() {
        print!("{text}");
    }
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

/// What a manually added model's branch resolves to today: `(commit, tag)`,
/// or `None` when the lookup could not be made.
pub type FollowFn<'a> = &'a dyn Fn(&str, &ManualModel) -> Option<(String, String)>;

/// Resolve every enabled model without changing anything: the marketplace
/// ones against the fresh registry, the manual ones against `follow`.
///
/// `follow` is what a manually added model's branch resolves to today -
/// `(commit, tag)` - or `None` when the lookup could not be made. A manual
/// entry is never an [`ChapError::UnknownModel`]: the deployment is where its
/// definition lives, so there is nothing for the catalogue to have dropped.
fn plan(project: &Project, registry: &Registry, follow: FollowFn) -> Result<Vec<ModelUpdate>> {
    let mut out = Vec::with_capacity(project.state.models.len());
    for (id, entry) in &project.state.models {
        let mut update = ModelUpdate::unchanged(id, entry);
        if let Some(manual) = project.state.manual.get(id) {
            update.manual = true;
            update.follow = manual.follow.clone();
            // A manual entry follows a branch or it is pinned; the channel
            // `.chaps/models.yaml` records only says which of the two.
            update.pinned = manual.follow.is_none();
            if manual.follow.is_some() {
                match follow(id, manual) {
                    Some((commit, tag)) => {
                        update.changed = tag != update.old_tag;
                        update.new_commit = Some(commit);
                        // The version of a manual entry is its tag: there is
                        // no version number to carry.
                        update.new_version = tag.clone();
                        update.new_tag = tag;
                    }
                    None => update.checked = false,
                }
            }
            out.push(update);
            continue;
        }
        if let Some(channel) = entry.channel {
            let model = registry
                .get(id)
                .ok_or_else(|| ChapError::UnknownModel(id.clone()))?;
            let version = model.resolve(&VersionSelector::Channel(channel))?;
            update.new_version = version.version.clone();
            update.new_tag = version.image_tag.clone();
            update.changed =
                update.new_version != update.old_version || update.new_tag != update.old_tag;
        }
        out.push(update);
    }
    Ok(out)
}

/// The newest published build of a manually added model's branch, as
/// `chaps update` asks for it.
///
/// Every failure is a warning and a `None`: a repository that will not answer
/// must not stop the marketplace half of the update, and leaving the pin
/// where it is is the safe half of that bargain.
fn newest_published(
    id: &str,
    entry: &ManualModel,
    endpoints: &manual::Endpoints,
) -> Option<(String, String)> {
    let (Some(repository), Some(branch)) = (&entry.repository, &entry.follow) else {
        output::warn(&format!(
            "{id} follows a branch but records no repository, so its pin cannot be checked"
        ));
        return None;
    };
    match manual::newest_published(repository, branch, endpoints) {
        Ok(Some(found)) => Some(found),
        Ok(None) => {
            output::warn(&format!(
                "{repository} has no published `sha-` build on {branch}, so {id} keeps the pin it has"
            ));
            None
        }
        Err(err) => {
            output::warn(&format!(
                "could not check {repository} for a newer build of {id} ({err:#}); \
                 the pin stays at {}",
                entry.tag
            ));
            None
        }
    }
}

/// Ask GitHub for the newest chap-core release, when its answer can matter.
///
/// The lookup is skipped for a moving tag nobody asked to pin and for a tag
/// that is not a version at all, so the unauthenticated rate limit is spent
/// only when there is a decision to make. A failed lookup is a warning, not a
/// failure: a marketplace update is still an update.
fn lookup_latest(current: &str, pin: bool, timeout: std::time::Duration) -> Option<String> {
    if !needs_release_lookup(current, pin) {
        return None;
    }
    match chapcore::latest_release(timeout) {
        Ok(tag) => Some(tag),
        Err(err) => {
            output::warn(&format!(
                "could not resolve the newest chap-core release ({err:#}); \
                 the chap-core pin stays at `{current}`"
            ));
            None
        }
    }
}

/// Whether the newest release can change what this run does.
fn needs_release_lookup(current: &str, pin: bool) -> bool {
    if chapcore::is_moving_tag(current) {
        return pin;
    }
    chapcore::release_version(current).is_some()
}

/// Work out what happens to the chap-core pin, without changing anything.
///
/// A release pin moves to a newer release; a moving tag stays put unless
/// `pin` says to convert it, because following `latest` is a choice the
/// operator made and `update` re-pulls it either way. A tag that is neither
/// (a `sha-` build, say) is not ours to move.
fn plan_chap_core(project: &Project, latest: Option<String>, pin: bool) -> ChapCoreUpdate {
    let current = project.state.chap_image_tag.clone();
    let moving = chapcore::is_moving_tag(&current);
    let new_tag = match (&latest, moving) {
        (Some(release), true) if pin => release.clone(),
        (Some(release), false) if chapcore::is_newer(release, &current) => release.clone(),
        _ => current.clone(),
    };
    ChapCoreUpdate {
        changed: new_tag != current,
        old_tag: current,
        new_tag,
        moving,
        latest_release: latest,
        compose_source: project.state.chap_compose_source.clone(),
    }
}

/// Move the chap-core pin: cache the compose file the new tag publishes,
/// rewrite the `.env` line and record the new tag.
///
/// A compose file that cannot be fetched is a warning, not a failure: the pin
/// itself is what the operator asked to move, and `compose.yml` keeps the
/// layout it has until the next successful fetch.
fn apply_chap_core(
    project: &mut Project,
    update: &mut ChapCoreUpdate,
    timeout: std::time::Duration,
) -> Result<()> {
    let tag = update.new_tag.clone();
    match chapcore::fetch_compose(&tag, timeout) {
        Ok(body) => {
            let chaps = project.chaps_dir();
            std::fs::create_dir_all(&chaps)
                .map_err(|e| anyhow::anyhow!("creating {}: {e}", chaps.display()))?;
            let path = chaps.join(cached_compose_file(&tag));
            std::fs::write(&path, &body)
                .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
            let source = ComposeSource::Fetched {
                url: chapcore::compose_url(&tag),
                tag: tag.clone(),
                sha256: chapcore::sha256_hex(body.as_bytes()),
            };
            project.state.chap_compose_source = source.clone();
            update.compose_source = source;
        }
        Err(err) => output::warn(&format!(
            "could not fetch chap-core's compose.ghcr.yml at {tag} ({err:#}); the image pin moves \
             but compose.yml keeps the layout it has"
        )),
    }

    // The tag has to reach .env as well, or compose still substitutes the old
    // value. Only the line this project wrote is touched.
    match set_env_chap_tag(&project.dir, &update.old_tag, &tag)? {
        EnvTag::Updated | EnvTag::NoFile => {}
        EnvTag::Commented => output::warn(&format!(
            ".env has {CHAP_TAG_ENV_VAR} commented out, so CHAP still follows the compose \
             default; set `{CHAP_TAG_ENV_VAR}={tag}` there to run the pin this update recorded"
        )),
        EnvTag::Foreign(value) => output::warn(&format!(
            ".env pins {CHAP_TAG_ENV_VAR}={value}, which is yours, not ours; it is left alone, so \
             CHAP keeps running {value} rather than {tag}"
        )),
        EnvTag::Absent => output::warn(&format!(
            ".env does not mention {CHAP_TAG_ENV_VAR}, so CHAP follows the compose default; \
             add `{CHAP_TAG_ENV_VAR}={tag}` there to run the pin this update recorded"
        )),
    }

    project.state.chap_image_tag = tag;
    Ok(())
}

fn run_compose(project: &Project, args: &[String]) -> Result<()> {
    let code = docker::run_compose(project, args)?;
    if code != 0 {
        return Err(ChapError::DockerFailed(code).into());
    }
    Ok(())
}

/// The chap-core row of the list, in the same shape as a model's.
fn chap_core_line(c: &ChapCoreUpdate) -> String {
    if c.changed {
        // The compose file only follows the pin once the run has actually
        // fetched it, and the plan is printed before that happens, so the
        // note is there for a report built after the fetch and nowhere else.
        let compose = match &c.compose_source {
            ComposeSource::Fetched { tag, .. } if tag == &c.new_tag => "  (compose.ghcr.yml too)",
            _ => "",
        };
        return format!("chap-core  {} -> {}{compose}", c.old_tag, c.new_tag);
    }
    if c.moving {
        return format!(
            "chap-core  {}  moving tag, re-pulled; pin it with `chaps update --pin-chap-core`",
            c.old_tag
        );
    }
    match &c.latest_release {
        Some(release) if release == &c.old_tag => {
            format!("chap-core  {}  unchanged, the newest release", c.old_tag)
        }
        Some(release) => format!(
            "chap-core  {}  unchanged (newest release: {release})",
            c.old_tag
        ),
        None => format!("chap-core  {}  unchanged", c.old_tag),
    }
}

/// What this run actually changed, as the closing line names it, or `None`
/// when the answer is "nothing".
///
/// Three clauses at most, in the order they matter: the pins that moved, and
/// then the images that arrived. A moving tag that pulled the same image
/// again is not a change and says nothing here, which is the whole point of
/// asking the image ids rather than trusting that a pull did something.
pub fn updated_phrase(
    models: usize,
    chap_core: Option<(&str, &str)>,
    pulled_new: &[String],
) -> Option<String> {
    let mut parts = Vec::new();
    if models > 0 {
        parts.push(format!(
            "{models} model pin{}",
            if models == 1 { "" } else { "s" }
        ));
    }
    if let Some((old, new)) = chap_core {
        parts.push(format!("chap-core {old} -> {new}"));
    }
    if !pulled_new.is_empty() {
        parts.push(format!(
            "new image{} for {}",
            if pulled_new.len() == 1 { "" } else { "s" },
            pulled_new.join(", ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// [`updated_phrase`] for a finished report.
fn updated(report: &UpdateReport) -> Option<String> {
    let chap_core = report.chap_core.changed.then_some((
        report.chap_core.old_tag.as_str(),
        report.chap_core.new_tag.as_str(),
    ));
    updated_phrase(report.changed().count(), chap_core, &report.pulled_new)
}

/// The one line the run ends on.
///
/// It answers the only question left: is there anything to do now? A stale
/// running service is that answer whether or not this run is what made it
/// stale, so the restart clause comes first and stands on its own.
pub fn closing_line(
    what: Option<&str>,
    restart_needed: &[String],
    running: Option<bool>,
) -> String {
    let head = match what {
        Some(what) => format!("updated {what}"),
        None => "already up to date".to_string(),
    };
    if !restart_needed.is_empty() {
        return format!(
            "{head}; restart needed: {} (run `chaps restart`)",
            restart_needed.join(", ")
        );
    }
    if what.is_none() {
        return head;
    }
    match running {
        Some(false) => {
            format!("{head}; CHAP is not running, the new versions start with `chaps up`")
        }
        Some(true) => format!("{head}; nothing needs a restart"),
        // Docker would not say what is running, so neither will we.
        None => format!("{head}; run `chaps restart` to apply it to whatever is running"),
    }
}

/// The one line a `--dry-run` ends on.
///
/// It says what the pins would do and nothing about restarting: the images
/// were not pulled and the files were not written, so what a running
/// container would then be out of step with does not exist yet.
pub fn dry_run_line(what: Option<&str>) -> String {
    match what {
        Some(what) => format!("would update {what}; nothing written (run `chaps update` to do it)"),
        None => "already up to date; nothing would change".to_string(),
    }
}

/// The plan: where the catalogue came from, and a row per thing that can move.
fn plan_text(report: &UpdateReport, out: &Out) -> String {
    let mut text = format!(
        "{} {} {}\n",
        out.key("registry:"),
        report.registry.url,
        out.dim(&format!("({})", report.registry.provenance.describe()))
    );
    if report.models.is_empty() {
        text.push_str(&out.dim("no models enabled"));
        text.push('\n');
    }
    for m in &report.models {
        // The new version is the only thing on the line that changed, so it is
        // the only thing that is coloured.
        let old = version_cell(m, &m.old_version, &m.old_tag);
        let line = if m.changed {
            format!(
                "  {}  {} -> {}",
                m.id,
                out.dim(&old),
                out.ok(&version_cell(m, &m.new_version, &m.new_tag))
            )
        } else {
            format!("  {}  {}  {}", m.id, out.dim(&old), out.dim(state_of(m)))
        };
        text.push_str(&line);
        text.push('\n');
    }
    text.push_str(&format!("  {}\n", chap_core_cell(out, &report.chap_core)));
    for component in &report.components {
        text.push_str(&format!("  {}\n", out.dim(&component_line(component))));
    }
    text
}

/// How one row's pin reads: `v1.0.0 (sha-fa880a1)` for a marketplace model,
/// and the tag alone for a manual one, whose version is its tag.
fn version_cell(m: &ModelUpdate, version: &str, tag: &str) -> String {
    if m.manual {
        return tag.to_string();
    }
    format!("v{version} ({tag})")
}

/// What a row that did not move says for itself.
fn state_of(m: &ModelUpdate) -> &'static str {
    if m.pinned {
        return "pinned, skipped";
    }
    if !m.checked {
        return "unchanged (could not check)";
    }
    "unchanged"
}

/// The chap-core row, with the new tag coloured the way a model row's is.
fn chap_core_cell(out: &Out, change: &ChapCoreUpdate) -> String {
    let line = chap_core_line(change);
    if !change.changed {
        return out.dim(&line);
    }
    match line.rsplit_once(" -> ") {
        Some((head, tail)) => format!("{} -> {}", out.dim(head), out.ok(tail)),
        None => out.ok(&line),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{EnableRequest, Selection, apply};
    use crate::project::ProjectState;
    use crate::registry::{Channel, load_embedded};

    fn project_with(ids: &[&str]) -> (tempfile::TempDir, Project, Registry) {
        let dir = tempfile::tempdir().unwrap();
        let registry = load_embedded().unwrap();
        let mut project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState::default(),
        };
        let sel = Selection {
            enable: ids.iter().map(|id| EnableRequest::new(*id)).collect(),
            disable: Vec::new(),
        };
        apply(&mut project, &registry, &sel, "0.1.0").unwrap();
        (dir, project, registry)
    }

    /// A lookup nobody is expected to make: every test that plans only
    /// marketplace models passes this, so a manual row would be obvious.
    fn no_lookup(id: &str, _: &ManualModel) -> Option<(String, String)> {
        panic!("{id} is not a manual model");
    }

    #[test]
    fn plan_reports_unchanged_when_the_channel_still_points_at_the_pin() {
        let (_dir, project, registry) = project_with(&["chapkit_ewars_model"]);
        let plan = plan(&project, &registry, &no_lookup).unwrap();
        assert_eq!(plan.len(), 1);
        assert!(!plan[0].changed && !plan[0].pinned);
        assert_eq!(plan[0].old_tag, plan[0].new_tag);
    }

    #[test]
    fn plan_moves_a_channel_pin_and_skips_an_exact_one() {
        let (_dir, mut project, mut registry) =
            project_with(&["chapkit_ewars_model", "auto_arima_chapkit"]);
        // Pretend the project was pinned to an older build of ewars.
        {
            let ewars = project.state.models.get_mut("chapkit_ewars_model").unwrap();
            ewars.version = "0.9.0".into();
            ewars.image_tag = "sha-0000000".into();
            ewars.channel = Some(Channel::Stable);
            let arima = project.state.models.get_mut("auto_arima_chapkit").unwrap();
            arima.channel = None;
            arima.image_tag = "sha-1111111".into();
        }
        // And that the marketplace moved auto_arima too; a pinned model ignores it.
        let arima = registry
            .models
            .iter_mut()
            .find(|m| m.id == "auto_arima_chapkit")
            .unwrap();
        arima.versions[0].image_tag = "sha-2222222".into();

        let plan = plan(&project, &registry, &no_lookup).unwrap();
        let ewars = plan.iter().find(|m| m.id == "chapkit_ewars_model").unwrap();
        assert!(ewars.changed);
        assert_eq!(ewars.old_version, "0.9.0");
        assert_eq!(ewars.new_version, "1.0.0");
        assert_eq!(ewars.new_tag, "sha-fa880a1");

        let arima = plan.iter().find(|m| m.id == "auto_arima_chapkit").unwrap();
        assert!(arima.pinned && !arima.changed);
        assert_eq!(arima.new_tag, "sha-1111111");
    }

    /// A project with one manually added model, enabled.
    fn project_with_manual(follow: Option<&str>) -> (tempfile::TempDir, Project, Registry) {
        let dir = tempfile::tempdir().unwrap();
        let mut project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState::default(),
        };
        project.state.manual.insert(
            "chapkit_ghr_model".to_string(),
            ManualModel {
                service_id: "chapkit-ghr-model".into(),
                display_name: "chapkit_ghr_model".into(),
                repository: Some("https://github.com/chap-models/chapkit_ghr_model".into()),
                image: "ghcr.io/chap-models/chapkit_ghr_model".into(),
                tag: "sha-1eb8cf1".into(),
                commit: None,
                follow: follow.map(str::to_string),
                data_dir: Some("/work/data".into()),
                user: Some("10001:10001".into()),
                runtime_amd64: true,
                added: "2026-09-24".into(),
            },
        );
        let mut registry = load_embedded().unwrap();
        assert!(registry.with_manual(&project.state.manual).is_empty());
        let mut request = EnableRequest::new("chapkit_ghr_model");
        request.selector = match follow {
            Some(_) => VersionSelector::Channel(Channel::Latest),
            None => VersionSelector::Exact("sha-1eb8cf1".into()),
        };
        let sel = Selection {
            enable: vec![request],
            disable: Vec::new(),
        };
        apply(&mut project, &registry, &sel, "0.1.0").unwrap();
        (dir, project, registry)
    }

    #[test]
    fn a_following_manual_model_moves_when_its_branch_has_a_newer_build() {
        let (_dir, project, registry) = project_with_manual(Some("main"));
        let plan = plan(&project, &registry, &|id, entry| {
            assert_eq!(id, "chapkit_ghr_model");
            assert_eq!(entry.follow.as_deref(), Some("main"));
            Some(("b1d6c31deadbeef".to_string(), "sha-b1d6c31".to_string()))
        })
        .unwrap();
        assert_eq!(plan.len(), 1);
        let row = &plan[0];
        assert!(row.manual && row.changed && row.checked && !row.pinned);
        assert_eq!(row.follow.as_deref(), Some("main"));
        assert_eq!(row.old_tag, "sha-1eb8cf1");
        assert_eq!(row.new_tag, "sha-b1d6c31");
        // The version of a manual entry is its tag, which is what the line
        // prints in place of a version number.
        assert_eq!(row.new_version, "sha-b1d6c31");
        assert_eq!(row.new_commit.as_deref(), Some("b1d6c31deadbeef"));
        let text = plan_text(
            &UpdateReport {
                registry: RegistryInfo {
                    url: "https://example.test/registry.yaml".into(),
                    provenance: Provenance::Network,
                },
                models: plan,
                chap_core: chap_core("latest", "latest", None),
                components: Vec::new(),
                pulled: false,
                pulled_new: Vec::new(),
                restart_needed: Vec::new(),
                stack_running: Some(false),
                dry_run: true,
            },
            &Out::default(),
        );
        assert!(
            text.contains("  chapkit_ghr_model  sha-1eb8cf1 -> sha-b1d6c31\n"),
            "{text}"
        );
    }

    #[test]
    fn a_following_manual_model_that_has_not_moved_is_unchanged() {
        let (_dir, project, registry) = project_with_manual(Some("main"));
        let plan = plan(&project, &registry, &|_, _| {
            Some(("1eb8cf1".to_string(), "sha-1eb8cf1".to_string()))
        })
        .unwrap();
        assert!(plan[0].manual && !plan[0].changed && plan[0].checked);
        assert_eq!(state_of(&plan[0]), "unchanged");
    }

    /// A repository that would not answer leaves the pin alone, and the row
    /// says that nothing was established rather than that nothing moved.
    #[test]
    fn a_manual_model_whose_lookup_failed_says_it_could_not_check() {
        let (_dir, project, registry) = project_with_manual(Some("main"));
        let plan = plan(&project, &registry, &|_, _| None).unwrap();
        assert!(plan[0].manual && !plan[0].changed && !plan[0].checked);
        assert_eq!(state_of(&plan[0]), "unchanged (could not check)");
        assert_eq!(plan[0].new_tag, "sha-1eb8cf1");
    }

    #[test]
    fn a_pinned_manual_model_is_never_looked_up() {
        let (_dir, project, registry) = project_with_manual(None);
        let plan = plan(&project, &registry, &|id, _| {
            panic!("{id} is pinned and must not be checked")
        })
        .unwrap();
        assert!(plan[0].manual && plan[0].pinned && !plan[0].changed);
        assert_eq!(state_of(&plan[0]), "pinned, skipped");
        assert_eq!(
            version_cell(&plan[0], "sha-1eb8cf1", "sha-1eb8cf1"),
            "sha-1eb8cf1"
        );
    }

    /// A manual entry is never an unknown model: the deployment is where its
    /// definition lives, so there is nothing the catalogue could have dropped.
    #[test]
    fn a_manual_model_the_marketplace_never_had_is_not_an_unknown_model() {
        let (_dir, project, _) = project_with_manual(Some("main"));
        // The catalogue without the manual entry folded in, which is what a
        // registry refresh hands back before `with_manual` runs.
        let registry = load_embedded().unwrap();
        assert!(registry.get("chapkit_ghr_model").is_none());
        let plan = plan(&project, &registry, &|_, _| None).unwrap();
        assert_eq!(plan.len(), 1);
        assert!(plan[0].manual);
    }

    #[test]
    fn plan_fails_when_a_channel_model_left_the_registry() {
        let (_dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let entry = project.state.models.remove("chapkit_ewars_model").unwrap();
        project.state.models.insert("vanished".into(), entry);
        let err = plan(&project, &registry, &no_lookup).expect_err("unknown model");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownModel(id)) if id == "vanished"
        ));
    }

    #[test]
    fn the_plan_lists_each_model_before_anything_is_pulled() {
        let report = UpdateReport {
            registry: RegistryInfo {
                url: "https://example.test/registry.yaml".into(),
                provenance: Provenance::Network,
            },
            models: vec![
                ModelUpdate {
                    id: "a".into(),
                    old_version: "1.0.0".into(),
                    old_tag: "sha-1111111".into(),
                    new_version: "1.1.0".into(),
                    new_tag: "sha-2222222".into(),
                    changed: true,
                    pinned: false,
                    ..row("a")
                },
                ModelUpdate {
                    id: "b".into(),
                    old_version: "1.0.0".into(),
                    old_tag: "sha-3333333".into(),
                    new_version: "1.0.0".into(),
                    new_tag: "sha-3333333".into(),
                    changed: false,
                    pinned: true,
                    ..row("b")
                },
            ],
            chap_core: chap_core("latest", "latest", None),
            components: Vec::new(),
            pulled: false,
            pulled_new: Vec::new(),
            restart_needed: Vec::new(),
            stack_running: Some(true),
            dry_run: true,
        };
        let text = plan_text(&report, &Out::default());
        assert!(text.starts_with("registry: https://example.test/registry.yaml (network)\n"));
        assert!(text.contains("  a  v1.0.0 (sha-1111111) -> v1.1.0 (sha-2222222)\n"));
        assert!(text.contains("  b  v1.0.0 (sha-3333333)  pinned, skipped\n"));
        assert!(text.contains(
            "  chap-core  latest  moving tag, re-pulled; pin it with `chaps update --pin-chap-core`\n"
        ));
        // The plan is the plan: nothing in it claims anything has happened.
        assert!(!text.contains("restart"), "{text}");
        assert!(!text.contains("pulled the"), "{text}");

        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["dry_run"], true);
        assert_eq!(value["stack_running"], true);
        assert_eq!(value["restart_needed"], serde_json::json!([]));
        assert_eq!(value["pulled_new"], serde_json::json!([]));
        assert_eq!(value["pulled"], false);
        assert!(
            value.get("restarted").is_none() && value.get("restart").is_none(),
            "update no longer touches containers: {value}"
        );
        assert_eq!(value["models"][0]["changed"], true);
        assert_eq!(value["registry"]["provenance"]["kind"], "network");
        assert_eq!(value["chap_core"]["old_tag"], "latest");
        assert_eq!(value["chap_core"]["changed"], false);
        assert_eq!(value["chap_core"]["moving"], true);
        assert_eq!(value["chap_core"]["compose_source"]["kind"], "embedded");
    }

    /// A row with everything a marketplace model's is, for the fields a test
    /// does not care about.
    fn row(id: &str) -> ModelUpdate {
        ModelUpdate::unchanged(
            id,
            &crate::project::EnabledModel {
                service_id: id.to_string(),
                image: format!("ghcr.io/chap-models/{id}"),
                image_tag: "sha-0000000".into(),
                version: "1.0.0".into(),
                channel: Some(Channel::Stable),
                host_port: None,
                data_dir: "/work/data".into(),
                user: "chapkit:chapkit".into(),
                platform: None,
                compose_file: format!("compose.{id}.yml"),
            },
        )
    }

    /// A [`ServiceCheck`] as `service_checks` would have built it.
    fn check(service: &str, images: (&str, &str), hashes: (&str, &str)) -> ServiceCheck {
        ServiceCheck {
            service: service.to_string(),
            running_image: images.0.to_string(),
            pulled_image: images.1.to_string(),
            running_hash: hashes.0.to_string(),
            current_hash: hashes.1.to_string(),
        }
    }

    #[test]
    fn a_service_is_stale_when_its_image_or_its_configuration_moved() {
        // Everything as it was created.
        assert!(!needs_restart(&check(
            "chap",
            ("sha256:a", "sha256:a"),
            ("h1", "h1")
        )));
        // The pull moved the tag under it.
        assert!(needs_restart(&check(
            "ocs",
            ("sha256:a", "sha256:b"),
            ("h1", "h1")
        )));
        // The files changed under it: a pin moved, a port, a secret.
        assert!(needs_restart(&check(
            "chap",
            ("sha256:a", "sha256:a"),
            ("h1", "h2")
        )));
        // Both at once is still one restart.
        assert!(needs_restart(&check(
            "chap",
            ("sha256:a", "sha256:b"),
            ("h1", "h2")
        )));
    }

    #[test]
    fn an_answer_docker_did_not_give_is_never_a_reason_to_restart() {
        for c in [
            check("chap", ("", "sha256:b"), ("h1", "h1")),
            check("chap", ("sha256:a", ""), ("h1", "h1")),
            check("chap", ("sha256:a", "sha256:a"), ("", "h2")),
            check("chap", ("sha256:a", "sha256:a"), ("h1", "")),
            check("chap", ("", ""), ("", "")),
        ] {
            assert!(!needs_restart(&c), "{c:?}");
        }
    }

    #[test]
    fn the_stale_services_keep_the_order_they_were_checked_in() {
        let checks = vec![
            check("chap", ("sha256:a", "sha256:b"), ("h1", "h1")),
            check("postgres", ("sha256:c", "sha256:c"), ("h2", "h2")),
            check("worker", ("sha256:a", "sha256:a"), ("h3", "h4")),
        ];
        assert_eq!(restart_needed(&checks), vec!["chap", "worker"]);
        assert!(restart_needed(&[]).is_empty());
    }

    /// Everything `service_checks` reads, for a two-service project whose
    /// `chap` image moved under it and whose `redis` did not.
    struct Maps {
        builds: BTreeMap<String, docker::ContainerBuild>,
        images: BTreeMap<String, String>,
        ids: BTreeMap<String, String>,
        hashes: BTreeMap<String, String>,
    }

    fn maps() -> Maps {
        let build = |image: &str, hash: &str| docker::ContainerBuild {
            image_id: image.to_string(),
            config_hash: hash.to_string(),
        };
        Maps {
            builds: BTreeMap::from([
                ("id-chap".to_string(), build("sha256:old", "h1")),
                ("id-redis".to_string(), build("sha256:r", "h2")),
            ]),
            images: BTreeMap::from([
                ("chap".to_string(), "ghcr.io/chap:latest".to_string()),
                ("redis".to_string(), "valkey:8".to_string()),
            ]),
            ids: BTreeMap::from([
                ("ghcr.io/chap:latest".to_string(), "sha256:new".to_string()),
                ("valkey:8".to_string(), "sha256:r".to_string()),
            ]),
            hashes: BTreeMap::from([
                ("chap".to_string(), "h1".to_string()),
                ("redis".to_string(), "h2".to_string()),
            ]),
        }
    }

    fn container(service: &str, id: &str) -> docker::Container {
        docker::Container {
            service: service.to_string(),
            id: id.to_string(),
            state: "running".to_string(),
            ..docker::Container::default()
        }
    }

    #[test]
    fn every_running_service_is_paired_with_what_it_would_be_made_of_today() {
        let m = maps();
        let running = vec![container("chap", "id-chap"), container("redis", "id-redis")];
        let checks = service_checks(&running, &m.builds, &m.images, &m.ids, &m.hashes);
        assert_eq!(checks.len(), 2);
        assert_eq!(
            checks[0],
            check("chap", ("sha256:old", "sha256:new"), ("h1", "h1"))
        );
        assert_eq!(
            checks[1],
            check("redis", ("sha256:r", "sha256:r"), ("h2", "h2"))
        );
        assert_eq!(restart_needed(&checks), vec!["chap"]);
    }

    #[test]
    fn a_container_docker_would_not_describe_costs_only_its_own_row() {
        let m = maps();
        // A service the files no longer mention, and one `docker inspect`
        // said nothing about: neither is a reason to claim a restart.
        let running = vec![
            container("gone", "id-gone"),
            container("chap", "id-unknown"),
            // A stopped container is not a running service.
            docker::Container {
                state: "exited".to_string(),
                ..container("redis", "id-redis")
            },
        ];
        let checks = service_checks(&running, &m.builds, &m.images, &m.ids, &m.hashes);
        assert_eq!(checks.len(), 2, "the exited one is left out");
        assert!(restart_needed(&checks).is_empty(), "{checks:?}");
    }

    #[test]
    fn the_pull_is_new_only_when_the_image_id_moved() {
        let images = BTreeMap::from([
            ("ocs".to_string(), "ghcr.io/ocs:main".to_string()),
            ("s3".to_string(), "minio:latest".to_string()),
            ("chap".to_string(), "ghcr.io/chap:v2.3.1".to_string()),
            ("gone".to_string(), "ghcr.io/nowhere:1".to_string()),
        ]);
        let before = BTreeMap::from([
            ("ghcr.io/ocs:main".to_string(), "sha256:a".to_string()),
            ("minio:latest".to_string(), "sha256:b".to_string()),
        ]);
        let after = BTreeMap::from([
            // The moving tag moved.
            ("ghcr.io/ocs:main".to_string(), "sha256:c".to_string()),
            // And this one pointed at the same image as yesterday.
            ("minio:latest".to_string(), "sha256:b".to_string()),
            // This one was not on the machine at all before.
            ("ghcr.io/chap:v2.3.1".to_string(), "sha256:d".to_string()),
        ]);
        // `gone` is absent from both: the pull did not fetch it, which is not
        // the same as it having fetched something new.
        assert_eq!(pulled_new(&images, &before, &after), vec!["chap", "ocs"]);
    }

    /// A [`ChapCoreUpdate`] as `plan_chap_core` would have built it.
    fn chap_core(old: &str, new: &str, latest: Option<&str>) -> ChapCoreUpdate {
        ChapCoreUpdate {
            old_tag: old.to_string(),
            new_tag: new.to_string(),
            changed: old != new,
            moving: chapcore::is_moving_tag(old),
            latest_release: latest.map(str::to_string),
            compose_source: ComposeSource::Embedded,
        }
    }

    /// A project whose chap-core pin is `tag`.
    fn pinned_to(tag: &str) -> Project {
        Project {
            dir: std::path::PathBuf::from("/nonexistent"),
            state: ProjectState {
                chap_image_tag: tag.to_string(),
                ..ProjectState::default()
            },
        }
    }

    #[test]
    fn a_release_pin_moves_to_a_newer_release_and_no_further() {
        let plan = plan_chap_core(&pinned_to("v2.3.0"), Some("v2.3.1".into()), false);
        assert!(plan.changed && !plan.moving);
        assert_eq!(
            (plan.old_tag.as_str(), plan.new_tag.as_str()),
            ("v2.3.0", "v2.3.1")
        );
        assert_eq!(plan.latest_release.as_deref(), Some("v2.3.1"));

        // Already current, and never backwards.
        let plan = plan_chap_core(&pinned_to("v2.3.1"), Some("v2.3.1".into()), false);
        assert!(!plan.changed);
        let plan = plan_chap_core(&pinned_to("v2.4.0"), Some("v2.3.1".into()), false);
        assert!(!plan.changed, "a release ahead of the newest one stays");

        // A failed lookup leaves everything where it is.
        let plan = plan_chap_core(&pinned_to("v2.3.0"), None, false);
        assert!(!plan.changed);
        assert_eq!(plan.new_tag, "v2.3.0");
    }

    #[test]
    fn a_moving_tag_only_moves_when_asked_to() {
        for tag in ["latest", "master", "dev"] {
            let plan = plan_chap_core(&pinned_to(tag), Some("v2.3.1".into()), false);
            assert!(!plan.changed, "{tag} follows itself");
            assert!(plan.moving, "{tag}");
            assert_eq!(plan.new_tag, tag);

            let plan = plan_chap_core(&pinned_to(tag), Some("v2.3.1".into()), true);
            assert!(plan.changed, "--pin-chap-core pins {tag}");
            assert_eq!(plan.new_tag, "v2.3.1");
            assert!(plan.moving, "the tag it came from was a moving one");
        }

        // Nothing to pin it to: the flag cannot invent a release.
        let plan = plan_chap_core(&pinned_to("latest"), None, true);
        assert!(!plan.changed);
    }

    #[test]
    fn a_tag_that_is_neither_a_release_nor_moving_is_left_alone() {
        let plan = plan_chap_core(&pinned_to("sha-abcdef0"), Some("v2.3.1".into()), true);
        assert!(!plan.changed && !plan.moving);
        assert_eq!(plan.new_tag, "sha-abcdef0");
    }

    #[test]
    fn the_release_lookup_is_skipped_when_it_cannot_matter() {
        assert!(needs_release_lookup("v2.3.0", false));
        assert!(needs_release_lookup("v2.3.0", true));
        assert!(!needs_release_lookup("latest", false));
        assert!(needs_release_lookup("latest", true));
        assert!(!needs_release_lookup("sha-abcdef0", false));
        assert!(!needs_release_lookup("sha-abcdef0", true));
    }

    #[test]
    fn the_chap_core_line_says_what_happened() {
        let mut moved = chap_core("v2.3.0", "v2.3.1", Some("v2.3.1"));
        // A plan, so the compose file has not moved with the pin yet.
        assert_eq!(chap_core_line(&moved), "chap-core  v2.3.0 -> v2.3.1");
        moved.compose_source = ComposeSource::Fetched {
            url: chapcore::compose_url("v2.3.1"),
            tag: "v2.3.1".into(),
            sha256: "a".repeat(64),
        };
        assert_eq!(
            chap_core_line(&moved),
            "chap-core  v2.3.0 -> v2.3.1  (compose.ghcr.yml too)"
        );
        let current = chap_core("v2.3.1", "v2.3.1", Some("v2.3.1"));
        assert_eq!(
            chap_core_line(&current),
            "chap-core  v2.3.1  unchanged, the newest release"
        );
        let ahead = chap_core("v2.4.0", "v2.4.0", Some("v2.3.1"));
        assert_eq!(
            chap_core_line(&ahead),
            "chap-core  v2.4.0  unchanged (newest release: v2.3.1)"
        );

        let unknown = chap_core("sha-abcdef0", "sha-abcdef0", None);
        assert_eq!(
            chap_core_line(&unknown),
            "chap-core  sha-abcdef0  unchanged"
        );
    }

    #[test]
    fn the_updated_phrase_names_only_what_actually_moved() {
        assert_eq!(updated_phrase(0, None, &[]), None);
        assert_eq!(updated_phrase(1, None, &[]).as_deref(), Some("1 model pin"));
        assert_eq!(
            updated_phrase(3, None, &[]).as_deref(),
            Some("3 model pins")
        );
        assert_eq!(
            updated_phrase(0, Some(("v2.3.0", "v2.3.1")), &[]).as_deref(),
            Some("chap-core v2.3.0 -> v2.3.1")
        );
        assert_eq!(
            updated_phrase(0, None, &["ocs".to_string()]).as_deref(),
            Some("new image for ocs")
        );
        assert_eq!(
            updated_phrase(
                2,
                Some(("v2.3.0", "v2.3.1")),
                &["chap".to_string(), "worker".to_string()]
            )
            .as_deref(),
            Some("2 model pins, chap-core v2.3.0 -> v2.3.1, new images for chap, worker")
        );
    }

    #[test]
    fn the_closing_line_is_one_of_four_things() {
        let stale = ["chap".to_string(), "worker".to_string()];

        // Nothing moved, nothing is stale: the short answer.
        assert_eq!(closing_line(None, &[], Some(true)), "already up to date");
        // A moving tag that pulled the image this machine already had counts
        // as nothing moving, because `updated` never names it.
        assert_eq!(closing_line(None, &[], Some(false)), "already up to date");

        // Something moved and running services are behind it.
        assert_eq!(
            closing_line(Some("1 model pin"), &stale, Some(true)),
            "updated 1 model pin; restart needed: chap, worker (run `chaps restart`)"
        );
        // Stale without this run having moved anything: someone edited `.env`
        // and never applied it. Still worth saying, and still one line.
        assert_eq!(
            closing_line(None, &stale, Some(true)),
            "already up to date; restart needed: chap, worker (run `chaps restart`)"
        );

        // Something moved and there is nothing running to be behind it.
        assert_eq!(
            closing_line(Some("chap-core v2.3.0 -> v2.3.1"), &[], Some(false)),
            "updated chap-core v2.3.0 -> v2.3.1; CHAP is not running, the new versions start \
             with `chaps up`"
        );
        // Something moved, the stack is up, and none of it was affected.
        assert_eq!(
            closing_line(Some("1 model pin"), &[], Some(true)),
            "updated 1 model pin; nothing needs a restart"
        );
        // Docker would not say what is running, so neither do we.
        assert_eq!(
            closing_line(Some("1 model pin"), &[], None),
            "updated 1 model pin; run `chaps restart` to apply it to whatever is running"
        );
    }

    #[test]
    fn a_dry_run_says_what_would_happen_and_claims_nothing_else() {
        assert_eq!(
            dry_run_line(None),
            "already up to date; nothing would change"
        );
        assert_eq!(
            dry_run_line(Some("1 model pin")),
            "would update 1 model pin; nothing written (run `chaps update` to do it)"
        );
        // A dry run pulls nothing, so it never claims a restart is needed.
        assert!(!dry_run_line(Some("1 model pin")).contains("restart"));
    }
}
