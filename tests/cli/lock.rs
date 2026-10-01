//! Two chaps commands on one deployment at once: the state lock in
//! `.chaps/lock` makes the second wait for the first.

use crate::common::*;
use std::collections::BTreeSet;
use std::process::{Command, Stdio};

/// `chaps --offline -C <project> <args>` as a plain process, so several can
/// run at the same time.
fn spawn(sandbox: &Sandbox, args: &[&str]) -> std::process::Child {
    Command::new(assert_cmd::cargo::cargo_bin("chaps"))
        .env("CHAPS_CACHE_DIR", sandbox.cache.path())
        .env("CHAPS_DATA_DIR", sandbox.cache.path().join("data"))
        .env("CHAPS_NO_DOCKER_PROBE", "1")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_TOKEN")
        .current_dir(sandbox.home.path())
        .arg("--offline")
        .arg("-C")
        .arg(sandbox.project())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("chaps starts")
}

#[test]
fn parallel_enables_with_port_auto_all_land_on_different_ports() {
    let sandbox = Sandbox::new();
    let base = port_base();
    sandbox
        .init(&["--models", "none", "--port-base", &base.to_string()])
        .assert()
        .success();

    let ids = [
        "chapkit_ewars_model",
        "chapkit_simple_multistep_model",
        "auto_arima_chapkit",
        "chapkit_rwanda_malaria_bym_model",
    ];
    let children: Vec<_> = ids
        .iter()
        .map(|id| spawn(&sandbox, &["models", "enable", id, "--port", "auto"]))
        .collect();
    for (id, child) in ids.iter().zip(children) {
        let out = child.wait_with_output().expect("chaps finishes");
        assert!(
            out.status.success(),
            "{id}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let models = state(&sandbox.project())["models"].clone();
    let mut ports = BTreeSet::new();
    for id in ids {
        let entry = &models[id];
        let port = entry["host_port"]
            .as_u64()
            .unwrap_or_else(|| panic!("{id} has a host port: {entry}"));
        assert!(ports.insert(port), "{id} shares port {port}: {models}");
    }
}
