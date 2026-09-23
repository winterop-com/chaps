//! `chaps update [--dry-run] [--no-restart]` — move the pins forward.
//!
//! Two kinds of pin move here. A model that follows a channel is re-resolved
//! against the marketplace, and the only other thing that moves those is
//! `chaps models enable`; exact pins (`--version`) never move. chap-core's own
//! pin moves too when it is a release tag and a newer release exists, which
//! also re-fetches the `compose.ghcr.yml` that release publishes. A moving
//! chap-core tag (`latest`, `master`, `dev`) is only refreshed by the pull,
//! unless `--pin-chap-core` turns it into a release pin.

use crate::chapcore;
use crate::cli::UpdateArgs;
use crate::commands::Ctx;
use crate::compose::sync::{EnvTag, refresh_env_pin, set_env_chap_tag};
use crate::compose::{sync, tag_env_var};
use crate::docker;
use crate::error::{ChapError, Result};
use crate::output;
use crate::project::{CHAP_TAG_ENV_VAR, ComposeSource, Project, cached_compose_file};
use crate::registry::{self, Provenance, Registry, VersionSelector};
use serde::Serialize;

/// The shape of `update --json`, and what the human output is built from.
#[derive(Debug, Serialize)]
pub struct UpdateReport {
    pub registry: RegistryInfo,
    pub models: Vec<ModelUpdate>,
    pub chap_core: ChapCoreUpdate,
    pub pulled: bool,
    pub restarted: bool,
    pub dry_run: bool,
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

/// One enabled model's before and after.
#[derive(Debug, Clone, Serialize)]
pub struct ModelUpdate {
    pub id: String,
    pub old_version: String,
    pub old_tag: String,
    pub new_version: String,
    pub new_tag: String,
    pub changed: bool,
    /// Pinned to an exact version, so the channel was not consulted.
    pub pinned: bool,
}

impl UpdateReport {
    pub fn changed(&self) -> impl Iterator<Item = &ModelUpdate> {
        self.models.iter().filter(|m| m.changed)
    }
}

/// Refresh the registry, re-resolve every channel-following model, sync,
/// pull and restart.
pub fn run(ctx: &Ctx, args: &UpdateArgs) -> Result<()> {
    let mut project = ctx.project()?;
    // No fallback on purpose: an update from a stale catalogue is not an update.
    let registry = registry::update(&ctx.registry)?;

    let models = plan(&project, &registry)?;
    let latest = lookup_latest(
        &project.state.chap_image_tag,
        args.pin_chap_core,
        ctx.registry.timeout,
    );
    let mut report = UpdateReport {
        registry: RegistryInfo {
            url: registry.url.clone(),
            provenance: registry.provenance.clone(),
        },
        models,
        chap_core: plan_chap_core(&project, latest, args.pin_chap_core),
        pulled: false,
        restarted: false,
        dry_run: args.dry_run,
    };
    if args.dry_run {
        return ctx.out.emit(&report, || human(&report));
    }

    if report.chap_core.changed {
        apply_chap_core(&mut project, &mut report.chap_core, ctx.registry.timeout)?;
    }
    for change in report.models.iter().filter(|m| m.changed) {
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

    run_compose(&project, &["pull".to_string()])?;
    report.pulled = true;
    if !args.no_restart {
        run_compose(&project, &["up".to_string(), "-d".to_string()])?;
        report.restarted = true;
    }
    ctx.out.emit(&report, || human(&report))
}

/// Resolve every enabled model against the fresh registry without changing
/// anything.
fn plan(project: &Project, registry: &Registry) -> Result<Vec<ModelUpdate>> {
    let mut out = Vec::with_capacity(project.state.models.len());
    for (id, entry) in &project.state.models {
        let mut update = ModelUpdate {
            id: id.clone(),
            old_version: entry.version.clone(),
            old_tag: entry.image_tag.clone(),
            new_version: entry.version.clone(),
            new_tag: entry.image_tag.clone(),
            changed: false,
            pinned: entry.channel.is_none(),
        };
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
            ".env has {CHAP_TAG_ENV_VAR} commented out, so the stack still follows the compose \
             default; set `{CHAP_TAG_ENV_VAR}={tag}` there to run the pin this update recorded"
        )),
        EnvTag::Foreign(value) => output::warn(&format!(
            ".env pins {CHAP_TAG_ENV_VAR}={value}, which is yours, not ours; it is left alone, so \
             the stack keeps running {value} rather than {tag}"
        )),
        EnvTag::Absent => output::warn(&format!(
            ".env does not mention {CHAP_TAG_ENV_VAR}, so the stack follows the compose default; \
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
        // fetched it, so a dry run says nothing about it.
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

/// The clause the totals line ends on.
fn chap_core_phrase(c: &ChapCoreUpdate, dry_run: bool) -> String {
    match (c.changed, c.moving, dry_run) {
        (true, _, true) => format!(
            "the chap-core pin would move {} -> {}",
            c.old_tag, c.new_tag
        ),
        (true, _, false) => format!("the chap-core pin moved {} -> {}", c.old_tag, c.new_tag),
        (false, true, true) => format!("chap-core `{}` would be re-pulled", c.old_tag),
        (false, true, false) => format!("chap-core `{}` re-pulled", c.old_tag),
        (false, false, _) => format!("the chap-core pin `{}` did not move", c.old_tag),
    }
}

fn human(report: &UpdateReport) -> String {
    let mut text = format!(
        "registry: {} ({})\n",
        report.registry.url,
        report.registry.provenance.describe()
    );
    if report.models.is_empty() {
        text.push_str("no models enabled\n");
    }
    for m in &report.models {
        let line = if m.pinned {
            format!(
                "  {}  v{} ({})  pinned, skipped",
                m.id, m.old_version, m.old_tag
            )
        } else if m.changed {
            format!(
                "  {}  v{} ({}) -> v{} ({})",
                m.id, m.old_version, m.old_tag, m.new_version, m.new_tag
            )
        } else {
            format!("  {}  v{} ({})  unchanged", m.id, m.old_version, m.old_tag)
        };
        text.push_str(&line);
        text.push('\n');
    }
    text.push_str(&format!("  {}\n", chap_core_line(&report.chap_core)));

    let changed = report.changed().count();
    if report.dry_run {
        text.push_str(&format!(
            "dry run: {changed} model pin(s) would move; {}\n",
            chap_core_phrase(&report.chap_core, true)
        ));
        text.push_str("nothing written\n");
    } else {
        text.push_str(&format!(
            "{changed} model pin(s) moved; {}; images {}; stack {}\n",
            chap_core_phrase(&report.chap_core, false),
            if report.pulled {
                "pulled"
            } else {
                "not pulled"
            },
            if report.restarted {
                "restarted"
            } else {
                "not restarted (run `chaps up`)"
            }
        ));
    }
    text
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

    #[test]
    fn plan_reports_unchanged_when_the_channel_still_points_at_the_pin() {
        let (_dir, project, registry) = project_with(&["chapkit_ewars_model"]);
        let plan = plan(&project, &registry).unwrap();
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

        let plan = plan(&project, &registry).unwrap();
        let ewars = plan.iter().find(|m| m.id == "chapkit_ewars_model").unwrap();
        assert!(ewars.changed);
        assert_eq!(ewars.old_version, "0.9.0");
        assert_eq!(ewars.new_version, "1.0.0");
        assert_eq!(ewars.new_tag, "sha-fa880a1");

        let arima = plan.iter().find(|m| m.id == "auto_arima_chapkit").unwrap();
        assert!(arima.pinned && !arima.changed);
        assert_eq!(arima.new_tag, "sha-1111111");
    }

    #[test]
    fn plan_fails_when_a_channel_model_left_the_registry() {
        let (_dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let entry = project.state.models.remove("chapkit_ewars_model").unwrap();
        project.state.models.insert("vanished".into(), entry);
        let err = plan(&project, &registry).expect_err("unknown model");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownModel(id)) if id == "vanished"
        ));
    }

    #[test]
    fn human_dry_run_lists_each_model_and_writes_nothing() {
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
                },
                ModelUpdate {
                    id: "b".into(),
                    old_version: "1.0.0".into(),
                    old_tag: "sha-3333333".into(),
                    new_version: "1.0.0".into(),
                    new_tag: "sha-3333333".into(),
                    changed: false,
                    pinned: true,
                },
            ],
            chap_core: chap_core("latest", "latest", None),
            pulled: false,
            restarted: false,
            dry_run: true,
        };
        let text = human(&report);
        assert!(text.starts_with("registry: https://example.test/registry.yaml (network)\n"));
        assert!(text.contains("  a  v1.0.0 (sha-1111111) -> v1.1.0 (sha-2222222)\n"));
        assert!(text.contains("  b  v1.0.0 (sha-3333333)  pinned, skipped\n"));
        assert!(text.contains(
            "  chap-core  latest  moving tag, re-pulled; pin it with `chaps update --pin-chap-core`\n"
        ));
        assert!(text.contains(
            "dry run: 1 model pin(s) would move; chap-core `latest` would be re-pulled\n"
        ));
        assert!(text.ends_with("nothing written\n"));

        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["dry_run"], true);
        assert_eq!(value["models"][0]["changed"], true);
        assert_eq!(value["registry"]["provenance"]["kind"], "network");
        assert_eq!(value["chap_core"]["old_tag"], "latest");
        assert_eq!(value["chap_core"]["changed"], false);
        assert_eq!(value["chap_core"]["moving"], true);
        assert_eq!(value["chap_core"]["compose_source"]["kind"], "embedded");
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
        assert_eq!(
            chap_core_phrase(&moved, true),
            "the chap-core pin would move v2.3.0 -> v2.3.1"
        );
        assert_eq!(
            chap_core_phrase(&moved, false),
            "the chap-core pin moved v2.3.0 -> v2.3.1"
        );

        let current = chap_core("v2.3.1", "v2.3.1", Some("v2.3.1"));
        assert_eq!(
            chap_core_line(&current),
            "chap-core  v2.3.1  unchanged, the newest release"
        );
        assert_eq!(
            chap_core_phrase(&current, true),
            "the chap-core pin `v2.3.1` did not move"
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

        let moving = chap_core("latest", "latest", None);
        assert_eq!(
            chap_core_phrase(&moving, false),
            "chap-core `latest` re-pulled"
        );
    }
}
