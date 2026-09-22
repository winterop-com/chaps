//! Rendering `.chaps/` into the compose files at the project root.
//!
//! `.chaps/models.yaml` is intent; `compose.<service_id>.yml` and
//! `compose.marketplace.yml` are artifacts. [`sync`] is the one place that
//! turns the former into the latter: `apply` ends in it, `chaps sync` calls it
//! directly and `chaps up` runs it before `docker compose up`.
//!
//! Rendering is deterministic and every file is compared before it is
//! written, so a second sync reports everything as unchanged.

use crate::compose::render::{NO_TAG_PINS, render_overlay, render_umbrella};
use crate::compose::spec::OverlaySpec;
use crate::compose::tag_env_var;
use crate::error::Result;
use crate::project::{BASE_COMPOSE, ENV_FILE, MARKETPLACE_COMPOSE, Project};
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

    // Desired artifacts, overlays in marketplace-id order, then the umbrella.
    let mut desired: Vec<(String, String)> = Vec::new();
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
        desired.push((model.compose_file.clone(), render_overlay(&spec)));
    }
    let overlays: Vec<String> = desired.iter().map(|(f, _)| f.clone()).collect();
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

    project.state.compose_files = vec![BASE_COMPOSE.to_string(), MARKETPLACE_COMPOSE.to_string()];
    project.state.rendered_files = desired.into_iter().map(|(f, _)| f).collect();
    project.save()?;
    Ok(report)
}

/// `compose.<something>.yml`, the shape of a model overlay.
fn is_overlay_name(name: &str) -> bool {
    name.starts_with("compose.")
        && name.ends_with(".yml")
        && name != MARKETPLACE_COMPOSE
        && name != BASE_COMPOSE
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
            vec!["compose.chapkit-ewars-model.yml", "compose.marketplace.yml"]
        );
        assert_eq!(
            project.state.rendered_files,
            vec!["compose.chapkit-ewars-model.yml", "compose.marketplace.yml"]
        );
        assert_eq!(report.summary(), "0 written, 2 unchanged, 0 removed");
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
        assert_eq!(report.summary(), "1 to write, 1 unchanged, 0 to remove");

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
    fn overlay_name_shape() {
        assert!(is_overlay_name("compose.chapkit-ewars-model.yml"));
        assert!(!is_overlay_name(MARKETPLACE_COMPOSE));
        assert!(!is_overlay_name(BASE_COMPOSE));
        assert!(!is_overlay_name("compose.yaml"));
        assert!(!is_overlay_name("notes.yml"));
    }
}
