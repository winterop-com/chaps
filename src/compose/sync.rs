//! Rendering `.chaps/` into the compose files at the project root.
//!
//! `.chaps/` is intent; `compose.yml`, `compose.chaps.yml`,
//! `compose.<service_id>.yml` and `compose.marketplace.yml` are artifacts.
//! [`sync`] is the one place that
//! turns the former into the latter: `apply` ends in it, `chaps sync` calls it
//! directly and `chaps up` runs it before `docker compose up`.
//!
//! Rendering is deterministic and every file is compared before it is
//! written, so a second sync reports everything as unchanged. `compose.yml`
//! is deterministic too because the copy of chap-core's `compose.ghcr.yml` it
//! is rendered from is kept in `.chaps/`; nothing here touches the network.

use crate::compose::overrides;
use crate::compose::render::{
    NO_TAG_PINS, render_base, render_chaps_overlay, render_overlay, render_umbrella,
};
use crate::compose::spec::{BaseSpec, OverlaySpec, UpstreamCompose};
use crate::compose::tag_env_var;
use crate::error::Result;
use crate::project::{
    BASE_COMPOSE, CHAP_TAG_ENV_VAR, CHAPS_COMPOSE, ComposeSource, ENV_FILE, MARKETPLACE_COMPOSE,
    Project, default_compose_files,
};
use crate::registry::Registry;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What [`sync`] did, or with `check` what it would have done.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    /// Files created or whose content changed.
    pub written: Vec<PathBuf>,
    /// Files that already had the rendered content.
    pub unchanged: Vec<PathBuf>,
    /// Overlays of models that are no longer enabled.
    pub removed: Vec<PathBuf>,
    /// Whether anything differed from what `.chaps/` describes.
    pub drift: bool,
    /// Whether this was a dry run.
    pub check: bool,
    pub warnings: Vec<String>,
}

impl SyncReport {
    /// One line: `2 written, 1 unchanged, 1 removed`.
    pub fn summary(&self) -> String {
        let (write, remove) = if self.check {
            ("to write", "to remove")
        } else {
            ("written", "removed")
        };
        format!(
            "{} {write}, {} unchanged, {} {remove}",
            self.written.len(),
            self.unchanged.len(),
            self.removed.len()
        )
    }
}

/// Render every enabled model's overlay and the umbrella from the project
/// state, remove overlays this CLI wrote for models that are no longer
/// enabled, and append missing `.env` pin comments.
///
/// With `check` nothing is written or removed and the state is left alone;
/// the report says what a real run would do. Otherwise
/// `state.rendered_files` is updated and `.chaps/` is saved.
pub fn sync(
    project: &mut Project,
    registry: &Registry,
    cli_version: &str,
    check: bool,
) -> Result<SyncReport> {
    let mut report = SyncReport {
        check,
        ..SyncReport::default()
    };
    let dir = project.dir.clone();
    if !check {
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", dir.display()))?;
    }

    // Desired artifacts: the base stack, the overlays in marketplace-id order,
    // then the umbrella that includes them.
    let mut desired: Vec<(String, String)> = Vec::new();
    // The umbrella includes the overlays and only the overlays; the base file
    // is one of the `-f` list, not something to include.
    let mut overlays: Vec<String> = Vec::new();
    let (base, base_warnings) = base_compose(project, cli_version);
    report.warnings.extend(base_warnings);
    if let Some(base) = base {
        desired.push((BASE_COMPOSE.to_string(), base));
    }
    // Always rendered, base file or not: it is the only place the API's host
    // port is decided, and it has to be a `-f` entry of its own because a
    // file in `include:` cannot override a service compose.yml defines.
    desired.push((
        CHAPS_COMPOSE.to_string(),
        render_chaps_overlay(project.state.api_port),
    ));
    for (id, model) in &project.state.models {
        let spec = match registry.get(id) {
            Some(m) => OverlaySpec::from_enabled(id, model, m, cli_version),
            None => {
                report.warnings.push(format!(
                    "{id} is enabled but not in the registry; its overlay header \
                     carries no display name or repository"
                ));
                OverlaySpec::from_enabled_without_registry(id, model, cli_version)
            }
        };
        // The overlay's init container chowns the data volume from busybox,
        // which resolves no account name of its own, so the user has to be
        // expressible as numbers. An unknown one still renders, with the
        // chapkit ids, but the operator should know the guess was taken.
        if overrides::numeric_user(&spec.user).is_none() {
            report.warnings.push(format!(
                "{id} runs as `{}`, which has no known uid:gid; the volume init \
                 container will chown {} to {} instead - pass `--user <uid>:<gid>` \
                 if that is wrong",
                spec.user,
                spec.data_dir,
                overrides::FALLBACK_UID_GID
            ));
        }
        overlays.push(model.compose_file.clone());
        desired.push((model.compose_file.clone(), render_overlay(&spec)));
    }
    desired.push((MARKETPLACE_COMPOSE.to_string(), render_umbrella(&overlays)));

    for (name, content) in &desired {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() == Some(content.as_str()) {
            report.unchanged.push(path);
            continue;
        }
        if !check {
            std::fs::write(&path, content)
                .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
        }
        report.written.push(path);
    }

    // Only overlays a previous sync wrote are candidates for removal, and
    // only when their service is gone: a hand-written compose.*.yml is never
    // ours to delete.
    let wanted: BTreeSet<&str> = desired.iter().map(|(f, _)| f.as_str()).collect();
    for name in &project.state.rendered_files {
        if wanted.contains(name.as_str()) || !is_overlay_name(name) {
            continue;
        }
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        if !check {
            std::fs::remove_file(&path)
                .map_err(|e| anyhow::anyhow!("removing {}: {e}", path.display()))?;
        }
        report.removed.push(path);
    }

    if let Some(env) = append_env_pins(project, check)? {
        report.written.push(env);
    }

    report.drift = !report.written.is_empty() || !report.removed.is_empty();
    if check {
        return Ok(report);
    }

    // A project written before compose.chaps.yml existed records a two-entry
    // `-f` list; the first sync after an upgrade puts the new file in it.
    project.state.compose_files = default_compose_files();
    project.state.rendered_files = desired.into_iter().map(|(f, _)| f).collect();
    project.save()?;
    Ok(report)
}

/// The `compose.yml` this project's state describes, plus any warning about
/// the source it is rendered from.
///
/// `None` means the recorded source cannot be replayed - the cached copy of
/// chap-core's `compose.ghcr.yml` is gone - in which case the file on disk is
/// left exactly as it is: a base stack we cannot reproduce is not one to
/// overwrite with a guess.
fn base_compose(project: &Project, cli_version: &str) -> (Option<String>, Vec<String>) {
    let embedded = || {
        render_base(&BaseSpec {
            cli_version: cli_version.to_string(),
            upstream: None,
        })
    };
    let ComposeSource::Fetched { tag, sha256, .. } = &project.state.chap_compose_source else {
        return (Some(embedded()), Vec::new());
    };

    let Some(path) = project.cached_compose_path() else {
        return (Some(embedded()), Vec::new());
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Ok(body) = std::fs::read_to_string(&path) else {
        return (
            None,
            vec![format!(
                ".chaps/{name} is missing, so {BASE_COMPOSE} is left as it is; \
                 re-run `chaps init --force --chap-tag {tag}` to fetch it again"
            )],
        );
    };

    let mut warnings = Vec::new();
    if crate::chapcore::sha256_hex(body.as_bytes()) != *sha256 {
        warnings.push(format!(
            ".chaps/{name} no longer matches the checksum recorded in .chaps/project.yaml; \
             {BASE_COMPOSE} follows the file as it is now"
        ));
    }
    let text = render_base(&BaseSpec {
        cli_version: cli_version.to_string(),
        upstream: Some(UpstreamCompose {
            tag: tag.clone(),
            body,
        }),
    });
    (Some(text), warnings)
}

/// `compose.<something>.yml`, the shape of a model overlay.
fn is_overlay_name(name: &str) -> bool {
    name.starts_with("compose.")
        && name.ends_with(".yml")
        && name != MARKETPLACE_COMPOSE
        && name != BASE_COMPOSE
        && name != CHAPS_COMPOSE
}

/// Add a commented image pin to `.env` for every enabled model that has none.
///
/// Only ever appends: the file belongs to the operator once `init` wrote it,
/// and it may hold passwords and tokens we must not rewrite. Returns the path
/// when something was (or with `check`, would be) appended.
fn append_env_pins(project: &Project, check: bool) -> Result<Option<PathBuf>> {
    let path = project.dir.join(ENV_FILE);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    let mut missing = Vec::new();
    for (id, model) in &project.state.models {
        let var = tag_env_var(id);
        if !mentions_var(&body, &var) {
            missing.push(format!("# {var}={}", model.image_tag));
        }
    }
    if missing.is_empty() {
        return Ok(None);
    }
    if check {
        return Ok(Some(path));
    }

    // The generated .env carries a placeholder under the pin heading so the
    // section is never a dangling title; the first real pin replaces it.
    let mut out: String = body
        .lines()
        .filter(|line| line.trim_end() != NO_TAG_PINS)
        .map(|line| format!("{line}\n"))
        .collect();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&missing.join("\n"));
    out.push('\n');
    std::fs::write(&path, out).map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(Some(path))
}

/// Move the commented `# <VAR>=<tag>` pin line of `var` to `tag`.
///
/// Only that one comment line changes. An active (uncommented) `VAR=` line is
/// the operator's own override and is left alone, as is a `.env` that does
/// not mention the variable at all (the next sync appends it). Returns
/// whether the file changed.
pub fn refresh_env_pin(dir: &Path, var: &str, tag: &str) -> Result<bool> {
    let path = dir.join(ENV_FILE);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let wanted = format!("# {var}={tag}");
    let mut changed = false;
    let mut out = String::with_capacity(body.len());
    for line in body.lines() {
        if is_commented_pin(line, var) && line != wanted {
            out.push_str(&wanted);
            changed = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !changed {
        return Ok(false);
    }
    std::fs::write(&path, out).map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(true)
}

/// What [`set_env_chap_tag`] found in `.env`, and therefore what the running
/// deployment will do with the new tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvTag {
    /// There is no `.env` (a project created with `init --no-env`).
    NoFile,
    /// The active `CHAP_IMAGE_TAG=` line now names the new tag.
    Updated,
    /// The only mention is a commented placeholder, so the deployment follows
    /// the compose default. Left alone: uncommenting it is the operator's call.
    Commented,
    /// An active line holds a value this project did not put there.
    Foreign(String),
    /// The file never mentions the variable.
    Absent,
}

/// Move the active `CHAP_IMAGE_TAG=` line of `.env` from `old` to `new`.
///
/// Exactly one line may change, and only when it still says what this project
/// recorded: an operator who pinned something else, or who left the generated
/// placeholder commented out, has made a decision that `chaps update` does not
/// get to undo. The outcome says which of those it was so the caller can warn.
pub fn set_env_chap_tag(dir: &Path, old: &str, new: &str) -> Result<EnvTag> {
    let path = dir.join(ENV_FILE);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(EnvTag::NoFile);
    };
    let assignment = format!("{CHAP_TAG_ENV_VAR}=");

    let mut out = String::with_capacity(body.len());
    let mut outcome = None;
    for line in body.lines() {
        let trimmed = line.trim();
        let value = trimmed.strip_prefix(&assignment).map(str::trim);
        match value {
            Some(value) if outcome.is_none() && (value == old || value == new) => {
                out.push_str(&format!("{CHAP_TAG_ENV_VAR}={new}"));
                outcome = Some(EnvTag::Updated);
            }
            Some(value) if outcome.is_none() => {
                out.push_str(line);
                outcome = Some(EnvTag::Foreign(value.to_string()));
            }
            _ => out.push_str(line),
        }
        out.push('\n');
    }

    let outcome = outcome.unwrap_or(if mentions_var(&body, CHAP_TAG_ENV_VAR) {
        EnvTag::Commented
    } else {
        EnvTag::Absent
    });
    if outcome == EnvTag::Updated && out != body {
        std::fs::write(&path, out)
            .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    }
    Ok(outcome)
}

/// Whether any line, commented or not, assigns `var`.
fn mentions_var(body: &str, var: &str) -> bool {
    body.lines().any(|line| {
        let line = line.trim_start().trim_start_matches('#').trim_start();
        line.starts_with(&format!("{var}="))
    })
}

/// `# VAR=...`, with any amount of whitespace around the `#`.
fn is_commented_pin(line: &str, var: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix('#') else {
        return false;
    };
    rest.trim_start().starts_with(&format!("{var}="))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compose::{EnableRequest, Selection, apply};
    use crate::project::ProjectState;
    use crate::registry::load_embedded;
    use tempfile::TempDir;

    const VERSION: &str = "0.1.0";

    fn project_with(ids: &[&str]) -> (TempDir, Project, Registry) {
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
        apply(&mut project, &registry, &sel, VERSION).unwrap();
        (dir, project, registry)
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
    }

    /// Write a cached `compose.ghcr.yml` into `.chaps/` and record it as the
    /// project's compose source, the way `init` does after a fetch.
    fn cache(project: &mut Project, tag: &str, body: &str) {
        let chaps = project.chaps_dir();
        std::fs::create_dir_all(&chaps).unwrap();
        std::fs::write(chaps.join(crate::project::cached_compose_file(tag)), body).unwrap();
        project.state.chap_compose_source = ComposeSource::Fetched {
            url: crate::chapcore::compose_url(tag),
            tag: tag.to_string(),
            sha256: crate::chapcore::sha256_hex(body.as_bytes()),
        };
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_second_sync_changes_nothing() {
        let (_dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert!(!report.drift, "{report:?}");
        assert!(report.written.is_empty() && report.removed.is_empty());
        assert_eq!(
            names(&report.unchanged),
            vec![
                "compose.yml",
                "compose.chaps.yml",
                "compose.chapkit-ewars-model.yml",
                "compose.marketplace.yml"
            ]
        );
        assert_eq!(
            project.state.rendered_files,
            vec![
                "compose.yml",
                "compose.chaps.yml",
                "compose.chapkit-ewars-model.yml",
                "compose.marketplace.yml"
            ]
        );
        assert_eq!(report.summary(), "0 written, 4 unchanged, 0 removed");
    }

    #[test]
    fn sync_renders_the_chaps_overlay_and_puts_it_in_the_f_list() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let overlay = dir.path().join(CHAPS_COMPOSE);
        assert!(overlay.is_file());
        assert!(read(&overlay).contains("${CHAP_API_PORT:-8000}:8000"));
        assert_eq!(
            project.state.compose_files,
            vec![
                "compose.yml",
                "compose.chaps.yml",
                "compose.marketplace.yml"
            ],
            "the override needs its own -f entry, between the base and the umbrella"
        );

        // The API port lives in .chaps/project.yaml, so moving it is drift.
        project.state.api_port = 8123;
        let report = sync(&mut project, &registry, VERSION, true).unwrap();
        assert!(report.drift);
        assert_eq!(names(&report.written), vec![CHAPS_COMPOSE]);
        assert!(
            read(&overlay).contains("8000}:8000"),
            "--check writes nothing"
        );

        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert_eq!(names(&report.written), vec![CHAPS_COMPOSE]);
        assert!(read(&overlay).contains("${CHAP_API_PORT:-8123}:8000"));
        assert!(!sync(&mut project, &registry, VERSION, true).unwrap().drift);

        // It is never removed as if it were a model overlay.
        project.state.models.clear();
        sync(&mut project, &registry, VERSION, false).unwrap();
        assert!(overlay.is_file());
    }

    #[test]
    fn a_project_from_before_the_chaps_overlay_is_migrated_by_one_sync() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        // What an older chaps wrote: no compose.chaps.yml anywhere.
        std::fs::remove_file(dir.path().join(CHAPS_COMPOSE)).unwrap();
        project.state.compose_files =
            vec![BASE_COMPOSE.to_string(), MARKETPLACE_COMPOSE.to_string()];
        project.state.rendered_files.retain(|f| f != CHAPS_COMPOSE);

        let report = sync(&mut project, &registry, VERSION, true).unwrap();
        assert!(report.drift, "the new file is missing");
        assert_eq!(names(&report.written), vec![CHAPS_COMPOSE]);
        assert!(
            !dir.path().join(CHAPS_COMPOSE).exists(),
            "--check writes nothing"
        );

        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert_eq!(names(&report.written), vec![CHAPS_COMPOSE]);
        assert!(dir.path().join(CHAPS_COMPOSE).is_file());
        assert_eq!(project.state.compose_files, default_compose_files());
        assert!(
            project
                .state
                .rendered_files
                .contains(&CHAPS_COMPOSE.to_string())
        );
        assert!(!sync(&mut project, &registry, VERSION, true).unwrap().drift);
    }

    #[test]
    fn check_reports_drift_without_touching_anything() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let overlay = dir.path().join("compose.chapkit-ewars-model.yml");
        std::fs::remove_file(&overlay).unwrap();

        let report = sync(&mut project, &registry, VERSION, true).unwrap();
        assert!(report.drift);
        assert!(report.check);
        assert_eq!(
            names(&report.written),
            vec!["compose.chapkit-ewars-model.yml"]
        );
        assert!(!overlay.exists(), "--check writes nothing");
        assert_eq!(report.summary(), "1 to write, 3 unchanged, 0 to remove");

        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert!(report.drift);
        assert!(overlay.is_file(), "a real sync re-creates the overlay");
        let again = sync(&mut project, &registry, VERSION, true).unwrap();
        assert!(!again.drift);
    }

    #[test]
    fn a_model_removed_from_the_state_loses_its_overlay_but_hand_written_files_stay() {
        let (dir, mut project, registry) =
            project_with(&["chapkit_ewars_model", "auto_arima_chapkit"]);
        let custom = dir.path().join("compose.custom.yml");
        std::fs::write(&custom, "services: {}\n").unwrap();

        // Simulate a hand edit of .chaps/models.yaml.
        project.state.models.remove("auto_arima_chapkit");
        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert_eq!(
            names(&report.removed),
            vec!["compose.auto-arima-chapkit.yml"]
        );
        assert!(!dir.path().join("compose.auto-arima-chapkit.yml").exists());
        assert!(custom.is_file(), "hand-written overlays are never removed");
        assert_eq!(names(&report.written), vec!["compose.marketplace.yml"]);

        // A listed name that is not an overlay shape is never removed either.
        std::fs::write(dir.path().join("notes.yml"), "x: 1\n").unwrap();
        project.state.rendered_files.push("notes.yml".into());
        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert!(report.removed.is_empty(), "{report:?}");
        assert!(dir.path().join("notes.yml").is_file());
    }

    #[test]
    fn a_model_missing_from_the_registry_still_renders_with_a_warning() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let mut entry = project.state.models["chapkit_ewars_model"].clone();
        entry.service_id = "gone-model".into();
        entry.compose_file = "compose.gone-model.yml".into();
        project.state.models.insert("gone_model".into(), entry);

        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("gone_model"));
        let body = std::fs::read_to_string(dir.path().join("compose.gone-model.yml")).unwrap();
        assert!(body.contains("  gone-model:\n"));
        assert!(body.contains("(gone_model)"));
        assert!(!sync(&mut project, &registry, VERSION, true).unwrap().drift);
    }

    #[test]
    fn a_user_with_no_known_ids_renders_with_a_warning() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert!(report.warnings.is_empty(), "{report:?}");

        project
            .state
            .models
            .get_mut("chapkit_ewars_model")
            .unwrap()
            .user = "nobody".into();
        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert_eq!(report.warnings.len(), 1, "{report:?}");
        assert!(report.warnings[0].contains("chapkit_ewars_model runs as `nobody`"));
        assert!(report.warnings[0].contains("1000:1000"));

        // The overlay still renders, with the fallback ids in the chown.
        let body =
            std::fs::read_to_string(dir.path().join("compose.chapkit-ewars-model.yml")).unwrap();
        assert!(body.contains("chown 1000:1000 /app/data"), "{body}");
        assert!(body.contains("    user: nobody\n"));

        // A numeric user is understood and warns about nothing.
        project
            .state
            .models
            .get_mut("chapkit_ewars_model")
            .unwrap()
            .user = "1000:1000".into();
        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert!(report.warnings.is_empty(), "{report:?}");
    }

    #[test]
    fn refresh_env_pin_moves_only_the_commented_line() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(ENV_FILE);
        std::fs::write(
            &env,
            "POSTGRES_PASSWORD=secret\n\
             # CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-1111111\n\
             AUTO_ARIMA_CHAPKIT_IMAGE_TAG=sha-2222222\n",
        )
        .unwrap();

        assert!(
            refresh_env_pin(dir.path(), "CHAPKIT_EWARS_MODEL_IMAGE_TAG", "sha-3333333").unwrap()
        );
        let body = std::fs::read_to_string(&env).unwrap();
        assert_eq!(
            body,
            "POSTGRES_PASSWORD=secret\n\
             # CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-3333333\n\
             AUTO_ARIMA_CHAPKIT_IMAGE_TAG=sha-2222222\n"
        );

        // Already there: nothing to do. Active line: the operator's, untouched.
        assert!(
            !refresh_env_pin(dir.path(), "CHAPKIT_EWARS_MODEL_IMAGE_TAG", "sha-3333333").unwrap()
        );
        assert!(
            !refresh_env_pin(dir.path(), "AUTO_ARIMA_CHAPKIT_IMAGE_TAG", "sha-4444444").unwrap()
        );
        assert!(!refresh_env_pin(dir.path(), "NOPE_IMAGE_TAG", "sha-4444444").unwrap());
        assert_eq!(std::fs::read_to_string(&env).unwrap(), body);
        assert!(!refresh_env_pin(&dir.path().join("missing"), "X", "y").unwrap());
    }

    #[test]
    fn check_counts_a_missing_env_pin_as_drift() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let env = dir.path().join(ENV_FILE);
        std::fs::write(&env, "POSTGRES_PASSWORD=secret\n").unwrap();
        let report = sync(&mut project, &registry, VERSION, true).unwrap();
        assert!(report.drift);
        assert_eq!(names(&report.written), vec![".env"]);
        assert_eq!(
            std::fs::read_to_string(&env).unwrap(),
            "POSTGRES_PASSWORD=secret\n"
        );

        sync(&mut project, &registry, VERSION, false).unwrap();
        assert!(
            std::fs::read_to_string(&env)
                .unwrap()
                .contains("\n# CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-fa880a1\n")
        );
        assert!(!sync(&mut project, &registry, VERSION, true).unwrap().drift);
    }

    #[test]
    fn the_base_file_is_rendered_from_the_embedded_copy_by_default() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let base = dir.path().join(BASE_COMPOSE);
        let original = read(&base);
        assert!(
            original
                .starts_with("# Generated by chaps init (0.1.0) from chap-core compose.ghcr.yml.")
        );

        // A hand edit of compose.yml is drift, and a real sync restores it.
        std::fs::write(&base, "services: {}\n").unwrap();
        let report = sync(&mut project, &registry, VERSION, true).unwrap();
        assert!(report.drift);
        assert_eq!(names(&report.written), vec!["compose.yml"]);
        assert_eq!(read(&base), "services: {}\n", "--check writes nothing");

        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert_eq!(names(&report.written), vec!["compose.yml"]);
        assert_eq!(read(&base), original);
        assert!(!sync(&mut project, &registry, VERSION, true).unwrap().drift);
    }

    #[test]
    fn a_fetched_source_renders_from_the_cached_copy() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let body = "services:\n  chap:\n    image: ghcr.io/x:${CHAP_IMAGE_TAG:-latest}\n";
        cache(&mut project, "v2.3.1", body);

        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert!(report.warnings.is_empty(), "{report:?}");
        assert_eq!(names(&report.written), vec!["compose.yml"]);
        let base = read(&dir.path().join(BASE_COMPOSE));
        assert_eq!(
            base,
            format!(
                "# Generated by chaps init (0.1.0) from chap-core compose.ghcr.yml at v2.3.1.\n\
                 # Model services live in compose.<service>.yml overlays included by compose.marketplace.yml.\n\
                 {body}"
            )
        );
        assert!(!sync(&mut project, &registry, VERSION, true).unwrap().drift);
    }

    #[test]
    fn an_edited_cached_copy_is_followed_but_reported() {
        let (dir, mut project, registry) = project_with(&[]);
        cache(
            &mut project,
            "v2.3.1",
            "services:\n  chap:\n    image: x:${CHAP_IMAGE_TAG:-latest}\n",
        );
        sync(&mut project, &registry, VERSION, false).unwrap();

        // The recorded checksum no longer matches, but the file on disk is
        // what the operator has: follow it, and say so.
        let cached = project.cached_compose_path().unwrap();
        std::fs::write(
            &cached,
            "services:\n  chap:\n    image: y:${CHAP_IMAGE_TAG:-latest}\n",
        )
        .unwrap();
        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert_eq!(report.warnings.len(), 1, "{report:?}");
        assert!(report.warnings[0].contains("compose.chap-core.v2.3.1.yml"));
        assert!(report.warnings[0].contains("checksum"));
        assert!(read(&dir.path().join(BASE_COMPOSE)).contains("image: y:"));

        // And a cached copy that is gone leaves compose.yml alone.
        let before = read(&dir.path().join(BASE_COMPOSE));
        std::fs::remove_file(&cached).unwrap();
        let report = sync(&mut project, &registry, VERSION, false).unwrap();
        assert_eq!(report.warnings.len(), 1, "{report:?}");
        assert!(report.warnings[0].contains("is missing"));
        assert!(!names(&report.written).contains(&"compose.yml".to_string()));
        assert!(!names(&report.unchanged).contains(&"compose.yml".to_string()));
        assert_eq!(read(&dir.path().join(BASE_COMPOSE)), before);
        assert!(
            !project
                .state
                .rendered_files
                .contains(&BASE_COMPOSE.to_string())
        );
    }

    #[test]
    fn set_env_chap_tag_moves_the_active_line_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(ENV_FILE);
        std::fs::write(
            &env,
            "POSTGRES_PASSWORD=secret\n\
             CHAP_IMAGE_TAG=v2.3.0\n\
             # CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-1111111\n",
        )
        .unwrap();

        assert_eq!(
            set_env_chap_tag(dir.path(), "v2.3.0", "v2.3.1").unwrap(),
            EnvTag::Updated
        );
        assert_eq!(
            read(&env),
            "POSTGRES_PASSWORD=secret\n\
             CHAP_IMAGE_TAG=v2.3.1\n\
             # CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-1111111\n"
        );
        // Running it again is a no-op, not a second rewrite.
        assert_eq!(
            set_env_chap_tag(dir.path(), "v2.3.0", "v2.3.1").unwrap(),
            EnvTag::Updated
        );
        assert!(read(&env).contains("CHAP_IMAGE_TAG=v2.3.1\n"));
    }

    #[test]
    fn set_env_chap_tag_keeps_its_hands_off_the_operators_choices() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(ENV_FILE);

        // The generated placeholder: commented, so the deployment follows the
        // compose default and uncommenting it is the operator's call.
        std::fs::write(&env, "# CHAP_IMAGE_TAG=latest\n").unwrap();
        assert_eq!(
            set_env_chap_tag(dir.path(), "latest", "v2.3.1").unwrap(),
            EnvTag::Commented
        );
        assert_eq!(read(&env), "# CHAP_IMAGE_TAG=latest\n");

        // An active line with someone else's value.
        std::fs::write(&env, "CHAP_IMAGE_TAG=v1.0.0\n").unwrap();
        assert_eq!(
            set_env_chap_tag(dir.path(), "v2.3.0", "v2.3.1").unwrap(),
            EnvTag::Foreign("v1.0.0".to_string())
        );
        assert_eq!(read(&env), "CHAP_IMAGE_TAG=v1.0.0\n");

        // Never mentioned, and no file at all.
        std::fs::write(&env, "POSTGRES_DB=chap_core\n").unwrap();
        assert_eq!(
            set_env_chap_tag(dir.path(), "v2.3.0", "v2.3.1").unwrap(),
            EnvTag::Absent
        );
        assert_eq!(read(&env), "POSTGRES_DB=chap_core\n");
        assert_eq!(
            set_env_chap_tag(&dir.path().join("elsewhere"), "a", "b").unwrap(),
            EnvTag::NoFile
        );
    }

    #[test]
    fn overlay_name_shape() {
        assert!(is_overlay_name("compose.chapkit-ewars-model.yml"));
        assert!(!is_overlay_name(MARKETPLACE_COMPOSE));
        assert!(!is_overlay_name(BASE_COMPOSE));
        assert!(!is_overlay_name("compose.yaml"));
        assert!(!is_overlay_name("notes.yml"));
    }
}
