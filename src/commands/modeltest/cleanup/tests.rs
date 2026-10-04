use super::*;
use serde_json::json;

fn held(configs: &[(&str, &str)], artifacts: &[&str]) -> Held {
    Held {
        configs: configs
            .iter()
            .map(|(id, name)| (id.to_string(), name.to_string()))
            .collect(),
        artifacts: artifacts.iter().map(|id| id.to_string()).collect(),
    }
}

#[test]
fn only_what_the_run_made_is_marked_for_deletion() {
    let before = held(&[("c1", "production")], &["a1"]);
    let after = held(
        &[("c1", "production"), ("c2", "test_config_01J")],
        &["a1", "a2", "a3"],
    );

    let made = created(&before, &after);
    assert_eq!(
        made.configs,
        vec![("c2".to_string(), "test_config_01J".to_string())]
    );
    assert_eq!(made.artifacts, vec!["a2", "a3"]);
}

#[test]
fn a_config_is_matched_by_id_and_not_by_name() {
    // A config somebody renamed while the test ran is still theirs.
    let before = held(&[("c1", "production")], &[]);
    let after = held(&[("c1", "renamed"), ("c2", "production")], &[]);

    let made = created(&before, &after);
    assert_eq!(
        made.configs,
        vec![("c2".to_string(), "production".to_string())]
    );
}

#[test]
fn nothing_is_marked_when_nothing_appeared() {
    let before = held(&[("c1", "production")], &["a1", "a2"]);
    // An artifact that went away is not something to delete.
    let after = held(&[("c1", "production")], &["a1"]);

    let made = created(&before, &after);
    assert!(made.configs.is_empty());
    assert!(made.artifacts.is_empty());
}

#[test]
fn a_list_is_read_bare_or_wrapped() {
    let entries = vec![
        ("c1".to_string(), "one".to_string()),
        ("c2".to_string(), String::new()),
    ];
    let list = json!([{"id": "c1", "name": "one"}, {"id": "c2"}]);
    for value in [
        list.clone(),
        json!({"items": list.clone()}),
        json!({"data": list}),
    ] {
        assert_eq!(entries_of(&value), Some(entries.clone()), "{value}");
    }
}

#[test]
fn an_entry_without_an_id_is_skipped_and_a_non_list_is_no_answer() {
    let value = json!([{"name": "no id"}, {"id": 7}, {"id": "c1"}]);
    assert_eq!(
        entries_of(&value),
        Some(vec![("c1".to_string(), String::new())])
    );
    // No answer, not an empty list: an empty list would read as "the service
    // holds nothing" and the diff after it would delete everything it holds.
    assert_eq!(entries_of(&json!({"detail": "Not Found"})), None);
    assert_eq!(entries_of(&json!("text")), None);
}
