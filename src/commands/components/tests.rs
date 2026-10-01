use super::disable::*;
use super::enable::*;
use super::*;
use crate::cli::{ComponentPortArg, ComponentsEnableArgs, OcsConfigArgs};
use crate::components::{DHIS2_COMPOSE, OCS_DEFAULT_PORT, S3Component};

#[test]
fn the_rows_cover_every_component_in_order() {
    let rows = rows(&Components::default(), 8000);
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["chap-core", "ocs", "s3", "dhis2"]);
    assert!(rows[0].enabled && rows[1..].iter().all(|r| !r.enabled));
    assert!(
        rows[0].compose_file.is_none(),
        "chap-core is the base stack"
    );
    assert_eq!(rows[1].compose_file.as_deref(), Some("compose.ocs.yml"));
    assert_eq!(rows[0].port, Some(8000), "chap-core's port is the API port");
    assert!(rows[1..].iter().all(|r| r.port.is_none()));
}

#[test]
fn an_enabled_component_shows_the_port_it_publishes() {
    let mut components = Components::default();
    components.set_enabled(Component::Ocs, true);
    components.set_enabled(Component::S3, true);
    let listed = rows(&components, 8000);
    assert_eq!(listed[1].port, Some(OCS_DEFAULT_PORT));
    assert_eq!(
        listed[2].port, None,
        "the store publishes nothing by default"
    );

    components.s3 = S3Component {
        enabled: true,
        port: Some(9002),
    };
    assert_eq!(rows(&components, 8000)[2].port, Some(9002));
}

#[test]
fn a_port_is_recorded_on_the_component_that_can_take_one() {
    let mut components = Components::default();
    set_port(&mut components, Component::Ocs, Some(9010)).unwrap();
    assert_eq!(components.ocs.port, Some(9010));
    set_port(&mut components, Component::S3, Some(9011)).unwrap();
    assert_eq!(components.s3.port, Some(9011));
    set_port(&mut components, Component::Dhis2, Some(18080)).unwrap();
    assert_eq!(components.dhis2.port, Some(18080));
    set_port(&mut components, Component::Dhis2, None).unwrap();
    assert_eq!(components.dhis2.port, None);

    // `--port none` is how a published component becomes an internal one,
    // and it reads the same on both.
    set_port(&mut components, Component::Ocs, None).unwrap();
    assert_eq!(components.ocs.port, None);
    set_port(&mut components, Component::S3, None).unwrap();
    assert_eq!(components.s3.port, None);

    let err = set_port(&mut components, Component::ChapCore, Some(8123))
        .expect_err("chap-core's port is the API port");
    assert!(err.to_string().contains("CHAP_API_PORT=8123"), "{err}");
}

/// `--base-url` has to be a URL OCS can append a path to, and clearing it
/// has to be possible: an instance that came out from behind a proxy should
/// not keep advertising the proxy's address.
#[test]
fn the_base_url_is_absolute_or_refused() {
    let args = |value: Option<&str>| ComponentsEnableArgs {
        name: "ocs".to_string(),
        port: None,
        base_url: value.map(str::to_string),
        read_only: false,
        read_write: false,
        url: None,
        models_host: None,
        tag: None,
        image: None,
        ocs: OcsConfigArgs::default(),
    };
    assert_eq!(base_url(&args(None)).unwrap(), None, "the flag was absent");
    assert_eq!(
        base_url(&args(Some("https://ocs.example.org/"))).unwrap(),
        Some(Some("https://ocs.example.org".to_string())),
        "the trailing slash goes: OCS appends a path to this"
    );
    assert_eq!(
        base_url(&args(Some("  "))).unwrap(),
        Some(None),
        "an empty value clears it"
    );
    let err = base_url(&args(Some("ocs.example.org"))).expect_err("no scheme");
    assert!(err.to_string().contains("absolute URL"), "{err}");
}

/// What `disable` stops before the definition goes away. The one-shot
/// `-init` companions belong to whatever they prepared, and chap-core's
/// services are upstream's, so they are everything else.
#[test]
fn a_component_owns_its_own_services_and_their_one_shot_companions() {
    let ocs = owns(Component::Ocs);
    assert!(ocs("ocs"));
    assert!(!ocs("s3") && !ocs("chap") && !ocs("chapkit-ewars-model"));

    let s3 = owns(Component::S3);
    assert!(s3("s3"));
    assert!(s3("s3-init"), "the bucket creator goes with it");
    assert!(!s3("ocs"));

    let core = owns(Component::ChapCore);
    assert!(core("chap") && core("chap-worker") && core("postgres"));
    assert!(!core("ocs") && !core("s3") && !core("s3-init"));

    // The sibling rule is drawn at the hyphen, so a name that merely starts
    // with a component's is not that component's service - and falls to
    // chap-core exactly as `chap` and `chap-worker` do.
    assert!(!ocs("ocsx") && !s3("s3x"));
    assert!(core("ocsx") && core("s3x"));

    // A model service is chap-core's on the same rule, which is the answer
    // it has always given; `disable chap-core` refuses while one is enabled.
    assert!(core("chapkit-ewars-model") && core("chapkit-ewars-model-init"));

    // And the point of asking the other components rather than listing the
    // names that are not chap-core's: whatever a component is called, its
    // own service and its siblings are its own, and chap-core is left with
    // what nothing else claims. This holds for every component there is, so
    // it will hold for a fourth one that brings several services.
    for component in Component::ALL {
        if *component == Component::ChapCore {
            continue;
        }
        let name = component.name();
        for service in [
            name.to_string(),
            format!("{name}-init"),
            format!("{name}-db"),
            format!("{name}-prep"),
        ] {
            assert!(owns(*component)(&service), "{name} owns {service}");
            assert!(!core(&service), "chap-core must not claim {service}");
        }
    }
}

/// What the shared stop path acts on. Only the components that went from on
/// to off: one that was already off has no containers to stop, and one that
/// was just turned *on* certainly does not.
#[test]
fn only_the_components_that_went_off_are_the_ones_to_stop() {
    let mut before = Components::default();
    before.set_enabled(Component::Ocs, true);
    before.set_enabled(Component::S3, true);

    let mut after = before.clone();
    after.set_enabled(Component::S3, false);
    assert_eq!(disabled_between(&before, &after), vec![Component::S3]);

    let off = Components::default();
    assert_eq!(
        disabled_between(&before, &off),
        vec![Component::Ocs, Component::S3],
        "in the order they are listed and rendered"
    );
    assert!(disabled_between(&off, &before).is_empty(), "both went on");
    assert!(disabled_between(&before, &before).is_empty());

    // chap-core is a component like the others here; `disable` is what
    // refuses to take it away while models are enabled.
    let mut core_off = before.clone();
    core_off.set_enabled(Component::ChapCore, false);
    assert_eq!(
        disabled_between(&before, &core_off),
        vec![Component::ChapCore]
    );
}

/// The data volume line a disable closes with, and the component that has
/// no volume of its own to name.
#[test]
fn a_kept_volume_is_named_with_the_command_that_would_remove_it() {
    let dir = tempfile::tempdir().unwrap();
    let project = Project {
        dir: dir.path().to_path_buf(),
        state: crate::project::ProjectState {
            compose_project: "hello1-abc123".to_string(),
            ..crate::project::ProjectState::default()
        },
    };
    // One volume, one line: every name an operator would have to type is
    // on a line of its own, and a component with one keeps the line it had.
    let all = |_: &str| true;
    let notes = kept_volume_notes(&project, Component::Ocs, &all);
    assert_eq!(notes.len(), 1, "ocs keeps one volume");
    let line = &notes[0];
    assert!(line.starts_with("kept volume hello1-abc123_"), "{line}");
    assert!(
        line.contains("`chaps components disable ocs --purge`"),
        "{line}"
    );
    assert!(line.contains("docker volume rm"), "{line}");
    assert_eq!(
        component_volumes(&project, Component::Ocs),
        vec!["hello1-abc123_ocs_data".to_string()]
    );

    // A component with three volumes names all three, each on its own line
    // with its own `docker volume rm`: an operator removing them by hand
    // needs every name, and the download cache is one of them because the
    // compose file declares it.
    let notes = kept_volume_notes(&project, Component::Dhis2, &all);
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert_eq!(
        component_volumes(&project, Component::Dhis2),
        vec![
            "hello1-abc123_dhis2_home".to_string(),
            "hello1-abc123_dhis2_db".to_string(),
            "hello1-abc123_dhis2_dump".to_string(),
        ]
    );
    for note in &notes {
        assert!(
            note.contains("`chaps components disable dhis2 --purge`"),
            "{note}"
        );
        assert!(note.contains("docker volume rm"), "{note}");
    }

    // chap-core's volumes are upstream's own, so there is nothing here to
    // name; `chaps down --volumes` is what removes them.
    assert!(kept_volume_notes(&project, Component::ChapCore, &all).is_empty());

    // A volume docker does not have was never created, so it was not kept
    // and is not named; only the one that exists is.
    let only_db = |name: &str| name.ends_with("_dhis2_db");
    let notes = kept_volume_notes(&project, Component::Dhis2, &only_db);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].starts_with("kept volume hello1-abc123_dhis2_db;"));
    assert!(kept_volume_notes(&project, Component::Ocs, &|_| false).is_empty());
    assert!(component_volumes(&project, Component::ChapCore).is_empty());
}

/// The `.env` note has to go out on the run that appended the section and on
/// no other, because "written .env" is also what a tag pin looks like.
#[test]
fn the_data_source_note_goes_out_only_when_the_section_was_appended() {
    let section = "# OCS data sources (optional)\n# ECMWF_DATASTORES_URL=\n# EDH_API_KEY=\n";
    assert!(data_sources_appended("POSTGRES_DB=chap_core\n", section));
    // A file that was already there keeps quiet, however much else changed.
    assert!(!data_sources_appended(
        section,
        &format!("{section}# CHAP_IMAGE_TAG=v2.3.1\n")
    ));
    assert!(!data_sources_appended("", ""), "--no-env: no file at all");
    assert!(
        !data_sources_appended(
            "POSTGRES_DB=chap_core\n",
            "POSTGRES_DB=chap_core\nS3_KEY=x\n"
        ),
        "some other section is not this one"
    );
}

/// The port the preflight asks about: the flag when it was given, and the
/// recorded port otherwise - which is there while the component is off, since
/// that is how a re-enable puts an instance back where it was.
#[test]
fn the_wanted_port_is_the_flag_or_the_one_already_recorded() {
    let args = |port: Option<ComponentPortArg>| ComponentsEnableArgs {
        name: "ocs".to_string(),
        port,
        base_url: None,
        read_only: false,
        read_write: false,
        url: None,
        models_host: None,
        tag: None,
        image: None,
        ocs: OcsConfigArgs::default(),
    };
    let mut components = Components::default();
    assert_eq!(
        wanted_port(&components, Component::Ocs, &args(None)),
        Some(OCS_DEFAULT_PORT),
        "off, but the port it would come back on"
    );
    assert_eq!(
        wanted_port(
            &components,
            Component::Ocs,
            &args(Some(ComponentPortArg(Some(18010))))
        ),
        Some(18010)
    );
    // `--port none` asks for no host port, so there is nothing to probe.
    assert_eq!(
        wanted_port(
            &components,
            Component::Ocs,
            &args(Some(ComponentPortArg(None)))
        ),
        None
    );

    assert_eq!(
        wanted_port(&components, Component::S3, &args(None)),
        None,
        "the store publishes nothing by default"
    );
    components.s3.port = Some(18011);
    assert_eq!(
        wanted_port(&components, Component::S3, &args(None)),
        Some(18011)
    );
    // chap-core's host port is the API port, which `--api-port` moves.
    assert_eq!(
        wanted_port(
            &components,
            Component::ChapCore,
            &args(Some(ComponentPortArg(Some(18012))))
        ),
        Some(18012),
        "the value is read; set_port is what refuses it"
    );
}

/// The DHIS2 pin that is deployed is the one the rendered compose file
/// defaults to, because that is the file compose reads and therefore the
/// image the existing database was migrated by.
#[test]
fn the_deployed_dhis2_tag_is_read_out_of_the_rendered_compose_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project {
        dir: dir.path().to_path_buf(),
        state: crate::project::ProjectState {
            compose_project: "hello1-abc123".to_string(),
            ..crate::project::ProjectState::default()
        },
    };
    // No file: no deployment yet, so nothing to warn about.
    assert_eq!(rendered_dhis2_tag(&project), None);
    project.state.components.dhis2.enabled = true;
    assert_eq!(dhis2_tag_moved(&project, &project.state.components), None);

    let spec = crate::compose::spec::Dhis2Spec::from_components(&project.state.components);
    std::fs::write(
        dir.path().join(DHIS2_COMPOSE),
        crate::compose::render::render_dhis2(&spec),
    )
    .unwrap();
    assert_eq!(
        rendered_dhis2_tag(&project).as_deref(),
        Some(crate::components::DHIS2_DEFAULT_TAG)
    );
    // The same tag is not a move, so there is nothing to say - and the
    // volume is never even asked about.
    assert_eq!(dhis2_tag_moved(&project, &project.state.components), None);

    // A component that is going off is not a component whose image moves.
    let mut off = project.state.components.clone();
    off.dhis2.enabled = false;
    off.dhis2.image_tag = "2.41".to_string();
    assert_eq!(dhis2_tag_moved(&project, &off), None);

    // A file from some other chaps, or a hand-edited one, still reads.
    std::fs::write(
        dir.path().join(DHIS2_COMPOSE),
        "services:\n  dhis2:\n    image: dhis2/core:${DHIS2_IMAGE_TAG:-2.41.7}\n",
    )
    .unwrap();
    assert_eq!(rendered_dhis2_tag(&project).as_deref(), Some("2.41.7"));
    // And one that does not carry the variable at all answers nothing
    // rather than guessing.
    std::fs::write(
        dir.path().join(DHIS2_COMPOSE),
        "services:\n  dhis2:\n    image: dhis2/core:2.42\n",
    )
    .unwrap();
    assert_eq!(rendered_dhis2_tag(&project), None);
}

#[test]
fn the_scaffold_request_follows_the_flags() {
    let empty = request(&OcsConfigArgs::default()).into_spec();
    assert!(empty.example);
    assert_eq!(empty.country_code, "LAO");

    let filled = request(&OcsConfigArgs {
        ocs_name: Some("Malawi".into()),
        ocs_country: Some("mwi".into()),
        ocs_bbox: Some("32.6,-17.2,35.9,-9.3".into()),
    })
    .into_spec();
    assert!(!filled.example);
    assert_eq!(filled.id, "malawi-climate-service");
    assert_eq!(filled.name, "Malawi Climate Service");
    assert_eq!(filled.extent_name, "Malawi");
    assert_eq!(filled.country_code, "MWI");
    assert_eq!(filled.bbox, "32.6, -17.2, 35.9, -9.3");
}
