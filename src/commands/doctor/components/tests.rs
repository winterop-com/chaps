use super::super::test_support::*;
use super::*;

/// A deployment with `dhis2` on and nothing else but chap-core.
fn dhis2_components() -> Components {
    let mut components = Components::default();
    components.set_enabled(Component::Dhis2, true);
    components
}

/// The DHIS2 half of the line: one file that has to be there, and one fact
/// that is only ever reported.
#[test]
fn the_components_line_reports_the_dhis2_config_and_the_seed() {
    let components = dhis2_components();

    // The file is mandatory. Without it DHIS2 throws on startup, so this is
    // a fault rather than something to keep an eye on.
    let (status, detail, fix) =
        components_verdict(&components, &facts(None), &Dhis2Facts { config: false });
    assert_eq!(status, Status::Fail);
    assert_eq!(detail, "chap-core, dhis2; dhis2/dhis.conf is missing");
    let fix = fix.unwrap();
    assert!(fix.contains("chaps sync"), "{fix}");
    assert!(fix.contains("dhis2/dhis.conf"), "{fix}");

    // There, and the seed is the default the pinned minor line publishes.
    let (status, detail, fix) =
        components_verdict(&components, &facts(None), &Dhis2Facts { config: true });
    assert_eq!(status, Status::Ok, "the seed is never a fault: {detail}");
    assert_eq!(fix, None, "nothing here is something to do");
    assert_eq!(
        detail,
        format!(
            "chap-core, dhis2; dhis2/dhis.conf present; seed: default ({}); \
                 no `chaps dhis2 connect` recorded",
            crate::compose::render::DHIS2_DEFAULT_SEED_URL
        )
    );
}

/// The connect record is reported and never judged, and never as a state:
/// the clause names the command and the time it ran, because that is the
/// whole of what `.chaps/components.yaml` knows. `chaps dhis2 show` is the
/// one thing that asks DHIS2.
#[test]
fn the_components_line_reports_the_recorded_connect_without_judging_it() {
    let present = Dhis2Facts { config: true };
    let mut components = dhis2_components();

    let (status, detail, fix) = components_verdict(&components, &facts(None), &present);
    assert_eq!(status, Status::Ok, "a step left is not a fault: {detail}");
    assert_eq!(
        fix, None,
        "doctor cannot check the route, so it advises none"
    );
    assert!(
        detail.ends_with("; no `chaps dhis2 connect` recorded"),
        "{detail}"
    );

    components.dhis2.connected_at = Some("2026-09-27T09:12:33Z".to_string());
    let (status, detail, fix) = components_verdict(&components, &facts(None), &present);
    assert_eq!(status, Status::Ok);
    assert_eq!(fix, None);
    assert!(
        detail.ends_with("; last `chaps dhis2 connect`: 2026-09-27T09:12:33Z"),
        "{detail}"
    );
    // Never "connected": nothing here asked DHIS2 anything.
    assert!(!detail.contains("connected:"), "{detail}");

    // A deployment with no chap-core has nothing to connect to, and
    // `chaps dhis2 connect` refuses there, so the clause is left off.
    components.set_enabled(Component::ChapCore, false);
    let (_, detail, _) = components_verdict(&components, &facts(None), &present);
    assert!(!detail.contains("chaps dhis2 connect"), "{detail}");
}

/// The three other answers the seed can have, each reported and none of
/// them judged: an empty database is a deployment that brings its own data.
#[test]
fn the_seed_note_says_which_of_the_answers_this_deployment_holds() {
    let present = Dhis2Facts { config: true };
    // The clause that follows the seed on every `dhis2` line, tested on its
    // own above; naming it here keeps these assertions about the tail of
    // the seed note rather than about the end of the string.
    const NO_CONNECT: &str = "; no `chaps dhis2 connect` recorded";

    let mut components = dhis2_components();
    components.dhis2.seed = Dhis2Seed::None;
    let (status, detail, fix) = components_verdict(&components, &facts(None), &present);
    assert_eq!(status, Status::Ok);
    assert_eq!(fix, None);
    assert!(
        detail.ends_with(&format!(
            "seed: none (the database starts empty){NO_CONNECT}"
        )),
        "{detail}"
    );

    // A dump of the operator's own, whether a URL or a path in the project.
    components.dhis2.seed = Dhis2Seed::parse("dumps/mine.sql.gz");
    let (_, detail, _) = components_verdict(&components, &facts(None), &present);
    assert!(
        detail.ends_with(&format!("seed: dumps/mine.sql.gz{NO_CONNECT}")),
        "{detail}"
    );

    // A minor line chaps publishes no dump for: the setting still says
    // `default`, and the database still starts empty, which is the half
    // that does not follow from the setting.
    components.dhis2.seed = Dhis2Seed::Default;
    components.dhis2.image_tag = "2.43".to_string();
    let (status, detail, fix) = components_verdict(&components, &facts(None), &present);
    assert_eq!(status, Status::Ok);
    assert_eq!(fix, None);
    assert!(
        detail.ends_with(&format!(
            "seed: default, and chaps knows no dump for 2.43 \
                 (the database starts empty){NO_CONNECT}"
        )),
        "{detail}"
    );
}

/// The line carries every enabled component. An `ocs` with something to
/// report used to be able to return before the rest of the line was
/// written, which is the shape this must never take again.
#[test]
fn a_failing_ocs_does_not_swallow_what_the_dhis2_beside_it_says() {
    let mut components = dhis2_components();
    components.set_enabled(Component::Ocs, true);
    let example =
        crate::compose::render::render_ocs_config(&crate::compose::spec::OcsConfigSpec::default());

    // OCS is only warning, and the DHIS2 fault is the worse of the two, so
    // it is the one the line's status comes from - and both are printed.
    let (status, detail, fix) = components_verdict(
        &components,
        &facts(Some(&example)),
        &Dhis2Facts { config: false },
    );
    assert_eq!(status, Status::Fail);
    assert!(
        detail.contains("ocs/climate-service.yaml still holds"),
        "{detail}"
    );
    assert!(detail.contains("dhis2/dhis.conf is missing"), "{detail}");
    // One next step per fault, and both of them on the line.
    let fix = fix.unwrap();
    assert!(fix.contains("--ocs-country"), "{fix}");
    assert!(fix.contains("dhis2/dhis.conf"), "{fix}");

    // And the other way round: OCS's own config gone is the fault, and the
    // DHIS2 seed still rides on the line.
    let (status, detail, fix) =
        components_verdict(&components, &facts(None), &Dhis2Facts { config: true });
    assert_eq!(status, Status::Fail);
    assert!(
        detail.contains("ocs/climate-service.yaml is missing"),
        "{detail}"
    );
    assert!(
        detail.contains("dhis2/dhis.conf present; seed:"),
        "{detail}"
    );
    assert!(fix.unwrap().contains("chaps sync"));
}

/// Two things the line reports and never judges: missing credentials, which
/// are optional, and the plugin count, which is a fact about a directory
/// the operator put there.
#[test]
fn the_components_line_notes_the_credentials_and_the_plugins_without_complaining() {
    let mut components = Components::default();
    components.set_enabled(crate::components::Component::Ocs, true);

    let (status, detail, fix) = ocs_verdict(
        &components,
        &OcsFacts {
            config: Some("id: mine\n"),
            credentials: false,
            plugins: Some(2),
            ..OcsFacts::default()
        },
    );
    assert_eq!(status, Status::Ok, "neither is a problem: {detail}");
    assert_eq!(fix, None);
    assert!(
        detail.contains("ERA5-Land: credentials unset (WorldPop and CHIRPS3 work without them)"),
        "{detail}"
    );
    assert!(detail.ends_with("plugins/: 2 files"), "{detail}");

    // One file is singular, and a set credential says nothing at all.
    let (_, detail, _) = ocs_verdict(
        &components,
        &OcsFacts {
            config: Some("id: mine\n"),
            credentials: true,
            plugins: Some(1),
            ..OcsFacts::default()
        },
    );
    assert!(!detail.contains("credentials unset"), "{detail}");
    assert!(detail.ends_with("plugins/: 1 file"), "{detail}");
}

/// The tail rides on the example-config warning too. Enabling `ocs` leaves
/// the example config behind by definition, so a tail that waited for the
/// operator's own file would say nothing in exactly the run where
/// ERA5-Land's missing key is the thing worth reading.
#[test]
fn the_example_config_warning_carries_the_same_tail_as_the_ok_line() {
    // The tail a deployment with no credentials and three plugin files
    // makes, spelled once for the two lines that have to carry the same one.
    const TAIL: &str = "; ERA5-Land: credentials unset (WorldPop and CHIRPS3 work \
                            without them); plugins/: 3 files";

    let mut components = Components::default();
    components.set_enabled(crate::components::Component::Ocs, true);
    let example =
        crate::compose::render::render_ocs_config(&crate::compose::spec::OcsConfigSpec::default());

    // A freshly enabled OCS: the scaffolded config, no credentials in
    // `.env`, and a plugin directory the operator has started filling.
    let (status, detail, fix) = ocs_verdict(
        &components,
        &OcsFacts {
            config: Some(&example),
            credentials: false,
            plugins: Some(3),
            ..OcsFacts::default()
        },
    );
    assert_eq!(status, Status::Warn);
    assert_eq!(
        detail,
        format!("chap-core, ocs; ocs/climate-service.yaml still holds OCS's example values{TAIL}")
    );
    // One fault, one next step: the facts are on the status line and never
    // in the advice, because none of them is something to do.
    let fix = fix.unwrap();
    assert!(fix.contains("--ocs-country"), "{fix}");
    assert!(!fix.contains("credentials"), "{fix}");

    // Credentials set: the warning says nothing about them either way.
    let (status, detail, _) = ocs_verdict(
        &components,
        &OcsFacts {
            config: Some(&example),
            credentials: true,
            ..OcsFacts::default()
        },
    );
    assert_eq!(status, Status::Warn);
    assert!(!detail.contains("credentials"), "{detail}");
    assert!(detail.ends_with("example values"), "{detail}");

    // The operator's own file: the same tail, after the same separator.
    let (status, edited, _) = ocs_verdict(
        &components,
        &OcsFacts {
            config: Some("id: mine\n"),
            credentials: false,
            plugins: Some(3),
            ..OcsFacts::default()
        },
    );
    assert_eq!(status, Status::Ok);
    assert!(edited.ends_with(TAIL), "{edited}");

    // A missing config keeps none of it: nothing holds datasets or reads a
    // credential until the file is back, and `chaps sync` is the one step.
    let (status, detail, fix) = ocs_verdict(
        &components,
        &OcsFacts {
            config: None,
            credentials: false,
            plugins: Some(3),
            ..OcsFacts::default()
        },
    );
    assert_eq!(status, Status::Fail);
    assert!(detail.ends_with("is missing"), "{detail}");
    assert!(fix.unwrap().contains("chaps sync"));
}

/// The same two facts `chaps status` puts on the OCS line: what the
/// instance holds, reported and never judged.
#[test]
fn the_components_line_reports_what_a_running_ocs_holds() {
    let mut components = Components::default();
    components.set_enabled(crate::components::Component::Ocs, true);

    let (status, detail, fix) = ocs_verdict(
        &components,
        &OcsFacts {
            config: Some("id: mine\n"),
            credentials: true,
            datasets: Some(3),
            data_bytes: Some(217_088 * 1024),
            ..OcsFacts::default()
        },
    );
    assert_eq!(status, Status::Ok, "neither is a problem: {detail}");
    assert_eq!(fix, None);
    assert_eq!(
        detail,
        "chap-core, ocs; ocs/climate-service.yaml present; 3 datasets; 212.0 MB data"
    );

    // One dataset is singular, and an instance that is not running, or
    // did not answer, says neither thing rather than saying nothing.
    let (_, detail, _) = ocs_verdict(
        &components,
        &OcsFacts {
            config: Some("id: mine\n"),
            credentials: true,
            datasets: Some(1),
            ..OcsFacts::default()
        },
    );
    assert!(detail.ends_with("; 1 dataset"), "{detail}");
    let (_, detail, _) = ocs_verdict(
        &components,
        &OcsFacts {
            config: Some("id: mine\n"),
            credentials: true,
            ..OcsFacts::default()
        },
    );
    assert!(detail.ends_with("present"), "{detail}");
}

/// Files at any depth, because OCS's own layout is `plugins/datasets/*.py`.
#[test]
fn the_plugin_count_reaches_into_subdirectories() {
    let dir = tempfile::tempdir().unwrap();
    let plugins = dir.path().join("plugins");
    assert_eq!(plugin_count(&plugins), None, "no directory, no count");

    std::fs::create_dir_all(plugins.join("datasets")).unwrap();
    assert_eq!(plugin_count(&plugins), Some(0));
    std::fs::write(plugins.join("datasets").join("clms_gpp.py"), "").unwrap();
    std::fs::write(plugins.join("datasets").join("clms_gpp.yaml"), "").unwrap();
    std::fs::write(plugins.join("README.md"), "").unwrap();
    assert_eq!(plugin_count(&plugins), Some(3));
}
