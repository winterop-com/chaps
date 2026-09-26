//! `chaps ui` — the browser: marketplace models on one page, the components
//! this deployment is made of on the other.
//!
//! The browser returns a [`Selection`](crate::compose::Selection); applying
//! it goes through [`crate::compose::apply()`] outside the terminal, and the
//! resulting [`ApplyReport`] is printed afterwards. The containers of a
//! component the save switches off are stopped in between, while the compose
//! files that name them are still there.

use crate::cli::UiArgs;
use crate::commands::Ctx;
use crate::components::Components;
use crate::compose::{ApplyReport, apply};
use crate::error::Result;
use crate::tui::run_tui;

/// Open the browser against the current project and catalogue.
///
/// `--json` is rejected before this is reached.
pub fn run(ctx: &Ctx, _args: &UiArgs) -> Result<()> {
    // The browser edits a deployment, so there has to be one; the error already
    // tells the user to run `chaps init`.
    let mut project = ctx.project()?;
    let registry = super::registry_for(ctx, Some(&project))?;

    let Some(selection) = run_tui(ctx, &project, &registry)? else {
        return Ok(());
    };
    if selection.is_empty() {
        println!("no changes");
        return Ok(());
    }

    // Answered before anything is stopped or written, so a selection that was
    // never going to apply leaves the deployment exactly as it was.
    crate::compose::apply::validate(&project, &registry, &selection)?;

    // The containers of a component being switched off go now, while the
    // compose files that define them are still there: a service whose
    // definition has just been removed cannot be stopped by name, and one left
    // running keeps its host port published long after the component is gone.
    // Volumes are not touched - `--purge` stays a `components disable` flag.
    let before = project.state.components.clone();
    let after: Components = selection
        .components
        .clone()
        .unwrap_or_else(|| before.clone());
    let stopped = super::components::stop_disabled_components(&project, &before, &after);

    let endpoints = crate::manual::Endpoints::from_env(ctx.registry.offline);
    let report = apply(&mut project, &registry, &selection, &endpoints)?;
    ctx.out.emit(&report, || human(&report, &stopped))?;
    Ok(())
}

/// The human rendering of what was applied.
fn human(report: &ApplyReport, notes: &[String]) -> String {
    let mut text = String::new();
    for warning in &report.warnings {
        text.push_str(&format!("warning: {warning}\n"));
    }

    for (label, models) in [("enabled", &report.enabled), ("updated", &report.updated)] {
        if models.is_empty() {
            continue;
        }
        text.push_str(&format!("{label}:\n"));
        for (id, model) in models {
            text.push_str(&format!(
                "  {id}  {}  {}\n",
                model.version,
                match model.host_port {
                    Some(port) => format!("port {port}"),
                    None => "internal".to_string(),
                }
            ));
        }
    }

    if !report.disabled.is_empty() {
        text.push_str("disabled:\n");
        for id in &report.disabled {
            text.push_str(&format!("  {id}\n"));
        }
    }

    if !report.components_enabled.is_empty() {
        text.push_str("components on:\n");
        for (name, port) in &report.components_enabled {
            text.push_str(&format!(
                "  {name}  {}\n",
                match port {
                    Some(port) => format!("http://localhost:{port}"),
                    None => "internal".to_string(),
                }
            ));
        }
    }
    if !report.components_disabled.is_empty() {
        text.push_str("components off:\n");
        for name in &report.components_disabled {
            text.push_str(&format!("  {name}\n"));
        }
    }
    // What stopping the containers of a switched-off component did, and which
    // data volumes it left behind.
    for note in notes {
        text.push_str(&format!("note: {note}\n"));
    }

    if report.is_empty() {
        text.push_str("no changes\n");
    } else {
        text.push_str("run `chaps up` to apply the new compose files\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::EnabledModel;
    use crate::registry::Channel;

    fn model(port: Option<u16>) -> EnabledModel {
        EnabledModel {
            service_id: "chapkit-ewars-model".into(),
            image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
            image_tag: "sha-fa880a1".into(),
            version: "1.0.0".into(),
            channel: Some(Channel::Stable),
            host_port: port,
            data_dir: "/app/data".into(),
            user: "chapkit:chapkit".into(),
            user_from: Default::default(),
            platform: Some("linux/amd64".into()),
            compose_file: "compose.chapkit-ewars-model.yml".into(),
        }
    }

    #[test]
    fn a_report_lists_what_changed_and_what_to_do_next() {
        let report = ApplyReport {
            enabled: vec![("chapkit_ewars_model".to_string(), model(Some(5001)))],
            updated: vec![("auto_arima_chapkit".to_string(), model(Some(5002)))],
            disabled: vec!["chapkit_simple_multistep_model".to_string()],
            warnings: vec!["templates are not for real forecasts".to_string()],
            ..ApplyReport::default()
        };
        let text = human(&report, &[]);
        assert!(text.starts_with("warning: templates are not for real forecasts\n"));
        assert!(text.contains("enabled:\n  chapkit_ewars_model  1.0.0  port 5001\n"));
        assert!(text.contains("updated:\n  auto_arima_chapkit  1.0.0  port 5002\n"));
        assert!(text.contains("disabled:\n  chapkit_simple_multistep_model\n"));
        assert!(text.ends_with("run `chaps up` to apply the new compose files\n"));
    }

    #[test]
    fn an_empty_report_says_so() {
        assert_eq!(human(&ApplyReport::default(), &[]), "no changes\n");
    }

    /// The seam the browser exists for: keys in, files on disk out. Everything
    /// except the terminal itself is exercised here.
    #[test]
    fn what_the_browser_selects_applies_to_a_real_project() {
        use crate::project::Project;
        use crate::registry::load_embedded;
        use crate::tui::app::{App, Outcome};
        use crate::tui::keys::action_for;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let dir = tempfile::tempdir().expect("temp dir");
        let registry = load_embedded().expect("embedded snapshot parses");
        let mut project = Project {
            dir: dir.path().to_path_buf(),
            state: Default::default(),
        };

        let mut app = App::new(&registry, &project.state);
        let press = |code| KeyEvent::new(code, KeyModifiers::NONE);

        // Move down one, enable that model, then save.
        assert!(
            app.reduce(action_for(app.mode, &press(KeyCode::Char('j'))))
                .is_none()
        );
        assert!(
            app.reduce(action_for(app.mode, &press(KeyCode::Char(' '))))
                .is_none()
        );
        assert_eq!(
            app.reduce(action_for(app.mode, &press(KeyCode::Char('s')))),
            Some(Outcome::Save)
        );

        let selection = app.selection();
        assert_eq!(selection.enable.len(), 1);
        let report = crate::compose::apply::apply_with(
            &mut project,
            &registry,
            &selection,
            &|_| false,
            &crate::compose::resolve::from_table,
        )
        .expect("the browser's selection is applicable");

        assert_eq!(report.enabled.len(), 1);
        let (id, model) = &report.enabled[0];
        assert_eq!(id, &selection.enable[0].id);
        assert!(dir.path().join(&model.compose_file).is_file());
        assert!(dir.path().join(".chaps/models.yaml").is_file());
        assert!(dir.path().join("compose.marketplace.yml").is_file());
        assert!(human(&report, &[]).contains("run `chaps up`"));
    }

    /// The same seam for the second page: Tab, space, save - and a component
    /// compose file on disk.
    #[test]
    fn what_the_browser_selects_on_the_components_page_applies_too() {
        use crate::components::{Component, OCS_COMPOSE};
        use crate::project::Project;
        use crate::registry::load_embedded;
        use crate::tui::app::{App, Outcome};
        use crate::tui::keys::action_for;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let dir = tempfile::tempdir().expect("temp dir");
        let registry = load_embedded().expect("embedded snapshot parses");
        let mut project = Project {
            dir: dir.path().to_path_buf(),
            state: Default::default(),
        };

        let mut app = App::new(&registry, &project.state);
        let press = |code| KeyEvent::new(code, KeyModifiers::NONE);

        // Tab to the components page, down to ocs, space, save.
        app.reduce(action_for(app.mode, &press(KeyCode::Tab)));
        app.reduce(action_for(app.mode, &press(KeyCode::Char('j'))));
        assert_eq!(app.selected_component(), Component::Ocs);
        app.reduce(action_for(app.mode, &press(KeyCode::Char(' '))));
        assert_eq!(
            app.reduce(action_for(app.mode, &press(KeyCode::Char('s')))),
            Some(Outcome::Save)
        );

        let selection = app.selection();
        assert!(selection.enable.is_empty() && selection.disable.is_empty());
        assert!(!selection.is_empty(), "the component set is the change");
        let report = crate::compose::apply::apply_with(
            &mut project,
            &registry,
            &selection,
            &|_| false,
            &crate::compose::resolve::from_table,
        )
        .expect("the browser's component set is applicable");

        assert_eq!(
            report.components_enabled,
            vec![("ocs".to_string(), Some(9000))]
        );
        assert!(dir.path().join(OCS_COMPOSE).is_file());
        assert!(dir.path().join(".chaps/components.yaml").is_file());
        let text = human(&report, &["stopped nothing".to_string()]);
        assert!(
            text.contains("components on:\n  ocs  http://localhost:9000\n"),
            "{text}"
        );
        assert!(text.contains("note: stopped nothing\n"), "{text}");
        assert!(
            text.ends_with("run `chaps up` to apply the new compose files\n"),
            "{text}"
        );
    }

    /// A component that was switched off is reported as off, with the notes
    /// from stopping its containers under it.
    #[test]
    fn the_report_names_the_components_that_were_switched_off() {
        let report = ApplyReport {
            components_disabled: vec!["ocs".to_string()],
            removed: vec![std::path::PathBuf::from("compose.ocs.yml")],
            ..ApplyReport::default()
        };
        let text = human(
            &report,
            &[
                "stopped ocs".to_string(),
                "kept the volume ocs_data".to_string(),
            ],
        );
        assert!(text.contains("components off:\n  ocs\n"), "{text}");
        assert!(text.contains("note: stopped ocs\n"), "{text}");
        assert!(text.contains("note: kept the volume ocs_data\n"), "{text}");
        assert!(
            text.ends_with("run `chaps up` to apply the new compose files\n"),
            "{text}"
        );
    }
}
