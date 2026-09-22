//! `chaps tui` / `chaps models tui` — the model browser.
//!
//! Owned by agent C. The browser returns a
//! [`Selection`](crate::compose::Selection); applying it goes through
//! [`crate::compose::apply`] outside the terminal, and the resulting
//! [`ApplyReport`](crate::compose::ApplyReport) is printed afterwards.

use crate::cli::TuiArgs;
use crate::commands::Ctx;
use crate::compose::{ApplyReport, apply};
use crate::error::Result;
use crate::registry;
use crate::tui::run_tui;

/// Open the browser against the current project and catalogue.
///
/// `--json` is rejected before this is reached.
pub fn run(ctx: &Ctx, _args: &TuiArgs) -> Result<()> {
    // The browser edits a deployment, so there has to be one; the error already
    // tells the user to run `chaps init`.
    let mut project = ctx.project()?;
    let registry = registry::load(&ctx.registry)?;

    let Some(selection) = run_tui(ctx, &project, &registry)? else {
        return Ok(());
    };
    if selection.is_empty() {
        println!("no changes");
        return Ok(());
    }

    let report = apply(&mut project, &registry, &selection, ctx.cli_version)?;
    ctx.out.emit(&report, || human(&report))?;
    Ok(())
}

/// The human rendering of what was applied.
fn human(report: &ApplyReport) -> String {
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
                "  {id}  {}  port {}\n",
                model.version, model.host_port
            ));
        }
    }

    if !report.disabled.is_empty() {
        text.push_str("disabled:\n");
        for id in &report.disabled {
            text.push_str(&format!("  {id}\n"));
        }
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

    fn model(port: u16) -> EnabledModel {
        EnabledModel {
            service_id: "chapkit-ewars-model".into(),
            image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
            image_tag: "sha-fa880a1".into(),
            version: "1.0.0".into(),
            channel: Some(Channel::Stable),
            host_port: port,
            data_dir: "/app/data".into(),
            user: "chapkit:chapkit".into(),
            platform: Some("linux/amd64".into()),
            compose_file: "compose.chapkit-ewars-model.yml".into(),
        }
    }

    #[test]
    fn a_report_lists_what_changed_and_what_to_do_next() {
        let report = ApplyReport {
            enabled: vec![("chapkit_ewars_model".to_string(), model(5001))],
            updated: vec![("auto_arima_chapkit".to_string(), model(5002))],
            disabled: vec!["chapkit_simple_multistep_model".to_string()],
            warnings: vec!["templates are not for real forecasts".to_string()],
            ..ApplyReport::default()
        };
        let text = human(&report);
        assert!(text.starts_with("warning: templates are not for real forecasts\n"));
        assert!(text.contains("enabled:\n  chapkit_ewars_model  1.0.0  port 5001\n"));
        assert!(text.contains("updated:\n  auto_arima_chapkit  1.0.0  port 5002\n"));
        assert!(text.contains("disabled:\n  chapkit_simple_multistep_model\n"));
        assert!(text.ends_with("run `chaps up` to apply the new compose files\n"));
    }

    #[test]
    fn an_empty_report_says_so() {
        assert_eq!(human(&ApplyReport::default()), "no changes\n");
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
        let report = apply(&mut project, &registry, &selection, "0.0.0-test")
            .expect("the browser's selection is applicable");

        assert_eq!(report.enabled.len(), 1);
        let (id, model) = &report.enabled[0];
        assert_eq!(id, &selection.enable[0].id);
        assert!(dir.path().join(&model.compose_file).is_file());
        assert!(dir.path().join(".chaps/models.yaml").is_file());
        assert!(dir.path().join("compose.marketplace.yml").is_file());
        assert!(human(&report).contains("run `chaps up`"));
    }
}
