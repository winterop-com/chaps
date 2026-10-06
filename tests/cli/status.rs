use crate::common::*;
use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use serde_json::Value as Json;
use std::path::Path;

/// Rewrite CRLF line endings as LF, for text that came out of the checkout
/// rather than out of the CLI: the CLI writes `\n` on every platform, and
/// git may not have.
fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// A server that answers every request with an HTML 200, on a port of its own.
///
/// Stands in for whatever else may hold chap-core's port: a dev server, a
/// proxy, a static site.
fn html_server() -> u16 {
    server(
        "text/html; charset=utf-8",
        "<!doctype html><html><body>a dev server</body></html>",
    )
}

#[test]
fn status_does_not_call_something_that_is_not_chap_core_up() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    let url = format!("http://127.0.0.1:{}", html_server());
    let mut status = sandbox.chap();
    status
        .arg("-C")
        .arg(&dir)
        .args(["status", "--url"])
        .arg(&url);
    let assert = status.assert().failure();
    let out = assert.get_output();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // A 200 is not enough: the report says down, and the one error line says
    // what answered instead.
    assert!(
        stdout.contains(&format!("chap-core   down   {url}")),
        "{stdout}"
    );
    assert!(stderr.contains("not chap-core"), "{stderr}");
    assert!(stderr.contains("text/html"), "{stderr}");
    assert!(!stdout.contains("   up   "), "{stdout}");

    // And --json stays one document, with the same verdict in it.
    let mut status = sandbox.chap();
    status
        .arg("-C")
        .arg(&dir)
        .args(["--json", "status", "--url"])
        .arg(&url);
    let out = status.assert().failure().get_output().stdout.clone();
    let report: Json = serde_json::from_slice(&out).expect("status --json is one document");
    assert_eq!(report["api"]["state"], "down");
    assert!(
        report["api"]["error"]
            .as_str()
            .unwrap()
            .contains("not chap-core")
    );
    assert_eq!(report["models"][0]["state"], "not-running");
    assert_eq!(report["models"][0]["reach"], "internal");
    assert_eq!(report["version"]["pinned"], true);
}

/// A chap-core that answers 401 is up and refusing the token, not down: the
/// row says so, the registry it could not read is not guessed at, and the one
/// error line is about the token rather than the container. `/health` is open
/// on a real chap-core, so the 401 comes from the registry.
#[test]
fn status_calls_a_refused_token_a_refused_token_and_not_down() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    let port = protected_chap_core_server();
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model",
            "--api-port",
            &port.to_string(),
        ])
        .assert()
        .success();
    let url = format!("http://127.0.0.1:{port}");

    let out = chap_in(&sandbox, &dir, &["status", "--url", &url])
        .env_remove("CHAP_API_TOKEN")
        .assert()
        .failure()
        .get_output()
        .clone();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains(&format!("chap-core   up, token rejected   {url}")),
        "{stdout}"
    );
    assert!(!stdout.contains("down"), "{stdout}");
    // No model table: every row would claim a registration it could not see.
    assert!(!stdout.contains("not registered"), "{stdout}");
    assert!(
        stderr.contains("is up and did not accept the API token"),
        "{stderr}"
    );
    assert!(
        stderr.contains("answers /v2/services with HTTP 401"),
        "{stderr}"
    );
    assert!(!stderr.contains("something in front of"), "{stderr}");
    assert!(!stderr.contains("not responding"), "{stderr}");

    let out = chap_in(&sandbox, &dir, &["--json", "status", "--url", &url])
        .env_remove("CHAP_API_TOKEN")
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let report: Json = serde_json::from_slice(&out).expect("status --json is one document");
    assert_eq!(report["api"]["state"], "rejected");
}

/// The version on the chap-core line is the one the running build reports on
/// `/system/info`, with its revision, and not the tag the project pins.
#[test]
fn status_reports_the_version_chap_core_runs() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    let port = chap_core_server();
    sandbox
        .init(&["--api-port", &port.to_string()])
        .assert()
        .success();
    let url = format!("http://127.0.0.1:{port}");

    let out = chap_in(&sandbox, &dir, &["--json", "status", "--url", &url])
        .output()
        .expect("varde runs")
        .stdout;
    let report: Json = serde_json::from_slice(&out).expect("status --json is one document");
    assert_eq!(report["api"]["state"], "up", "{report}");
    assert_eq!(report["version"]["value"], CHAP_CORE_VERSION);
    assert_eq!(report["version"]["pinned"], false);
    assert_eq!(report["version"]["revision"], CHAP_CORE_REVISION);
}

#[test]
fn the_wrappers_speak_up_for_a_project_that_was_never_started() {
    if !docker_ready() {
        return;
    }
    let sandbox = Sandbox::new();
    // A directory of its own: compose names the project after it, and these
    // wrappers ask docker about that name.
    let dir = sandbox.home.path().join("varde-never-started");
    // An API port of this test's own: `status` below has to find nothing
    // answering for "Chap is not running" to be the truth about it, and the
    // default 8000 is a port a developer may well be serving something on.
    let api_port = free_port();
    let mut init = sandbox.chap();
    init.arg("init")
        .arg(&dir)
        .args(["--models", "chapkit_ewars_model", "--api-port"])
        .arg(api_port.to_string());
    init.assert().success();

    // `docker compose logs` on a project with no containers prints nothing at
    // all and exits 0; the wrapper says what is going on, and fails.
    chap_in(&sandbox, &dir, &["logs"])
        .assert()
        .failure()
        .stdout(predicates::str::contains(
            "nothing is running for this project; start Chap with `varde up`",
        ));

    // The same line for `docker ps`, which is a question, not a failure.
    chap_in(&sandbox, &dir, &["docker", "ps"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "nothing is running for this project",
        ));

    // Under --json the answer is still one document: an empty list.
    let out = chap_in(&sandbox, &dir, &["--json", "docker", "ps"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("docker ps --json is one document");
    assert_eq!(value, serde_json::json!([]));

    // `down` says what it stopped, even when that was nothing.
    chap_in(&sandbox, &dir, &["down"])
        .assert()
        .success()
        .stdout(predicates::str::contains("nothing was running"));

    // `status` on a deployment that was never started is one line about the
    // stack, not a table of models that cannot have registered.
    chap_in(&sandbox, &dir, &["status"])
        .assert()
        .failure()
        .stdout(predicates::str::contains(
            "Chap is not running; start it with `varde up`",
        ));
}

#[test]
fn up_offers_both_names_for_running_in_the_foreground() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    chap_in(&sandbox, &dir, &["up", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("-a, --attach"))
        .stdout(predicates::str::contains("[alias: --foreground]"))
        .stdout(predicates::str::contains(
            "Run in the foreground and stream all logs (Ctrl-C stops Chap)",
        ));
}

#[test]
fn the_command_reference_chapter_matches_the_help_texts() {
    // `docs-markdown` needs neither a project nor the network, so it runs
    // straight out of the repository.
    let out = Command::cargo_bin("varde")
        .expect("the varde binary is built")
        .arg("docs-markdown")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let generated = String::from_utf8(out).expect("the reference is UTF-8");

    // Read through whatever the checkout did to it: a Windows clone with
    // git's default `core.autocrlf` has CRLF on disk, and `--help` is
    // rendered with `\n` everywhere.
    let committed = normalize_newlines(&read(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/reference.md"),
    ));

    assert_eq!(
        generated, committed,
        "docs/reference.md no longer matches the `--help` texts; \
         run `make docs-reference` and commit the result"
    );
}

#[test]
fn init_without_the_flag_leaves_both_secrets_commented() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains("API token:").not());

    let env = sandbox.env();
    assert!(env.contains("\n# CHAP_API_TOKEN=\n"), "{env}");
    assert!(env.contains("\n# SERVICEKIT_REGISTRATION_KEY=\n"), "{env}");
    assert_eq!(env_value(&env, "CHAP_API_TOKEN"), None);
    assert_eq!(env_value(&env, "SERVICEKIT_REGISTRATION_KEY"), None);

    // Nothing is protected, and the overlay ships the line commented out, the
    // way chap-core's own example does.
    let state = state(&dir);
    assert_eq!(state["auth"]["api_token"], false);
    assert_eq!(state["auth"]["registration_key"], false);
    assert!(!overlay_sends_the_key(&dir));
    assert!(
        read(&dir.join("compose.chapkit-ewars-model.yml"))
            .contains("      # SERVICEKIT_REGISTRATION_KEY:")
    );
}
