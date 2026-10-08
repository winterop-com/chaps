//! chap-core's own pin: the release lookup, `--chap-tag`, the move backwards
//! and the fetch of the compose file the new tag publishes.

use crate::chapcore;
use crate::commands::Ctx;
use crate::compose::sync::{EnvTag, set_env_chap_tag};
use crate::error::Result;
use crate::output::Out;
use crate::project::{CHAP_TAG_ENV_VAR, ComposeSource, Project, cached_compose_file};
use serde::Serialize;

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

/// Ask GitHub for the newest chap-core release, when its answer can matter.
///
/// The lookup is skipped for a moving tag nobody asked to pin and for a tag
/// that is not a version at all, so the unauthenticated rate limit is spent
/// only when there is a decision to make. A failed lookup is a warning, not a
/// failure: a marketplace update is still an update.
pub(super) fn lookup_latest(
    current: &str,
    pin: bool,
    requested: Option<&str>,
    dry_run: bool,
    timeout: std::time::Duration,
    warnings: &mut Vec<String>,
) -> Option<String> {
    if !needs_release_lookup(current, pin, requested) {
        return None;
    }
    match chapcore::latest_release(timeout) {
        Ok(tag) => Some(tag),
        Err(err) => {
            warnings.push(lookup_failed(
                &format!("{err:#}"),
                current,
                requested,
                dry_run,
            ));
            None
        }
    }
}

/// What a failed release lookup says about the pin.
///
/// Without `--chap-tag` the pin stays where it is. With `--chap-tag latest`
/// the pin still moves, because the operator named the tag; only the compose
/// file that goes with it cannot be found.
pub fn lookup_failed(err: &str, current: &str, requested: Option<&str>, dry_run: bool) -> String {
    match requested {
        Some(tag) => format!(
            "could not resolve the newest chap-core release ({err}); the pin {} to `{tag}`, \
             and compose.yml keeps the layout it has",
            if dry_run { "would move" } else { "moves" }
        ),
        None => format!(
            "could not resolve the newest chap-core release ({err}); \
             the chap-core pin stays at `{current}`"
        ),
    }
}

/// Whether the newest release can change what this run does.
///
/// A run given `--chap-tag` has been told where to go, so the only reason
/// left to ask is `latest`: the deployment follows that moving tag, and the
/// compose file that goes with it is the one the newest release publishes.
pub fn needs_release_lookup(current: &str, pin: bool, requested: Option<&str>) -> bool {
    if let Some(tag) = requested {
        // A tag that is already the pin moves nothing and fetches nothing.
        return tag == chapcore::LATEST_TAG && tag != current;
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
pub(super) fn plan_chap_core(
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
    // `latest` is the newest release under another name, so a pin to that
    // release is the same image and not a move backwards.
    let same_release = current == chapcore::LATEST_TAG && latest.as_deref() == Some(&new_tag);
    ChapCoreUpdate {
        changed,
        backwards: changed && !same_release && chapcore::is_backwards(&current, &new_tag),
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
/// of the three by definition; anything else is taken as an exact pin (see
/// [`exact_pin_warning`] for the note that goes with it). A lookup that could not be
/// made - `--offline`, a rate limit, a network that is not there - is a
/// warning and the tag as typed: not being able to check is not the same as
/// having checked.
pub fn check_chap_tag(
    tag: &str,
    offline: bool,
    timeout: std::time::Duration,
    warnings: &mut Vec<String>,
) -> Result<String> {
    if tag.is_empty() {
        return Err(anyhow::anyhow!(
            "--chap-tag needs a tag: a release such as `v2.3.1`, or `latest`, `master` or `dev`"
        ));
    }
    match chapcore::tag_kind(tag) {
        chapcore::TagKind::Moving | chapcore::TagKind::Other => {}
        chapcore::TagKind::Release => {
            if offline {
                warnings.push(format!(
                    "--offline: whether chap-core has released {tag} cannot be looked up; the tag \
                     is taken as given"
                ));
            } else {
                match chapcore::release_exists(tag, timeout) {
                    Ok(true) => {}
                    Ok(false) => {
                        return Err(anyhow::anyhow!(
                            "chap-core has no release {tag}; run `varde update --list-tags` to see \
                             the tags this deployment can move to"
                        ));
                    }
                    Err(err) => warnings.push(format!(
                        "whether chap-core has released {tag} cannot be looked up ({err:#}); the \
                         tag is taken as given"
                    )),
                }
            }
        }
    }
    Ok(tag.to_string())
}

/// The note for a `--chap-tag` that is neither a release nor a moving tag.
///
/// It is given only once the pin moves (or would move, in a dry run), so it
/// never stands next to an error that says nothing was changed.
pub fn exact_pin_warning(tag: &str, dry_run: bool) -> String {
    format!(
        "{tag} is neither a chap-core release nor a moving tag, so it {} as an exact pin \
         and `varde update` will never move it",
        if dry_run {
            "would be recorded"
        } else {
            "is recorded"
        }
    )
}

/// The two images a chap-core tag has to name on ghcr: the API and the
/// worker, as `(image, repository)`.
pub const CHAP_IMAGES: &[(&str, &str)] = &[
    ("chap-core", "dhis2-chap/chap-core"),
    ("chap-worker", "dhis2-chap/chap-worker"),
];

/// Check that ghcr has both chap-core images for `tag`, before the pin moves.
///
/// A tag can be a real name and still have no image: `dev` is a branch of
/// chap-core, and ghcr has no `chap-worker:dev`. The pull would then fail after
/// the pin moved, so this asks ghcr first, in the dry run too. A lookup that
/// could not be made is a warning and the tag as typed.
pub fn check_chap_images(
    tag: &str,
    ghcr: &str,
    timeout: std::time::Duration,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let mut missing = Vec::new();
    for (image, repository) in CHAP_IMAGES {
        let found = crate::manual::ghcr::Client::anonymous(ghcr, repository, timeout)
            .and_then(|client| client.tag_exists(tag));
        match found {
            Ok(true) => {}
            Ok(false) => missing.push(format!("{image}:{tag}")),
            Err(err) => {
                warnings.push(format!(
                    "whether ghcr.io has {image}:{tag} cannot be looked up ({err:#}); the tag \
                     is taken as given"
                ));
                return Ok(());
            }
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "ghcr.io has no image {}, so the pull would fail; nothing was changed; pick another \
         tag, for example a release from `varde update --list-tags`",
        missing.join(" and ")
    ))
}

/// What a failed pull says when this run had just moved chap-core's pin.
///
/// The pin is recorded before the pull, so a deployment whose images would not
/// come down is pointed at a tag it cannot run. Naming the way back is the
/// difference between that and a directory an operator has to repair by hand.
pub fn pull_failed_after_switch(old: &str, new: &str) -> String {
    format!(
        "the pull failed after the pins moved; chap-core is now pinned to {new}, and \
         `varde update --chap-tag {old} --yes` puts it back"
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
         migrated by the newer one; run `varde backup create` first"
    )
}

/// Get a yes for a move backwards, or refuse to make it.
///
/// `--yes` is how a script says it meant it. A run nobody is watching -
/// `--json`, or no terminal on stdin - refuses rather than assume, the way
/// `varde down --volumes` does with the volumes it is about to destroy.
pub(super) fn confirm_backwards(ctx: &Ctx, chap_core: &ChapCoreUpdate) -> Result<()> {
    use std::io::{BufRead, IsTerminal, Write};

    let how = match chap_core.requested {
        Some(_) => format!(
            "run `varde update --chap-tag {} --yes` to confirm it",
            chap_core.new_tag
        ),
        None => "run `varde update --pin-chap-core --yes` to confirm it".to_string(),
    };
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
pub(super) fn apply_chap_core(
    project: &mut Project,
    update: &mut ChapCoreUpdate,
    timeout: std::time::Duration,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let tag = update.new_tag.clone();
    match update.compose_ref.clone() {
        // A `--chap-tag` run without a ref is `latest` after a failed release
        // lookup, and that lookup has already said so.
        None if update.requested.is_some() => {}
        None => warnings.push(format!(
            "there is no chap-core ref to fetch compose.ghcr.yml from for {tag}; the image pin \
             moves but compose.yml keeps the layout it has"
        )),
        Some(reference) => match chapcore::fetch_compose(&reference, timeout) {
            Ok(body) => {
                let varde = project.varde_dir();
                std::fs::create_dir_all(&varde)
                    .map_err(|e| anyhow::anyhow!("creating {}: {e}", varde.display()))?;
                let path = varde.join(cached_compose_file(&reference));
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
            Err(err) => warnings.push(format!(
                "could not fetch chap-core's compose.ghcr.yml at {reference} ({err:#}); the image \
                 pin moves but compose.yml keeps the layout it has"
            )),
        },
    }

    // The tag has to reach .env as well, or compose still substitutes the old
    // value. Only the line this project wrote is touched.
    match set_env_chap_tag(&project.dir, &update.old_tag, &tag)? {
        EnvTag::Updated | EnvTag::NoFile => {}
        EnvTag::Commented => warnings.push(format!(
            ".env has {CHAP_TAG_ENV_VAR} commented out, so Chap still follows the compose \
             default; set `{CHAP_TAG_ENV_VAR}={tag}` there to run the pin this update recorded"
        )),
        EnvTag::Foreign(value) => warnings.push(format!(
            ".env pins {CHAP_TAG_ENV_VAR}={value}, which is yours, not ours; it is left alone, so \
             Chap keeps running {value} rather than {tag}"
        )),
        EnvTag::Absent => warnings.push(format!(
            ".env does not mention {CHAP_TAG_ENV_VAR}, so Chap follows the compose default; \
             add `{CHAP_TAG_ENV_VAR}={tag}` there to run the pin this update recorded"
        )),
    }

    project.state.chap_image_tag = tag;
    Ok(())
}

/// The chap-core row of the list, in the same shape as a model's.
pub fn chap_core_line(c: &ChapCoreUpdate) -> String {
    if c.changed {
        // The plan is printed before the fetch, so the note says what this
        // run is going to fetch: a compose file at another ref than the one
        // the deployment has. `latest` after the release it points at keeps
        // the file it has, and a branch brings its own.
        let brings_new = match (&c.compose_ref, &c.compose_source) {
            (None, _) => false,
            (Some(reference), ComposeSource::Fetched { tag, .. }) => reference != tag,
            (Some(_), _) => true,
        };
        let compose = if brings_new {
            "  (compose.ghcr.yml too)"
        } else {
            ""
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
            "chap-core  {}  moving tag, re-pulled; pin it with `varde update --pin-chap-core`",
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

/// The chap-core row, with the new tag coloured the way a model row's is.
pub(super) fn chap_core_cell(out: &Out, change: &ChapCoreUpdate, width: usize) -> String {
    let line = super::report::pad_name(chap_core_line(change), "chap-core", width);
    if !change.changed {
        return out.dim(&line);
    }
    match line.rsplit_once(" -> ") {
        Some((head, tail)) => format!("{} -> {}", out.dim(head), out.ok(tail)),
        None => out.ok(&line),
    }
}
