//! `varde models expose` on a model whose container runs: its own live port
//! is not a conflict. Unix only: the stand-in docker is a shell script.

#![cfg(unix)]

use crate::common::*;
use predicates::prelude::PredicateBooleanExt;

/// An expose that changes nothing, after the port is live, says so; it does
/// not count the model's own container as a holder of the port.
#[test]
fn expose_again_on_the_own_live_port_changes_nothing() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    let base = port_base();
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model",
            "--port-base",
            &base.to_string(),
        ])
        .assert()
        .success();
    let port = base.to_string();
    sandbox
        .models(&["expose", "chapkit_ewars_model", "--port", &port])
        .assert()
        .success();

    // The model's container now publishes the port, as after `varde up`.
    let _live = std::net::TcpListener::bind(("0.0.0.0", base)).expect("the port is free");
    let (_fake, bin) = docker_publishing(&[("chapkit-ewars-model", Some(base))]);
    let cli = &["-C", dir.to_str().unwrap(), "models", "expose"];
    for request in [port.as_str(), "auto"] {
        let mut args = cli.to_vec();
        args.extend(["chapkit_ewars_model", "--port", request]);
        chap_with_docker(&sandbox, sandbox.home.path(), &bin, &args)
            .assert()
            .success()
            .stdout(predicates::str::contains(format!(
                "chapkit-ewars-model is already exposed on http://localhost:{base}; nothing changed"
            )))
            .stdout(predicates::str::contains("varde up").not());
    }
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        base
    );
}

/// A no-op expose before `varde up` on a container that runs without the
/// port says that `varde up` applies it.
#[test]
fn expose_again_before_up_on_a_running_container_names_varde_up() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    let base = port_base();
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model",
            "--port-base",
            &base.to_string(),
        ])
        .assert()
        .success();
    let port = base.to_string();
    sandbox
        .models(&["expose", "chapkit_ewars_model", "--port", &port])
        .assert()
        .success();

    let (_fake, bin) = docker_publishing(&[("chapkit-ewars-model", None)]);
    let args = [
        "-C",
        dir.to_str().unwrap(),
        "models",
        "expose",
        "chapkit_ewars_model",
        "--port",
        &port,
    ];
    chap_with_docker(&sandbox, sandbox.home.path(), &bin, &args)
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "its running container does not publish this port yet; run `varde up` to apply it",
        ));
}
