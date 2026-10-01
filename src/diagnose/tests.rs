use super::*;

/// The tail of a chap-core log after it was started against a database
/// volume another deployment of the same name had created.
const CHAP_CORE_LOG: &str = concat!(
    "chap-1  | INFO:     Started server process [1]\n",
    "chap-1  | INFO:     Waiting for application startup.\n",
    "chap-1  |   File \"/usr/local/lib/python3.11/site-packages/sqlalchemy/engine/base.py\", line 145, in __init__\n",
    "chap-1  |     self._dbapi_connection = engine.raw_connection()\n",
    "chap-1  | sqlalchemy.exc.OperationalError: (psycopg2.OperationalError) connection to server at \"postgres\" (192.168.32.2), port 5432 failed: FATAL:  password authentication failed for user \"chap\"\n",
    "chap-1  | (Background on this error at: https://sqlalche.me/e/20/e3q8)\n",
    "chap-1  | ERROR:    Application startup failed. Exiting.\n",
);

#[test]
fn the_why_lines_are_the_last_error_looking_ones() {
    let why = why_lines(CHAP_CORE_LOG);
    assert_eq!(why.len(), 2, "{why:#?}");
    assert!(
        why[0].starts_with("sqlalchemy.exc.OperationalError:"),
        "{why:#?}"
    );
    assert!(
        why[0].contains("password authentication failed for user \"chap\""),
        "the whole line is kept, however long it is: {why:#?}"
    );
    assert_eq!(why[1], "ERROR:    Application startup failed. Exiting.");
    // The traceback frames and the startup chatter are not the failure.
    assert!(
        why.iter()
            .all(|line| !line.contains("Started server process")),
        "{why:#?}"
    );
    // The compose column is dropped: the heading names the service.
    assert!(
        why.iter().all(|line| !line.contains("chap-1  |")),
        "{why:#?}"
    );
    // It is cut for the screen, and only there.
    let printed = block(&[Unhealthy::of("chap", CHAP_CORE_LOG)], &Out::default());
    assert!(printed.contains("password authentica..."), "{printed}");
}

#[test]
fn a_password_failure_maps_to_the_volume_it_came_from() {
    let entry = Unhealthy::of("chap", CHAP_CORE_LOG);
    assert_eq!(entry.service, "chap");
    let hint = entry.hint.expect("a password failure has a hint");
    assert!(
        hint.contains("the database volume holds a different password"),
        "{hint}"
    );
    assert!(hint.contains("chaps down --volumes"), "{hint}");
    assert!(hint.contains("chaps doctor"), "{hint}");

    // The bare line chap-core prints, without the traceback around it.
    let bare = Unhealthy::of(
        "chap",
        "FATAL:  password authentication failed for user \"chap\"\n",
    );
    assert_eq!(bare.why.len(), 1);
    assert!(bare.hint.is_some());
}

#[test]
fn a_refused_database_maps_to_the_database() {
    let logs = concat!(
        "chap-1  | INFO:     Waiting for application startup.\n",
        "chap-1  | psycopg2.OperationalError: connection to server at \"postgres\" \
             (192.168.32.2), port 5432 failed: Connection refused\n",
        "chap-1  | \tIs the server running on that host and accepting TCP/IP connections?\n",
    );
    let entry = Unhealthy::of("chap", logs);
    assert_eq!(
        entry.hint.as_deref(),
        Some("the database or valkey did not come up; run `chaps logs postgres`")
    );
}

#[test]
fn only_composes_own_column_is_stripped_from_a_line() {
    // What `docker compose logs` writes in front of every line.
    assert_eq!(
        why_lines("chap-1  | ERROR: it broke\n"),
        vec!["ERROR: it broke".to_string()]
    );
    // A pipe inside the message is part of the message.
    let piped = "ERROR: running `psql -tAc 'select 1' | grep -q 1` failed\n";
    assert_eq!(why_lines(piped)[0], piped.trim());
    // And so is one in a table a log drew for itself.
    let table = "ERROR: expected | got | diff\n";
    assert_eq!(why_lines(table)[0], table.trim());
}

#[test]
fn a_log_with_nothing_wrong_in_it_says_nothing() {
    let logs = "chap-1  | INFO:     Application startup complete.\n\
                    chap-1  | INFO:     Uvicorn running on http://0.0.0.0:8000\n";
    let entry = Unhealthy::of("chap", logs);
    assert!(entry.is_empty());
    assert!(why_lines("").is_empty());
    assert!(hint_for(&[]).is_none());
    assert!(block(&[entry], &Out::default()).is_empty());
}

#[test]
fn at_most_five_lines_survive_and_repeats_collapse() {
    let repeated = "connection refused\n".repeat(40);
    assert_eq!(why_lines(&repeated).len(), 1, "a retry loop says one thing");

    // A loop that alternates two lines says two things, not forty.
    let alternating =
        "ERROR: could not connect to postgres\nretrying: connection refused\n".repeat(20);
    assert_eq!(why_lines(&alternating).len(), 2, "{alternating}");

    // The documentation link SQLAlchemy prints under every error holds
    // the word "error" and none of the cause.
    let noisy = "OperationalError: password authentication failed for user \"chap\"\n\
                     (Background on this error at: https://sqlalche.me/e/20/e3q8)\n";
    let why = why_lines(noisy);
    assert_eq!(why.len(), 1, "{why:#?}");
    assert!(why[0].starts_with("OperationalError:"), "{why:#?}");

    let many: String = (0..12)
        .map(|n| format!("error number {n}\n"))
        .collect::<String>();
    let why = why_lines(&many);
    assert_eq!(why.len(), MAX_LINES);
    assert_eq!(
        why[0], "error number 7",
        "the newest lines are the ones kept"
    );
    assert_eq!(why[MAX_LINES - 1], "error number 11");

    // A line nobody can read is cut rather than printed in full.
    let long = format!("error {}\n", "x".repeat(400));
    let printed = lines(&[Unhealthy::of("chap", &long)]);
    assert!(printed[1].ends_with("..."), "{printed:#?}");
    assert!(printed[1].chars().count() <= MAX_WIDTH + 2);
}

/// What compose writes to stderr when chap-core will not become healthy.
const UP_STDERR: &str = concat!(
    " Container trap-a-1ab2c3-postgres-1  Healthy\n",
    " Container trap-a-1ab2c3-chap-1  Error\n",
    "dependency failed to start: container trap-a-1ab2c3-chap-1 is unhealthy\n",
);

fn container(service: &str, name: &str, health: &str) -> Container {
    Container {
        service: service.to_string(),
        name: name.to_string(),
        state: "running".to_string(),
        health: health.to_string(),
        ..Container::default()
    }
}

#[test]
fn the_failed_up_names_the_service_behind_the_container() {
    assert!(is_health_failure(UP_STDERR));
    let containers = vec![
        container("postgres", "trap-a-1ab2c3-postgres-1", "healthy"),
        container("chap", "trap-a-1ab2c3-chap-1", "unhealthy"),
    ];
    assert_eq!(
        blamed_services(UP_STDERR, &containers, Some("trap-a-1ab2c3")),
        vec!["chap".to_string()]
    );
    // Without a `ps` to match against, the name is taken apart instead.
    assert_eq!(
        blamed_services(UP_STDERR, &[], Some("trap-a-1ab2c3")),
        vec!["chap".to_string()]
    );
    // And without even the project name.
    assert_eq!(
        blamed_services(UP_STDERR, &[], None),
        vec!["trap-a-1ab2c3-chap".to_string()]
    );
}

#[test]
fn stderr_that_is_not_a_health_failure_is_left_alone() {
    for text in [
        "",
        "Error response from daemon: pull access denied",
        "port is already allocated",
    ] {
        assert!(!is_health_failure(text), "{text:?}");
        assert!(blamed_services(text, &[], None).is_empty(), "{text:?}");
    }
}

#[test]
fn a_container_name_comes_apart_the_way_compose_put_it_together() {
    assert_eq!(
        service_of_container("demo-1ab2c3-chap-1", Some("demo-1ab2c3")),
        "chap"
    );
    // The v1 spelling some deployments still carry.
    assert_eq!(service_of_container("demo_chap_1", Some("demo")), "chap");
    // A service name with a separator of its own survives.
    assert_eq!(
        service_of_container("demo-chapkit-ewars-model-1", Some("demo")),
        "chapkit-ewars-model"
    );
    // A name that does not start with the project is not cut into.
    assert_eq!(service_of_container("chap-1", Some("other")), "chap");
}

#[test]
fn the_block_names_the_service_and_indents_what_it_said() {
    let entry = Unhealthy::of("chap", CHAP_CORE_LOG);
    let text = block(std::slice::from_ref(&entry), &Out::default());
    assert!(text.starts_with("why chap is unhealthy:\n"), "{text}");
    assert!(
        text.contains("  sqlalchemy.exc.OperationalError:"),
        "{text}"
    );
    assert!(
        text.contains("password authentica..."),
        "cut for the screen: {text}"
    );
    assert!(
        text.contains("\n  the database volume holds a different password"),
        "{text}"
    );
    // The same thing as lines, for an error message.
    let lines = lines(std::slice::from_ref(&entry));
    assert_eq!(lines[0], "why chap is unhealthy:");
    assert!(lines.len() >= 3);
    // The headline is the line that names the cause, not just the newest
    // one: what a `doctor` check has room for is the reason.
    assert!(
        headline(std::slice::from_ref(&entry))
            .unwrap()
            .contains("password authentication failed"),
        "{entry:#?}"
    );
    assert!(
        headline(std::slice::from_ref(&entry))
            .unwrap()
            .chars()
            .count()
            <= HEADLINE_WIDTH,
        "a headline fits the one line a doctor check gives it"
    );
    // With nothing recognisable in them, the newest line is the headline.
    let plain = Unhealthy::of("chap", "ERROR: it broke\nERROR: it broke again\n");
    assert_eq!(
        headline(std::slice::from_ref(&plain)).as_deref(),
        Some("ERROR: it broke again")
    );
    assert_eq!(headline(&[]), None);

    // A container whose healthcheck has not failed - because docker keeps
    // resetting it while the container crash-loops - says the other thing.
    let down = Unhealthy::of_down("chap", CHAP_CORE_LOG);
    assert_eq!(down.heading(), "why chap is down:");
    assert!(!down.unhealthy);
    assert_eq!(down.why, entry.why, "the log is read the same way");
    assert!(block(&[down], &Out::default()).starts_with("why chap is down:\n"));
}
