//! Which configured model `varde models test --backtest` uses: the one
//! `--config` names, or the one it picks, and always the line that says so.

use crate::common::*;
use serde_json::Value as Json;
use std::path::PathBuf;

/// The three stand-in models, pointed at a chap-core that has configured
/// models of each of them.
fn tested_project(sandbox: &Sandbox) -> PathBuf {
    let port = chap_core_server();
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model,chapkit_rwanda_malaria_bym_model,auto_arima_chapkit",
            "--api-port",
            &port.to_string(),
        ])
        .assert()
        .success();
    sandbox.project()
}

fn text_of(out: &std::process::Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("the rows are text")
}

/// The variant name of the passing model's configured model that the
/// backtest picks without `--config`.
fn picked() -> String {
    format!("{PASSING_MODEL}_1790267117082761")
}

/// Without `--config`, the line under the row and `--json` say which
/// configured model the scores are of.
#[test]
fn a_backtest_says_which_configured_model_it_used() {
    let sandbox = Sandbox::new();
    let dir = tested_project(&sandbox);

    let out = chap_in(
        &sandbox,
        &dir,
        &["models", "test", "chapkit_ewars_model", "--backtest"],
    )
    .assert()
    .success()
    .get_output()
    .clone();
    let text = text_of(&out);
    assert!(
        text.contains(&format!(
            "\n  configured model: {} (id {PASSING_CONFIGURED})\n",
            picked()
        )),
        "{text}"
    );

    let out = chap_in(
        &sandbox,
        &dir,
        &[
            "--json",
            "models",
            "test",
            "chapkit_ewars_model",
            "--backtest",
        ],
    )
    .assert()
    .success()
    .get_output()
    .clone();
    let report: Json = serde_json::from_slice(&out.stdout).expect("one document");
    let used = &report["models"][0]["configured_model"];
    assert_eq!(used["id"], Json::from(PASSING_CONFIGURED), "{report}");
    assert_eq!(used["variant"], Json::from(picked()), "{report}");
    assert_eq!(
        used["name"],
        Json::from(format!("{PASSING_MODEL}:{}", picked())),
        "{report}"
    );
}

/// `--config` names the configured model by its variant name, and that is
/// the one the backtest is of.
#[test]
fn config_names_the_configured_model_to_backtest() {
    let sandbox = Sandbox::new();
    let dir = tested_project(&sandbox);
    let variant = format!("test_config_{TEST_CONFIG}");

    let out = chap_in(
        &sandbox,
        &dir,
        &[
            "models",
            "test",
            "chapkit_ewars_model",
            "--backtest",
            "--config",
            &variant,
        ],
    )
    .assert()
    .success()
    .get_output()
    .clone();
    let text = text_of(&out);
    assert!(
        text.contains(&format!("  configured model: {variant} (id 19)\n")),
        "{text}"
    );
    let seen = recorded(&sandbox, &dir);
    assert_eq!(seen["backtests"][0]["modelId"], Json::from(19), "{seen}");
}

/// A name the model has no configured model of is a skip that names the
/// ones there are and the command that lists them.
#[test]
fn config_that_names_no_configured_model_is_a_skip_with_the_names() {
    let sandbox = Sandbox::new();
    let dir = tested_project(&sandbox);

    let out = chap_in(
        &sandbox,
        &dir,
        &[
            "models",
            "test",
            "chapkit_ewars_model",
            "--backtest",
            "--config",
            "weekly",
        ],
    )
    .assert()
    .failure()
    .get_output()
    .clone();
    let text = text_of(&out);
    assert!(
        text.contains(&format!(
            "chap-core has no configured model weekly for {PASSING_MODEL} 1.0.1; it has \
             test_config_{TEST_CONFIG}, {}",
            picked()
        )),
        "{text}"
    );
    assert!(
        text.contains(
            "  run `varde models configs list chapkit_ewars_model` to see its configured models"
        ),
        "{text}"
    );
    let seen = recorded(&sandbox, &dir);
    assert_eq!(seen["datasets"], serde_json::json!([]), "{seen}");
}

/// With `--all`, a name applies to each model that has it, and a model
/// without it is a skip.
#[test]
fn config_with_all_backtests_each_model_that_has_it() {
    let sandbox = Sandbox::new();
    let dir = tested_project(&sandbox);

    let out = chap_in(
        &sandbox,
        &dir,
        &[
            "models",
            "test",
            "--all",
            "--backtest",
            "--config",
            "default",
        ],
    )
    .assert()
    .failure()
    .get_output()
    .clone();
    let text = text_of(&out);
    // The passing model's `default` is archived, so it has none.
    assert!(
        text.contains(&format!(
            "chap-core has no configured model default for {PASSING_MODEL} 1.0.1"
        )),
        "{text}"
    );
    // The failing model has one, and its backtest ran.
    assert!(
        text.contains("  configured model: default (id 15)\n"),
        "{text}"
    );
    let seen = recorded(&sandbox, &dir);
    assert_eq!(seen["backtests"][0]["modelId"], Json::from(15), "{seen}");
}

/// `--config` is a choice of the backtest, and the model level has none.
#[test]
fn config_without_backtest_is_refused() {
    let sandbox = Sandbox::new();
    let dir = tested_project(&sandbox);
    chap_in(
        &sandbox,
        &dir,
        &["models", "test", "chapkit_ewars_model", "--config", "x"],
    )
    .assert()
    .failure()
    .stderr(predicates::str::contains("--backtest"));
}
