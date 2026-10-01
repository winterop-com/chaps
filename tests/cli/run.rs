//! `chaps run`, `ps` and `stop`: one model at a time, in a group chaps keeps
//! under its data directory or in the deployment the command is inside. Unix
//! only: the stand-in docker is a shell script.

#![cfg(unix)]

use crate::common::*;
use serde_json::Value as Json;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A `docker` whose compose commands succeed and whose `ps` reports the
/// listed services running in every project.
fn docker_running(services: &[&str]) -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let rows: String = services
        .iter()
        .map(|s| format!("{{\"Service\":\"{s}\",\"State\":\"running\"}}\\n"))
        .collect();
    let script = format!(
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *' ps '*) printf '{rows}'; exit 0;;\n\
         esac\n\
         exit 0\n"
    );
    let docker = bin.join("docker");
    std::fs::write(&docker, script).expect("the fake docker");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake docker");
    }
    (temp, bin)
}

fn data(sandbox: &Sandbox) -> PathBuf {
    sandbox.cache.path().join("data")
}

fn run_json(sandbox: &Sandbox, cwd: &Path, bin: &Path, args: &[&str]) -> Json {
    let mut argv = vec!["--json"];
    argv.extend_from_slice(args);
    let out = chap_with_docker(sandbox, cwd, bin, &argv)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("one JSON document")
}

#[test]
fn run_outside_a_deployment_starts_the_model_in_a_group_on_loopback() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_running(&["chapkit-ewars-model"]);
    let cwd = sandbox.home.path();

    let doc = run_json(
        &sandbox,
        cwd,
        &bin,
        &["run", "chapkit_ewars_model", "--no-wait"],
    );
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["id"], "chapkit_ewars_model");
    assert_eq!(doc["group"], "default");
    assert_eq!(doc["bind"], "127.0.0.1");
    let port = doc["port"].as_u64().expect("a host port");
    assert_eq!(doc["url"], format!("http://localhost:{port}"));

    let dir = data(&sandbox).join("run").join("default");
    assert_eq!(PathBuf::from(doc["project_dir"].as_str().unwrap()), dir);
    let overlay = read(&dir.join("compose.chapkit-ewars-model.yml"));
    assert!(
        overlay.contains(&format!("\"127.0.0.1:{port}:8000\"")),
        "{overlay}"
    );
    // Nothing lands in the working directory.
    assert!(!cwd.join(".chaps").exists());
    assert!(!cwd.join("compose.yml").exists());

    // A second run of the same model starts it as it is.
    let again = run_json(
        &sandbox,
        cwd,
        &bin,
        &["run", "chapkit-ewars-model", "--no-wait"],
    );
    assert_eq!(again["enabled"], false);
    assert_eq!(again["port"], port);
}

#[test]
fn groups_keep_models_apart_and_ps_and_stop_find_them() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_running(&["chapkit-ewars-model", "auto-arima-chapkit"]);
    let cwd = sandbox.home.path();

    run_json(
        &sandbox,
        cwd,
        &bin,
        &["run", "chapkit_ewars_model", "--no-wait"],
    );
    run_json(
        &sandbox,
        cwd,
        &bin,
        &["run", "auto_arima_chapkit", "--group", "trial", "--no-wait"],
    );

    let ps = run_json(&sandbox, cwd, &bin, &["ps"]);
    let models = ps["models"].as_array().expect("a list");
    assert_eq!(models.len(), 2, "{ps}");
    // Nothing really listens on the first group's port, and the second group
    // still does not hand it out again.
    assert_ne!(models[0]["port"], models[1]["port"], "{ps}");
    assert_eq!(models[0]["group"], "default");
    assert_eq!(models[1]["group"], "trial");
    assert_eq!(models[1]["id"], "auto_arima_chapkit");

    let text = String::from_utf8(
        chap_with_docker(&sandbox, cwd, &bin, &["ps"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .expect("text");
    assert!(text.starts_with("GROUP"), "{text}");

    let only = run_json(&sandbox, cwd, &bin, &["ps", "--group", "trial"]);
    assert_eq!(only["models"].as_array().unwrap().len(), 1);

    // stop finds the model in whichever group has it.
    let stopped = run_json(&sandbox, cwd, &bin, &["stop", "auto_arima_chapkit"]);
    assert_eq!(stopped["ok"], true);
    assert_eq!(stopped["stopped"][0]["group"], "trial");

    let stopped = run_json(&sandbox, cwd, &bin, &["stop", "--all"]);
    assert_eq!(stopped["stopped"][0]["id"], "chapkit_ewars_model");
    let ps = run_json(&sandbox, cwd, &bin, &["ps"]);
    assert!(ps["models"].as_array().unwrap().is_empty(), "{ps}");
}

#[test]
fn a_group_labels_its_containers_with_the_run_kind_and_its_name() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_running(&["chapkit-ewars-model"]);
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

    let dir = data(&sandbox).join("run").join("trial");
    let project = read(&dir.join(".chaps").join("project.yaml"));
    assert!(project.contains("\ngroup: trial\n"), "{project}");
    let overlay = read(&dir.join("compose.chapkit-ewars-model.yml"));
    for line in [
        "      com.winterop.chaps.role: model\n",
        "      com.winterop.chaps.model: \"chapkit_ewars_model\"\n",
        "      com.winterop.chaps.kind: run\n",
        "      com.winterop.chaps.group: \"trial\"\n",
    ] {
        // Once on the model and once on its init container.
        assert_eq!(overlay.matches(line).count(), 2, "{line}{overlay}");
    }
}

#[test]
fn run_inside_a_deployment_uses_it_and_refuses_a_group() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_running(&["auto-arima-chapkit"]);
    sandbox
        .init(&["--models", "none", "--port-base", &port_base().to_string()])
        .assert()
        .success();
    let dir = sandbox.project();

    let doc = run_json(
        &sandbox,
        &dir,
        &bin,
        &["run", "auto_arima_chapkit", "--no-wait"],
    );
    assert_eq!(doc["group"], Json::Null);
    let canonical = |p: &Path| std::fs::canonicalize(p).expect("an existing path");
    assert_eq!(
        canonical(Path::new(doc["project_dir"].as_str().unwrap())),
        canonical(&dir)
    );
    assert!(state(&dir)["models"]["auto_arima_chapkit"].is_object());
    // A deployment of the caller's own is not a group, and its labels say so.
    let overlay = read(&dir.join("compose.auto-arima-chapkit.yml"));
    assert!(
        overlay.contains("com.winterop.chaps.kind: init\n"),
        "{overlay}"
    );
    assert!(!overlay.contains("com.winterop.chaps.group"), "{overlay}");
    assert!(!read(&dir.join(".chaps").join("project.yaml")).contains("group:"));
    assert!(!data(&sandbox).join("run").exists());

    chap_with_docker(
        &sandbox,
        &dir,
        &bin,
        &["run", "auto_arima_chapkit", "--group", "x"],
    )
    .assert()
    .failure()
    .stderr(predicates::str::contains(
        "--group names a `chaps run` group",
    ));
}

#[test]
fn ps_and_stop_before_any_run_say_so() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_running(&[]);
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
