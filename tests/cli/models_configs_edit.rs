//! `varde models configs list`, `add` and `archive` against a stand-in
//! chap-core in the shape of 2.4.

use crate::common::*;
use serde_json::Value as Json;
use std::path::PathBuf;

/// A deployment of `models` pointed at a chap-core 2.4 stand-in.
fn fresh_project(sandbox: &Sandbox, models: &str) -> PathBuf {
    let port = fresh_chap_core_server();
    sandbox
        .init(&["--models", models, "--api-port", &port.to_string()])
        .assert()
        .success();
    sandbox.project()
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("the lines are text")
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("the lines are text")
}

const EWARS: &str = "chapkit_ewars_model";

#[test]
fn add_creates_a_configured_model_with_checked_values() {
    let sandbox = Sandbox::new();
    let dir = fresh_project(&sandbox, EWARS);
    chap_in(&sandbox, &dir, &["models", "configs", "sync"])
        .assert()
        .success();

    let output = chap_in(
        &sandbox,
        &dir,
        &[
            "models",
            "configs",
            "add",
            EWARS,
            "--name",
            "short_lags",
            "--set",
            "n_lags=2",
            "--set",
            "lags=1,2,3",
            "--set",
            "method=exact",
            "--covariates",
            "rainfall,rainfall,mean_temperature",
        ],
    )
    .assert()
    .success()
    .get_output()
    .clone();
    assert_eq!(
        stdout_of(&output),
        format!("created configured model short_lags of {EWARS}\n")
    );
    let seen = recorded(&sandbox, &dir);
    let posts = seen["configured_posts"].as_array().expect("a list");
    let post = posts.last().expect("a post");
    assert_eq!(post["name"], "short_lags");
    assert_eq!(post["model_template_id"], TEMPLATE_IDS);
    assert_eq!(
        post["user_option_values"],
        serde_json::json!({"n_lags": 2, "lags": [1, 2, 3], "method": "exact"})
    );
    assert_eq!(
        post["additional_continuous_covariates"],
        serde_json::json!(["rainfall", "mean_temperature"])
    );

    // The same name again is refused, and nothing is posted.
    let output = chap_in(
        &sandbox,
        &dir,
        &["models", "configs", "add", EWARS, "--name", "short_lags"],
    )
    .assert()
    .code(2)
    .get_output()
    .clone();
    assert!(
        stderr_of(&output).contains(&format!(
            "{EWARS} has a configured model short_lags already; give another --name, or \
             archive it first with `varde models configs archive {EWARS} short_lags`"
        )),
        "{}",
        stderr_of(&output)
    );

    // The bare command lists them, with where each one comes from.
    let output = chap_in(&sandbox, &dir, &["models", "configs"])
        .assert()
        .success()
        .get_output()
        .clone();
    let text = stdout_of(&output);
    assert!(text.contains("NAME"), "{text}");
    assert!(text.contains("SOURCE"), "{text}");
    assert!(!text.contains("ARCHIVED"), "{text}");
    let line = text
        .lines()
        .find(|line| line.starts_with("short_lags"))
        .expect("a row");
    assert!(line.contains("rainfall,mean_temperature"), "{line}");
    assert!(line.contains("lags=1,2,3 method=exact n_lags=2"), "{line}");
    assert!(line.ends_with("by hand"), "{line}");
    let line = text
        .lines()
        .find(|line| line.starts_with("monthly_climate"))
        .expect("a row");
    assert!(line.ends_with("marketplace"), "{line}");
    assert!(text.ends_with("4 configured models of 1 model\n"), "{text}");

    let report = json_of(&mut chap_in(
        &sandbox,
        &dir,
        &["--json", "models", "configs", "list", EWARS],
    ));
    let configs = report["models"][0]["configs"].as_array().expect("a list");
    let added = configs
        .iter()
        .find(|config| config["variant"] == "short_lags")
        .expect("the added one");
    assert_eq!(added["name"], format!("{PASSING_MODEL}:short_lags"));
    assert_eq!(added["source"], "by hand");
    assert_eq!(added["values"]["n_lags"], 2);
}

#[test]
fn add_refuses_an_unknown_key_a_bad_value_and_a_required_covariate() {
    let sandbox = Sandbox::new();
    let dir = fresh_project(
        &sandbox,
        &format!("{EWARS},chapkit_rwanda_malaria_bym_model"),
    );

    let refused = |args: &[&str]| -> String {
        let mut all = vec!["models", "configs", "add"];
        all.extend_from_slice(args);
        let output = chap_in(&sandbox, &dir, &all)
            .assert()
            .code(2)
            .get_output()
            .clone();
        stderr_of(&output)
    };
    let text = refused(&[EWARS, "--name", "x", "--set", "lag=2"]);
    assert!(
        text.contains(&format!(
            "`lag` is not an option of {EWARS}; its options are label (text or null), lags \
             (list of integers), method (one of fast, exact), n_lags (integer), precision \
             (number), seasonal (true or false)"
        )),
        "{text}"
    );
    let text = refused(&[EWARS, "--name", "x", "--set", "n_lags=two"]);
    assert!(
        text.contains("`two` is not a value of `n_lags`, which takes an integer, such as `3`"),
        "{text}"
    );
    let text = refused(&[EWARS, "--name", "x", "--set", "n_lags"]);
    assert!(text.contains("is not KEY=VALUE"), "{text}");
    let text = refused(&[EWARS, "--name", "x", "--covariates", "population"]);
    assert!(
        text.contains("`population` is a required covariate"),
        "{text}"
    );
    let text = refused(&[
        "chapkit_rwanda_malaria_bym_model",
        "--name",
        "x",
        "--covariates",
        "rainfall",
    ]);
    assert!(text.contains("takes no additional covariates"), "{text}");
    let text = refused(&[EWARS]);
    assert!(
        text.contains("give it with --name NAME, or use -i"),
        "{text}"
    );
    // -i needs a terminal, and a test has none.
    let text = refused(&[EWARS, "-i"]);
    assert!(text.contains("-i asks in a terminal"), "{text}");

    let seen = recorded(&sandbox, &dir);
    assert_eq!(seen["configured_posts"], Json::Array(Vec::new()));
}

#[test]
fn archive_archives_and_list_all_shows_it() {
    let sandbox = Sandbox::new();
    let dir = fresh_project(&sandbox, EWARS);
    chap_in(&sandbox, &dir, &["models", "configs", "sync"])
        .assert()
        .success();

    let output = chap_in(
        &sandbox,
        &dir,
        &["models", "configs", "archive", EWARS, "monthly_climate"],
    )
    .assert()
    .success()
    .get_output()
    .clone();
    assert_eq!(
        stdout_of(&output),
        format!("archived configured model monthly_climate of {EWARS}\n")
    );
    let seen = recorded(&sandbox, &dir);
    assert_eq!(
        seen["deleted"],
        serde_json::json!([format!("/v1/crud/configured-models/{CREATED_IDS}")])
    );

    let text = stdout_of(
        chap_in(&sandbox, &dir, &["models", "configs", "list"])
            .assert()
            .success()
            .get_output(),
    );
    assert!(!text.contains("monthly_climate"), "{text}");
    let text = stdout_of(
        chap_in(&sandbox, &dir, &["models", "configs", "--all"])
            .assert()
            .success()
            .get_output(),
    );
    assert!(text.contains("ARCHIVED"), "{text}");
    let line = text
        .lines()
        .find(|line| line.starts_with("monthly_climate"))
        .expect("a row");
    assert!(line.ends_with("yes"), "{line}");

    // A name it does not have names the ones it has.
    let output = chap_in(
        &sandbox,
        &dir,
        &["models", "configs", "archive", EWARS, "weekly"],
    )
    .assert()
    .failure()
    .get_output()
    .clone();
    assert!(
        stderr_of(&output).contains(&format!(
            "{EWARS} has no configured model weekly; its configured models are \
             monthly_population_only, monthly_region_seasonal"
        )),
        "{}",
        stderr_of(&output)
    );

    // The last two: then sync makes them again, and the report says so.
    chap_in(
        &sandbox,
        &dir,
        &[
            "models",
            "configs",
            "archive",
            EWARS,
            "monthly_population_only",
        ],
    )
    .assert()
    .success();
    let output = chap_in(
        &sandbox,
        &dir,
        &[
            "models",
            "configs",
            "archive",
            EWARS,
            "monthly_region_seasonal",
        ],
    )
    .assert()
    .success()
    .get_output()
    .clone();
    assert!(
        stdout_of(&output).contains(
            "has no other configured model now, so `varde models configs sync` and `varde up \
             --wait` create its configured models again"
        ),
        "{}",
        stdout_of(&output)
    );
}

#[test]
fn list_says_how_to_create_them_when_there_are_none() {
    let sandbox = Sandbox::new();
    let dir = fresh_project(&sandbox, EWARS);
    let output = chap_in(&sandbox, &dir, &["models", "configs"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert_eq!(
        stdout_of(&output),
        format!(
            "{EWARS}: chap-core has no configured model of it; `varde models configs sync \
             {EWARS}` creates them\n"
        )
    );
}

#[test]
fn list_names_the_way_out_when_chap_core_is_down() {
    let sandbox = Sandbox::new();
    sandbox
        .init(&["--models", EWARS, "--api-port", "18799"])
        .assert()
        .success();
    let dir = sandbox.project();
    let output = chap_in(&sandbox, &dir, &["models", "configs"])
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(
        stderr_of(&output).contains("is not responding"),
        "{}",
        stderr_of(&output)
    );
}
