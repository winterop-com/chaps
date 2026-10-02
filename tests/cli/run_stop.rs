//! `chaps stop` on `chaps run` groups: stopping one model or a whole group,
//! and what `--purge` takes with it. Unix only: the stand-in docker is a shell
//! script.

#![cfg(unix)]

use crate::common::*;
use serde_json::Value as Json;

#[test]
fn ps_and_stop_before_any_run_say_so() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_running_services(&[]);
    let cwd = sandbox.home.path();
    chap_with_docker(&sandbox, cwd, &bin, &["ps"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "nothing has been started with `chaps run` yet",
        ));
    chap_with_docker(&sandbox, cwd, &bin, &["stop", "chapkit_ewars_model"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("`chaps ps` lists what is"));
}

#[test]
fn stop_with_a_group_and_no_id_stops_the_group_and_purge_removes_it() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_running_services(&["chapkit-ewars-model", "auto-arima-chapkit"]);
    let cwd = sandbox.home.path();
    for model in ["chapkit_ewars_model", "auto_arima_chapkit"] {
        run_json(
            &sandbox,
            cwd,
            &bin,
            &["run", model, "--group", "trial", "--no-wait"],
        );
    }
    let dir = data(&sandbox).join("run").join("trial");

    let stopped = run_json(&sandbox, cwd, &bin, &["stop", "--group", "trial"]);
    assert_eq!(stopped["stopped"].as_array().unwrap().len(), 2, "{stopped}");
    assert_eq!(stopped["removed"], serde_json::json!([]));
    assert!(dir.exists(), "a stop without --purge keeps the group");

    let purged = run_json(
        &sandbox,
        cwd,
        &bin,
        &["stop", "--group", "trial", "--purge"],
    );
    assert_eq!(purged["removed"], serde_json::json!(["trial"]), "{purged}");
    assert!(!dir.exists(), "an emptied group is taken away by --purge");
}

/// A model stopped without `--purge` keeps its volume and loses its overlay,
/// so `compose down --volumes` no longer knows the volume; purging the group
/// finds it by the compose project label and removes it.
#[test]
fn purging_a_group_removes_the_volume_of_a_model_stopped_before() {
    let sandbox = Sandbox::new();
    let (_fake, bin, log) = docker_with_leftover_volume(true);
    let cwd = sandbox.home.path();
    run_json(
        &sandbox,
        cwd,
        &bin,
        &[
            "run",
            "chapkit_ewars_model",
            "--group",
            "trial",
            "--no-wait",
        ],
    );
    run_json(&sandbox, cwd, &bin, &["stop", "chapkit_ewars_model"]);
    let dir = data(&sandbox).join("run").join("trial");
    assert!(dir.exists());

    let purged = run_json(
        &sandbox,
        cwd,
        &bin,
        &["stop", "--group", "trial", "--purge"],
    );
    assert_eq!(purged["removed"], serde_json::json!(["trial"]), "{purged}");
    let volume = purged["removed_volumes"][0].as_str().expect("a volume");
    assert!(volume.ends_with("_ck_old_model_data"), "{purged}");
    assert!(
        read(&log)
            .lines()
            .any(|l| l == format!("volume rm {volume}")),
        "{}",
        read(&log)
    );
    assert!(!dir.exists());
}

/// A volume docker will not remove keeps the group, so nothing is left that
/// no chaps command can reach, and the error says how to finish.
#[test]
fn a_volume_docker_will_not_remove_keeps_the_group() {
    let sandbox = Sandbox::new();
    let (_fake, bin, _log) = docker_with_leftover_volume(false);
    let cwd = sandbox.home.path();
    run_json(
        &sandbox,
        cwd,
        &bin,
        &[
            "run",
            "chapkit_ewars_model",
            "--group",
            "trial",
            "--no-wait",
        ],
    );
    let out = chap_with_docker(
        &sandbox,
        cwd,
        &bin,
        &["--json", "stop", "--group", "trial", "--purge"],
    )
    .assert()
    .failure()
    .get_output()
    .stdout
    .clone();
    let doc: Json = serde_json::from_slice(&out).expect("one JSON document");
    assert!(
        doc["error"].as_str().unwrap().contains("volume is in use"),
        "{doc}"
    );
    assert!(
        doc["hint"]
            .as_str()
            .unwrap()
            .contains("`chaps stop --group trial --purge`"),
        "{doc}"
    );
    assert!(data(&sandbox).join("run").join("trial").exists());
}

/// `stop ID --purge` takes away the group it emptied and no other: a group a
/// plain `stop` emptied earlier is keeping its data on purpose.
#[test]
fn purging_one_model_leaves_another_empty_group_and_its_data_alone() {
    let sandbox = Sandbox::new();
    let (_fake, bin, log) = docker_with_leftover_volume(true);
    let cwd = sandbox.home.path();
    run_json(
        &sandbox,
        cwd,
        &bin,
        &["run", "chapkit_ewars_model", "--group", "keep", "--no-wait"],
    );
    run_json(&sandbox, cwd, &bin, &["stop", "chapkit_ewars_model"]);
    run_json(
        &sandbox,
        cwd,
        &bin,
        &["run", "auto_arima_chapkit", "--no-wait"],
    );

    let purged = run_json(
        &sandbox,
        cwd,
        &bin,
        &["stop", "auto_arima_chapkit", "--purge"],
    );
    assert_eq!(
        purged["removed"],
        serde_json::json!(["default"]),
        "{purged}"
    );
    let keep = data(&sandbox).join("run").join("keep");
    assert!(keep.exists(), "the other empty group stays");
    let calls = read(&log);
    assert!(
        !calls.lines().any(|l| l.starts_with("volume rm keep-")),
        "{calls}"
    );
}
