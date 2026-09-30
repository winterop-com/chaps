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
//! `--chap-tag` is the other direction: not forward, but somewhere else
//! entirely. It is the same run with chap-core's destination decided by the
//! operator rather than by the release feed, and the one move that can cost
//! data - a release older than the one running, or a release after a moving
//! tag, since `dev` and `master` are ahead of every release - is named and
//! confirmed before anything is fetched, written or pulled. `--list-tags`
//! answers where a deployment can go, and writes nothing at all.
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
use crate::compose::resolve::{self, UserSource};
use crate::compose::sync::{EnvTag, refresh_env_pin, set_env_chap_tag};
use crate::compose::{API_SERVICE, sync, tag_env_var};
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
    /// `None` for a deployment chap-core is not a component of: there is no
    /// pin to move and no release to look up.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chap_core: Option<ChapCoreUpdate>,
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
    /// The tag `--chap-tag` asked for, when this run was given one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested: Option<String>,
    /// The git ref whose `compose.ghcr.yml` goes with [`new_tag`]: the tag
    /// itself for a release or a branch, the newest release for `latest`
    /// (which is the release that publishes it), and `None` when there is
    /// nothing to fetch or nothing that could be resolved.
    ///
    /// [`new_tag`]: ChapCoreUpdate::new_tag
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compose_ref: Option<String>,
    /// Whether this move can run an older schema against a database a newer
    /// chap-core has already migrated.
    pub backwards: bool,
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
    if components.dhis2.enabled {
        out.push(component_update(
            crate::compose::DHIS2_SERVICE,
            &components.dhis2.image,
            components::DHIS2_TAG_ENV_VAR,
            &components.dhis2.image_tag,
            &env,
        ));
    }
    out
}

/// What a run that is about to re-pull the DHIS2 image is told while `dhis2_db`
/// is already there.
///
/// Not the line `chaps components enable dhis2` prints. That one is about a tag
/// an operator asked to change, and names the two tags. Here nothing on the
/// screen moves at all: `dhis2/core:2.42` is a minor line, so the same tag is a
/// newer patch release tomorrow, and a pin an operator set by hand is re-pulled
/// too. That is the whole reason the line exists - the pull is silent, the next
/// `chaps up` migrates the schema, and DHIS2 migrates forward only, so the only
/// way back from a migration that was not wanted is the archive taken before it.
pub fn dhis2_pull_warning(image: &str, tag: &str) -> String {
    format!(
        "this pull can bring a newer `{image}:{tag}`, and a newer DHIS2 migrates `dhis2_db` \
         irreversibly on the next `chaps up`: run `chaps backup create` first - an older image \
         on a migrated database answers healthy while every API request 404s"
    )
}

/// That warning, when this run has earned it: the component is enabled and its
/// database volume is already on this machine.
///
/// Best-effort about docker like every other step that needs it: no CLI and no
/// daemon both answer "no such volume", which is the quiet answer - a warning
/// about a database that may not exist would be worse than none.
fn dhis2_pull_note(project: &Project, components: &[ComponentUpdate]) -> Option<String> {
    let row = components
        .iter()
        .find(|c| c.name == crate::compose::DHIS2_SERVICE)?;
    let volume = project.prefixed_volume(crate::compose::render::DHIS2_DB_VOLUME)?;
    if !docker::volume_exists(&volume) {
        return None;
    }
    Some(dhis2_pull_warning(&row.image, &row.tag))
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
    /// The account the model runs as today.
    pub old_user: String,
    /// What the image at the new tag declares, which is the same string
    /// unless the image changed what it runs as between the two builds.
    pub new_user: String,
    /// Where [`new_user`] came from.
    ///
    /// [`new_user`]: ModelUpdate::new_user
    pub user_from: UserSource,
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
            old_user: entry.user.clone(),
            new_user: entry.user.clone(),
            user_from: entry.user_from,
        }
    }

    /// The user this row moves, when it moves one.
    fn moved_user(&self) -> Option<(&str, &str)> {
        (self.new_user != self.old_user).then_some((&self.old_user, &self.new_user))
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
    // A listing writes nothing and asks the marketplace nothing, so it is
    // answered before the refresh that the rest of the command needs.
    if args.list_tags {
        return list_tags(ctx, &project);
    }
    // A chap-core built from a checkout has no pin either: the checkout is
    // the version, and `chaps up` builds it as it is.
    if let ComposeSource::Checkout { path } = &project.state.chap_compose_source
        && (args.chap_tag.is_some() || args.pin_chap_core)
    {
        return Err(anyhow::anyhow!(
            "this deployment builds chap-core from the checkout at {path}; check out the \
             version you want there and run `chaps up`"
        ));
    }
    let own_chap_core = project
        .state
        .components
        .is_enabled(components::Component::ChapCore);
    let has_chap_core = own_chap_core
        && !matches!(
            project.state.chap_compose_source,
            ComposeSource::Checkout { .. }
        );
    if !own_chap_core && (args.chap_tag.is_some() || args.pin_chap_core) {
        return Err(anyhow::anyhow!(
            "{} moves chap-core's pin, and this deployment has no chap-core; \
             `chaps components enable chap-core` adds it",
            if args.chap_tag.is_some() {
                "--chap-tag"
            } else {
                "--pin-chap-core"
            }
        ));
    }
    // The tag `--chap-tag` names is checked before anything else happens: a
    // typo should cost one lookup, not a marketplace refresh and a pull.
    let requested = match &args.chap_tag {
        Some(tag) => Some(check_chap_tag(
            tag.trim(),
            ctx.registry.offline,
            ctx.registry.timeout,
        )?),
        None => None,
    };
    let requested = requested.as_deref();
    // No fallback on purpose: an update from a stale catalogue is not an update.
    let registry = super::refreshed_registry_for(ctx, Some(&project))?;

    // The manual entries are resolved against their own repositories, which
    // is the one lookup the registry refresh above cannot make.
    let endpoints = manual::Endpoints::from_env(ctx.registry.offline);
    let models = plan(&project, &registry, &|id, entry| {
        newest_published(id, entry, &endpoints)
    })?;
    // A new tag is a new image, and an image can change what it runs as
    // between two builds - which is the difference between a model that works
    // and one whose own binaries it may not execute. Only the rows that move
    // are asked, and only about the user: moving the data directory here
    // would point the service at an empty path next to a volume holding its
    // database.
    let mut models = models;
    resolve_users(&project, &mut models, &endpoints);
    // Without chap-core there is no pin for the newest release to move.
    let latest = has_chap_core
        .then(|| {
            lookup_latest(
                &project.state.chap_image_tag,
                args.pin_chap_core,
                requested,
                ctx.registry.timeout,
            )
        })
        .flatten();
    // Nothing here starts or stops a container, so this answer holds for the
    // whole run and the dry run answers the same question.
    let running = docker::running_containers(&project);
    let mut report = UpdateReport {
        registry: RegistryInfo {
            url: registry.url.clone(),
            provenance: registry.provenance.clone(),
        },
        models,
        chap_core: has_chap_core
            .then(|| plan_chap_core(&project, latest, args.pin_chap_core, requested)),
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
    // Going back to an older chap-core is the one move this command makes
    // that can cost data, so it is named before anything is fetched, written
    // or pulled - in the dry run too, where it is half of what there is to see.
    if let Some(core) = report.chap_core.as_ref().filter(|c| c.backwards) {
        output::warn(&backwards_warning(&core.old_tag, &core.new_tag));
    }
    // And the same for DHIS2, which needs no move to be at risk: the pull alone
    // can bring an image that migrates `dhis2_db` on the next `chaps up`, and
    // nothing in the plan above would show it. In the dry run too, where it is
    // the only thing this run has to say about that.
    if let Some(warning) = dhis2_pull_note(&project, &report.components) {
        output::warn(&warning);
    }
    if args.dry_run {
        return ctx
            .out
            .emit(&report, || dry_run_line(updated(&report).as_deref()));
    }
    // And confirmed, once it is about to actually happen.
    if let Some(core) = report.chap_core.as_ref().filter(|c| c.backwards)
        && !args.yes
    {
        confirm_backwards(ctx, core)?;
    }

    if let Some(core) = report.chap_core.as_mut().filter(|c| c.changed) {
        apply_chap_core(&mut project, core, ctx.registry.timeout)?;
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
        entry.user = change.new_user.clone();
        entry.user_from = change.user_from;
        refresh_env_pin(&project.dir, &tag_env_var(&change.id), &change.new_tag)?;
    }
    let synced = sync(&mut project, &registry, false)?;
    for warning in &synced.warnings {
        output::warn(warning);
    }

    // After the sync, so these are the images the files pin now, and before
    // the pull, so the difference the pull makes can be seen at all.
    let images = docker::service_images(&project);
    let refs: Vec<String> = images.values().cloned().collect();
    let before = docker::image_ids(&refs);

    // A pull that fails once chap-core's pin has moved leaves the deployment
    // describing images docker could not fetch - upstream publishing a tag
    // for one of its images and not the other is enough to get there - so the
    // failure says where the pin is now and how to put it back.
    if let Err(err) = run_compose(&project, &["pull".to_string()]) {
        if let Some(core) = report.chap_core.as_ref().filter(|c| c.changed) {
            return Err(err.context(pull_failed_after_switch(&core.old_tag, &core.new_tag)));
        }
        return Err(err);
    }
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

/// Re-read the user of every marketplace model the plan moves to a new tag.
///
/// A manual entry is left alone: `models add` resolved its user against the
/// image, and `models remove` plus a fresh add is how that answer is revisited.
/// A row that is not moving is left alone too - the image behind it has not
/// changed, so neither has what it runs as.
fn resolve_users(project: &Project, models: &mut [ModelUpdate], endpoints: &manual::Endpoints) {
    for update in models.iter_mut().filter(|m| m.changed && !m.manual) {
        let Some(entry) = project.state.models.get(&update.id) else {
            continue;
        };
        let resolution = resolve::from_image(
            &resolve::Request {
                id: &update.id,
                image: &entry.image,
                image_tag: &update.new_tag,
                data_dir_flag: None,
                user_flag: None,
            },
            endpoints,
        );
        for note in &resolution.notes {
            output::warn(note);
        }
        update.new_user = resolution.user;
        update.user_from = resolution.user_from;
    }
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
fn lookup_latest(
    current: &str,
    pin: bool,
    requested: Option<&str>,
    timeout: std::time::Duration,
) -> Option<String> {
    if !needs_release_lookup(current, pin, requested) {
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
///
/// A run given `--chap-tag` has been told where to go, so the only reason
/// left to ask is `latest`: the deployment follows that moving tag, and the
/// compose file that goes with it is the one the newest release publishes.
pub fn needs_release_lookup(current: &str, pin: bool, requested: Option<&str>) -> bool {
    if let Some(tag) = requested {
        return tag == chapcore::LATEST_TAG;
    }
    if chapcore::is_moving_tag(current) {
        return pin;
    }
    chapcore::release_version(current).is_some()
}

/// The git ref whose `compose.ghcr.yml` goes with an image tag.
///
/// A release tag and a branch name are both refs of the repository, so the
/// file is fetched at the tag itself. `latest` is not a ref at all - it is an
/// image tag the newest release publishes - so the file comes from that
/// release, and a run that could not resolve it has no ref to fetch.
pub fn compose_ref(tag: &str, latest: Option<&str>) -> Option<String> {
    if tag == chapcore::LATEST_TAG {
        return latest.map(str::to_string);
    }
    Some(tag.to_string())
}

/// Work out what happens to the chap-core pin, without changing anything.
///
/// `requested` is `--chap-tag`, and it decides on its own: the operator said
/// where this deployment goes. Without it, a release pin moves to a newer
/// release and a moving tag stays put unless `pin` says to convert it,
/// because following `latest` is a choice the operator made and `update`
/// re-pulls it either way. A tag that is neither (a `sha-` build, say) is not
/// ours to move.
fn plan_chap_core(
    project: &Project,
    latest: Option<String>,
    pin: bool,
    requested: Option<&str>,
) -> ChapCoreUpdate {
    let current = project.state.chap_image_tag.clone();
    let moving = chapcore::is_moving_tag(&current);
    let new_tag = match requested {
        Some(tag) => tag.to_string(),
        None => match (&latest, moving) {
            (Some(release), true) if pin => release.clone(),
            (Some(release), false) if chapcore::is_newer(release, &current) => release.clone(),
            _ => current.clone(),
        },
    };
    let changed = new_tag != current;
    ChapCoreUpdate {
        changed,
        backwards: changed && chapcore::is_backwards(&current, &new_tag),
        compose_ref: changed
            .then(|| compose_ref(&new_tag, latest.as_deref()))
            .flatten(),
        old_tag: current,
        new_tag,
        moving,
        latest_release: latest,
        compose_source: project.state.chap_compose_source.clone(),
        requested: requested.map(str::to_string),
    }
}

/// Check the tag `--chap-tag` was given, and hand it back as it will be
/// recorded.
///
/// A release tag has to be one chap-core has actually released, because a tag
/// nobody published is a deployment that will not start; a moving tag is one
/// of the three by definition; anything else is taken as an exact pin, with
/// the note that nothing will ever move it again. A lookup that could not be
/// made - `--offline`, a rate limit, a network that is not there - is a
/// warning and the tag as typed: not being able to check is not the same as
/// having checked.
pub fn check_chap_tag(tag: &str, offline: bool, timeout: std::time::Duration) -> Result<String> {
    if tag.is_empty() {
        return Err(anyhow::anyhow!(
            "--chap-tag needs a tag: a release such as `v2.3.1`, or `latest`, `master` or `dev`"
        ));
    }
    match chapcore::tag_kind(tag) {
        chapcore::TagKind::Moving => {}
        chapcore::TagKind::Other => output::warn(&format!(
            "{tag} is neither a chap-core release nor a moving tag, so it is recorded as an exact \
             pin and `chaps update` will never move it"
        )),
        chapcore::TagKind::Release => {
            if offline {
                output::warn(&format!(
                    "--offline: whether chap-core has released {tag} cannot be looked up; the tag \
                     is taken as given"
                ));
            } else {
                match chapcore::release_exists(tag, timeout) {
                    Ok(true) => {}
                    Ok(false) => {
                        return Err(anyhow::anyhow!(
                            "chap-core has no release {tag}; run `chaps update --list-tags` to see \
                             the tags this deployment can move to"
                        ));
                    }
                    Err(err) => output::warn(&format!(
                        "whether chap-core has released {tag} cannot be looked up ({err:#}); the \
                         tag is taken as given"
                    )),
                }
            }
        }
    }
    Ok(tag.to_string())
}

/// What a failed pull says when this run had just moved chap-core's pin.
///
/// The pin is recorded before the pull, so a deployment whose images would not
/// come down is pointed at a tag it cannot run. Naming the way back is the
/// difference between that and a directory an operator has to repair by hand.
pub fn pull_failed_after_switch(old: &str, new: &str) -> String {
    format!(
        "the pull failed after the pins moved; chap-core is now pinned to {new}, and \
         `chaps update --chap-tag {old} --yes` puts it back"
    )
}

/// What a move backwards is warned about, before it is made.
///
/// It says the risk and the one command that makes it recoverable, and
/// nothing about chap-core's migrations: which release migrated what is not
/// something this CLI knows, and a confident sentence about it would be an
/// invention.
pub fn backwards_warning(old: &str, new: &str) -> String {
    format!(
        "moving chap-core from {old} to {new} can run an older schema against a database \
         migrated by the newer one; run `chaps backup create` first"
    )
}

/// Get a yes for a move backwards, or refuse to make it.
///
/// `--yes` is how a script says it meant it. A run nobody is watching -
/// `--json`, or no terminal on stdin - refuses rather than assume, the way
/// `chaps down --volumes` does with the volumes it is about to destroy.
fn confirm_backwards(ctx: &Ctx, chap_core: &ChapCoreUpdate) -> Result<()> {
    use std::io::{BufRead, IsTerminal, Write};

    let how = "pass `--yes` to `chaps update --chap-tag` to confirm it";
    if ctx.out.json {
        return Err(anyhow::anyhow!(
            "moving chap-core backwards needs an answer and --json has nobody to ask; {how}"
        ));
    }
    if !std::io::stdin().is_terminal() {
        return Err(anyhow::anyhow!(
            "moving chap-core backwards needs an answer and this is not a terminal; {how}"
        ));
    }
    eprint!(
        "\nmove chap-core from {} to {}? [y/N] ",
        chap_core.old_tag, chap_core.new_tag
    );
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|e| anyhow::anyhow!("reading the answer: {e}"))?;
    if matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "cancelled; nothing was fetched, written or pulled"
    ))
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
    match update.compose_ref.clone() {
        None => output::warn(&format!(
            "there is no chap-core ref to fetch compose.ghcr.yml from for {tag}; the image pin \
             moves but compose.yml keeps the layout it has"
        )),
        Some(reference) => match chapcore::fetch_compose(&reference, timeout) {
            Ok(body) => {
                let chaps = project.chaps_dir();
                std::fs::create_dir_all(&chaps)
                    .map_err(|e| anyhow::anyhow!("creating {}: {e}", chaps.display()))?;
                let path = chaps.join(cached_compose_file(&reference));
                std::fs::write(&path, &body)
                    .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
                let source = ComposeSource::Fetched {
                    url: chapcore::compose_url(&reference),
                    tag: reference.clone(),
                    sha256: chapcore::sha256_hex(body.as_bytes()),
                };
                project.state.chap_compose_source = source.clone();
                update.compose_source = source;
            }
            Err(err) => output::warn(&format!(
                "could not fetch chap-core's compose.ghcr.yml at {reference} ({err:#}); the image \
                 pin moves but compose.yml keeps the layout it has"
            )),
        },
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
pub fn chap_core_line(c: &ChapCoreUpdate) -> String {
    if c.changed {
        // The compose file only follows the pin once the run has actually
        // fetched it, and the plan is printed before that happens, so the
        // note is there for a report built after the fetch and nowhere else.
        let fetched = c.compose_ref.as_deref().unwrap_or(&c.new_tag);
        let compose = match &c.compose_source {
            ComposeSource::Fetched { tag, .. } if tag == fetched => "  (compose.ghcr.yml too)",
            _ => "",
        };
        return format!("chap-core  {} -> {}{compose}", c.old_tag, c.new_tag);
    }
    // `--chap-tag` naming the tag this deployment already runs: nothing to
    // switch, and saying so is the whole answer the run owes.
    if c.requested.is_some() {
        return format!(
            "chap-core  {}  already the pin, nothing to switch",
            c.old_tag
        );
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
    let pins = (!parts.is_empty()).then(|| parts.join(", "));
    // The images are a clause of their own, with its own verb: a pull is not
    // an update of anything, and "updated new image for ocs" is not English.
    let images = (!pulled_new.is_empty()).then(|| match pulled_new {
        [one] => format!("pulled a new image for {one}"),
        many => format!("pulled new images for {}", many.join(", ")),
    });
    match (pins, images) {
        (Some(pins), Some(images)) => Some(format!("{pins} and {images}")),
        (pins, images) => pins.or(images),
    }
}

/// [`updated_phrase`] for a finished report.
fn updated(report: &UpdateReport) -> Option<String> {
    let chap_core = report
        .chap_core
        .as_ref()
        .filter(|c| c.changed)
        .map(|c| (c.old_tag.as_str(), c.new_tag.as_str()));
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
        // A run that only pulled says so in its own verb.
        Some(what) if what.starts_with("pulled ") => what.to_string(),
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
        if let Some(user) = user_line(m) {
            text.push_str(&format!("    {}\n", out.dim(&user)));
        }
    }
    let tense = |line: String| would(line, report.dry_run);
    if let Some(core) = &report.chap_core {
        text.push_str(&format!("  {}\n", tense(chap_core_cell(out, core))));
    }
    for component in &report.components {
        text.push_str(&format!(
            "  {}\n",
            out.dim(&tense(component_line(component)))
        ));
    }
    text
}

/// A plan line in the tense of the run: a dry run pulls nothing, so a moving
/// tag there *would be* re-pulled rather than being said to have been.
fn would(line: String, dry_run: bool) -> String {
    if dry_run {
        line.replace("re-pulled", "would be re-pulled")
    } else {
        line
    }
}

/// What a row says about the account the model runs as, when the new tag
/// changed it: `user 1000:1000 -> root (image config)`.
///
/// Nothing at all when it did not, which is almost every row: an image that
/// kept its `USER` line has nothing to report here.
fn user_line(m: &ModelUpdate) -> Option<String> {
    let (old, new) = m.moved_user()?;
    Some(format!("user {old} -> {new} ({})", m.user_from.label()))
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

// ---------------------------------------------------------------------------
// --list-tags
// ---------------------------------------------------------------------------

/// What `chaps update --list-tags` prints, and the whole of its `--json`.
#[derive(Debug, Serialize)]
pub struct TagList {
    /// The tag `.chaps/project.yaml` records today.
    pub pin: String,
    /// Whether the releases could be listed at all. False is `--offline` or a
    /// lookup that did not arrive, and the table is then the moving tags plus
    /// this deployment's own pin.
    pub releases_listed: bool,
    pub tags: Vec<TagRow>,
}

/// One tag a deployment can move chap-core to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TagRow {
    pub tag: String,
    /// `release`, `moving`, or `exact` for a pin that is neither.
    pub kind: &'static str,
    /// The day the release was published, or the branch behind a moving tag
    /// was last committed to. `null` when it could not be had cheaply.
    pub published: Option<String>,
    /// Whether this is the tag the deployment records today.
    pub pinned: bool,
    /// Whether this is the newest release.
    pub newest: bool,
    /// Which build a pinned moving tag is actually running, when its container
    /// is up and docker could be asked.
    pub running: Option<String>,
}

/// The rows of the listing: the moving tags, then the releases newest first,
/// and this deployment's pin when it is neither.
pub fn tag_rows(
    pin: &str,
    releases: &[chapcore::Release],
    published: &BTreeMap<String, String>,
    running: Option<&str>,
) -> Vec<TagRow> {
    let row = |tag: &str, kind: &'static str, day: Option<String>, newest: bool| {
        let pinned = tag == pin;
        TagRow {
            tag: tag.to_string(),
            kind,
            published: day.filter(|d| !d.is_empty()),
            pinned,
            newest,
            // Which image a release tag names is not in question, so the
            // digest is only ever printed where the tag does not say.
            running: (pinned && kind == chapcore::TagKind::Moving.label())
                .then(|| running.map(str::to_string))
                .flatten(),
        }
    };
    let mut rows: Vec<TagRow> = chapcore::MOVING_TAGS
        .iter()
        .map(|tag| {
            row(
                tag,
                chapcore::TagKind::Moving.label(),
                published.get(*tag).cloned(),
                false,
            )
        })
        .collect();

    let mut sorted: Vec<&chapcore::Release> = releases.iter().collect();
    sorted.sort_by_key(|release| std::cmp::Reverse(chapcore::release_version(&release.tag)));
    for (index, release) in sorted.iter().enumerate() {
        rows.push(row(
            &release.tag,
            chapcore::TagKind::Release.label(),
            Some(release.published.clone()),
            index == 0,
        ));
    }
    // A pin the listing does not otherwise hold - an older release, or a
    // `sha-` build - is still where this deployment is, and the point of the
    // table is to say where it can go from there.
    if !rows.iter().any(|row| row.tag == pin) {
        rows.push(row(pin, chapcore::tag_kind(pin).label(), None, false));
    }
    rows
}

/// The NOTE cell: what this row is to this deployment, and to the release
/// line. `-` for a tag that is neither pinned nor the newest.
pub fn tag_note(row: &TagRow) -> String {
    let mut parts = Vec::new();
    if row.pinned {
        parts.push(match &row.running {
            Some(digest) => format!("pinned (moving, running {digest})"),
            None => "pinned".to_string(),
        });
    }
    if row.newest {
        parts.push("newest".to_string());
    }
    if parts.is_empty() {
        return "-".to_string();
    }
    parts.join(", ")
}

/// The one line the listing ends on: what to do with a tag from the table.
pub fn tag_closing_line(list: &TagList) -> String {
    format!(
        "chap-core is pinned to {}; move it with `chaps update --chap-tag <TAG>`",
        list.pin
    )
}

/// The table, then that line.
fn tag_table(out: &Out, list: &TagList) -> String {
    let rows: Vec<Vec<String>> = list
        .tags
        .iter()
        .map(|row| {
            let note = tag_note(row);
            vec![
                row.tag.clone(),
                out.dim(row.kind),
                out.dim(row.published.as_deref().unwrap_or("-")),
                if row.pinned {
                    out.ok(&note)
                } else {
                    out.dim(&note)
                },
            ]
        })
        .collect();
    let mut text = out.table(&["TAG", "KIND", "PUBLISHED", "NOTE"], &rows);
    text.push('\n');
    text.push_str(&out.cmd(&out.backticks(&tag_closing_line(list))));
    text
}

/// `chaps update --list-tags`: where this deployment can move chap-core to.
///
/// It writes nothing and pulls nothing, so it is also the command to run
/// before `--chap-tag`, and it works offline: the three moving tags and this
/// deployment's own pin need nothing from the network.
fn list_tags(ctx: &Ctx, project: &Project) -> Result<()> {
    let pin = project.state.chap_image_tag.clone();
    let timeout = ctx.registry.timeout;
    let offline = ctx.registry.offline;
    let releases = if offline {
        output::warn(
            "--offline: the chap-core releases were not listed, so this is the moving tags and \
             this deployment's own pin",
        );
        Vec::new()
    } else {
        match chapcore::releases(chapcore::LIST_LIMIT, timeout) {
            Ok(list) => list,
            Err(err) => {
                output::warn(&format!(
                    "could not list the chap-core releases ({err:#}), so this is the moving tags \
                     and this deployment's own pin"
                ));
                Vec::new()
            }
        }
    };

    // What each moving tag was last built from, where it is one request away:
    // `dev` and `master` from their branches, and `latest` from the release
    // that publishes it. A branch that will not answer costs its own cell.
    let mut published = BTreeMap::new();
    if !offline {
        for tag in chapcore::MOVING_TAGS {
            if *tag == chapcore::LATEST_TAG {
                continue;
            }
            if let Some(day) = chapcore::branch_updated(tag, timeout) {
                published.insert((*tag).to_string(), day);
            }
        }
    }
    if let Some(newest) = releases.first() {
        published.insert(chapcore::LATEST_TAG.to_string(), newest.published.clone());
    }

    // Which build the pin is on, for the one kind of tag that does not say.
    let running = chapcore::is_moving_tag(&pin)
        .then(|| {
            docker::running_containers(project)
                .as_deref()
                .and_then(|containers| docker::running_build(containers, API_SERVICE))
        })
        .flatten();

    let list = TagList {
        releases_listed: !releases.is_empty(),
        tags: tag_rows(&pin, &releases, &published, running.as_deref()),
        pin,
    };
    ctx.out.emit(&list, || tag_table(&ctx.out, &list))
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
mod tests;
