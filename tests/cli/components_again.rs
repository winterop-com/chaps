//! `varde components enable` on a component that is already enabled: what
//! changed, and when there is nothing for `varde up` to apply.

use crate::common::*;
use predicates::prelude::PredicateBooleanExt;

/// The same `--port` again changes nothing and names no apply step.
#[test]
fn the_same_port_again_changes_nothing() {
    let sandbox = Sandbox::new();
    let ocs_port = free_port().to_string();
    sandbox.init(&["--models", "none"]).assert().success();
    sandbox
        .components(&["enable", "ocs", "--port", &ocs_port])
        .assert()
        .success()
        .stdout(predicates::str::contains("run `varde up` to apply"));
    sandbox
        .components(&["enable", "ocs", "--port", &ocs_port])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "ocs was already enabled on http://localhost:{ocs_port}; nothing changed"
        )))
        .stdout(predicates::str::contains("varde up").not());
}

/// `--base-url` says what it set; the same value again changes nothing.
#[test]
fn a_new_base_url_is_named_and_the_same_one_changes_nothing() {
    let sandbox = Sandbox::new();
    let ocs_port = free_port().to_string();
    sandbox.init(&["--models", "none"]).assert().success();
    sandbox
        .components(&["enable", "ocs", "--port", &ocs_port])
        .assert()
        .success();
    let args = ["enable", "ocs", "--base-url", "https://ocs.example.org"];
    sandbox
        .components(&args)
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "set the ocs base URL to https://ocs.example.org",
        ))
        .stdout(predicates::str::contains("run `varde up` to apply"));
    sandbox
        .components(&args)
        .assert()
        .success()
        .stdout(predicates::str::contains("nothing changed"))
        .stdout(predicates::str::contains("base URL").not())
        .stdout(predicates::str::contains("varde up").not());
}
