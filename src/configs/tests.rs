use super::*;
use crate::registry::load_embedded;
use serde_json::json;

fn template(free: bool) -> Template {
    parse_template(&json!({
        "id": 30,
        "name": "chapkit-ewars-model",
        "version": "1.0.1",
        "displayName": "EWARS",
        "requiredCovariates": ["population"],
        "allowFreeAdditionalContinuousCovariates": free,
        "userOptions": {"n_lags": {"type": "integer", "default": 3}}
    }))
    .expect("a template")
}

fn listing() -> Json {
    json!([
        {"id": 3, "name": "chapkit-ewars-model:monthly_climate", "version": "1.0.1",
         "archived": false, "additionalContinuousCovariates": ["rainfall"],
         "userOptionValues": {"n_lags": [3, 3]}},
        {"id": 1, "name": "chapkit-ewars-model", "version": "1.0.0", "archived": true},
        {"id": 4, "name": "chapkit-ewars-model:test_config_01ABC", "version": "1.0.1"},
        {"id": 5, "name": "chapkit-ewars-model-two:mine"},
        {"id": 6, "name": "other"}
    ])
}

#[test]
fn a_template_is_read_in_either_spelling() {
    let camel = template(true);
    assert_eq!(camel.id, 30);
    assert_eq!(camel.version.as_deref(), Some("1.0.1"));
    assert_eq!(camel.display_name.as_deref(), Some("EWARS"));
    assert_eq!(camel.required_covariates, ["population"]);
    assert!(camel.free_covariates);
    assert_eq!(camel.options.len(), 1);
    let snake = parse_template(&json!({
        "id": 2, "name": "x", "required_covariates": ["a"],
        "allow_free_additional_continuous_covariates": true,
        "user_options": {"k": {"type": "string"}}
    }))
    .expect("a template");
    assert_eq!(snake.required_covariates, ["a"]);
    assert!(snake.free_covariates);
    assert_eq!(snake.options[0].key, "k");
    assert!(parse_template(&json!({"name": "no id"})).is_none());
}

#[test]
fn the_configs_of_a_template_are_found_by_name_and_sorted_by_id() {
    let configs = configs_of(&listing(), "chapkit-ewars-model");
    let variants: Vec<&str> = configs.iter().map(|c| c.variant.as_str()).collect();
    // The bare name is `default`; another template with the same start is
    // not this one's.
    assert_eq!(
        variants,
        ["default", "monthly_climate", "test_config_01ABC"]
    );
    assert!(configs[0].archived);
    assert_eq!(configs[1].covariates, ["rainfall"]);
    assert_eq!(configs[1].values["n_lags"], json!([3, 3]));
    assert!(configs[2].is_test());
}

#[test]
fn the_source_says_who_made_a_config() {
    let entry = load_embedded()
        .expect("snapshot")
        .models
        .into_iter()
        .find(|m| m.id == "chapkit_ewars_model")
        .expect("entry");
    let marketplace = sync::Source::Marketplace(Box::new(entry));
    let configs = configs_of(&listing(), "chapkit-ewars-model");
    assert_eq!(source_label(&configs[1], Some(&marketplace)), "marketplace");
    assert_eq!(source_label(&configs[2], Some(&marketplace)), "models test");
    assert_eq!(source_label(&configs[0], Some(&marketplace)), "by hand");
    assert_eq!(
        source_label(&configs[0], Some(&sync::Source::Custom)),
        "service default"
    );
    assert_eq!(source_label(&configs[1], None), "by hand");
}

fn draft(variant: &str, covariates: &[&str]) -> Draft {
    Draft {
        variant: variant.to_string(),
        values: Map::new(),
        covariates: covariates.iter().map(|c| c.to_string()).collect(),
    }
}

#[test]
fn check_refuses_a_live_name_a_required_covariate_and_a_bad_name() {
    let existing = configs_of(&listing(), "chapkit-ewars-model");
    let free = template(true);
    assert_eq!(
        check(&draft("weekly", &["rainfall"]), &free, &existing, "m"),
        Ok(())
    );
    // An archived name may be used again.
    assert_eq!(check(&draft("default", &[]), &free, &existing, "m"), Ok(()));
    let err = check(&draft("monthly_climate", &[]), &free, &existing, "m").expect_err("live");
    assert_eq!(
        err,
        "m has a configured model monthly_climate already; give another --name, or archive it \
         first with `varde models configs archive m monthly_climate`"
    );
    let err = check(&draft("w", &["population"]), &free, &existing, "m").expect_err("required");
    assert!(
        err.starts_with("`population` is a required covariate of m"),
        "{err}"
    );
    let err = check(&draft("w", &["rain fall"]), &free, &existing, "m").expect_err("name");
    assert!(err.contains("is not a covariate name"), "{err}");
    let err =
        check(&draft("w", &["rainfall"]), &template(false), &existing, "m").expect_err("not free");
    assert_eq!(err, "m takes no additional covariates; remove --covariates");
    assert!(check_variant("").is_err());
    assert!(check_variant("a b").is_err());
    assert!(check_variant("a:b").is_err());
    assert!(check_variant("test_config_x").is_err());
    assert_eq!(check_variant("weekly-2.v1_a"), Ok(()));
}

#[test]
fn covariates_drop_blanks_and_repeats() {
    assert_eq!(
        parse_covariates(" rainfall,,mean_temperature, rainfall "),
        ["rainfall", "mean_temperature"]
    );
    assert!(parse_covariates("").is_empty());
}

#[test]
fn sync_leaves_a_model_alone_while_another_config_of_its_version_is_live() {
    let mut configs = configs_of(&listing(), "chapkit-ewars-model");
    let climate = configs[1].clone();
    // The other rows are an archived one, an old version and a test.
    assert!(!sync_leaves_alone(&configs, &climate));
    configs.push(Config {
        id: 9,
        variant: "mine".to_string(),
        name: "chapkit-ewars-model:mine".to_string(),
        ..climate.clone()
    });
    assert!(sync_leaves_alone(&configs, &climate));
}
