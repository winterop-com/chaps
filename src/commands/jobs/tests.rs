use super::*;

fn out() -> Out {
    Out::default()
}

fn job(id: &str, status: &str) -> Job {
    Job {
        id: id.to_string(),
        kind: "create_backtest".to_string(),
        name: "eval".to_string(),
        status: status.to_string(),
        start_time: Some("2026-09-24T16:00:00".to_string()),
        end_time: Some("2026-09-24T16:00:30".to_string()),
        result: Some("4".to_string()),
        prediction_setup_id: None,
    }
}

/// Epoch seconds for `2026-09-24T16:01:00Z`.
const NOW: u64 = 1_790_265_660;

#[test]
fn an_empty_list_says_where_a_job_would_come_from() {
    let text = human_list(&[], NOW, &out());
    assert!(text.starts_with("no jobs yet;"), "{text}");
    assert!(text.contains("Modeling App"), "{text}");
    assert!(text.ends_with('\n'));
}

#[test]
fn the_table_is_followed_by_the_line_it_adds_up_to() {
    let jobs = vec![
        job("aaaaaaaa-1111", "SUCCESS"),
        job("bbbbbbbb-2222", "FAILURE"),
    ];
    let text = human_list(&jobs, NOW, &out());
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("ID  "), "{text}");
    assert!(lines[0].contains("DURATION"), "{text}");
    assert!(lines[1].starts_with("aaaaaaaa..."), "{text}");
    assert!(text.contains("2 jobs: 1 done, 1 failed"), "{text}");
    assert!(
        text.contains("run `varde jobs logs bbbbbbbb-2222` to see why"),
        "{text}"
    );
}

#[test]
fn a_clean_list_offers_no_failure_hint() {
    let text = human_list(&[job("aaaaaaaa-1111", "SUCCESS")], NOW, &out());
    assert!(text.contains("1 job: 1 done"), "{text}");
    assert!(!text.contains("logs"), "{text}");
}

#[test]
fn show_prints_every_field_and_ends_on_the_next_step() {
    let job = job("aaaaaaaa-1111", "SUCCESS");
    let text = human_show(&job, "SUCCESS", Some(4), NOW, &out());
    for label in [
        "Job",
        "Type",
        "Name",
        "Status",
        "Started",
        "Ended",
        "Duration",
        "Result",
        "Database result",
    ] {
        assert!(text.contains(label), "{label} is missing:\n{text}");
    }
    assert!(text.contains("1m ago"), "{text}");
    assert!(text.contains("30s"), "{text}");
    assert!(text.contains("row 4 in chap-core's database"), "{text}");

    // An empty field is left out rather than printed as a blank.
    let mut bare = job.clone();
    bare.end_time = None;
    bare.result = None;
    let text = human_show(&bare, "STARTED", None, NOW, &out());
    assert!(!text.contains("Ended"), "{text}");
    assert!(!text.contains("Result"), "{text}");
    assert!(text.contains("still running"), "{text}");
}

#[test]
fn the_next_step_follows_the_status() {
    let job = job("abc", "SUCCESS");
    assert!(next_step(&job, "FAILURE", None).starts_with("run `varde jobs logs abc`"));
    assert!(next_step(&job, "STARTED", None).contains("varde jobs cancel abc"));
    assert!(next_step(&job, "REVOKED", None).contains("was cancelled"));
    assert!(next_step(&job, "SUCCESS", Some(9)).contains("/v1/crud/backtests/9"));
    assert!(next_step(&job, "SUCCESS", None).contains("to see what it did"));

    // The collection follows the job type rather than always being the
    // backtests one.
    let mut dataset = job.clone();
    dataset.kind = "create_dataset".to_string();
    assert!(next_step(&dataset, "SUCCESS", Some(1)).contains("/v1/crud/datasets/1"));
    let mut prediction = job.clone();
    prediction.kind = "create_prediction".to_string();
    assert!(next_step(&prediction, "SUCCESS", Some(2)).contains("/v1/crud/predictions/2"));
    // A job type this CLI has never seen names the row and no path.
    let mut unknown = job.clone();
    unknown.kind = "create_something_new".to_string();
    let text = next_step(&unknown, "SUCCESS", Some(3));
    assert!(text.contains("row 3 in chap-core's database"), "{text}");
    assert!(!text.contains("/v1/crud/"), "{text}");
}

#[test]
fn the_message_is_chap_cores_own_when_it_sent_one() {
    let answer = |body: &str| crate::api::Answer {
        status: 200,
        reason: "OK".to_string(),
        content_type: crate::api::JSON.to_string(),
        body: body.as_bytes().to_vec(),
    };
    assert_eq!(
        message_of(&answer(r#"{"message":"Job cancelled"}"#), "cancelled"),
        "Job cancelled"
    );
    assert_eq!(message_of(&answer("{}"), "cancelled"), "job cancelled");
    assert_eq!(message_of(&answer("not json"), "deleted"), "job deleted");
}

#[test]
fn a_body_that_is_not_a_list_is_named_in_the_error() {
    assert_eq!(kind_of(&serde_json::json!({})), "an object");
    assert_eq!(kind_of(&serde_json::json!("x")), "a string");
    assert_eq!(kind_of(&serde_json::json!(null)), "null");
    assert_eq!(kind_of(&serde_json::json!([])), "a list");
}
