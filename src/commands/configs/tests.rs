use super::*;
use serde_json::json;

fn listed() -> Vec<Listed> {
    let rows = json!([
        {"id": 1, "name": "svc:monthly", "archived": false,
         "additionalContinuousCovariates": ["rainfall", "mean_temperature"],
         "userOptionValues": {"n_lags": 3, "method": "exact"}},
        {"id": 2, "name": "svc", "archived": true}
    ]);
    configs::configs_of(&rows, "svc")
        .into_iter()
        .map(|config| Listed {
            source: if config.archived {
                "by hand"
            } else {
                "marketplace"
            },
            config,
        })
        .collect()
}

#[test]
fn a_row_has_the_name_covariates_options_and_source() {
    let rows = rows(&listed(), false);
    assert_eq!(
        rows[0],
        [
            "monthly",
            "rainfall,mean_temperature",
            "method=exact n_lags=3",
            "marketplace"
        ]
    );
    // Nothing is a dash, not an empty cell.
    assert_eq!(rows[1], ["default", "-", "-", "by hand"]);
    assert_eq!(headers(false), HEADERS);
}

#[test]
fn all_adds_the_archived_column() {
    let rows = rows(&listed(), true);
    assert_eq!(rows[0].last().map(String::as_str), Some("no"));
    assert_eq!(rows[1].last().map(String::as_str), Some("yes"));
    assert_eq!(headers(true).last(), Some(&"ARCHIVED"));
}

#[test]
fn the_list_is_a_table_under_each_model_that_has_configs() {
    let out = crate::output::Out::detect(false, true);
    let models = vec![
        ModelConfigs {
            id: "first".to_string(),
            service_id: "svc".to_string(),
            configs: listed(),
        },
        ModelConfigs {
            id: "empty".to_string(),
            service_id: "none".to_string(),
            configs: Vec::new(),
        },
    ];
    let text = human_list(&models, false, &out);
    assert!(text.starts_with("first\n"), "{text}");
    assert!(text.contains("NAME"), "{text}");
    assert!(!text.contains("empty"), "{text}");
}
