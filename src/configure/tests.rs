use super::*;
use crate::registry::load_embedded;
use crate::registry::model::Configuration;

/// A marketplace entry from the snapshot built into the binary.
fn entry(id: &str) -> Model {
    load_embedded()
        .expect("the embedded snapshot parses")
        .models
        .into_iter()
        .find(|model| model.id == id)
        .expect("the entry is in the snapshot")
}

fn row(name: &str, archived: bool, version: Option<&str>) -> ConfiguredModel {
    ConfiguredModel {
        id: 1,
        name: name.to_string(),
        archived,
        covariates: Vec::new(),
        version: version.map(str::to_string),
        health: None,
    }
}

#[test]
fn every_configuration_of_the_entry_is_one_request() {
    let ewars = entry("chapkit_ewars_model");
    let made = requests(&Source::Marketplace(Box::new(ewars)), 7);
    let names: Vec<&str> = made.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "monthly_climate",
            "monthly_population_only",
            "monthly_region_seasonal"
        ]
    );
    let climate = &made[0];
    assert_eq!(climate.model_template_id, 7);
    // The covariate list is a field of its own, and the horizon is gone.
    assert_eq!(
        climate.additional_continuous_covariates.as_deref(),
        Some(["rainfall".to_string(), "mean_temperature".to_string()].as_slice())
    );
    for key in RESERVED_CONFIG_KEYS {
        assert!(!climate.user_option_values.contains_key(*key), "{key}");
    }
    assert_eq!(
        climate.user_option_values["n_lags"],
        serde_json::json!([3, 3])
    );
    assert_eq!(climate.user_option_values["region_seasonal"], false);
    // An empty list in the config is kept: it is a choice, not a default.
    assert_eq!(
        made[1].additional_continuous_covariates.as_deref(),
        Some([].as_slice())
    );
}

#[test]
fn a_configuration_without_covariates_gets_the_entry_defaults() {
    let mut model = entry("chapkit_simple_multistep_model");
    model.configurations.clear();
    model.configurations.insert(
        "plain".to_string(),
        Configuration {
            description: None,
            config: serde_json::json!({"prediction_periods": 3, "n_samples": 10}),
        },
    );
    let made = requests(&Source::Marketplace(Box::new(model)), 3);
    assert_eq!(made.len(), 1);
    assert_eq!(
        made[0].additional_continuous_covariates.as_deref(),
        Some(
            [
                "rainfall".to_string(),
                "mean_temperature".to_string(),
                "mean_relative_humidity".to_string()
            ]
            .as_slice()
        )
    );
    assert_eq!(
        serde_json::Value::Object(made[0].user_option_values.clone()),
        serde_json::json!({"n_samples": 10})
    );
}

#[test]
fn an_entry_without_configurations_gets_one_default() {
    let mut model = entry("chapkit_ewars_model");
    model.configurations.clear();
    let made = requests(&Source::Marketplace(Box::new(model)), 5);
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].name, DEFAULT_CONFIGURATION);
    assert!(made[0].user_option_values.is_empty());
    assert_eq!(
        made[0].additional_continuous_covariates.as_deref(),
        Some(["rainfall".to_string(), "mean_temperature".to_string()].as_slice())
    );
}

#[test]
fn a_custom_model_gets_one_default_without_covariates() {
    let made = requests(&Source::Custom, 9);
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].name, DEFAULT_CONFIGURATION);
    let body = serde_json::to_value(&made[0]).expect("JSON");
    assert_eq!(
        body,
        serde_json::json!({"name": "default", "model_template_id": 9, "user_option_values": {}})
    );
}

#[test]
fn a_model_with_a_live_configured_model_is_configured() {
    let service = "chapkit-ewars-model";
    // The bare name and a named configuration both count.
    assert!(is_configured(&[row(service, false, None)], service, None));
    assert!(is_configured(
        &[row(
            "chapkit-ewars-model:monthly_climate",
            false,
            Some("1.0.0")
        )],
        service,
        Some("1.0.0")
    ));
    // Nothing at all, or only another model.
    assert!(!is_configured(&[], service, None));
    assert!(!is_configured(
        &[row("chapkit-ewars-model-two", false, None)],
        service,
        None
    ));
}

#[test]
fn archived_rows_test_leftovers_and_old_versions_do_not_count() {
    let service = "chapkit-ewars-model";
    assert!(!is_configured(&[row(service, true, None)], service, None));
    assert!(!is_configured(
        &[row(
            "chapkit-ewars-model:test_config_01M3A4TSB2YHRPFBEFCT4TMX4A",
            false,
            None
        )],
        service,
        None
    ));
    // An update registers a new version, and the old one's rows do not run it.
    assert!(!is_configured(
        &[row(service, false, Some("1.0.0"))],
        service,
        Some("1.1.0")
    ));
    // Without a version on either side the name decides.
    assert!(is_configured(
        &[row(service, false, None)],
        service,
        Some("1.1.0")
    ));
    assert!(is_configured(
        &[row(service, false, Some("1.0.0"))],
        service,
        Some("")
    ));
}

#[test]
fn the_command_says_one_line_for_each_configured_model() {
    let outcomes = vec![
        ModelOutcome {
            id: "chapkit_ewars_model".to_string(),
            service_id: "chapkit-ewars-model".to_string(),
            outcome: Outcome::Created {
                configured: vec![
                    "monthly_climate".to_string(),
                    "monthly_region_seasonal".to_string(),
                ],
            },
        },
        ModelOutcome {
            id: "auto_arima_chapkit".to_string(),
            service_id: "auto-arima-chapkit".to_string(),
            outcome: Outcome::NotRegistered,
        },
    ];
    let mut lines = Report::default();
    command_lines(&outcomes, &mut lines);
    let texts: Vec<&str> = lines.messages().iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "chapkit_ewars_model: created configured model monthly_climate",
            "chapkit_ewars_model: created configured model monthly_region_seasonal",
            "auto_arima_chapkit: not registered with chap-core; run `varde models configure` \
             again once `varde status` shows it registered",
        ]
    );

    // Folded into another command: one line, and nothing for the rest.
    let mut lines = Report::default();
    folded_lines(&outcomes, false, &mut lines);
    let texts: Vec<&str> = lines.messages().iter().map(|m| m.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "created configured models in chap-core for chapkit_ewars_model (monthly_climate, \
          monthly_region_seasonal)"
        ]
    );
}
