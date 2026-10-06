//! `varde up --wait`: the command holds until chap-core and the models answer,
//! and fails naming what never did. Unix only: the stand-in docker is a shell
//! script.

#![cfg(unix)]

use crate::common::*;
use std::path::PathBuf;
use tempfile::TempDir;

/// A `docker` whose compose `up` succeeds and whose `ps` reports chap and
/// the given services running.
fn running_docker(services: &[&str]) -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let mut rows = String::from("{\"Service\":\"chap\",\"State\":\"running\"}\\n");
    for service in services {
        rows.push_str(&format!(
            "{{\"Service\":\"{service}\",\"State\":\"running\"}}\\n"
        ));
    }
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

fn deployment(sandbox: &Sandbox, models: &str) -> (PathBuf, u16) {
    let port = chap_core_server();
    sandbox
        .init(&["--models", models, "--api-port", &port.to_string()])
        .assert()
        .success();
    (sandbox.project(), port)
}

#[test]
fn up_wait_returns_once_chap_core_and_the_models_answer() {
    let sandbox = Sandbox::new();
    let (dir, port) = deployment(&sandbox, "chapkit_ewars_model");
    let (_fake, bin) = running_docker(&[PASSING_MODEL]);

    let out = chap_with_docker(&sandbox, &dir, &bin, &["up", "--no-preflight", "--wait"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).expect("text");
    assert!(text.contains("ready in "), "{text}");
    let line = |name: &str| {
        text.lines()
            .find(|l| l.trim_start().starts_with(name))
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        line("chap-core"),
        ["chap-core", "up", &format!("http://localhost:{port}")],
        "{text}"
    );
    assert_eq!(line("chapkit-ewars-model")[1], "registered", "{text}");
}

#[test]
fn up_wait_fails_on_its_deadline_naming_what_never_answered() {
    let sandbox = Sandbox::new();
    let (dir, _) = deployment(&sandbox, "chapkit_simple_multistep_model");
    let (_fake, bin) = running_docker(&["chapkit-simple-multistep-model"]);

    chap_with_docker(
        &sandbox,
        &dir,
        &bin,
        &["up", "--no-preflight", "--wait", "--timeout", "1"],
    )
    .assert()
    .failure()
    .stderr(predicates::str::contains(
        "not ready after 1s: chapkit-simple-multistep-model (running, not registered)",
    ));
}

#[test]
fn timeout_needs_wait() {
    let sandbox = Sandbox::new();
    let (dir, _) = deployment(&sandbox, "none");
    chap_in(&sandbox, &dir, &["up", "--timeout", "5"])
        .assert()
        .failure();
}

#[test]
fn up_json_is_one_document_with_the_models_and_what_wait_found() {
    let sandbox = Sandbox::new();
    let (dir, port) = deployment(&sandbox, "chapkit_ewars_model");
    let (_fake, bin) = running_docker(&[PASSING_MODEL]);

    let out = chap_with_docker(
        &sandbox,
        &dir,
        &bin,
        &["--json", "up", "--no-preflight", "--wait"],
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let doc: serde_json::Value = serde_json::from_slice(&out).expect("one JSON document");
    assert_eq!(doc["ok"], true);
    assert_eq!(doc["api_url"], format!("http://localhost:{port}"));
    assert_eq!(doc["models"][0]["service_id"], PASSING_MODEL);
    assert_eq!(doc["wait"]["ready"], true);
    assert_eq!(doc["wait"]["models"][0]["state"], "registered");
}
