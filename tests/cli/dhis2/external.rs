//! varde dhis2 against a DHIS2 that runs elsewhere, recorded with varde dhis2 use.

use super::*;

/// A deployment of chap-core alone, with nothing on PATH pretending to be
/// docker: an external DHIS2 has no container, so nothing may ask for one.
fn external_dhis2_sandbox() -> (Sandbox, PathBuf, TempDir) {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--api-port", &free_port().to_string()])
        .assert()
        .success();
    let empty = tempfile::tempdir().expect("an empty PATH entry");
    (sandbox, dir, empty)
}

/// The shape most real deployments have: Chap beside a DHIS2 that already runs
/// elsewhere, reached with a personal access token.
#[cfg(unix)]
#[test]
fn dhis2_use_records_an_external_dhis2_and_every_verb_talks_to_it() {
    let stand_in = Dhis2StandIn::new();
    let (sandbox, dir, empty) = external_dhis2_sandbox();
    let url = format!("http://127.0.0.1:{}", stand_in.port);
    let env = dir.join(".env");
    let body = read(&env);
    std::fs::write(&env, format!("{body}DHIS2_API_TOKEN=d2p_sekret\n")).unwrap();

    let assert = dhis2_chap(
        &sandbox,
        &dir,
        empty.path(),
        None,
        &["use", &format!("{url}/"), "--chap-url", EXTERNAL_CHAP_URL],
    )
    .assert()
    .success()
    .stdout(predicates::str::contains(format!(
        "recorded the external DHIS2 at {url} in `.varde/components.yaml`"
    )))
    .stdout(predicates::str::contains("answers /api/ping"))
    .stdout(predicates::str::contains(
        "API token from `.env` (accepted)",
    ))
    .stdout(predicates::str::contains(EXTERNAL_CHAP_URL_TARGET))
    .stdout(predicates::str::contains("run `varde dhis2 connect`"));
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(!stdout.contains("sekret"), "{stdout}");
    assert_eq!(stand_in.authorization(), "ApiToken d2p_sekret");

    // Recorded as state, trailing slash and all taken off.
    let components = read(&dir.join(".varde").join("components.yaml"));
    assert!(components.contains("dhis2-external:"), "{components}");
    assert!(
        components.contains(&format!("url: {url}\n")),
        "{components}"
    );

    // The same thing again is a report, not a write.
    dhis2_chap(
        &sandbox,
        &dir,
        empty.path(),
        None,
        &["use", &url, "--chap-url", EXTERNAL_CHAP_URL],
    )
    .assert()
    .success()
    .stdout(predicates::str::contains(
        "is already recorded; nothing changed",
    ));

    // `route` points it at the recorded chap-core URL, not the compose alias
    // no server outside this deployment could resolve.
    dhis2_chap(&sandbox, &dir, empty.path(), None, &["route"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "external DHIS2 2.42.6 at {url}, as `ops` (API token from `.env`)"
        )))
        .stdout(predicates::str::contains(format!(
            "created the `chap` route at {EXTERNAL_CHAP_URL_TARGET}"
        )));
    assert_eq!(
        stand_in.route().expect("a route")["url"],
        EXTERNAL_CHAP_URL_TARGET
    );

    // `connect` on a DHIS2 varde does not run is the route and nothing more:
    // no app installed on it, no analytics run started on it.
    dhis2_chap(&sandbox, &dir, empty.path(), None, &["connect"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "skipped: generating analytics tables on a DHIS2 varde does not run; \
             `varde dhis2 analytics` starts a run if its admin agrees",
        ))
        .stdout(predicates::str::contains(
            "`varde dhis2 apps` installs them",
        ))
        .stdout(predicates::str::contains(
            "the Modeling App can reach Chap once it is installed",
        ));
    assert!(
        !stand_in.asked().iter().any(
            |seen| seen.starts_with("POST /api/resourceTables/analytics")
                || seen.starts_with("POST /api/appHub/")
        ),
        "{:?}",
        stand_in.asked()
    );

    let report = json_of(&mut dhis2_chap(
        &sandbox,
        &dir,
        empty.path(),
        None,
        &["show", "--json"],
    ));
    assert_eq!(report["instance"]["external"], true);
    assert_eq!(report["instance"]["auth"], "token");
    assert_eq!(report["instance"]["credential_from"], "env-file");
    assert_eq!(report["instance"]["user"], "ops");
    assert_eq!(report["target"], EXTERNAL_CHAP_URL_TARGET);
    assert_eq!(report["route"]["ours"], true);
    assert!(!report.to_string().contains("sekret"), "{report}");

    // The component cannot go on beside it: both would be this deployment's
    // DHIS2.
    sandbox
        .components(&["enable", "dhis2"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "run `varde dhis2 use --clear` first",
        ));

    // And forgetting it says what `varde dhis2` talks to now.
    dhis2_chap(&sandbox, &dir, empty.path(), None, &["use", "--clear"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "forgot the external DHIS2 at {url}"
        )));
    let components = read(&dir.join(".varde").join("components.yaml"));
    assert!(!components.contains("dhis2-external"), "{components}");
    dhis2_chap(&sandbox, &dir, empty.path(), None, &["use", "--clear"])
        .assert()
        .success()
        .stdout(predicates::str::contains("nothing to clear"));
}

/// varde did not create an external DHIS2, so it knows none of its passwords:
/// `admin` / `district` is never tried, and the report names what to set.
/// An external DHIS2 whose admin installed both apps already: `connect` sets
/// the route and says the Modeling App reaches Chap now, not once installed.
#[cfg(unix)]
#[test]
fn connect_on_an_external_dhis2_with_both_apps_says_it_is_ready() {
    let stand_in = Dhis2StandIn::with(Dhis2State {
        apps: vec![
            serde_json::json!({"name": "Modeling", "key": "modeling", "version": "7.1.0"}),
            serde_json::json!({"name": "DHIS2 Climate App", "key": "dhis2-climate-app", "version": "1.16.2"}),
        ],
        ..Dhis2State::default()
    });
    let (sandbox, dir, empty) = external_dhis2_sandbox();
    let url = format!("http://127.0.0.1:{}", stand_in.port);
    let env = dir.join(".env");
    let body = read(&env);
    std::fs::write(&env, format!("{body}DHIS2_API_TOKEN=d2p_sekret\n")).unwrap();
    dhis2_chap(
        &sandbox,
        &dir,
        empty.path(),
        None,
        &["use", &url, "--chap-url", EXTERNAL_CHAP_URL],
    )
    .assert()
    .success();

    let assert = dhis2_chap(&sandbox, &dir, empty.path(), None, &["connect"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "skipped: installing apps: both are there already",
        ))
        .stdout(predicates::str::contains(
            "the Modeling App can reach Chap; open DHIS2 with `varde open dhis2`",
        ));
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(!stdout.contains("once it is installed"), "{stdout}");
}

#[cfg(unix)]
#[test]
fn an_external_dhis2_without_credentials_is_never_sent_the_default() {
    let stand_in = Dhis2StandIn::new();
    let (sandbox, dir, empty) = external_dhis2_sandbox();
    let url = format!("http://127.0.0.1:{}", stand_in.port);

    let report = json_of(&mut dhis2_chap(
        &sandbox,
        &dir,
        empty.path(),
        None,
        &["use", &url, "--chap-url", EXTERNAL_CHAP_URL, "--json"],
    ));
    assert_eq!(report["outcome"], "recorded");
    assert_eq!(report["probe"]["answered"], true);
    assert_eq!(report["probe"]["credential"], Json::Null);
    // The names that message asks for are in `.env` already, commented out,
    // and with no password: varde did not create this DHIS2.
    let env = read(&dir.join(".env"));
    assert!(env.contains("\n# DHIS2_ADMIN_USERNAME=admin\n"), "{env}");
    assert!(env.contains("\n# DHIS2_ADMIN_PASSWORD=\n"), "{env}");
    assert!(env.contains("\n# DHIS2_API_TOKEN=\n"), "{env}");
    // The problem names the ways in; the next step does not say them again.
    assert!(
        report["probe"]["problem"]
            .as_str()
            .unwrap()
            .contains("DHIS2_API_TOKEN"),
        "{report}"
    );
    assert_eq!(
        report["next"],
        "once one of them is in `.env`, run `varde dhis2 connect`"
    );

    dhis2_chap(&sandbox, &dir, empty.path(), None, &["show"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("did not deploy it"))
        .stderr(predicates::str::contains("`DHIS2_ADMIN_PASSWORD`"));
    assert_eq!(stand_in.authorization(), "", "{:?}", stand_in.asked());

    // A password exported for the run is used, with the user it names.
    dhis2_chap(&sandbox, &dir, empty.path(), None, &["show"])
        .env("VARDE_DHIS2_USERNAME", "ops")
        .env("VARDE_DHIS2_PASSWORD", "theirs")
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "as `ops` (password from VARDE_DHIS2_PASSWORD)",
        ));
}

/// The first `use` needs both URLs, a component that is on refuses it, and a
/// URL that is not one is said to be not one.
#[test]
fn dhis2_use_refuses_what_it_cannot_record() {
    let (sandbox, dir, empty) = external_dhis2_sandbox();
    let use_ = |args: &[&str]| {
        let mut cmd = sandbox.chap();
        cmd.env("PATH", empty.path())
            .arg("-C")
            .arg(&dir)
            .arg("dhis2")
            .arg("use")
            .args(args);
        cmd
    };

    use_(&["https://dhis2.example.org"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("`--chap-url URL` is needed"));
    use_(&["dhis2.example.org", "--chap-url", EXTERNAL_CHAP_URL])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "give it with http:// or https://",
        ));
    use_(&["--chap-url", EXTERNAL_CHAP_URL])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "no external DHIS2 is recorded yet",
        ));
    // Nothing recorded and nothing asked: a report, naming both ways on.
    use_(&[])
        .assert()
        .success()
        .stdout(predicates::str::contains("no external DHIS2 is recorded"))
        .stdout(predicates::str::contains(
            "varde dhis2 use URL --chap-url URL",
        ));

    sandbox
        .components(&["enable", "dhis2", "--port", &free_port().to_string()])
        .assert()
        .success();
    use_(&["https://dhis2.example.org", "--chap-url", EXTERNAL_CHAP_URL])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "run `varde components disable dhis2` first",
        ));
}

/// An external DHIS2 is recorded as connected only when the route works and
/// both apps are there, as a local one is: without the apps, `varde up` and
/// `varde status` keep the connect hint.
#[cfg(unix)]
#[test]
fn connect_on_an_external_dhis2_without_the_apps_records_nothing() {
    // An external DHIS2 records it under `dhis2_external`.
    let recorded = |dir: &std::path::Path| {
        read(&dir.join(".varde").join("components.yaml"))
            .lines()
            .any(|line| line.trim_start().starts_with("connected_at: 20"))
    };
    for (apps, connected) in [
        (Vec::new(), false),
        (
            vec![
                serde_json::json!({"name": "Modeling", "key": "modeling", "version": "7.1.0"}),
                serde_json::json!({"name": "DHIS2 Climate App", "key": "dhis2-climate-app", "version": "1.16.2"}),
            ],
            true,
        ),
    ] {
        let stand_in = Dhis2StandIn::with(Dhis2State {
            apps,
            ..Dhis2State::default()
        });
        let (sandbox, dir, empty) = external_dhis2_sandbox();
        let url = format!("http://127.0.0.1:{}", stand_in.port);
        let env = dir.join(".env");
        let body = read(&env);
        std::fs::write(&env, format!("{body}DHIS2_API_TOKEN=d2p_sekret\n")).unwrap();
        dhis2_chap(
            &sandbox,
            &dir,
            empty.path(),
            None,
            &["use", &url, "--chap-url", EXTERNAL_CHAP_URL],
        )
        .assert()
        .success();
        dhis2_chap(&sandbox, &dir, empty.path(), None, &["connect"])
            .assert()
            .success();
        assert_eq!(recorded(&dir), connected);
    }
}
