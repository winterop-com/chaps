use super::*;
use crate::compose::render::NO_TAG_PINS;
use crate::error::PortHolder;
use crate::project::{
    DEFAULT_PORT_RANGE, ENV_FILE, MARKETPLACE_COMPOSE, ProjectState, default_compose_files,
};
use crate::registry::{Channel, load_embedded};
use serde_yaml_ng::Value;
use tempfile::TempDir;

/// Nothing is listening on this machine, as far as these tests care.
fn all_free(_: u16) -> bool {
    false
}

/// [`apply`] with the host probe and the user lookup stubbed out, which is
/// how every test here runs: either one would make the result depend on
/// the machine - what it listens on, and what it has pulled.
fn apply(project: &mut Project, registry: &Registry, sel: &Selection) -> Result<ApplyReport> {
    apply_with(project, registry, sel, &all_free, &resolve::from_table)
}

fn project() -> (TempDir, Project) {
    let dir = tempfile::tempdir().unwrap();
    let project = Project {
        dir: dir.path().to_path_buf(),
        state: ProjectState::default(),
    };
    (dir, project)
}

fn enable(ids: &[&str]) -> Selection {
    Selection {
        enable: ids.iter().map(|id| EnableRequest::new(*id)).collect(),
        ..Selection::default()
    }
}

/// A selection that only asks for a component set.
fn components(build: impl Fn(&mut Components)) -> Selection {
    let mut wanted = Components::default();
    build(&mut wanted);
    Selection {
        components: Some(wanted),
        ..Selection::default()
    }
}

/// The same, asking for an automatically allocated host port.
fn publish(ids: &[&str]) -> Selection {
    let mut sel = enable(ids);
    for req in &mut sel.enable {
        req.port = Some(PortRequest::Auto);
    }
    sel
}

/// The version and image tag the snapshot's `stable` channel points at,
/// so the assertions below follow a marketplace refresh instead of
/// pinning a sha that the next one invalidates.
fn stable_pin(registry: &Registry, id: &str) -> (String, String) {
    let version = registry
        .get(id)
        .expect("vendored model")
        .resolve(&VersionSelector::Channel(Channel::Stable))
        .expect("stable resolves");
    (version.version.clone(), version.image_tag.clone())
}

fn umbrella_includes(project: &Project) -> Vec<String> {
    let body = std::fs::read_to_string(project.dir.join(MARKETPLACE_COMPOSE)).unwrap();
    let doc: Value = serde_yaml_ng::from_str(&body).unwrap();
    doc.get("include")
        .and_then(Value::as_sequence)
        .map(|s| s.iter().map(|v| v.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

#[test]
fn apply_writes_the_overlay_the_umbrella_and_the_state() {
    let registry = load_embedded().unwrap();
    let (dir, mut project) = project();
    let report = apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();

    assert_eq!(report.enabled.len(), 1);
    assert!(report.updated.is_empty() && report.disabled.is_empty());
    let (id, entry) = &report.enabled[0];
    assert_eq!(id, "chapkit_ewars_model");
    assert_eq!(
        entry.host_port, None,
        "a model publishes no host port unless one was asked for"
    );
    assert_eq!(entry.channel, Some(Channel::Stable));
    assert_eq!(
        entry.image_tag,
        stable_pin(&registry, "chapkit_ewars_model").1
    );
    assert_eq!(entry.data_dir, "/app/data");
    assert_eq!(entry.user, "1000:1000");
    assert_eq!(entry.user_from, UserSource::Table, "the test resolver");
    assert_eq!(entry.platform.as_deref(), Some("linux/amd64"));
    assert_eq!(entry.compose_file, "compose.chapkit-ewars-model.yml");

    assert!(dir.path().join("compose.chapkit-ewars-model.yml").is_file());
    assert_eq!(
        umbrella_includes(&project),
        vec!["compose.chapkit-ewars-model.yml"]
    );
    assert_eq!(project.state.compose_files, default_compose_files());

    // .chaps/ is written, not just held in memory.
    let reloaded = Project::load(dir.path()).unwrap();
    assert_eq!(reloaded.state.models["chapkit_ewars_model"], *entry);
}

#[test]
fn auto_hands_out_the_lowest_free_port_and_none_publishes_nothing() {
    let registry = load_embedded().unwrap();
    let (_dir, mut project) = project();
    let report = apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
    assert_eq!(report.enabled[0].1.host_port, Some(5001));

    let report = apply(&mut project, &registry, &publish(&["auto_arima_chapkit"])).unwrap();
    assert_eq!(report.enabled[0].1.host_port, Some(5002));

    // A third one, left internal, takes no port at all.
    let report = apply(
        &mut project,
        &registry,
        &enable(&["chapkit_simple_multistep_model"]),
    )
    .unwrap();
    assert_eq!(report.enabled[0].1.host_port, None);
    assert_eq!(project.used_ports(), BTreeSet::from([5001, 5002]));

    // The umbrella lists every overlay, sorted by marketplace id.
    assert_eq!(
        umbrella_includes(&project),
        vec![
            "compose.auto-arima-chapkit.yml",
            "compose.chapkit-ewars-model.yml",
            "compose.chapkit-simple-multistep-model.yml",
        ]
    );
}

#[test]
fn auto_skips_a_port_something_on_this_machine_is_listening_on() {
    let registry = load_embedded().unwrap();
    let (_dir, mut project) = project();
    let busy = |port: u16| (5001..=5003).contains(&port);
    let report = apply_with(
        &mut project,
        &registry,
        &publish(&["chapkit_ewars_model"]),
        &busy,
        &resolve::from_table,
    )
    .unwrap();
    assert_eq!(report.enabled[0].1.host_port, Some(5004));
}

#[test]
fn an_explicit_port_is_claimed_and_conflicts_say_who_holds_it() {
    let registry = load_embedded().unwrap();
    let (_dir, mut project) = project();
    let mut sel = enable(&["chapkit_ewars_model"]);
    sel.enable[0].port = Some(PortRequest::Fixed(5100));
    apply(&mut project, &registry, &sel).unwrap();
    assert_eq!(
        project.state.models["chapkit_ewars_model"].host_port,
        Some(5100)
    );

    let mut clash = enable(&["auto_arima_chapkit"]);
    clash.enable[0].port = Some(PortRequest::Fixed(5100));
    let err = apply(&mut project, &registry, &clash).expect_err("5100 is taken");
    assert!(matches!(
        err.downcast_ref::<ChapError>(),
        Some(ChapError::PortInUse {
            port: 5100,
            holder: PortHolder::ComposeFile
        })
    ));

    // A port nothing in the project claims, but that the machine does.
    let mut listening = enable(&["auto_arima_chapkit"]);
    listening.enable[0].port = Some(PortRequest::Fixed(5200));
    let err = apply_with(
        &mut project,
        &registry,
        &listening,
        &|port| port == 5200,
        &resolve::from_table,
    )
    .expect_err("something is listening on 5200");
    assert!(matches!(
        err.downcast_ref::<ChapError>(),
        Some(ChapError::PortInUse {
            port: 5200,
            holder: PortHolder::Host
        })
    ));
    assert!(!project.state.models.contains_key("auto_arima_chapkit"));

    let mut out_of_range = enable(&["auto_arima_chapkit"]);
    out_of_range.enable[0].port = Some(PortRequest::Fixed(80));
    let err = apply(&mut project, &registry, &out_of_range).expect_err("out of range");
    assert!(matches!(
        err.downcast_ref::<ChapError>(),
        Some(ChapError::PortOutOfRange { port: 80, .. })
    ));
}

#[test]
fn re_enabling_keeps_the_port_and_reports_an_update() {
    let registry = load_embedded().unwrap();
    let (_dir, mut project) = project();
    apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
    apply(&mut project, &registry, &publish(&["auto_arima_chapkit"])).unwrap();

    let mut sel = enable(&["chapkit_ewars_model"]);
    sel.enable[0].selector = VersionSelector::Exact(stable_pin(&registry, "chapkit_ewars_model").0);
    sel.enable[0].user = Some("1000:1000".into());
    let report = apply(&mut project, &registry, &sel).unwrap();
    assert!(report.enabled.is_empty());
    assert_eq!(report.updated.len(), 1);
    let entry = &report.updated[0].1;
    assert_eq!(
        entry.host_port,
        Some(5001),
        "the port is kept across a rewrite"
    );
    assert_eq!(entry.user, "1000:1000");
    // An exact pin follows no channel.
    assert_eq!(entry.channel, None);

    // An explicit request is what takes the port away again, and frees it.
    let mut sel = enable(&["chapkit_ewars_model"]);
    sel.enable[0].port = Some(PortRequest::None);
    let report = apply(&mut project, &registry, &sel).unwrap();
    assert_eq!(report.updated[0].1.host_port, None);
    assert_eq!(project.used_ports(), BTreeSet::from([5002]));

    // And asking for it back lands on 5001 once more.
    let report = apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
    assert_eq!(report.updated[0].1.host_port, Some(5001));
}

#[test]
fn disable_removes_the_overlay_and_frees_its_port() {
    let registry = load_embedded().unwrap();
    let (dir, mut project) = project();
    apply(
        &mut project,
        &registry,
        &publish(&["chapkit_ewars_model", "auto_arima_chapkit"]),
    )
    .unwrap();

    // The service id is accepted as well as the marketplace id.
    let sel = Selection {
        disable: vec!["chapkit-ewars-model".into()],
        ..Selection::default()
    };
    let report = apply(&mut project, &registry, &sel).unwrap();
    assert_eq!(report.disabled, vec!["chapkit_ewars_model"]);
    assert_eq!(report.removed.len(), 1);
    assert!(!dir.path().join("compose.chapkit-ewars-model.yml").exists());
    assert_eq!(
        umbrella_includes(&project),
        vec!["compose.auto-arima-chapkit.yml"]
    );

    // 5001 is free again.
    let report = apply(
        &mut project,
        &registry,
        &publish(&["chapkit_simple_multistep_model"]),
    )
    .unwrap();
    assert_eq!(report.enabled[0].1.host_port, Some(5001));
}

#[test]
fn disabling_something_that_is_not_enabled_is_an_error() {
    let registry = load_embedded().unwrap();
    let (_dir, mut project) = project();
    let sel = Selection {
        disable: vec!["auto_arima_chapkit".into()],
        ..Selection::default()
    };
    let err = apply(&mut project, &registry, &sel).expect_err("not enabled");
    assert!(matches!(
        err.downcast_ref::<ChapError>(),
        Some(ChapError::UnknownModel(id)) if id == "auto_arima_chapkit"
    ));
}

#[test]
fn an_unknown_model_is_rejected_before_anything_is_written() {
    let registry = load_embedded().unwrap();
    let (dir, mut project) = project();
    let err = apply(&mut project, &registry, &enable(&["nope"])).expect_err("unknown");
    assert!(matches!(
        err.downcast_ref::<ChapError>(),
        Some(ChapError::UnknownModel(id)) if id == "nope"
    ));
    assert!(!dir.path().join(".chaps").exists());
    assert!(!dir.path().join(MARKETPLACE_COMPOSE).exists());
}

#[test]
fn a_template_needs_allow_template_and_warns() {
    let registry = load_embedded().unwrap();
    let (_dir, mut project) = project();
    let err = apply(
        &mut project,
        &registry,
        &enable(&["chapkit_minimalist_example_py"]),
    )
    .expect_err("templates are not deployable");
    assert!(matches!(
        err.downcast_ref::<ChapError>(),
        Some(ChapError::IsTemplate(_))
    ));

    let mut sel = enable(&["chapkit_minimalist_example_py"]);
    sel.enable[0].allow_template = true;
    let report = apply(&mut project, &registry, &sel).unwrap();
    assert_eq!(report.enabled.len(), 1);
    assert_eq!(report.warnings.len(), 1);
    assert!(report.warnings[0].contains("template"));
}

/// Publishing a host port is no reason to move the version a deployment
/// runs: that is what the browser's port-only change carries
/// `keep_version` for.
#[test]
fn keep_version_leaves_the_recorded_pin_exactly_where_it_is() {
    let registry = load_embedded().unwrap();
    let (dir, mut project) = project();
    apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();

    // A deployment running a version pinned exactly, which is what the
    // marketplace moving on looks like from here.
    let entry = project
        .state
        .models
        .get_mut("chapkit_ewars_model")
        .expect("just enabled");
    entry.version = "0.9.0".to_string();
    entry.image_tag = "sha-older".to_string();
    entry.channel = None;

    let mut sel = publish(&["chapkit_ewars_model"]);
    sel.enable[0].keep_version = true;
    let report = apply(&mut project, &registry, &sel).unwrap();
    let entry = &report.updated[0].1;
    assert_eq!(entry.host_port, Some(5001), "the port change went through");
    assert_eq!(entry.version, "0.9.0");
    assert_eq!(entry.image_tag, "sha-older");
    assert_eq!(entry.channel, None, "an exact pin follows no channel");
    let overlay = std::fs::read_to_string(dir.path().join("compose.chapkit-ewars-model.yml"))
        .expect("the overlay was rewritten");
    assert!(overlay.contains("sha-older"), "{overlay}");

    // The same request without it is what asks the registry again.
    let report = apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
    let entry = &report.updated[0].1;
    let (version, tag) = stable_pin(&registry, "chapkit_ewars_model");
    assert_eq!(entry.version, version);
    assert_eq!(entry.image_tag, tag);
    assert_eq!(entry.channel, Some(Channel::Stable));

    // A model that is not enabled has no version to keep, so it resolves.
    let mut sel = enable(&["auto_arima_chapkit"]);
    sel.enable[0].keep_version = true;
    let report = apply(&mut project, &registry, &sel).unwrap();
    assert_eq!(
        report.enabled[0].1.version,
        stable_pin(&registry, "auto_arima_chapkit").0
    );
}

/// `init --force` deletes the previous deployment's overlays before
/// handing over to `apply`, so it asks first. Every answer has to be
/// available while all of it is still on disk.
#[test]
fn validate_answers_for_the_whole_selection_and_touches_nothing() {
    let registry = load_embedded().unwrap();
    let (dir, mut project) = project();

    let err = validate(
        &project,
        &registry,
        &enable(&["chapkit_minimalist_example_py"]),
    )
    .expect_err("templates are not deployable");
    assert!(matches!(
        err.downcast_ref::<ChapError>(),
        Some(ChapError::IsTemplate(_))
    ));
    let err = validate(&project, &registry, &enable(&["nope"])).expect_err("unknown");
    assert!(matches!(
        err.downcast_ref::<ChapError>(),
        Some(ChapError::UnknownModel(id)) if id == "nope"
    ));
    let sel = Selection {
        disable: vec!["auto_arima_chapkit".into()],
        ..Selection::default()
    };
    let err = validate(&project, &registry, &sel).expect_err("not enabled");
    assert!(matches!(
        err.downcast_ref::<ChapError>(),
        Some(ChapError::UnknownModel(_))
    ));
    let mut sel = enable(&["chapkit_ewars_model"]);
    sel.enable[0].selector = VersionSelector::Exact("9.9.9".into());
    let err = validate(&project, &registry, &sel).expect_err("no such version");
    assert!(matches!(
        err.downcast_ref::<ChapError>(),
        Some(ChapError::UnknownVersion { .. })
    ));

    // A selection that would apply says nothing, and none of these
    // questions wrote anything.
    validate(&project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();
    assert!(!dir.path().join(".chaps").exists());
    assert!(!dir.path().join(MARKETPLACE_COMPOSE).exists());
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());

    // And what it allows, apply() goes on to do.
    apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();
}

#[test]
fn the_umbrella_is_written_even_with_no_models() {
    let registry = load_embedded().unwrap();
    let (dir, mut project) = project();
    let report = apply(&mut project, &registry, &Selection::default()).unwrap();
    assert!(!report.is_empty(), "the umbrella was written");
    let body = std::fs::read_to_string(dir.path().join(MARKETPLACE_COMPOSE)).unwrap();
    assert!(body.contains("services: {}"));
    assert!(umbrella_includes(&project).is_empty());
}

#[test]
fn ports_already_published_by_a_hand_written_overlay_are_skipped() {
    let registry = load_embedded().unwrap();
    let (dir, mut project) = project();
    std::fs::write(
        dir.path().join("compose.mine.yml"),
        "services:\n  mine:\n    ports:\n      - \"5001:8000\"\n",
    )
    .unwrap();
    let report = apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
    assert_eq!(report.enabled[0].1.host_port, Some(5002));
}

#[test]
fn env_pins_are_appended_once_and_nothing_else_is_touched() {
    let registry = load_embedded().unwrap();
    let (dir, mut project) = project();
    let env = dir.path().join(ENV_FILE);
    std::fs::write(
        &env,
        format!("POSTGRES_PASSWORD=secret\n# pins\n{NO_TAG_PINS}\n"),
    )
    .unwrap();

    apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();
    let body = std::fs::read_to_string(&env).unwrap();
    assert!(body.starts_with("POSTGRES_PASSWORD=secret\n"));
    assert!(body.contains(&format!(
        "\n# CHAPKIT_EWARS_MODEL_IMAGE_TAG={}\n",
        stable_pin(&registry, "chapkit_ewars_model").1
    )));

    // A second run adds nothing; only the new model's pin appears.
    apply(&mut project, &registry, &enable(&["auto_arima_chapkit"])).unwrap();
    let body = std::fs::read_to_string(&env).unwrap();
    assert_eq!(body.matches("CHAPKIT_EWARS_MODEL_IMAGE_TAG").count(), 1);
    assert!(body.contains(&format!(
        "\n# AUTO_ARIMA_CHAPKIT_IMAGE_TAG={}\n",
        stable_pin(&registry, "auto_arima_chapkit").1
    )));
    assert!(
        !body.contains("none yet"),
        "the placeholder gives way to pins"
    );
}

#[test]
fn without_an_env_file_nothing_is_created() {
    let registry = load_embedded().unwrap();
    let (dir, mut project) = project();
    apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();
    assert!(!dir.path().join(ENV_FILE).exists());
}

/// A selection that says nothing about models still applies: the component
/// set goes into `.chaps/components.yaml` and the same sync renders the
/// file for it.
#[test]
fn a_selection_that_only_changes_a_component_renders_its_compose_file() {
    let registry = load_embedded().unwrap();
    let (dir, mut project) = project();
    let sel = components(|wanted| wanted.set_enabled(Component::Ocs, true));
    assert!(!sel.is_empty(), "a wanted component set is a change");

    let report = apply(&mut project, &registry, &sel).unwrap();
    assert_eq!(
        report.components_enabled,
        vec![("ocs".to_string(), Some(crate::components::OCS_DEFAULT_PORT))]
    );
    assert!(report.components_disabled.is_empty());
    assert!(report.enabled.is_empty() && report.disabled.is_empty());
    assert!(project.state.components.ocs.enabled);
    assert!(dir.path().join(crate::components::OCS_COMPOSE).is_file());
    // And it is written, not just held in memory.
    let reloaded = Project::load(dir.path()).unwrap();
    assert!(reloaded.state.components.ocs.enabled);

    // Turning it off again removes what was rendered for it, and says so.
    let report = apply(
        &mut project,
        &registry,
        &components(|wanted| wanted.set_enabled(Component::Ocs, false)),
    )
    .unwrap();
    assert_eq!(report.components_disabled, vec!["ocs".to_string()]);
    assert!(report.components_enabled.is_empty());
    assert!(!dir.path().join(crate::components::OCS_COMPOSE).exists());

    // A set that matches what the project has changes nothing about it.
    let report = apply(&mut project, &registry, &components(|_| {})).unwrap();
    assert!(report.components_enabled.is_empty());
    assert!(report.components_disabled.is_empty());
}

/// Compose would merge two models on one service into one, so a model
/// whose service name another enabled model holds is refused, unless the
/// same selection disables that other model.
#[test]
fn two_models_cannot_share_a_compose_service() {
    let registry = load_embedded().unwrap();
    let (_dir, mut project) = project();
    apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();
    let mut twin = project.state.models["chapkit_ewars_model"].clone();
    twin.image = "ewars".to_string();
    project.state.models.insert("ewars_dev".to_string(), twin);

    let err = apply(&mut project, &registry, &enable(&["chapkit_ewars_model"]))
        .expect_err("ewars_dev holds the service");
    assert!(
        err.to_string().contains("`chaps models disable ewars_dev`"),
        "{err}"
    );

    let mut swap = enable(&["chapkit_ewars_model"]);
    swap.disable = vec!["ewars_dev".to_string()];
    apply(&mut project, &registry, &swap).unwrap();
}

/// A selection that takes chap-core away leaves the models running on
/// their own, and publishes a host port for each one that had none: with
/// nothing on the compose network to reach them, that port is the way in.
#[test]
fn turning_chap_core_off_publishes_the_models_it_leaves_behind() {
    let registry = load_embedded().unwrap();
    let (dir, mut project) = project();
    apply(&mut project, &registry, &enable(&["chapkit_ewars_model"])).unwrap();
    assert_eq!(project.state.models["chapkit_ewars_model"].host_port, None);

    let off = components(|wanted| wanted.set_enabled(Component::ChapCore, false));
    let report = apply(&mut project, &registry, &off).unwrap();
    assert_eq!(report.components_disabled, vec!["chap-core".to_string()]);
    assert_eq!(report.updated.len(), 1);
    assert_eq!(
        project.state.models["chapkit_ewars_model"].host_port,
        Some(5001)
    );

    let overlay =
        std::fs::read_to_string(dir.path().join("compose.chapkit-ewars-model.yml")).unwrap();
    assert!(
        !overlay.contains("SERVICEKIT_ORCHESTRATOR_URL"),
        "{overlay}"
    );
    assert!(!overlay.contains("chap:\n"), "waits for no chap: {overlay}");
    assert!(overlay.contains("\"5001:8000\""), "{overlay}");
}

/// A model enabled into a deployment without chap-core is published on a
/// host port straight away, unless the caller explicitly asked for none.
#[test]
fn a_model_enabled_without_chap_core_gets_a_host_port() {
    let registry = load_embedded().unwrap();
    let (_dir, mut project) = project();
    let mut standalone = components(|wanted| wanted.set_enabled(Component::ChapCore, false));
    standalone.enable = vec![EnableRequest::new("chapkit_ewars_model")];
    let report = apply(&mut project, &registry, &standalone).unwrap();
    assert_eq!(report.enabled[0].1.host_port, Some(5001));

    let mut none = Selection::default();
    let mut req = EnableRequest::new("chapkit_ewars_model");
    req.port = Some(PortRequest::None);
    none.enable = vec![req];
    let report = apply(&mut project, &registry, &none).unwrap();
    assert_eq!(
        report.updated[0].1.host_port, None,
        "asked for none, got none"
    );
}

/// A component's host port gets the same question a model's does, and the
/// answer is a warning rather than a refusal - the deployment is not up
/// yet, and the port is still easy to change.
#[test]
fn a_component_port_something_is_listening_on_is_a_warning() {
    let registry = load_embedded().unwrap();
    let (_dir, mut project) = project();
    let sel = components(|wanted| {
        wanted.set_enabled(Component::Ocs, true);
        wanted.ocs.port = Some(18094);
    });
    let report = apply_with(
        &mut project,
        &registry,
        &sel,
        &|port| port == 18094,
        &resolve::from_table,
    )
    .unwrap();
    assert!(
        report.warnings.iter().any(|line| line.contains("18094")),
        "{:?}",
        report.warnings
    );
    assert_eq!(
        project.state.components.ocs.port,
        Some(18094),
        "it went through anyway"
    );

    // A port nothing holds says nothing.
    let report = apply(
        &mut project,
        &registry,
        &components(|wanted| {
            wanted.set_enabled(Component::Ocs, true);
            wanted.ocs.port = Some(18095);
        }),
    )
    .unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn a_port_range_from_a_narrower_port_base_is_honoured() {
    let registry = load_embedded().unwrap();
    let (_dir, mut project) = project();
    project.state.port_range = (5500, DEFAULT_PORT_RANGE.1);
    let report = apply(&mut project, &registry, &publish(&["chapkit_ewars_model"])).unwrap();
    assert_eq!(report.enabled[0].1.host_port, Some(5500));
}
