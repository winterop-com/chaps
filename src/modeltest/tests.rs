use super::configured::ConfiguredModel;
use super::dataset::{Observation, period_code};
use super::summary::first_sentence;
use super::*;

/// What a passing `chapkit test` prints, cut to the block this parser
/// reads plus the rule above it.
const PASSED: &str = "\
==================================================
TEST SUMMARY
==================================================
Service URL:           http://127.0.0.1:8000
Elapsed time:          16.85s
Configs created:       1
Trainings completed:   1
Trainings failed:      0
Predictions completed: 1
Predictions failed:    0
Validations run:       2
Validations failed:    0

Result: ALL TESTS PASSED
";

/// The same for a run whose prediction failed: the `[FAILED]` line goes to
/// stderr and the counts to stdout, and the caller hands both over at once.
const FAILED: &str = "\
  [FAILED] Prediction job 01M3A4TSB2YHRPFBEFCT4TMX4A: Job failed: predict script failed with \
exit code 1; diagnostic artifact 01M3A4TSB9WZ8V5XN0A4F4Y1M6 holds the workspace, stdout and \
stderr; stderr tail: Loading required package: Matrix | \
Error in inla.inlaprogram.has.crashed() : The inla program crashed. | Execution halted
==================================================
TEST SUMMARY
==================================================
Service URL:           http://127.0.0.1:8000
Elapsed time:          12.12s
Configs created:       1
Trainings completed:   1
Trainings failed:      0
Predictions completed: 0
Predictions failed:    1
Validations run:       2
Validations failed:    0

Result: 1 FAILURE(S)
";

fn run(id: &str, service_id: &str, verdict: Verdict, seconds: u64, summary: &str) -> Run {
    let mut run = Run::new(id, service_id, Level::Model);
    run.seconds = seconds;
    run.end(verdict, summary, None)
}

#[test]
fn a_passing_run_is_read_off_the_summary_block() {
    let summary = parse_summary(PASSED).expect("the block is there");
    assert!(summary.passed);
    assert_eq!(summary.elapsed, Some(16.85));
    assert_eq!(summary.configs_created, 1);
    assert_eq!(summary.trainings_completed, 1);
    assert_eq!(summary.predictions_completed, 1);
    assert_eq!(summary.validations_run, 2);
    assert_eq!(summary.validations_failed, 0);
    assert!(summary.failures.is_empty());
    assert_eq!(summary.did(), "1 training, 1 prediction");

    // The counts are read, not guessed: two predictions read as two.
    let two =
        parse_summary(&PASSED.replace("Predictions completed: 1", "Predictions completed: 2"))
            .expect("still a block");
    assert_eq!(two.did(), "1 training, 2 predictions");
}

#[test]
fn a_failing_run_names_the_phase_and_the_reason_from_the_stderr_tail() {
    let summary = parse_summary(FAILED).expect("the block is there");
    assert!(!summary.passed);
    assert_eq!(summary.predictions_failed, 1);
    assert_eq!(summary.trainings_completed, 1);
    assert_eq!(summary.failures.len(), 1);
    assert_eq!(
        summary.why(),
        "predict: Error in inla.inlaprogram.has.crashed() : The inla program crashed."
    );

    // A training failure is the other phase, and the package-loading line
    // that came before the error is not the reason.
    let training = FAILED
        .replace("Prediction job", "Training job")
        .replace("predict script", "train script");
    let summary = parse_summary(&training).expect("a block");
    assert!(
        summary.why().starts_with("train: Error in inla"),
        "{}",
        summary.why()
    );

    // No `[FAILED]` line at all - chapkit counted a failure and said
    // nothing else - still reads as something rather than as nothing.
    let quiet = parse_summary(
        &PASSED
            .replace("Predictions failed:    0", "Predictions failed:    1")
            .replace("Result: ALL TESTS PASSED", "Result: 1 FAILURE(S)"),
    )
    .expect("a block");
    assert!(!quiet.passed);
    assert_eq!(quiet.why(), "1 prediction failed");
}

#[test]
fn a_run_with_no_summary_block_reads_as_nothing_at_all() {
    assert!(parse_summary("").is_none());
    assert!(parse_summary("service unavailable\n").is_none());
    // The heading has to be there; the counts alone are not the block.
    assert!(parse_summary("Trainings completed:   1\n").is_none());
}

#[test]
fn an_image_without_chapkit_is_told_apart_from_a_model_that_failed() {
    // `docker compose exec` with nothing to exec.
    assert!(chapkit_missing(
        1,
        "OCI runtime exec failed: exec failed: unable to start container process: \
             exec: \"chapkit\": executable file not found in $PATH: unknown"
    ));
    // A shell in the container answering for itself.
    assert!(chapkit_missing(127, "sh: 1: chapkit: not found"));
    assert!(chapkit_missing(
        2,
        "Usage: chapkit [OPTIONS] COMMAND\nError: No such command 'test'."
    ));
    // A model whose own error says something was not found is not this.
    assert!(!chapkit_missing(
        1,
        "Error in library(INLA) : there is no package called 'INLA'"
    ));
}

#[test]
fn the_frame_is_transposed_into_one_observation_per_value() {
    let frame = serde_json::json!({
        "columns": ["time_period", "location", "disease_cases", "rainfall"],
        "data": [
            ["2020-01", "location_0", 12.0, 100.5],
            ["2020-01", "location_1", 7.0, 90.0],
            ["2020-02", "location_0", serde_json::Value::Null, 80.25],
        ],
    });
    let (observations, locations) = observations(&frame).expect("a frame");
    assert_eq!(locations, vec!["location_0", "location_1"]);
    // Two feature columns per row, and neither index column becomes one.
    assert_eq!(observations.len(), 6);
    assert_eq!(
        observations[0],
        Observation {
            period: "202001".to_string(),
            org_unit: "location_0".to_string(),
            value: Some(12.0),
            feature_name: "disease_cases".to_string(),
        }
    );
    // A hole in the frame goes over as a known-missing observation.
    assert_eq!(observations[4].value, None);
    assert_eq!(observations[4].feature_name, "disease_cases");
    assert_eq!(observations[4].period, "202002");
    // The wire names are chap-core's, not Rust's.
    let wire = serde_json::to_value(&observations[0]).expect("JSON");
    assert_eq!(wire["orgUnit"], serde_json::json!("location_0"));
    assert_eq!(wire["featureName"], serde_json::json!("disease_cases"));

    // A weekly frame keeps the W chap-core's period code has.
    let weekly = serde_json::json!({
        "columns": ["time_period", "location", "disease_cases"],
        "data": [["2020-W01", "location_0", 1.0], ["2020-W52", "location_0", 2.0]],
    });
    let (weekly, _) = super::observations(&weekly).expect("a frame");
    assert_eq!(weekly[0].period, "2020W01");
    assert_eq!(weekly[1].period, "2020W52");
    assert_eq!(period_code("2020-01"), "202001");
    assert_eq!(period_code(" 2020-W01 "), "2020W01");
}

#[test]
fn a_frame_that_is_not_one_says_which_column_is_missing() {
    let err =
        observations(&serde_json::json!({"columns": [], "data": []})).expect_err("no time_period");
    assert!(err.to_string().contains("`time_period`"), "{err}");
    let err = observations(&serde_json::json!({"data": []})).expect_err("no columns");
    assert!(err.to_string().contains("`columns`"), "{err}");
    let err = observations(&serde_json::json!({"columns": ["time_period", "location"]}))
        .expect_err("no rows");
    assert!(err.to_string().contains("`data`"), "{err}");
}

#[test]
fn every_feature_carries_the_top_level_id_chap_core_matches_on() {
    // chapkit puts the location in `properties.id` and nothing at the top
    // level, and chap-core drops an org unit whose feature has no `id`.
    let geo = serde_json::json!({
        "type": "FeatureCollection",
        "bbox": [0.0, 0.0, 1.0, 1.0],
        "features": [
            {"type": "Feature", "geometry": {"type": "Point", "coordinates": [0.0, 0.0]},
             "properties": {"id": "location_0"}},
            {"type": "Feature", "geometry": null, "properties": {"id": "location_1"}},
        ],
    });
    let locations = vec!["location_0".to_string(), "location_1".to_string()];
    let built = feature_collection(Some(&geo), &locations);
    assert_eq!(built["type"], serde_json::json!("FeatureCollection"));
    assert_eq!(built["features"][0]["id"], serde_json::json!("location_0"));
    assert_eq!(built["features"][1]["id"], serde_json::json!("location_1"));
    // The geometry is chapkit's and is passed through untouched.
    assert_eq!(
        built["features"][0]["geometry"]["type"],
        serde_json::json!("Point")
    );
    assert_eq!(built["bbox"], serde_json::json!([0.0, 0.0, 1.0, 1.0]));

    // A feature that already has one keeps it.
    let with_id = serde_json::json!({
        "type": "FeatureCollection",
        "features": [{"type": "Feature", "id": "kept", "properties": {"id": "other"}}],
    });
    let built = feature_collection(Some(&with_id), &locations);
    assert_eq!(built["features"][0]["id"], serde_json::json!("kept"));

    // A model that needs no geometry gets one feature per org unit with
    // none, which is enough for chap-core to know they exist.
    let built = feature_collection(None, &locations);
    assert_eq!(built["features"].as_array().expect("a list").len(), 2);
    assert_eq!(built["features"][0]["id"], serde_json::json!("location_0"));
    assert_eq!(built["features"][0]["geometry"], serde_json::Value::Null);
    assert_eq!(
        built["features"][1]["properties"]["id"],
        serde_json::json!("location_1")
    );
    // A `geo` with no features in it is no geo at all.
    assert_eq!(
        feature_collection(Some(&serde_json::json!({})), &locations)["features"][0]["geometry"],
        serde_json::Value::Null
    );
}

#[test]
fn a_row_pads_the_model_column_and_nothing_else() {
    let names = vec![
        "chapkit-ewars-model".to_string(),
        "chapkit-rwanda-malaria-bym-model".to_string(),
    ];
    let width = name_width(&names);
    assert_eq!(width, 32);

    let passed = run(
        "chapkit_ewars_model",
        "chapkit-ewars-model",
        Verdict::Pass,
        17,
        "1 training, 1 prediction",
    );
    assert_eq!(
        row(&passed, passed.verdict.label(), width),
        "chapkit-ewars-model                 pass   17s   1 training, 1 prediction"
    );

    let failed = run(
        "chapkit_rwanda_malaria_bym_model",
        "chapkit-rwanda-malaria-bym-model",
        Verdict::Fail,
        12,
        "predict: the inla program crashed",
    );
    assert_eq!(
        row(&failed, failed.verdict.label(), width),
        "chapkit-rwanda-malaria-bym-model    FAIL   12s   predict: the inla program crashed"
    );

    // A minute-long backtest reads in two units, and a row with nothing
    // to say in its last cell does not end in a space.
    let mut slow = run("x", "auto-arima-chapkit", Verdict::Pass, 129, "");
    assert_eq!(
        row(&slow, slow.verdict.label(), width),
        format!(
            "auto-arima-chapkit{}pass   2m 9s",
            " ".repeat(14 + NAME_GAP)
        )
    );
    slow.summary = "crps 20.0  mae 29.0  rmse 34.9".to_string();
    assert!(
        row(&slow, slow.verdict.label(), width).ends_with("2m 9s   crps 20.0  mae 29.0  rmse 34.9")
    );

    // A run that took one digit of seconds is right-aligned into the
    // column, so its summary starts where every other row's does.
    let quick = run(
        "x",
        "chapkit-ewars-model",
        Verdict::Pass,
        6,
        "1 training, 1 prediction",
    );
    assert_eq!(
        row(&quick, quick.verdict.label(), width),
        "chapkit-ewars-model                 pass    6s   1 training, 1 prediction"
    );
    // Only a failure shouts.
    assert_eq!(Verdict::Pass.label(), "pass");
    assert_eq!(Verdict::Fail.label(), "FAIL");
    assert_eq!(Verdict::Skip.label(), "skip");
}

#[test]
fn the_header_says_which_level_is_being_run() {
    assert_eq!(
        header(5, Level::Model),
        "testing 5 models (model level; add --backtest to go through chap-core)"
    );
    assert!(header(1, Level::Model).starts_with("testing 1 model ("));
    assert_eq!(
        header(2, Level::Backtest),
        "testing 2 models (through chap-core: a dataset, a backtest and its scores)"
    );
}

#[test]
fn the_closing_line_counts_what_happened_and_only_a_failure_is_a_failure() {
    let pass = |id: &str| run(id, id, Verdict::Pass, 1, "1 training, 1 prediction");
    let all: Vec<Run> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|id| pass(id))
        .collect();
    assert_eq!(closing(&all), "5 of 5 models pass");
    assert!(!any_failed(&all));

    // One failure names the model, so the command in the line works as
    // typed rather than having to be adapted.
    let mut one_bad = all.clone();
    one_bad[4] = run(
        "ewars",
        "chapkit-ewars-model",
        Verdict::Fail,
        12,
        "predict: boom",
    );
    assert_eq!(
        closing(&one_bad),
        "4 of 5 models pass; run `varde models test ewars -v` for the full output"
    );
    assert!(any_failed(&one_bad));

    // Two failures cannot, so the line says so.
    let mut two_bad = one_bad.clone();
    two_bad[3] = run("ghr", "chapkit-ghr-model", Verdict::Fail, 9, "train: boom");
    assert!(
        closing(&two_bad).contains("varde models test <id> -v"),
        "{}",
        closing(&two_bad)
    );

    // With a skip in it the three buckets are counted instead: "4 of 5"
    // would be read as one model having failed.
    let mut skipped = all.clone();
    skipped[3] = run("ghr", "chapkit-ghr-model", Verdict::Fail, 9, "train: boom");
    skipped[4] = run(
        "arima",
        "auto-arima-chapkit",
        Verdict::Skip,
        0,
        "not running",
    );
    assert_eq!(
        closing(&skipped),
        "3 pass, 1 fail, 1 skipped; run `varde models test ghr -v` for the full output"
    );
    // A skip on its own is not a failure and the run still exits zero.
    let only_skip = vec![run(
        "arima",
        "auto-arima-chapkit",
        Verdict::Skip,
        0,
        "not running",
    )];
    assert_eq!(closing(&only_skip), "0 pass, 1 skipped");
    assert!(!any_failed(&only_skip));
    assert_eq!(closing(&[]), "0 of 0 models pass");
    // The verb agrees with how many passed, not with how many there were.
    let one = run("ewars", "chapkit-ewars-model", Verdict::Pass, 1, "");
    assert_eq!(closing(std::slice::from_ref(&one)), "1 of 1 model passes");
}

#[test]
fn the_json_shape_is_the_documented_one() {
    let mut model = run(
        "chapkit_ewars_model",
        "chapkit-ewars-model",
        Verdict::Pass,
        17,
        "1 training, 1 prediction",
    );
    let value = serde_json::to_value(&model).expect("JSON");
    assert_eq!(value["id"], serde_json::json!("chapkit_ewars_model"));
    assert_eq!(
        value["service_id"],
        serde_json::json!("chapkit-ewars-model")
    );
    assert_eq!(value["level"], serde_json::json!("model"));
    assert_eq!(value["result"], serde_json::json!("pass"));
    assert_eq!(value["seconds"], serde_json::json!(17));
    assert_eq!(
        value["summary"],
        serde_json::json!("1 training, 1 prediction")
    );
    assert_eq!(value["detail"], serde_json::Value::Null);
    // The three that only a backtest has are absent rather than null.
    for absent in ["job_id", "backtest_id", "metrics"] {
        assert!(value.get(absent).is_none(), "{absent} should be left out");
    }

    model.level = Level::Backtest;
    model.job_id = Some("f424cbe3".to_string());
    model.backtest_id = Some(5);
    model.metrics = Some(serde_json::json!({"crps": 4.83, "mae": 6.68}));
    model.detail = Some("run `varde jobs logs f424cbe3`".to_string());
    let value = serde_json::to_value(&model).expect("JSON");
    assert_eq!(value["level"], serde_json::json!("backtest"));
    assert_eq!(value["job_id"], serde_json::json!("f424cbe3"));
    assert_eq!(value["backtest_id"], serde_json::json!(5));
    assert_eq!(value["metrics"]["crps"], serde_json::json!(4.83));
    assert_eq!(
        value["detail"],
        serde_json::json!("run `varde jobs logs f424cbe3`")
    );
}

#[test]
fn the_scores_cell_is_three_of_the_fourteen_chap_core_reports() {
    let metrics = serde_json::json!({
        "ratio_above_truth": 0.45,
        "crps": 20.04,
        "crps_log1p": 0.09,
        "mae": 28.96,
        "mape": 12.98,
        "coverage_10_90": 0.6,
        "rmse": 34.88,
    });
    assert_eq!(metrics_cell(&metrics), "crps 20.0  mae 29.0  rmse 34.9");
    // Only the ones that are there, and something honest when none are.
    assert_eq!(metrics_cell(&serde_json::json!({"mae": 1.0})), "mae 1.0");
    assert_eq!(metrics_cell(&serde_json::json!({})), "no scores reported");
}

/// `(id, name, archived)` as a row of a configured-model listing.
fn configured(id: i64, name: &str, archived: bool) -> ConfiguredModel {
    ConfiguredModel {
        id,
        name: name.to_string(),
        archived,
        covariates: Vec::new(),
        version: None,
        health: None,
    }
}

#[test]
fn the_configured_model_of_a_service_is_its_own_name_then_one_of_its_configs() {
    let bare = configured(15, "chapkit-ewars-model", false);
    let synced = configured(
        19,
        "chapkit-ewars-model:chapkit-ewars-model_179026711",
        false,
    );
    let left_behind = configured(
        12,
        "chapkit-ewars-model:test_config_01M3A4TSAZTTDYS0SJK62R4S5A",
        false,
    );
    let other = configured(16, "chapkit-ghr-model", false);

    // The bare name is the service itself and wins, whatever else is
    // listed and whatever the ids are.
    let all = vec![
        other.clone(),
        left_behind.clone(),
        synced.clone(),
        bare.clone(),
    ];
    assert_eq!(
        configured_model_for(&all, "chapkit-ewars-model"),
        Some(&bare)
    );

    // Without it - a service that re-registered, which is the state this
    // exists for - a config of the service is the answer, and the one
    // `chapkit test` left behind is the last resort.
    let synced_only = vec![other.clone(), left_behind.clone(), synced.clone()];
    assert_eq!(
        configured_model_for(&synced_only, "chapkit-ewars-model"),
        Some(&synced)
    );
    assert_eq!(
        configured_model_for(&[other.clone(), left_behind.clone()], "chapkit-ewars-model"),
        Some(&left_behind)
    );

    // Two configs of the same service: the lowest id, so two runs of the
    // same command backtest the same model.
    let second = configured(
        9,
        "chapkit-ewars-model:chapkit-ewars-model_179026761",
        false,
    );
    assert_eq!(
        configured_model_for(&[synced.clone(), second.clone()], "chapkit-ewars-model"),
        Some(&second)
    );

    // An archived row is a name chap-core keeps and nothing runs, so the
    // config is chosen over it rather than it over the config.
    let retired = configured(3, "chapkit-ewars-model", true);
    assert_eq!(
        configured_model_for(&[retired.clone(), synced.clone()], "chapkit-ewars-model"),
        Some(&synced)
    );
    assert_eq!(
        configured_model_for(&[retired], "chapkit-ewars-model"),
        None
    );

    // Nothing for this service at all, and a prefix that only looks like
    // one: `chapkit-ewars-model-2` is a different service.
    assert_eq!(configured_model_for(&[other], "chapkit-ewars-model"), None);
    assert_eq!(configured_model_for(&[], "chapkit-ewars-model"), None);
    let neighbour = configured(4, "chapkit-ewars-model-2:config", false);
    assert_eq!(
        configured_model_for(&[neighbour], "chapkit-ewars-model"),
        None
    );
}

#[test]
fn a_configured_model_listing_is_read_down_to_the_three_fields_the_choice_needs() {
    let listed = serde_json::json!([
        {"id": 15, "name": "chapkit-ewars-model", "archived": true, "usesChapkit": true,
         "version": "1.0.0", "sourceDigest": "cafe"},
        {"id": 19, "name": "chapkit-ewars-model:cfg", "archived": false,
         "additionalContinuousCovariates": ["rainfall", 7, "mean_temperature"]},
        // No `archived` at all reads as a live row, and a row without an
        // id or a name is not one.
        {"id": 20, "name": "auto-arima-chapkit"},
        {"name": "no id"},
        {"id": 21},
        "not a row",
    ]);
    let models = configured_models(&listed);
    assert_eq!(models.len(), 3);
    assert!(models[0].archived);
    assert_eq!(models[1].id, 19);
    assert_eq!(models[1].name, "chapkit-ewars-model:cfg");
    // The covariates are the names in the list, and a row without one has none.
    assert_eq!(models[1].covariates, vec!["rainfall", "mean_temperature"]);
    assert!(models[0].covariates.is_empty());
    assert!(!models[2].archived);
    // An answer that is not a list at all is no configured model.
    assert!(configured_models(&serde_json::json!({"detail": "Not Found"})).is_empty());

    let chosen = configured_model_for(&models, "chapkit-ewars-model").expect("the config");
    assert_eq!(chosen.id, 19);
}

#[test]
fn a_covariate_the_sample_lacks_takes_over_a_spare_feature_column() {
    let mut frame = serde_json::json!({
        "columns": ["time_period", "location", "disease_cases", "rainfall",
                    "feature_0", "feature_1"],
        "data": [["2020-01", "location_0", 3.0, 1.0, 2.0, 4.0]],
    });
    let wanted: Vec<String> = ["rainfall", "mean_relative_humidity", "soil", "wind"]
        .map(String::from)
        .to_vec();
    let renamed = fill_covariates(&mut frame, &wanted);

    // One already there is left alone, the next two take the spares in
    // order, and one with no spare left stays missing.
    assert_eq!(
        renamed,
        vec![
            (
                "feature_0".to_string(),
                "mean_relative_humidity".to_string()
            ),
            ("feature_1".to_string(), "soil".to_string()),
        ]
    );
    assert_eq!(
        frame["columns"],
        serde_json::json!([
            "time_period",
            "location",
            "disease_cases",
            "rainfall",
            "mean_relative_humidity",
            "soil"
        ])
    );
    // The rows are untouched: a rename, not a new series.
    assert_eq!(frame["data"][0][4], serde_json::json!(2.0));

    // A configuration that asks for a `feature_N` by name keeps it, and a
    // frame with nothing to rename is left as it was.
    let mut frame = serde_json::json!({"columns": ["feature_0", "feature_1"]});
    let wanted = vec!["feature_0".to_string(), "humidity".to_string()];
    assert_eq!(
        fill_covariates(&mut frame, &wanted),
        vec![("feature_1".to_string(), "humidity".to_string())]
    );
    assert!(fill_covariates(&mut serde_json::json!({}), &wanted).is_empty());
}

#[test]
fn a_log_with_no_stderr_section_still_yields_the_line_that_says_why() {
    // What a job that failed inside chap-core leaves: a Python traceback
    // and no model output at all, because the model was never reached.
    let log = "\
2026-09-24 18:00:30,440 [INFO] chap_status: Starting backtest for model '6'
Traceback (most recent call last):
  File \"/app/.venv/lib/python3.13/site-packages/pandas/core/indexes/base.py\", line 6355
    raise KeyError(f\"{not_found} not in index\")
KeyError: \"['mean_relative_humidity'] not in index\"
";
    assert_eq!(
        log_hint(log).as_deref(),
        Some("KeyError: \"['mean_relative_humidity'] not in index\"")
    );
    // A log with nothing in it says nothing rather than something wrong.
    assert_eq!(log_hint(""), None);
    assert_eq!(log_hint("   \n\n").as_deref(), None);
    // The stderr section is still the better answer where there is one,
    // and that is `jobs::stderr_hint`'s job rather than this one's.
    let with_section = format!("{log}--- stderr ---\nError in f() : boom\n");
    assert_eq!(
        crate::jobs::stderr_hint(&with_section).as_deref(),
        Some("Error in f() : boom")
    );
}

#[test]
fn a_reason_is_cut_at_the_first_sentence_and_a_version_number_is_not_one() {
    assert_eq!(
        first_sentence("the inla program crashed. See the log for details."),
        "the inla program crashed"
    );
    // A full stop inside a version is not the end of a sentence.
    assert_eq!(
        first_sentence("chapkit 2.0.0 refused the config"),
        "chapkit 2.0.0 refused the config"
    );
    // Nothing to cut at is the whole line.
    assert_eq!(first_sentence("  boom  "), "boom");
}
