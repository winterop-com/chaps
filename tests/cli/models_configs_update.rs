//! `varde models configs update`, `export` and `add --from` against a
//! stand-in chap-core in the shape of 2.4.

use crate::common::*;
use std::path::PathBuf;

const EWARS: &str = "chapkit_ewars_model";

/// A deployment of `models` pointed at a chap-core 2.4 stand-in, with the
/// marketplace configurations made.
fn synced_project(sandbox: &Sandbox, models: &str) -> PathBuf {
    let port = fresh_chap_core_server();
    sandbox
        .init(&["--models", models, "--api-port", &port.to_string()])
        .assert()
        .success();
    let dir = sandbox.project();
    chap_in(sandbox, &dir, &["models", "configs", "sync"])
        .assert()
        .success();
    dir
}

fn stdout_of(output: &std::process::Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("the lines are text")
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("the lines are text")
}

fn run(sandbox: &Sandbox, dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    chap_in(sandbox, dir, args).output().expect("varde runs")
}

#[test]
fn update_posts_the_new_values_and_archives_the_old_row() {
    let sandbox = Sandbox::new();
    let dir = synced_project(&sandbox, EWARS);

    let output = run(
        &sandbox,
        &dir,
        &[
            "models",
            "configs",
            "update",
            EWARS,
            "monthly_climate",
            "--set",
            "method=exact",
            "--unset",
            "n_lags",
        ],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert_eq!(
        stdout_of(&output),
        format!(
            "updated configured model monthly_climate of {EWARS}; chap-core keeps the old \
             values as an archived configured model\n"
        )
    );
    let seen = recorded(&sandbox, &dir);
    let posts = seen["configured_posts"].as_array().expect("a list");
    let post = posts.last().expect("the update");
    assert_eq!(post["name"], "monthly_climate");
    assert_eq!(
        post["user_option_values"],
        serde_json::json!({"method": "exact", "precision": 0.01, "region_seasonal": false})
    );
    // The covariates stay as they were.
    assert_eq!(
        post["additional_continuous_covariates"],
        serde_json::json!(["rainfall", "mean_temperature"])
    );
    // The post went first, then the archive of the old row.
    assert_eq!(
        seen["deleted"],
        serde_json::json!([format!("/v1/crud/configured-models/{CREATED_IDS}")])
    );

    // The same values again change nothing.
    let output = run(
        &sandbox,
        &dir,
        &[
            "models",
            "configs",
            "update",
            EWARS,
            "monthly_climate",
            "--set",
            "method=exact",
        ],
    );
    assert_eq!(
        stdout_of(&output),
        format!(
            "configured model monthly_climate of {EWARS} has these values already; nothing \
             changed\n"
        )
    );
    // No options is the form, which needs a terminal.
    let output = run(
        &sandbox,
        &dir,
        &["models", "configs", "update", EWARS, "monthly_climate"],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr_of(&output).contains("there is no terminal here; give the new values with --set"),
        "{}",
        stderr_of(&output)
    );
    // A name it does not have names the ones it has.
    let output = run(
        &sandbox,
        &dir,
        &[
            "models",
            "configs",
            "update",
            EWARS,
            "weekly",
            "--set",
            "max_lag=2",
        ],
    );
    assert!(!output.status.success());
    assert!(
        stderr_of(&output).contains(&format!("{EWARS} has no configured model weekly")),
        "{}",
        stderr_of(&output)
    );
}

#[test]
fn export_writes_the_marketplace_format_and_add_from_reads_it() {
    let sandbox = Sandbox::new();
    let dir = synced_project(&sandbox, EWARS);
    let output = run(
        &sandbox,
        &dir,
        &[
            "models",
            "configs",
            "add",
            EWARS,
            "--name",
            "mine",
            "--set",
            "max_lag=2",
        ],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));

    // Standard output is the YAML alone.
    let output = run(&sandbox, &dir, &["models", "configs", "export"]);
    assert!(output.status.success(), "{}", stderr_of(&output));
    let yaml = stdout_of(&output);
    assert!(yaml.starts_with("configurations:\n"), "{yaml}");
    assert!(yaml.contains("  mine:\n"), "{yaml}");
    assert!(yaml.contains("prediction_periods: 3"), "{yaml}");

    let file = dir.join("exported.yaml");
    let output = run(
        &sandbox,
        &dir,
        &[
            "models",
            "configs",
            "export",
            EWARS,
            "--out",
            "exported.yaml",
            "-v",
        ],
    );
    let text = stdout_of(&output);
    assert!(
        text.starts_with(&format!(
            "wrote 4 configurations of {EWARS} to exported.yaml\n"
        )),
        "{text}"
    );
    // The stand-in has no config schema, so the horizon of `mine` is
    // chapkit's own.
    assert!(
        text.contains("prediction_periods of mine comes from chapkit's default"),
        "{text}"
    );
    assert_eq!(std::fs::read_to_string(&file).expect("the file"), yaml);

    // Into another deployment: every configuration, by its name.
    let other = Sandbox::new();
    let port = fresh_chap_core_server();
    other
        .init(&["--models", EWARS, "--api-port", &port.to_string()])
        .assert()
        .success();
    let other_dir = other.project();
    let from = file.display().to_string();
    let output = run(
        &other,
        &other_dir,
        &["models", "configs", "add", EWARS, "--from", &from],
    );
    assert!(output.status.success(), "{}", stderr_of(&output));
    assert!(
        stdout_of(&output).contains(&format!("created configured model mine of {EWARS}")),
        "{}",
        stdout_of(&output)
    );
    let seen = recorded(&other, &other_dir);
    let posts = seen["configured_posts"].as_array().expect("a list");
    assert_eq!(posts.len(), 4);
    let mine = posts.iter().find(|p| p["name"] == "mine").expect("mine");
    assert_eq!(
        mine["user_option_values"],
        serde_json::json!({"max_lag": 2})
    );

    // A second time every name is live, and nothing is posted.
    let output = run(
        &other,
        &other_dir,
        &["models", "configs", "add", EWARS, "--from", &from],
    );
    assert_eq!(output.status.code(), Some(2));
    let seen = recorded(&other, &other_dir);
    assert_eq!(seen["configured_posts"].as_array().map(Vec::len), Some(4));
}

#[test]
fn export_of_several_models_asks_for_one() {
    let sandbox = Sandbox::new();
    let dir = synced_project(
        &sandbox,
        &format!("{EWARS},chapkit_rwanda_malaria_bym_model"),
    );
    let output = run(&sandbox, &dir, &["models", "configs", "export"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr_of(&output).contains(
            "an export is the configurations of one model; name one of chapkit_ewars_model, \
             chapkit_rwanda_malaria_bym_model"
        ),
        "{}",
        stderr_of(&output)
    );
}
