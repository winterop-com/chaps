//! Which configured model a backtest is of.

use super::*;

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
        configured_model_for(&all, "chapkit-ewars-model", None),
        Some(&bare)
    );

    // Without it - a service that re-registered, which is the state this
    // exists for - a config of the service is the answer, and the one
    // `chapkit test` left behind is the last resort.
    let synced_only = vec![other.clone(), left_behind.clone(), synced.clone()];
    assert_eq!(
        configured_model_for(&synced_only, "chapkit-ewars-model", None),
        Some(&synced)
    );
    assert_eq!(
        configured_model_for(
            &[other.clone(), left_behind.clone()],
            "chapkit-ewars-model",
            None
        ),
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
        configured_model_for(
            &[synced.clone(), second.clone()],
            "chapkit-ewars-model",
            None
        ),
        Some(&second)
    );

    // An archived row is a name chap-core keeps and nothing runs, so the
    // config is chosen over it rather than it over the config.
    let retired = configured(3, "chapkit-ewars-model", true);
    assert_eq!(
        configured_model_for(
            &[retired.clone(), synced.clone()],
            "chapkit-ewars-model",
            None
        ),
        Some(&synced)
    );
    assert_eq!(
        configured_model_for(&[retired], "chapkit-ewars-model", None),
        None
    );

    // Nothing for this service at all, and a prefix that only looks like
    // one: `chapkit-ewars-model-2` is a different service.
    assert_eq!(
        configured_model_for(&[other], "chapkit-ewars-model", None),
        None
    );
    assert_eq!(configured_model_for(&[], "chapkit-ewars-model", None), None);
    let neighbour = configured(4, "chapkit-ewars-model-2:config", false);
    assert_eq!(
        configured_model_for(&[neighbour], "chapkit-ewars-model", None),
        None
    );
}

/// `configured(..)` of a version.
fn versioned(id: i64, name: &str, version: &str) -> ConfiguredModel {
    ConfiguredModel {
        version: Some(version.to_string()),
        ..configured(id, name, false)
    }
}

/// A service that registers with a new version keeps the configured models
/// of the old one, and chap-core refuses to run those: the backtest takes a
/// configured model of the version it registered with, or none.
#[test]
fn the_configured_model_is_one_of_the_version_the_service_registered_with() {
    let old = versioned(15, "minimalist", "1.0.1");
    let new = versioned(21, "minimalist:fast", "1.0.2");
    let models = [old.clone(), new.clone()];
    assert_eq!(
        configured_model_for(&models, "minimalist", Some("1.0.2")),
        Some(&new)
    );
    assert_eq!(
        configured_model_for(&models, "minimalist", Some("1.0.1")),
        Some(&old)
    );
    assert_eq!(
        configured_model_for(std::slice::from_ref(&old), "minimalist", Some("1.0.2")),
        None
    );
    // No version on either side: the name is the whole answer.
    assert_eq!(
        configured_model_for(std::slice::from_ref(&old), "minimalist", None),
        Some(&old)
    );
    assert_eq!(
        configured_model_for(
            &[configured(3, "minimalist", false)],
            "minimalist",
            Some("9")
        ),
        Some(&configured(3, "minimalist", false))
    );
}

/// `--config NAME` is a variant name, as `varde models configs` shows it,
/// among the live configured models of the registered version.
#[test]
fn a_variant_name_picks_a_live_configured_model_of_the_version() {
    let service = "chapkit-ewars-model";
    let bare = versioned(15, service, "1.0.1");
    let weekly = versioned(19, "chapkit-ewars-model:weekly", "1.0.1");
    let old = versioned(9, "chapkit-ewars-model:monthly", "1.0.0");
    let retired = ConfiguredModel {
        archived: true,
        ..versioned(4, "chapkit-ewars-model:retired", "1.0.1")
    };
    let other = versioned(30, "chapkit-ghr-model:weekly", "1.0.1");
    let models = [bare.clone(), weekly.clone(), old, retired, other];

    assert_eq!(bare.variant(service).as_deref(), Some("default"));
    assert_eq!(weekly.variant(service).as_deref(), Some("weekly"));
    assert_eq!(weekly.variant("chapkit-ewars"), None);

    let pick = |wanted: &str| configured_variant(&models, service, Some("1.0.1"), wanted);
    assert_eq!(pick("weekly"), Ok(&weekly));
    assert_eq!(pick("chapkit-ewars-model:weekly"), Ok(&weekly));
    assert_eq!(pick("default"), Ok(&bare));
    // An old version, an archived row and another model's are not there; the
    // error names the ones there are.
    let there = Err(vec!["default".to_string(), "weekly".to_string()]);
    assert_eq!(pick("monthly"), there);
    assert_eq!(pick("retired"), there);
    assert_eq!(
        configured_variant(&models, service, Some("2.0.0"), "weekly"),
        Err(Vec::new())
    );
}

#[test]
fn the_line_under_a_backtest_row_names_its_configured_model() {
    let used = UsedModel {
        id: Some(19),
        name: "chapkit-ewars-model:weekly".to_string(),
        variant: "weekly".to_string(),
    };
    assert_eq!(used.line(), "configured model: weekly (id 19)");
    let by_name = UsedModel { id: None, ..used };
    assert_eq!(
        by_name.line(),
        "configured model: weekly (by name, as chap-core could not list them)"
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

    let chosen = configured_model_for(&models, "chapkit-ewars-model", None).expect("the config");
    assert_eq!(chosen.id, 19);
}
