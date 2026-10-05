use crate::common::*;
use serde_json::Value as Json;

/// `doctor --json` is the same checklist as one document.
#[test]
fn doctor_json_is_a_list_of_checks_and_a_summary() {
    let (_cache, mut cmd) = bare();
    let out = cmd
        .args(["--offline", "--json", "doctor"])
        .output()
        .expect("doctor runs");
    let report: Json = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("doctor --json did not parse: {e}"));

    let checks = report["checks"].as_array().expect("an array of checks");
    assert!(!checks.is_empty());
    for check in checks {
        for field in ["id", "name", "status", "detail", "fix"] {
            assert!(check.get(field).is_some(), "{check} has no `{field}`");
        }
        assert!(
            ["ok", "warn", "fail", "skip"].contains(&check["status"].as_str().unwrap()),
            "{check} has an unknown status"
        );
    }
    let summary = &report["summary"];
    let count = |key: &str| summary[key].as_u64().unwrap_or_else(|| panic!("no {key}"));
    assert_eq!(
        (count("ok") + count("warn") + count("fail") + count("skip")) as usize,
        checks.len(),
        "the summary has to add up to the checks: {report}"
    );
}

/// Inside a freshly written deployment the two checks that are about the
/// files alone must both pass, and `--offline` must keep every probe that
/// would touch the network out of the run.
#[test]
fn doctor_in_a_fresh_project_finds_the_files_in_order_and_skips_the_network() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "default"]).assert().success();

    let out = sandbox
        .chap()
        .arg("-C")
        .arg(sandbox.project())
        .args(["--json", "doctor"])
        .output()
        .expect("doctor runs");
    let report: Json = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("doctor --json did not parse: {e}"));

    assert_eq!(doctor_status(&report, "project-files"), "ok", "{report}");
    assert_eq!(doctor_status(&report, "sync"), "ok", "{report}");
    // `init` generates a password, writes the pin comments and leaves
    // authentication off, which is a complete `.env` and not a warning.
    assert_eq!(doctor_status(&report, "env"), "ok", "{report}");
    // Nothing has ever been started here: the way out is `chaps up`, or, on a
    // machine whose docker is not answering, starting Docker first.
    assert_eq!(doctor_status(&report, "health"), "skip", "{report}");
    let fix = doctor_check(&report, "health")["fix"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        fix.contains("chaps up") || fix.contains("start Docker"),
        "{report}"
    );

    // Every network probe, and the image check that would follow one.
    for id in ["net-ghcr", "net-marketplace", "net-releases", "net-github"] {
        assert_eq!(doctor_status(&report, id), "skip", "{report}");
        assert!(
            doctor_check(&report, id)["detail"]
                .as_str()
                .unwrap()
                .contains("offline"),
            "{id} does not say why it was skipped: {report}"
        );
    }
    let image = doctor_check(&report, "image-chapkit-ewars-model");
    assert_eq!(image["status"], "skip", "{report}");
    assert!(image["detail"].as_str().unwrap().contains("offline"));

    // A moving chap-core tag is nothing to compare against a release list.
    assert_eq!(doctor_status(&report, "chap-core-pin"), "ok", "{report}");
}

/// The `registry pin <id>` line: what the marketplace pins for an enabled
/// model, against what that model's own repository has published since.
///
/// The hub's catalogue pins `sha-fa880a1`, committed on 1 September, while
/// `main` has published `sha-1eb8cf1` on the 8th. That lag is what the check
/// exists for: it is the marketplace that is behind, not the deployment, so
/// the line warns and points at the marketplace rather than at `chaps`.
#[test]
fn doctor_warns_when_the_marketplace_pin_lags_the_model_repository() {
    let (sandbox, _dir, port) = added_sandbox(Hub::new().publishing(MARKETPLACE_TAG));
    sandbox
        .online(port)
        .args(["models", "enable", "chapkit_ewars_model"])
        .assert()
        .success();

    let report = json_of(sandbox.online(port).args(["--json", "doctor"]));
    let check = doctor_check(&report, "registry-pin-chapkit_ewars_model");
    assert_eq!(check["status"], "warn", "{report}");
    assert_eq!(
        check["name"], "registry pin chapkit_ewars_model",
        "{report}"
    );
    assert_eq!(
        check["detail"].as_str().unwrap(),
        "the registry pins sha-fa880a1 (2026-09-01) but main's newest build is \
         sha-1eb8cf1 (2026-09-08)",
        "{report}"
    );
    assert!(
        check["fix"]
            .as_str()
            .unwrap()
            .starts_with("ask the marketplace maintainers for a new pin"),
        "{report}"
    );
    // A marketplace that lags is nothing a deployment has to act on: the pin
    // lines warn, never fail. (The rest of the report depends on the machine,
    // a runner without docker fails its docker checks, so only these lines
    // are judged.)
    let pin_failures: Vec<&serde_json::Value> = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["id"].as_str().unwrap_or("").starts_with("registry-pin-"))
        .filter(|c| c["status"] == "fail")
        .collect();
    assert!(pin_failures.is_empty(), "{report}");

    // The same deployment with the network switched off asks nothing, and
    // every one of these lines says that is why.
    let offline = json_of(
        sandbox
            .chap()
            .arg("-C")
            .arg(sandbox.project())
            .args(["--json", "doctor"]),
    );
    let check = doctor_check(&offline, "registry-pin-chapkit_ewars_model");
    assert_eq!(check["status"], "skip", "{offline}");
    assert!(
        check["detail"].as_str().unwrap().contains("--offline"),
        "{offline}"
    );
}

/// The same line when the catalogue is up to date: the commit it pins is the
/// newest one the branch has a published build for.
#[test]
fn doctor_confirms_a_marketplace_pin_that_is_the_newest_build() {
    let (sandbox, _dir, port) = added_sandbox(Hub::new().pinning(OLD_SHA));
    sandbox
        .online(port)
        .args(["models", "enable", "chapkit_ewars_model"])
        .assert()
        .success();

    let report = json_of(sandbox.online(port).args(["--json", "doctor"]));
    let check = doctor_check(&report, "registry-pin-chapkit_ewars_model");
    assert_eq!(check["status"], "ok", "{report}");
    assert_eq!(
        check["detail"].as_str().unwrap(),
        format!("{OLD_TAG} is the newest build on main"),
        "{report}"
    );
    assert_eq!(check["fix"], Json::Null, "nothing to do: {report}");
}

/// Every request to GitHub's REST API carries the token when the environment
/// names one, and carries nothing when it does not - and the `github api`
/// line says which of the two hourly limits this address is spending.
///
/// The token matters because 60 requests an hour per address is one busy
/// morning, or one CI runner sharing its egress address with everybody else,
/// and a `doctor` whose pin lines all skip should say why.
#[test]
fn doctor_sends_the_github_token_and_says_what_is_left_of_the_hour() {
    const TOKEN: &str = "ghp_the_tests_own_token";
    let sandbox = Sandbox::new();
    sandbox
        .init(&["--models", "none", "--api-port", &free_port().to_string()])
        .assert()
        .success();
    let (port, seen) = Hub::new().start_seen();

    /// What the hub has been asked since it was last read, and by whom.
    fn asked(seen: &Seen) -> Vec<(String, Option<String>)> {
        let mut log = seen.lock().expect("the request log outlives its panics");
        std::mem::take(&mut *log)
    }

    // With a token in the environment, every REST request carries it.
    let report = json_of(
        sandbox
            .online(port)
            .env("GITHUB_TOKEN", TOKEN)
            .args(["--json", "doctor"]),
    );
    let check = doctor_check(&report, "net-github");
    assert_eq!(check["name"], "github api", "{report}");
    assert_eq!(check["status"], "ok", "{report}");
    assert_eq!(
        check["detail"].as_str().unwrap(),
        "reachable, 4990 of 5000 requests left this hour (token)",
        "{report}"
    );
    assert_eq!(check["fix"], Json::Null, "nothing to do: {report}");

    let log = asked(&seen);
    let api: Vec<&(String, Option<String>)> = log
        .iter()
        .filter(|(path, _)| path.starts_with("/repos/") || path.starts_with("/rate_limit"))
        .collect();
    assert!(
        api.iter().any(|(path, _)| path.starts_with("/rate_limit")),
        "the quota is asked for through /rate_limit: {log:?}"
    );
    assert!(api.len() > 1, "doctor asks GitHub more than once: {log:?}");
    for (path, authorization) in &api {
        assert_eq!(
            authorization.as_deref(),
            Some(format!("Bearer {TOKEN}").as_str()),
            "{path} went out without the token"
        );
    }
    // The raw file host is not the API and needs no credential.
    for (path, authorization) in &log {
        if path.ends_with("/compose.ghcr.yml") {
            assert_eq!(
                authorization, &None,
                "{path} was sent a token it never needs"
            );
        }
    }

    // A `-v` trace says the header went out and never what was in it: these
    // lines end up in bug reports.
    let out = sandbox
        .online(port)
        .env("GITHUB_TOKEN", TOKEN)
        .args(["-vv", "--json", "doctor"])
        .output()
        .expect("doctor runs");
    let trace = String::from_utf8(out.stderr).expect("utf-8");
    assert!(
        trace.contains("Authorization: Bearer <token>"),
        "the trace has to say the header was sent:\n{trace}"
    );
    assert!(!trace.contains(TOKEN), "the token reached a trace line");
    let _ = asked(&seen);

    // `GH_TOKEN` is the same door: the `gh` CLI's variable, read when
    // `GITHUB_TOKEN` names nothing.
    let report = json_of(
        sandbox
            .online(port)
            .env("GH_TOKEN", TOKEN)
            .args(["--json", "doctor"]),
    );
    assert!(
        doctor_check(&report, "net-github")["detail"]
            .as_str()
            .unwrap()
            .contains("(token)"),
        "{report}"
    );
    assert!(
        asked(&seen)
            .iter()
            .filter(|(path, _)| path.starts_with("/repos/"))
            .all(|(_, authorization)| authorization.is_some()),
        "GH_TOKEN has to reach GitHub like GITHUB_TOKEN does"
    );

    // With neither variable set, nothing goes out with an Authorization
    // header at all, and the line says what a token would buy.
    let report = json_of(sandbox.online(port).args(["--json", "doctor"]));
    let check = doctor_check(&report, "net-github");
    assert_eq!(check["status"], "ok", "{report}");
    assert_eq!(
        check["detail"].as_str().unwrap(),
        "reachable, 43 of 60 requests left this hour (no token; set GITHUB_TOKEN for 5000)",
        "{report}"
    );
    let log = asked(&seen);
    assert!(!log.is_empty(), "the run asked the hub for something");
    for (path, authorization) in &log {
        assert_eq!(
            authorization, &None,
            "{path} carried a credential nothing set"
        );
    }

    // `--offline` asks nothing, and the line names the flag as the reason.
    let offline = json_of(
        sandbox
            .chap()
            .arg("-C")
            .arg(sandbox.project())
            .args(["--json", "doctor"]),
    );
    let check = doctor_check(&offline, "net-github");
    assert_eq!(check["status"], "skip", "{offline}");
    assert!(
        check["detail"].as_str().unwrap().contains("--offline"),
        "{offline}"
    );
    assert!(asked(&seen).is_empty(), "--offline asks nothing");
}
