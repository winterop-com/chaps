use super::*;
use serde_json::json;

/// User options as chap-core stores them for a chapkit model: the
/// `properties` pydantic writes, one of each shape.
fn schema() -> Json {
    json!({
        "n_lags": {"type": "integer", "default": 6, "description": "Lags of the target."},
        "precision": {"type": "number", "default": 0.5, "title": "Precision"},
        "seasonal": {"type": "boolean", "default": true},
        "method": {"enum": ["fast", "exact"], "type": "string", "default": "fast"},
        "lags": {"type": "array", "items": {"type": "integer"}, "default": [3, 3]},
        "label": {"anyOf": [{"type": "string"}, {"type": "null"}], "default": null},
        "shape": {"$ref": "#/$defs/Shape"},
        "prediction_periods": {"type": "integer", "default": 3}
    })
}

fn option(key: &str) -> UserOption {
    parse_options(&schema())
        .into_iter()
        .find(|option| option.key == key)
        .expect("the option")
}

#[test]
fn every_shape_of_a_pydantic_property_is_read() {
    let options = parse_options(&schema());
    let keys: Vec<&str> = options.iter().map(|o| o.key.as_str()).collect();
    // Sorted, and without the reserved key chap-core handles itself.
    assert_eq!(
        keys,
        [
            "label",
            "lags",
            "method",
            "n_lags",
            "precision",
            "seasonal",
            "shape"
        ]
    );
    assert_eq!(option("n_lags").kind, Kind::Integer);
    assert_eq!(option("n_lags").default, Some(json!(6)));
    assert_eq!(
        option("n_lags").description.as_deref(),
        Some("Lags of the target.")
    );
    // A title stands in for a missing description.
    assert_eq!(
        option("precision").description.as_deref(),
        Some("Precision")
    );
    assert_eq!(
        option("lags").kind,
        Kind::List {
            items: Box::new(Kind::Integer)
        }
    );
    let label = option("label");
    assert_eq!(label.kind, Kind::String);
    assert!(label.nullable);
    assert_eq!(label.kind_name(), "text or null");
    assert_eq!(option("shape").kind, Kind::Any);
    assert_eq!(option("method").kind_name(), "one of fast, exact");
    assert_eq!(option("lags").kind_name(), "list of integers");
}

#[test]
fn a_value_is_parsed_as_the_kind_of_its_option() {
    assert_eq!(parse_value(&option("n_lags"), "4"), Ok(json!(4)));
    assert_eq!(parse_value(&option("precision"), "0.25"), Ok(json!(0.25)));
    assert_eq!(parse_value(&option("seasonal"), "False"), Ok(json!(false)));
    assert_eq!(parse_value(&option("method"), "exact"), Ok(json!("exact")));
    assert_eq!(parse_value(&option("lags"), "1, 2,3"), Ok(json!([1, 2, 3])));
    assert_eq!(parse_value(&option("lags"), "[4,5]"), Ok(json!([4, 5])));
    assert_eq!(parse_value(&option("lags"), ""), Ok(json!([])));
    assert_eq!(parse_value(&option("label"), "null"), Ok(Json::Null));
    assert_eq!(parse_value(&option("label"), "north"), Ok(json!("north")));
    assert_eq!(
        parse_value(&option("shape"), r#"{"a":1}"#),
        Ok(json!({"a": 1}))
    );
    assert_eq!(parse_value(&option("shape"), "wide"), Ok(json!("wide")));
}

#[test]
fn a_bad_value_names_the_kind_and_an_example() {
    assert_eq!(
        parse_value(&option("n_lags"), "4.5"),
        Err("`4.5` is not a value of `n_lags`, which takes an integer, such as `3`".to_string())
    );
    assert_eq!(
        parse_value(&option("seasonal"), "yes"),
        Err(
            "`yes` is not a value of `seasonal`, which takes true or false, such as `true`"
                .to_string()
        )
    );
    assert_eq!(
        parse_value(&option("method"), "slow"),
        Err(
            "`slow` is not a value of `method`, which takes one of fast, exact, such as `fast`"
                .to_string()
        )
    );
    assert_eq!(
        parse_value(&option("lags"), "1,x"),
        Err(
            "`1,x` is not a value of `lags`, which takes a list of integers, such as `3,3`"
                .to_string()
        )
    );
    assert!(parse_value(&option("precision"), "NaN").is_err());
    // null only where the option takes it.
    assert!(parse_value(&option("n_lags"), "null").is_err());
}

#[test]
fn set_is_split_at_the_first_equals_sign() {
    assert_eq!(
        parse_set("label=a=b"),
        Ok(("label".to_string(), "a=b".to_string()))
    );
    assert_eq!(
        parse_set(" n_lags = 3 "),
        Ok(("n_lags".to_string(), "3".to_string()))
    );
    assert!(parse_set("n_lags").is_err());
    assert!(parse_set("=3").is_err());
}

#[test]
fn values_refuse_an_unknown_key_with_the_options_and_a_key_twice() {
    let options = parse_options(&schema());
    let sets = vec![("n_lag".to_string(), "3".to_string())];
    let err = values_of(&options, &sets, "m").expect_err("unknown");
    assert!(
        err.starts_with("`n_lag` is not an option of m; its options are label (text or null), "),
        "{err}"
    );
    assert!(err.contains("n_lags (integer)"), "{err}");
    let sets = vec![
        ("n_lags".to_string(), "3".to_string()),
        ("n_lags".to_string(), "4".to_string()),
    ];
    assert_eq!(
        values_of(&options, &sets, "m"),
        Err("`n_lags` is set twice; give each option once".to_string())
    );
    assert_eq!(
        values_of(&[], &[("a".to_string(), "1".to_string())], "m"),
        Err("`a` is not an option of m, which has no options; remove `--set`".to_string())
    );
    let sets = vec![
        ("n_lags".to_string(), "3".to_string()),
        ("method".to_string(), "exact".to_string()),
    ];
    let values = values_of(&options, &sets, "m").expect("good");
    assert_eq!(
        Json::Object(values),
        json!({"n_lags": 3, "method": "exact"})
    );
}

#[test]
fn the_short_form_cuts_with_dots() {
    let mut values = Map::new();
    values.insert("n_lags".to_string(), json!(3));
    values.insert("lags".to_string(), json!([1, 2]));
    assert_eq!(short(&values, 40), "lags=1,2 n_lags=3");
    assert_eq!(short(&values, 10), "lags=1,...");
}
