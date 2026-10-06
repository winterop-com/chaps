//! `varde cleanup`: deleting what deployments whose directory is gone left in
//! docker, and nothing else. Unix only: the stand-in docker is a shell script.

#![cfg(unix)]

use crate::common::*;
use serde_json::Value as Json;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A `docker` in which every compose project has two volumes, one free and
/// one a container still mounts, and a network, and a project whose
/// directory is called `running` still has a container; it writes each call
/// to `calls.log`.
fn docker_with_leftovers() -> (TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let log = temp.path().join("calls.log");
    // The networks `network rm` took, one empty file each.
    let gone = temp.path().join("networks-removed");
    std::fs::create_dir_all(&gone).expect("a directory for removed networks");
    let script = format!(
        "#!/bin/sh\n\
         echo \"$*\" >> '{log}'\n\
         case \"$*\" in\n\
         'ps -a -q --filter volume='*_busy_data) echo 0123456789ab; exit 0;;\n\
         'ps -a -q --filter label=com.docker.compose.project=running-'*) echo 0123456789ab; exit 0;;\n\
         'ps -a -q --filter '*) exit 0;;\n\
         'network rm '*) touch \"{gone}/$3\"; exit 0;;\n\
         'network inspect '*) test ! -e \"{gone}/$3\"; exit $?;;\n\
         'volume ls --filter label=com.docker.compose.project='*) \
         p=\"${{4#label=com.docker.compose.project=}}\"; \
         echo \"${{p}}_ck_model_data\"; echo \"${{p}}_busy_data\"; exit 0;;\n\
         esac\n\
         exit 0\n",
        log = log.display(),
        gone = gone.display()
    );
    let docker = bin.join("docker");
    std::fs::write(&docker, script).expect("the fake docker");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake docker");
    }
    (temp, bin, log)
}

/// Two deployments varde wrote, `kept` and `gone`, with `gone`'s directory
/// deleted afterwards. Returns their compose project names.
fn one_kept_one_gone(sandbox: &Sandbox) -> (String, String) {
    let mut names = Vec::new();
    for dir in ["kept", "gone"] {
        let dir = sandbox.home.path().join(dir);
        sandbox
            .chap()
            .args(["init"])
            .arg(&dir)
            .args(["--only", "none", "--models", "none"])
            .assert()
            .success();
        names.push(state(&dir)["compose_project"].as_str().unwrap().to_string());
    }
    std::fs::remove_dir_all(sandbox.home.path().join("gone")).unwrap();
    (names[0].clone(), names[1].clone())
}

fn record(sandbox: &Sandbox) -> String {
    read(&sandbox.cache.path().join("data").join("deployments.yaml"))
}

fn json(sandbox: &Sandbox, bin: &Path, args: &[&str]) -> Json {
    let mut argv = vec!["--json"];
    argv.extend_from_slice(args);
    let out = chap_with_docker(sandbox, sandbox.home.path(), bin, &argv)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("one JSON document")
}

#[test]
fn init_records_the_deployment_for_cleanup() {
    let sandbox = Sandbox::new();
    let (kept, gone) = one_kept_one_gone(&sandbox);
    let record = record(&sandbox);
    assert!(record.contains(&format!("{kept}:")), "{record}");
    assert!(record.contains(&format!("{gone}:")), "{record}");
}

#[test]
fn a_dry_run_lists_the_gone_deployments_free_volumes_and_deletes_nothing() {
    let sandbox = Sandbox::new();
    let (kept, gone) = one_kept_one_gone(&sandbox);
    let (_fake, bin, log) = docker_with_leftovers();

    let doc = json(&sandbox, &bin, &["cleanup", "--dry-run"]);
    assert_eq!(doc["present"], 1, "{doc}");
    let leftovers = doc["leftovers"].as_array().unwrap();
    assert_eq!(leftovers.len(), 1, "{doc}");
    assert_eq!(leftovers[0]["project"], gone.as_str());
    // The volume a container mounts is not offered.
    assert_eq!(
        leftovers[0]["volumes"],
        serde_json::json!([format!("{gone}_ck_model_data")])
    );
    assert_eq!(leftovers[0]["network"], format!("{gone}_default"));

    let calls = read(&log);
    assert!(!calls.contains("volume rm"), "{calls}");
    assert!(
        !calls.contains(&kept),
        "the kept deployment is never asked about: {calls}"
    );
}

#[test]
fn under_json_cleanup_refuses_to_delete_without_yes() {
    let sandbox = Sandbox::new();
    one_kept_one_gone(&sandbox);
    let (_fake, bin, log) = docker_with_leftovers();
    let out = chap_with_docker(&sandbox, sandbox.home.path(), &bin, &["--json", "cleanup"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let doc: Json = serde_json::from_slice(&out).expect("one JSON document");
    assert!(
        doc["hint"]
            .as_str()
            .unwrap()
            .contains("`varde cleanup --yes`"),
        "{doc}"
    );
    assert!(!read(&log).contains("volume rm"));
}

#[test]
fn cleanup_yes_deletes_the_leftovers_and_forgets_the_deployment() {
    let sandbox = Sandbox::new();
    let (kept, gone) = one_kept_one_gone(&sandbox);
    let (_fake, bin, log) = docker_with_leftovers();

    let doc = json(&sandbox, &bin, &["cleanup", "--yes"]);
    assert_eq!(
        doc["removed_volumes"],
        serde_json::json!([format!("{gone}_ck_model_data")]),
        "{doc}"
    );
    assert_eq!(
        doc["removed_networks"],
        serde_json::json!([format!("{gone}_default")])
    );
    let calls = read(&log);
    assert!(
        calls
            .lines()
            .any(|l| l == format!("volume rm {gone}_ck_model_data")),
        "{calls}"
    );
    assert!(
        !calls
            .lines()
            .any(|l| l == format!("volume rm {gone}_busy_data")),
        "a volume a container mounts stays: {calls}"
    );

    let record = record(&sandbox);
    assert!(!record.contains(&gone), "{record}");
    assert!(record.contains(&kept), "{record}");

    // A second run has nothing left to do, and says so.
    let text = String::from_utf8(
        chap_with_docker(&sandbox, sandbox.home.path(), &bin, &["cleanup"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert!(
        text.contains("nothing to clean up: 1 recorded deployment is still in place"),
        "{text}"
    );
}

/// A removed deployment that still has a container is not cleaned up: its
/// volumes may be in use, and the line names the command that stops it.
#[test]
fn a_removed_deployment_with_containers_is_kept_and_says_how_to_stop_them() {
    let sandbox = Sandbox::new();
    let dir = sandbox.home.path().join("running");
    sandbox
        .chap()
        .args(["init"])
        .arg(&dir)
        .args(["--only", "none", "--models", "none"])
        .assert()
        .success();
    let name = state(&dir)["compose_project"].as_str().unwrap().to_string();
    std::fs::remove_dir_all(&dir).unwrap();
    let (_fake, bin, log) = docker_with_leftovers();

    let doc = json(&sandbox, &bin, &["cleanup", "--yes"]);
    assert_eq!(doc["leftovers"], serde_json::json!([]), "{doc}");
    assert_eq!(doc["kept"][0]["project"], name.as_str());
    assert!(
        doc["kept"][0]["reason"]
            .as_str()
            .unwrap()
            .contains(&format!("`docker compose -p {name} down`")),
        "{doc}"
    );
    assert!(!read(&log).contains("volume rm"));
}
