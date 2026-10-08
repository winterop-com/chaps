use super::*;

fn render(build: impl FnOnce(&mut Report)) -> String {
    let mut lines = Report::default();
    build(&mut lines);
    lines.text()
}
use crate::project::{EnabledModel, ProjectState};
use std::collections::BTreeMap;

fn project_with_ewars(host_port: Option<u16>) -> Project {
    let model = EnabledModel {
        reads_port: false,
        service_id: "chapkit-ewars-model".into(),
        image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
        image_tag: "sha-fa880a1".into(),
        version: "1.0.0".into(),
        channel: Some(Channel::Stable),
        host_port,
        bind: None,
        data_dir: "/app/data".into(),
        user: "chapkit:chapkit".into(),
        user_from: Default::default(),
        platform: Some("linux/amd64".into()),
        compose_file: "compose.chapkit-ewars-model.yml".into(),
    };
    Project {
        dir: std::path::PathBuf::from("/tmp/chapx"),
        state: ProjectState {
            models: BTreeMap::from([("chapkit_ewars_model".to_string(), model)]),
            ..ProjectState::default()
        },
    }
}

/// What `--purge` calls a model that `.varde/models.yaml` no longer holds,
/// and therefore which volume it removes.
#[test]
fn a_purge_names_the_volume_by_the_marketplace_id() {
    let registry = crate::registry::load_embedded().expect("the embedded snapshot parses");
    for typed in ["chapkit_ewars_model", "chapkit-ewars-model"] {
        let listed = registry.get(typed).map(|model| model.id.as_str());
        assert_eq!(purge_id(listed, typed), "chapkit_ewars_model");
        assert_eq!(
            crate::compose::volume_name(&purge_id(listed, typed)),
            "ck_chapkit_ewars_model_data"
        );
    }

    // An entry the marketplace no longer lists is only the name that was
    // typed: a service id folds back to the id that named the volume, and
    // a marketplace id is already it.
    assert_eq!(purge_id(None, "left-the-market"), "left_the_market");
    assert_eq!(purge_id(None, "left_the_market"), "left_the_market");
}

#[test]
fn enabled_id_accepts_both_identifiers() {
    let project = project_with_ewars(None);
    assert_eq!(
        enabled_id(&project, "chapkit_ewars_model").as_deref(),
        Some("chapkit_ewars_model")
    );
    assert_eq!(
        enabled_id(&project, "chapkit-ewars-model").as_deref(),
        Some("chapkit_ewars_model")
    );
    assert_eq!(enabled_id(&project, "auto_arima_chapkit"), None);
}

/// An `enabled` report for the one model `project_with_ewars` holds.
fn enabled_report(project: &Project) -> ApplyReport {
    ApplyReport {
        enabled: vec![(
            "chapkit_ewars_model".to_string(),
            project.state.models["chapkit_ewars_model"].clone(),
        )],
        ..ApplyReport::default()
    }
}

#[test]
fn the_summary_ends_with_the_next_step() {
    let project = project_with_ewars(Some(5001));
    let text = render(|lines| summary(&enabled_report(&project), &[], &project, lines));
    assert!(text.contains("enabled chapkit_ewars_model v1.0.0 on http://localhost:5001"));
    assert!(text.ends_with("run `varde up` to apply\n"));
}

#[test]
fn a_model_with_no_host_port_is_summarised_with_the_proxy_url() {
    let project = project_with_ewars(None);
    let text = render(|lines| summary(&enabled_report(&project), &[], &project, lines));
    assert!(
        text.contains(
            "enabled chapkit_ewars_model v1.0.0 at \
                 http://localhost:8700/v2/services/chapkit-ewars-model/run/"
        ),
        "{text}"
    );
}

/// A manually added model's version is its image tag; printing `v` in
/// front of it would claim a version number it does not have.
#[test]
fn a_manual_models_line_names_its_tag_rather_than_a_version() {
    let mut project = project_with_ewars(None);
    let entry = project
        .state
        .models
        .get_mut("chapkit_ewars_model")
        .expect("just built");
    entry.version = "sha-b1d6c31".into();
    entry.image_tag = "sha-b1d6c31".into();
    let text = render(|lines| summary(&enabled_report(&project), &[], &project, lines));
    assert!(
        text.contains("enabled chapkit_ewars_model sha-b1d6c31 at "),
        "{text}"
    );
    assert!(!text.contains("vsha-"), "{text}");
}

#[test]
fn a_disable_summary_names_the_removed_file() {
    let project = project_with_ewars(Some(5001));
    let report = ApplyReport {
        disabled: vec!["chapkit_ewars_model".to_string()],
        removed: vec![project.dir.join("compose.chapkit-ewars-model.yml")],
        ..ApplyReport::default()
    };
    let text = render(|lines| summary(&report, &[], &project, lines));
    assert!(text.contains("disabled chapkit_ewars_model"));
    assert!(text.contains("hint: removed compose.chapkit-ewars-model.yml"));
    // Nothing is left for `up` to apply once a model is taken away.
    assert!(text.ends_with("hint: `varde status` shows what runs now\n"));
}

#[test]
fn a_port_change_says_what_happened_and_what_to_do_next() {
    let project = project_with_ewars(None);
    let exposed = PortChange {
        id: "chapkit_ewars_model".into(),
        service_id: "chapkit-ewars-model".into(),
        host_port: Some(5001),
        previous: None,
        url: "http://localhost:5001".into(),
        written: vec![project.dir.join("compose.chapkit-ewars-model.yml")],
        applied: false,
    };
    let text = render(|lines| port_summary(&exposed, &project, &[], lines));
    assert!(text.starts_with("exposed chapkit-ewars-model on http://localhost:5001\n"));
    assert!(text.contains("hint: wrote compose.chapkit-ewars-model.yml\n"));
    assert!(text.ends_with("run `varde up` to apply\n"));
    assert!(!text.contains("no change"));

    let internal = PortChange {
        host_port: None,
        previous: Some(5001),
        url: project.proxy_url("chapkit-ewars-model"),
        written: Vec::new(),
        ..exposed
    };
    let text = render(|lines| port_summary(&internal, &project, &["careful".to_string()], lines));
    assert!(text.starts_with(
        "unexposed chapkit-ewars-model; it stays registered with chap-core and \
             reachable at http://localhost:8700/v2/services/chapkit-ewars-model/run/\n"
    ));
    assert!(text.contains("warning: careful\n"));

    // Asking for what is already there is a no-op worth saying out loud.
    let again = PortChange {
        host_port: None,
        previous: None,
        ..internal
    };
    assert!(render(|lines| port_summary(&again, &project, &[], lines)).contains("(no change)"));
}

/// A port that is already there and already published has no apply step.
#[test]
fn an_applied_port_with_no_change_names_no_apply_step() {
    let project = project_with_ewars(Some(5001));
    let change = PortChange {
        id: "chapkit_ewars_model".into(),
        service_id: "chapkit-ewars-model".into(),
        host_port: Some(5001),
        previous: Some(5001),
        url: "http://localhost:5001".into(),
        written: Vec::new(),
        applied: true,
    };
    let text = render(|lines| port_summary(&change, &project, &[], lines));
    assert_eq!(
        text,
        "exposed chapkit-ewars-model on http://localhost:5001 (no change)\n"
    );
    let pending = PortChange {
        applied: false,
        ..change
    };
    let text = render(|lines| port_summary(&pending, &project, &[], lines));
    assert!(
        text.ends_with("(no change)\nrun `varde up` to apply\n"),
        "{text}"
    );
}
