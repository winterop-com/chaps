use super::*;
use crate::registry::load_embedded;
use serde_json::json;

fn entry() -> Model {
    load_embedded()
        .expect("snapshot")
        .models
        .into_iter()
        .find(|m| m.id == "chapkit_simple_multistep_model")
        .expect("entry")
}

fn configs() -> Vec<Config> {
    let rows = json!([
        {"id": 1, "name": "svc:monthly_climate",
         "additionalContinuousCovariates": ["rainfall", "mean_temperature"],
         "userOptionValues": {"n_target_lags": 6, "n_samples": 100}},
        {"id": 2, "name": "svc:deep_trees", "additionalContinuousCovariates": [],
         "userOptionValues": {"rf_max_depth": 20}},
        {"id": 3, "name": "svc"}
    ]);
    super::super::configs_of(&rows, "svc")
}

#[test]
fn an_export_takes_the_horizon_from_the_twin_or_the_service() {
    let entry = entry();
    let (out, from) = configurations(&configs(), "m", Some(&entry), Some(12));
    assert_eq!(from["monthly_climate"], PeriodsFrom::Marketplace);
    assert_eq!(from["deep_trees"], PeriodsFrom::Service);
    let climate = &out["monthly_climate"];
    assert_eq!(climate.config[PERIODS_KEY], json!(3));
    assert_eq!(
        climate.config[COVARIATES_KEY],
        json!(["rainfall", "mean_temperature"])
    );
    // The twin's description; a configured model by hand gets one that
    // names it.
    assert_eq!(
        climate.description,
        entry.configurations["monthly_climate"].description
    );
    let deep = &out["deep_trees"];
    assert_eq!(deep.config[PERIODS_KEY], json!(12));
    assert_eq!(deep.config["rf_max_depth"], json!(20));
    assert_eq!(
        deep.description.as_deref(),
        Some("The configured model deep_trees of m.")
    );
    // The bare name is the configuration `default`.
    assert!(out.contains_key("default"));
    let (_, from) = configurations(&configs(), "m", None, None);
    assert_eq!(from["deep_trees"], PeriodsFrom::Chapkit);
}

#[test]
fn an_export_reads_back_as_a_marketplace_entry_with_the_same_values() {
    let entry = entry();
    let (out, _) = configurations(&configs(), "m", Some(&entry), Some(12));
    let yaml = to_yaml(&out);
    assert!(yaml.starts_with("configurations:\n"), "{yaml}");
    assert!(!yaml.contains('#'), "{yaml}");

    // The registry's own types read it, as they read an entry.
    #[derive(serde::Deserialize)]
    struct Entry {
        configurations: BTreeMap<String, Configuration>,
    }
    let read: Entry = serde_yaml_ng::from_str(&yaml).expect("the marketplace format");
    assert_eq!(read.configurations.len(), out.len());
    for (name, configuration) in &out {
        assert_eq!(
            read.configurations[name].config, configuration.config,
            "{name}"
        );
        assert_eq!(
            read.configurations[name].description,
            configuration.description
        );
    }

    // And back into configured models: the same values and covariates.
    let back = from_yaml(&yaml).expect("parses");
    for config in configs() {
        let draft = draft_of(&config.variant, &back[&config.variant]).expect("a draft");
        assert_eq!(draft.values, config.values, "{}", config.variant);
        assert_eq!(draft.covariates, config.covariates, "{}", config.variant);
    }
}

#[test]
fn a_whole_marketplace_entry_is_a_file_add_reads() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/vendor/marketplace/models/chapkit_simple_multistep_model.yaml"
    ))
    .expect("the vendored entry");
    let read = from_yaml(&text).expect("parses");
    let draft = draft_of("monthly_climate", &read["monthly_climate"]).expect("a draft");
    assert!(draft.values.get(PERIODS_KEY).is_none());
    assert_eq!(draft.values["rf_max_depth"], json!(10));
    assert_eq!(draft.covariates.len(), 3);
    assert!(from_yaml("name: x\n").is_err());
    assert!(from_yaml(": not yaml [").is_err());
}

#[test]
fn values_from_a_file_are_checked_against_the_options() {
    let template = super::super::parse_template(&json!({
        "id": 1, "name": "svc",
        "userOptions": {"n": {"type": "integer"}, "m": {"enum": ["a", "b"]}}
    }))
    .expect("a template");
    let draft = |values: Json| Draft {
        variant: "x".to_string(),
        values: values.as_object().cloned().unwrap_or_default(),
        covariates: Vec::new(),
    };
    assert_eq!(
        check_values(&draft(json!({"n": 2, "m": "b"})), &template, "s"),
        Ok(())
    );
    let err = check_values(&draft(json!({"n": "two"})), &template, "s").expect_err("kind");
    assert_eq!(
        err,
        "x: `two` is not a value of `n`, which takes an integer"
    );
    let err = check_values(&draft(json!({"k": 1})), &template, "s").expect_err("key");
    assert!(err.starts_with("x: `k` is not an option of s"), "{err}");
    assert_eq!(
        schema_periods(&json!({"properties": {"prediction_periods": {"default": 12}}})),
        Some(12)
    );
}
