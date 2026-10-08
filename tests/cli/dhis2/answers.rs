//! varde dhis2 against answers of DHIS2 that are easy to read wrong: the
//! epoch as a timestamp, a malformed token, a job that is not listed yet.

use super::*;

/// DHIS2 2.42.6 on an empty database sends the epoch as its last analytics
/// success. That is no run, and `show` says so.
#[cfg(unix)]
#[test]
fn dhis2_show_reads_the_epoch_as_no_analytics_run() {
    let stand_in = Dhis2StandIn::with(Dhis2State {
        epoch_analytics: true,
        ..Dhis2State::default()
    });
    let (sandbox, dir, _temp, bin) = dhis2_connected(&stand_in);

    dhis2_chap(&sandbox, &dir, &bin, None, &["show"])
        .assert()
        .success()
        .stdout(predicates::str::contains("analytics  never run"))
        .stdout(predicates::str::contains(
            "missing: analytics has never run",
        ))
        .stdout(predicates::str::contains("1970").not());

    let report = json_of(&mut dhis2_chap(
        &sandbox,
        &dir,
        &bin,
        None,
        &["show", "--json"],
    ));
    assert_eq!(report["analytics"], "never", "{report}");
    assert_eq!(report["last_analytics"], "", "{report}");
}

/// The first request through a route that was just repointed can fail once on
/// a real DHIS2. `route` asks a second time before it warns.
#[cfg(unix)]
#[test]
fn dhis2_route_asks_twice_through_a_route_it_has_just_repointed() {
    let stand_in = Dhis2StandIn::with(Dhis2State {
        route: Some(external_chap_route()),
        proxy_failures: 1,
        ..Dhis2State::default()
    });
    let (sandbox, dir, _temp, bin) = dhis2_connected(&stand_in);

    dhis2_chap(&sandbox, &dir, &bin, None, &["route"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "repointed the `chap` route at http://chap:8000/**; chap-core answered through it",
        ))
        .stderr(predicates::str::contains("nothing answered").not());
    let health = stand_in
        .asked()
        .iter()
        .filter(|seen| *seen == "GET /api/routes/chap/run/health")
        .count();
    assert_eq!(health, 2, "{:?}", stand_in.asked());
}

/// A seeded DHIS2 without chap-core has a `chap` route aimed at a stranger.
/// `show` does not name a target for it or send the reader to
/// `varde dhis2 connect`, which refuses there.
#[cfg(unix)]
#[test]
fn dhis2_show_without_chap_core_names_the_steps_that_work() {
    let stand_in = Dhis2StandIn::with(Dhis2State {
        route: Some(external_chap_route()),
        ..Dhis2State::default()
    });
    let (sandbox, dir, _temp, bin) = dhis2_connected(&stand_in);
    sandbox
        .components(&["disable", "chap-core"])
        .assert()
        .success();

    dhis2_chap(&sandbox, &dir, &bin, None, &["show"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "target     none (this deployment has no chap-core)",
        ))
        .stdout(predicates::str::contains("http://chap:8000").not())
        .stdout(predicates::str::contains("not at this deployment").not())
        .stdout(predicates::str::contains("varde dhis2 connect").not())
        .stdout(predicates::str::contains(
            "`varde components enable chap-core` adds it",
        ))
        .stdout(predicates::str::contains("`varde dhis2 apps` for the apps"));
    assert!(
        !stand_in.was_asked("GET /api/routes/chap/run/health"),
        "{:?}",
        stand_in.asked()
    );
}

/// A run asked for a moment ago is in DHIS2's job list before the notifier
/// lists it. A second `analytics` adopts it and does not start another.
#[cfg(unix)]
#[test]
fn dhis2_analytics_adopts_a_run_that_is_accepted_and_not_started() {
    let stand_in = Dhis2StandIn::with(Dhis2State {
        analytics_queued: true,
        ..Dhis2State::default()
    });
    let (sandbox, dir, _temp, bin) = dhis2_connected(&stand_in);

    dhis2_chap(&sandbox, &dir, &bin, None, &["analytics"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "the analytics run that was already going finished",
        ))
        .stdout(predicates::str::contains("(job job-q)"));
    assert!(
        !stand_in
            .asked()
            .iter()
            .any(|seen| seen.starts_with("POST /api/resourceTables/analytics")),
        "{:?}",
        stand_in.asked()
    );
}
