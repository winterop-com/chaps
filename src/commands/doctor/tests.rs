use super::test_support::*;
use super::*;

#[test]
fn the_columns_line_up_whatever_the_names_are() {
    let report = report(
        vec![
            Check::ok("docker-cli", "docker cli", "docker 29.8.1"),
            Check::warn("disk", "disk", "1.0 GB free", "free up space"),
            Check::fail("api-port", "api port", "8000 is in use", "free it"),
        ],
        Some("/srv/mychap"),
    );
    assert_eq!(
        render(&report, &Out::default()),
        "ok    docker cli  docker 29.8.1\n\
             warn  disk        1.0 GB free\n      \
             free up space\n\
             fail  api port    8000 is in use\n      \
             free it\n\
             \n"
    );
    let mut lines = crate::output::Report::default();
    closing(&report, &mut lines);
    assert_eq!(lines.text(), "3 checks: 1 ok, 1 warn, 1 fail\n");
}

#[test]
fn without_a_project_a_hint_says_where_the_rest_would_be() {
    let outside = report(vec![Check::ok("varde", "varde", "v0.2.0")], None);
    let mut lines = crate::output::Report::default();
    closing(&outside, &mut lines);
    assert_eq!(
        lines.text(),
        format!("1 checks: 1 ok, 0 warn, 0 fail\nhint: {NO_PROJECT}\n")
    );

    // Inside one, the hint is not there at all.
    let inside = report(vec![Check::ok("varde", "varde", "v0.2.0")], Some("/srv/x"));
    let mut lines = crate::output::Report::default();
    closing(&inside, &mut lines);
    assert!(
        !lines.text().contains("no deployment here"),
        "{}",
        lines.text()
    );
}

#[test]
fn every_status_word_fits_the_column() {
    for status in [Status::Ok, Status::Warn, Status::Fail, Status::Skip] {
        assert!(
            status.word().len() <= STATUS_WIDTH,
            "`{}` does not fit",
            status.word()
        );
    }
}

#[test]
fn the_summary_counts_each_status_and_only_a_failure_exits_non_zero() {
    let checks = vec![
        Check::ok("a", "a", ""),
        Check::ok("b", "b", ""),
        Check::warn("c", "c", "", "do something"),
        Check::skip("d", "d", ""),
    ];
    let counted = summary(&checks);
    assert_eq!(
        counted,
        Summary {
            ok: 2,
            warn: 1,
            fail: 0,
            skip: 1
        }
    );
    assert_eq!(exit_code(&counted), 0, "a warning is not a failure");
    assert_eq!(
        summary_line(&counted),
        "4 checks: 2 ok, 1 warn, 0 fail, 1 skipped"
    );

    let mut failed = checks;
    failed.push(Check::fail("e", "e", "", "fix it"));
    let counted = summary(&failed);
    assert_eq!(counted.fail, 1);
    assert_eq!(exit_code(&counted), 1);

    // With nothing skipped the sentence is the three words that matter.
    let counted = summary(&[Check::ok("a", "a", "")]);
    assert_eq!(summary_line(&counted), "1 checks: 1 ok, 0 warn, 0 fail");
}

#[test]
fn json_is_checks_and_a_summary_and_nothing_else() {
    let report = report(
        vec![
            Check::ok("docker-cli", "docker cli", "docker 29.8.1"),
            Check::warn("disk", "disk space", "4.0 GB free", "free up space"),
        ],
        Some("/srv/mychap"),
    );
    let value: serde_json::Value = serde_json::to_value(&report).unwrap();
    let object = value.as_object().expect("an object");
    assert_eq!(
        object.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["checks", "summary"],
        "the project path is not part of the document"
    );
    assert_eq!(
        value["checks"][0],
        serde_json::json!({
            "id": "docker-cli",
            "name": "docker cli",
            "status": "ok",
            "detail": "docker 29.8.1",
            "fix": null,
        })
    );
    assert_eq!(value["checks"][1]["status"], "warn");
    assert_eq!(value["checks"][1]["fix"], "free up space");
    assert_eq!(
        value["summary"],
        serde_json::json!({"ok": 1, "warn": 1, "fail": 0, "skip": 0})
    );
}

#[test]
fn the_components_line_reports_the_set_and_the_ocs_config() {
    // Nothing but chap-core: there is no config file to have an opinion on.
    let (status, detail, fix) = ocs_verdict(&Components::default(), &facts(None));
    assert_eq!(status, Status::Ok);
    assert_eq!(detail, "chap-core");
    assert_eq!(fix, None);

    let mut components = Components::default();
    components.set_enabled(crate::components::Component::Ocs, true);

    // The component is on and its instance config is gone: the container
    // would start with nothing to be an instance of.
    let (status, detail, fix) = ocs_verdict(&components, &facts(None));
    assert_eq!(status, Status::Fail);
    assert!(
        detail.contains("ocs/climate-service.yaml is missing"),
        "{detail}"
    );
    assert!(fix.unwrap().contains("varde sync"));

    // Present, but still the example: it deploys, it just deploys the
    // wrong country.
    let example =
        crate::compose::render::render_ocs_config(&crate::compose::spec::OcsConfigSpec::default());
    let (status, detail, fix) = ocs_verdict(&components, &facts(Some(&example)));
    assert_eq!(status, Status::Warn);
    assert!(detail.starts_with("chap-core, ocs; "), "{detail}");
    assert!(detail.contains("example values"), "{detail}");
    assert!(fix.unwrap().contains("--ocs-country"));

    // Edited, note deleted: nothing left to say.
    let (status, detail, fix) = ocs_verdict(&components, &facts(Some("id: mine\n")));
    assert_eq!(status, Status::Ok);
    assert!(
        detail.ends_with("ocs/climate-service.yaml present"),
        "{detail}"
    );
    assert_eq!(fix, None);
}
