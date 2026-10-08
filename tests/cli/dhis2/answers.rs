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
        .stdout(predicates::str::contains("missing: analytics has never run"))
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
