//! `varde models configure`, and the same step in `models test --backtest`
//! and `varde status`, against a stand-in chap-core in the shape of 2.4.

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

#[test]
fn configure_creates_the_configured_models_of_each_registered_model() {
    let sandbox = Sandbox::new();
    let dir = fresh_project(
        &sandbox,
        "chapkit_ewars_model,chapkit_simple_multistep_model",
    );

    let output = chap_in(&sandbox, &dir, &["models", "configure"])
        .assert()
        .success()
        .get_output()
        .clone();
    let text = stdout_of(&output);
    // One line for each configured model, from the entry in the snapshot.
    for name in [
        "monthly_climate",
        "monthly_population_only",
        "monthly_region_seasonal",
    ] {
        assert!(
            text.contains(&format!(
                "chapkit_ewars_model: created configured model {name}"
            )),
            "{text}"
        );
    }
    // The stand-in has no registration for the multistep model.
    assert!(
        text.contains(
            "chapkit_simple_multistep_model: not registered with chap-core; run `varde models \
             configure` again once `varde status` shows it registered"
        ),
        "{text}"
    );

    let seen = recorded(&sandbox, &dir);
    assert_eq!(seen["templates"], serde_json::json!([PASSING_MODEL]));
    let posts = seen["configured_posts"].as_array().expect("a list");
    assert_eq!(posts.len(), 3, "{posts:?}");
    let climate = &posts[0];
    assert_eq!(climate["name"], "monthly_climate");
    assert_eq!(climate["model_template_id"], TEMPLATE_IDS);
    // The reserved keys are not user options.
    assert!(climate["user_option_values"]["prediction_periods"].is_null());
    assert!(climate["user_option_values"]["additional_continuous_covariates"].is_null());
    assert_eq!(
        climate["user_option_values"]["n_lags"],
        serde_json::json!([3, 3])
    );
    assert_eq!(
        climate["additional_continuous_covariates"],
        serde_json::json!(["rainfall", "mean_temperature"])
    );

    // A second run finds them and adds nothing.
    let output = chap_in(
        &sandbox,
        &dir,
        &["models", "configure", "chapkit_ewars_model"],
    )
    .assert()
    .success()
    .get_output()
    .clone();
    assert_eq!(
        stdout_of(&output),
        "chapkit_ewars_model: chap-core has a configured model of it already\n"
    );
    let seen = recorded(&sandbox, &dir);
    assert_eq!(seen["configured_posts"].as_array().map(Vec::len), Some(3));
}

#[test]
fn configure_warns_and_fails_when_chap_core_refuses_a_template() {
    let sandbox = Sandbox::new();
    let dir = fresh_project(&sandbox, "auto_arima_chapkit");

    let output = chap_in(&sandbox, &dir, &["--json", "models", "configure"])
        .assert()
        .failure()
        .get_output()
        .clone();
    let report: Json = serde_json::from_slice(&output.stdout).expect("one document");
    assert_eq!(report["ok"], false);
    assert_eq!(report["models"][0]["state"], "failed");
    let error = report["models"][0]["error"].as_str().unwrap_or_default();
    assert!(error.contains("HTTP 409 Conflict"), "{error}");
    let message = report["messages"][0].clone();
    assert_eq!(message["level"], "warning");
}

#[test]
fn status_shows_a_model_that_has_no_configured_model() {
    let sandbox = Sandbox::new();
    let dir = fresh_project(&sandbox, "chapkit_ewars_model");
    // `--url`: no container runs here, and the stand-in is what answers.
    let port = state(&dir)["api_port"].as_u64().expect("the port");
    let url = format!("http://127.0.0.1:{port}");

    // Read-only: status says it, and creates nothing.
    let report = json_of(&mut chap_in(
        &sandbox,
        &dir,
        &["--json", "status", "--url", &url],
    ));
    assert_eq!(report["models"][0]["id"], PASSING_MODEL);
    assert_eq!(report["models"][0]["state"], "not-configured");
    let texts: Vec<&str> = report["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter_map(|m| m["text"].as_str())
        .collect();
    assert!(
        texts.contains(
            &"chapkit-ewars-model: chap-core has no configured model for it, so nothing can run \
              it; run `varde models configure`"
        ),
        "{texts:?}"
    );
    assert_eq!(
        recorded(&sandbox, &dir)["configured_posts"],
        Json::Array(Vec::new())
    );

    let text = stdout_of(
        chap_in(&sandbox, &dir, &["status", "--url", &url])
            .assert()
            .get_output(),
    );
    assert!(text.contains("registered, not configured"), "{text}");

    chap_in(&sandbox, &dir, &["models", "configure"])
        .assert()
        .success();
    let report = json_of(&mut chap_in(
        &sandbox,
        &dir,
        &["--json", "status", "--url", &url],
    ));
    assert_eq!(report["models"][0]["state"], "registered");
}

/// The backtest configures the model first, then backtests the configured
/// model it made.
#[test]
fn models_test_backtest_configures_the_model_first() {
    let sandbox = Sandbox::new();
    let dir = fresh_project(&sandbox, "chapkit_ewars_model");

    let output = chap_in(&sandbox, &dir, &["models", "test", "--all", "--backtest"])
        .assert()
        .success()
        .get_output()
        .clone();
    let text = stdout_of(&output);
    assert!(text.contains("chapkit-ewars-model    pass"), "{text}");
    assert!(
        text.contains(
            "created configured models in chap-core for chapkit_ewars_model (monthly_climate, \
             monthly_population_only, monthly_region_seasonal)"
        ),
        "{text}"
    );
    let seen = recorded(&sandbox, &dir);
    // The first one it made, by its integer id.
    assert_eq!(seen["backtests"][0]["modelId"], CREATED_IDS);
}
