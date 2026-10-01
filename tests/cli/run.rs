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

/// A `docker` that refuses `compose up` the way a pull of a missing image
/// does, and answers everything else with success.
fn docker_failing_up() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let script = "#!/bin/sh\n\
         case \"$*\" in\n\
         *' up '*) echo ' m Pulling' >&2; \
         echo 'Error response from daemon: pull access denied' >&2; \
         echo 'denied' >&2; exit 1;;\n\
         esac\n\
         exit 0\n";
    let docker = bin.join("docker");
    std::fs::write(&docker, script).expect("the fake docker");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake docker");
    }
    (temp, bin)
}

/// A `docker` whose local image store holds every image asked about, as one
/// that runs as user 1000 in `/app`, and whose compose commands succeed.
fn docker_with_local_images() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let script = "#!/bin/sh\n\
         case \"$*\" in\n\
         'image inspect'*) printf '1000\\t/app\\tnull\\t[\"serve\"]\\n'; exit 0;;\n\
         esac\n\
         exit 0\n";
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

#[test]
fn stop_with_a_group_and_no_id_stops_the_group_and_purge_removes_it() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_running(&["chapkit-ewars-model", "auto-arima-chapkit"]);
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

#[test]
fn run_of_an_unknown_id_says_so_and_makes_no_group() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_running(&[]);
    let cwd = sandbox.home.path();
    let out = chap_with_docker(&sandbox, cwd, &bin, &["--json", "run", "does_not_exist"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let doc: Json = serde_json::from_slice(&out).expect("one JSON document");
    assert_eq!(doc["ok"], false);
    assert_eq!(
        doc["hint"], "`chaps models search does_not_exist` finds one",
        "{doc}"
    );
    assert!(!data(&sandbox).join("run").join("default").exists());
}

#[test]
fn a_usage_error_under_json_is_json() {
    let sandbox = Sandbox::new();
    let out = chap_in(&sandbox, sandbox.home.path(), &["--json", "stop"])
        .assert()
        .code(2)
        .get_output()
        .stdout
        .clone();
    let doc: Json = serde_json::from_slice(&out).expect("one JSON document");
    assert_eq!(doc["ok"], false);
    assert!(
        doc["error"]
            .as_str()
            .unwrap()
            .contains("required arguments were not provided"),
        "{doc}"
    );
    assert_eq!(doc["hint"], "`chaps stop --help` lists what it takes");
}

#[test]
fn a_run_that_cannot_start_takes_its_model_back_out() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_failing_up();
    let cwd = sandbox.home.path();
    let out = chap_with_docker(
        &sandbox,
        cwd,
        &bin,
        &["--json", "run", "chapkit_ewars_model", "--group", "trial"],
    )
    .assert()
    .failure()
    .get_output()
    .stdout
    .clone();
    let doc: Json = serde_json::from_slice(&out).expect("one JSON document");
    assert!(
        doc["error"]
            .as_str()
            .unwrap()
            .contains("Error response from daemon: pull access denied"),
        "{doc}"
    );
    assert_eq!(
        doc["hint"],
        "fix that, then `chaps run chapkit_ewars_model --group trial` tries again"
    );
    let dir = data(&sandbox).join("run").join("trial");
    let models = &state(&dir)["models"];
    assert!(models.as_object().is_none_or(|m| m.is_empty()), "{models}");
}

#[test]
fn parallel_runs_into_a_new_group_all_land_in_it() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_running(&["chapkit-ewars-model", "auto-arima-chapkit"]);
    let cwd = sandbox.home.path();
    let codes: Vec<Option<i32>> = std::thread::scope(|scope| {
        let handles: Vec<_> = ["chapkit_ewars_model", "auto_arima_chapkit"]
            .into_iter()
            .map(|model| {
                let (sandbox, bin) = (&sandbox, &bin);
                scope.spawn(move || {
                    chap_with_docker(
                        sandbox,
                        cwd,
                        bin,
                        &["--json", "run", model, "--group", "fresh", "--no-wait"],
                    )
                    .output()
                    .expect("chaps ran")
                    .status
                    .code()
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(codes, vec![Some(0), Some(0)]);
    let ps = run_json(&sandbox, cwd, &bin, &["ps", "--group", "fresh"]);
    assert_eq!(ps["models"].as_array().unwrap().len(), 2, "{ps}");
}

/// `models remove` has no `--group`, so an error from inside a group names it
/// with the `-C <group dir>` that reaches the group from anywhere.
#[test]
fn an_error_inside_a_group_names_models_remove_with_the_group_dir() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_with_local_images();
    let cwd = sandbox.home.path();
    run_json(
        &sandbox,
        cwd,
        &bin,
        &[
            "run",
            "localone:1",
            "--id",
            "mine",
            "--group",
            "trial",
            "--no-wait",
        ],
    );
    let out = chap_with_docker(
        &sandbox,
        cwd,
        &bin,
        &[
            "--json",
            "run",
            "localtwo:1",
            "--id",
            "mine",
            "--group",
            "trial",
            "--no-wait",
        ],
    )
    .assert()
    .failure()
    .get_output()
    .stdout
    .clone();
    let doc: Json = serde_json::from_slice(&out).expect("one JSON document");
    let dir = data(&sandbox).join("run").join("trial");
    let said = format!("{} {}", doc["error"], doc["hint"]);
    assert!(
        said.contains(&format!("`chaps -C {} models remove mine`", dir.display())),
        "{doc}"
    );
}

/// `logs` has no `--group` either: a run into a group names its log with `-C`.
#[test]
fn a_run_into_a_group_names_its_log_with_the_group_dir() {
    let sandbox = Sandbox::new();
    let (_fake, bin) = docker_running(&["chapkit-ewars-model"]);
    let cwd = sandbox.home.path();
    let out = chap_with_docker(
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
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let text = String::from_utf8(out).expect("text");
    let dir = data(&sandbox).join("run").join("trial");
    assert!(
        text.contains(&format!(
            "`chaps -C {} logs chapkit-ewars-model`",
            dir.display()
        )),
        "{text}"
    );
    assert!(
        text.contains("`chaps stop chapkit_ewars_model --group trial`"),
        "{text}"
    );
}
