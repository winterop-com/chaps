use super::{ModelState, ModelStatus, hints, link_strays};

fn row(id: &str, state: ModelState, reach: &str) -> ModelStatus {
    ModelStatus {
        id: id.to_string(),
        state,
        reach: reach.to_string(),
        host_port: None,
        last_ping: None,
        registered_as: None,
        young: false,
        added_from: None,
        unreachable: None,
    }
}

#[test]
fn a_model_registered_under_its_own_id_is_told_to_take_that_id() {
    let mut rows = vec![
        row("my-multistep", ModelState::RunningNotRegistered, "internal"),
        row("other", ModelState::RunningNotRegistered, "internal"),
        row(
            "chapkit-simple-multistep-model",
            ModelState::Unmanaged,
            "http://ee5e63ab53bd:8000",
        ),
        // A host that is not a container id is somebody else's service.
        row(
            "elsewhere",
            ModelState::Unmanaged,
            "http://models.example.org:8000",
        ),
    ];
    let containers = vec![
        (
            "my-multistep".to_string(),
            "ee5e63ab53bd0123456789abcdef".to_string(),
        ),
        (
            "other".to_string(),
            "aaaaaaaaaaaa0123456789abcdef".to_string(),
        ),
    ];
    link_strays(&mut rows, &containers);
    assert_eq!(
        rows[0].registered_as.as_deref(),
        Some("chapkit-simple-multistep-model")
    );
    assert_eq!(rows[1].registered_as, None);

    let said = hints(&rows, false, None);
    assert!(
        said[0].contains("its container registered as `chapkit-simple-multistep-model`"),
        "{said:?}"
    );
    assert!(
        said[0].contains("`--service-id chapkit-simple-multistep-model`"),
        "{said:?}"
    );
    assert!(!said[0].contains("restart"), "{said:?}");
    // The one that is simply not registered keeps the restart hint.
    assert!(said[1].contains("chaps restart --all other"), "{said:?}");
}

use super::*;

const URL: &str = "http://localhost:8000";

const SERVICES: &str = r#"{
      "count": 2,
      "services": [
        {
          "id": "chapkit-ewars-model",
          "url": "http://chapkit-ewars-model:8000",
          "info": {
            "id": "chapkit-ewars-model",
            "display_name": "CHAP-EWARS",
            "version": "1.0.0",
            "author": "someone",
            "unknown_future_field": 42
          },
          "registered_at": "2026-09-22T09:00:00Z",
          "last_ping_at": "2026-09-22T09:04:30Z",
          "expires_at": "2026-09-22T09:09:30Z"
        },
        {
          "id": "auto-arima-chapkit",
          "url": "http://auto-arima-chapkit:8000",
          "info": { "id": "auto-arima-chapkit", "display_name": "Auto ARIMA", "version": "1.2.0" },
          "registered_at": "2026-09-22T09:01:00Z",
          "last_ping_at": "2026-09-22T09:04:31Z",
          "expires_at": "2026-09-22T09:09:31Z"
        }
      ]
    }"#;

#[test]
fn services_payload_is_flattened() {
    let services = parse_services(SERVICES).expect("sample payload parses");
    assert_eq!(services.len(), 2);
    assert_eq!(services[0].id, "chapkit-ewars-model");
    assert_eq!(services[0].display_name, "CHAP-EWARS");
    assert_eq!(services[0].version, "1.0.0");
    assert_eq!(services[0].url, "http://chapkit-ewars-model:8000");
    assert_eq!(services[0].last_ping_at, "2026-09-22T09:04:30Z");
    assert_eq!(services[0].expires_at, "2026-09-22T09:09:30Z");
    assert_eq!(services[1].version, "1.2.0");
}

#[test]
fn services_payload_tolerates_missing_fields() {
    let services = parse_services(r#"{"count":1,"services":[{"id":"x"}]}"#).unwrap();
    assert_eq!(services.len(), 1);
    assert_eq!(services[0].id, "x");
    // Without an info block the id doubles as the display name.
    assert_eq!(services[0].display_name, "x");
    assert!(services[0].version.is_empty());
    assert!(services[0].url.is_empty());
}

#[test]
fn services_payload_falls_back_to_the_info_id() {
    let services = parse_services(r#"{"count":1,"services":[{"info":{"id":"y"}}]}"#).unwrap();
    assert_eq!(services[0].id, "y");
    assert_eq!(services[0].display_name, "y");
}

#[test]
fn an_empty_registry_parses_to_nothing() {
    assert!(
        parse_services(r#"{"count":0,"services":[]}"#)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_services_body_without_the_envelope_is_not_chap_cores() {
    // Well-formed JSON is not enough: the registry is `{count, services}`,
    // and anything else means something else is on the port.
    for body in [
        "<html>nope</html>",
        "{}",
        r#"{"services":[]}"#,
        r#"{"count":0}"#,
        r#"[{"id":"x"}]"#,
        r#"{"detail":"Not Found"}"#,
    ] {
        assert!(parse_services(body).is_err(), "{body}");
    }
}

/// OCS's dataset list, as `GET /datasets?f=json` answers it.
const DATASETS: &str = r#"{
      "kind": "DatasetList",
      "items": [
        { "dataset_id": "worldpop", "dataset_name": "WorldPop", "period_type": "year" },
        { "dataset_id": "chirps3", "dataset_name": "CHIRPS3", "period_type": "day" },
        { "dataset_id": "era5-land", "dataset_name": "ERA5-Land", "period_type": "day",
          "unknown_future_field": 42 }
      ],
      "links": []
    }"#;

#[test]
fn the_dataset_list_is_counted_and_nothing_else_is() {
    assert_eq!(parse_dataset_count(DATASETS), Some(3));
    assert_eq!(
        parse_dataset_count(r#"{"kind":"DatasetList","items":[]}"#),
        Some(0)
    );
    assert_eq!(
        parse_dataset_count(r#"{"kind":"DatasetList","items":[{"dataset_id":"a"}]}"#),
        Some(1)
    );

    // Anything that is not OCS's own envelope is no count at all: the
    // number sits on a line that says the instance is up.
    for body in [
        "",
        "<html>nope</html>",
        "{}",
        r#"{"items":[]}"#,
        r#"{"kind":"Dataset","items":[]}"#,
        r#"{"kind":"DatasetList"}"#,
        r#"{"kind":"DatasetList","items":{}}"#,
        r#"[{"dataset_id":"a"}]"#,
        r#"{"count":2,"services":[]}"#,
    ] {
        assert_eq!(parse_dataset_count(body), None, "{body}");
    }
}

#[test]
fn health_is_up_only_for_chap_cores_own_json() {
    let ApiHealth::Up { status, message } = parse_health(
        URL,
        "application/json",
        r#"{"status":"success","message":"healthy"}"#,
    ) else {
        panic!("chap-core's own body is up");
    };
    assert_eq!(status, "success");
    assert_eq!(message, "healthy");

    // A message chap-core did not send is not a reason to call it down.
    let ApiHealth::Up { status, message } =
        parse_health(URL, "application/json", r#"{"status":"ok"}"#)
    else {
        panic!("expected up");
    };
    assert_eq!(status, "ok");
    assert!(message.is_empty());
}

#[test]
fn an_html_200_is_down_and_names_the_content_type() {
    let ApiHealth::Down { error } = parse_health(
        URL,
        "text/html; charset=utf-8",
        "<!doctype html><html><body>hello</body></html>",
    ) else {
        panic!("HTML is not chap-core");
    };
    assert_eq!(
        error,
        "port 8000 answers but it is not chap-core (got text/html)"
    );
}

#[test]
fn a_non_json_200_without_a_content_type_is_quoted_back() {
    let ApiHealth::Down { error } = parse_health(URL, "", "not json at all") else {
        panic!("plain text is not chap-core");
    };
    assert_eq!(
        error,
        "port 8000 answers but it is not chap-core (got `not json at all`)"
    );

    // A long body is cut, so the line stays one line.
    let ApiHealth::Down { error } = parse_health(URL, "", &"x".repeat(200)) else {
        panic!("expected down");
    };
    assert!(error.ends_with("...`)"), "{error}");
    assert!(error.len() < 120, "{error}");

    // An empty 200 says so rather than quoting nothing.
    let ApiHealth::Down { error } = parse_health(URL, "", "   ") else {
        panic!("expected down");
    };
    assert!(error.contains("an empty body"), "{error}");
}

#[test]
fn json_without_a_status_field_is_down() {
    // The shape a reverse proxy or another API answers with.
    let ApiHealth::Down { error } =
        parse_health(URL, "application/json", r#"{"detail":"Not Found"}"#)
    else {
        panic!("JSON without a status is not chap-core");
    };
    assert!(error.contains("not chap-core"), "{error}");
    assert!(error.contains("without a `status` field"), "{error}");

    // An empty JSON object used to count as up; it no longer does.
    assert!(matches!(
        parse_health(URL, "application/json", "{}"),
        ApiHealth::Down { .. }
    ));
}

#[test]
fn the_endpoint_is_named_by_its_port_when_it_has_one() {
    assert_eq!(
        is_not_chap_core("http://127.0.0.1:54321", "text/html"),
        "port 54321 answers but it is not chap-core (got text/html)"
    );
    // No port to name: the URL itself is the next best thing.
    assert_eq!(
        is_not_chap_core("https://chap.example.test", "text/html"),
        "https://chap.example.test answers but it is not chap-core (got text/html)"
    );
    assert_eq!(port_of("http://localhost:8000/health"), Some(8000));
    assert_eq!(port_of("http://localhost"), None);
}

#[test]
fn a_401_is_about_the_token_and_not_about_who_answered() {
    // A token we sent and chap-core refused.
    let error = token_rejected(URL, SERVICES_PATH, true);
    assert!(
        error.starts_with("port 8000 answers /v2/services with HTTP 401:"),
        "{error}"
    );
    assert!(
        error.contains("the API token in .env is not accepted"),
        "{error}"
    );
    assert!(error.contains("chaps auth show --reveal"), "{error}");
    // Never the other verdict: the port is chap-core's, the token is wrong.
    assert!(!error.contains("not chap-core"), "{error}");

    // A path chap-core leaves open says so as well: the 401 cannot have
    // come from its own middleware.
    assert!(
        !error.contains("is open even when"),
        "/v2/services is not open: {error}"
    );

    // A protected chap-core and a `.env` that has no token for it.
    let error = token_rejected(URL, HEALTH_PATH, false);
    assert!(
        error.contains("it requires an API token and .env sets none"),
        "{error}"
    );
    assert!(error.contains("chaps auth enable"), "{error}");
    assert!(!error.contains("not chap-core"), "{error}");
    assert!(
        error.contains("/health is open even when CHAP_API_TOKEN is set"),
        "{error}"
    );

    // The endpoint is named the same way the other verdicts name it.
    assert!(
        token_rejected("https://chap.example.test", HEALTH_PATH, true)
            .starts_with("https://chap.example.test answers /health")
    );
}

#[test]
fn a_registry_that_is_not_chap_cores_says_what_it_expected() {
    let error = services_are_not_chap_core(URL, "HTTP 404");
    assert_eq!(
        error,
        "port 8000 answers but it is not chap-core: /v2/services returned HTTP 404, \
             not {count, services}"
    );
    assert!(services_are_not_chap_core(URL, "text/html").contains("not chap-core"));
}

#[test]
fn the_version_is_read_wherever_the_info_body_keeps_it() {
    assert_eq!(
        parse_version(r#"{"version":"2.3.1","name":"chap-core"}"#).as_deref(),
        Some("2.3.1")
    );
    assert_eq!(
        parse_version(r#"{"chap_core_version":"v2.3.1"}"#).as_deref(),
        Some("v2.3.1")
    );
    assert_eq!(
        parse_version(r#"{"info":{"version":"2.4.0"}}"#).as_deref(),
        Some("2.4.0")
    );
    for body in [
        "{}",
        "<html>",
        r#"{"version":""}"#,
        r#"{"version":2}"#,
        r#"{"other":{"version":"1"}}"#,
    ] {
        assert_eq!(parse_version(body), None, "{body}");
    }
}

#[test]
fn the_version_label_marks_the_pin() {
    assert_eq!(
        ApiVersion {
            value: "2.3.1".into(),
            pinned: false,
            revision: None,
        }
        .label(),
        "2.3.1"
    );
    assert_eq!(
        ApiVersion {
            value: "v2.3.1".into(),
            pinned: true,
            revision: None,
        }
        .label(),
        "v2.3.1 (pinned)"
    );
    assert!(
        ApiVersion {
            value: String::new(),
            pinned: true,
            revision: None,
        }
        .label()
        .is_empty()
    );
}

#[test]
fn a_moving_tag_says_which_build_it_is_running() {
    let sha = "7bf2a98739f46b57487c8cb05e9ddd29778080e9";
    assert_eq!(
        moving_build_cell("master", Some("cc09e3654ff2"), Some(sha)),
        "master: running cc09e3654ff2, revision 7bf2a98739f4"
    );
    // Either half on its own, and nothing at all when neither could be had.
    assert_eq!(
        moving_build_cell("dev", Some("cc09e3654ff2"), None),
        "dev: running cc09e3654ff2"
    );
    assert_eq!(
        moving_build_cell("dev", None, Some("a1b2c3d")),
        "dev: revision a1b2c3d"
    );
    assert_eq!(moving_build_cell("dev", None, None), "");
    assert_eq!(moving_build_cell("dev", Some(""), Some("")), "");
}

#[test]
fn a_full_commit_is_shortened_and_anything_else_is_not() {
    assert_eq!(
        short_revision("7bf2a98739f46b57487c8cb05e9ddd29778080e9"),
        "7bf2a98739f4"
    );
    assert_eq!(short_revision("a1b2c3d"), "a1b2c3d");
    assert_eq!(short_revision("2.4.0.dev0+g7bf2a98"), "2.4.0.dev0+g7bf2a98");
    assert_eq!(short_revision(""), "");
}

#[test]
fn the_revision_comes_off_the_info_body_when_there_is_one() {
    let body = r#"{"chap_core_version":"2.4.0.dev0","revision":"7bf2a98739f4"}"#;
    assert_eq!(parse_revision(body).as_deref(), Some("7bf2a98739f4"));
    assert_eq!(parse_version(body).as_deref(), Some("2.4.0.dev0"));
    // A build with no GIT_REVISION reports an empty one, which is no answer.
    assert_eq!(parse_revision(r#"{"revision":""}"#), None);
    assert_eq!(parse_revision(r#"{"chap_core_version":"2.3.1"}"#), None);
    assert_eq!(parse_revision("<html>"), None);
}

#[test]
fn missing_is_expected_minus_registered() {
    let registered = parse_services(SERVICES).unwrap();
    let expected = vec![
        "auto-arima-chapkit".to_string(),
        "chapkit-ewars-model".to_string(),
        "chapkit-simple-multistep-model".to_string(),
    ];
    assert_eq!(
        missing_ids(&expected, &registered),
        vec!["chapkit-simple-multistep-model".to_string()]
    );
    assert!(missing_ids(&[], &registered).is_empty());
    assert_eq!(missing_ids(&expected, &[]), expected);
}

/// A set of running compose services, as `docker compose ps` would give.
fn running(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

/// One registered service, pinged `age` seconds before [`NOW`].
fn registered(id: &str, age: u64) -> RegisteredService {
    RegisteredService {
        id: id.to_string(),
        url: format!("http://{id}:8000"),
        display_name: id.to_string(),
        version: "1.0.0".to_string(),
        last_ping_at: crate::backup::timestamp(NOW - age),
        expires_at: crate::backup::timestamp(NOW + 300 - age),
    }
}

/// A fixed "now", so the relative times in these tests do not move.
const NOW: u64 = 1_790_147_400;

/// The models of the sample deployment: one published, two internal.
fn enabled() -> Vec<(String, Option<u16>)> {
    vec![
        ("chapkit-ewars-model".to_string(), Some(5001)),
        ("chapkit-rwanda-malaria-bym-model".to_string(), None),
        ("auto-arima-chapkit".to_string(), None),
    ]
}

#[test]
fn a_row_state_comes_from_registration_and_the_running_set() {
    let rows = model_rows(
        &enabled(),
        &[registered("chapkit-ewars-model", 12)],
        &running(&[
            "chap",
            "chapkit-ewars-model",
            "chapkit-rwanda-malaria-bym-model",
        ]),
        NOW,
    );
    assert_eq!(rows.len(), 3, "one row per enabled model");

    assert_eq!(rows[0].id, "chapkit-ewars-model");
    assert_eq!(rows[0].state, ModelState::Registered);
    assert_eq!(
        rows[0].reach, "http://localhost:5001",
        "`--json` keeps the URL it has always carried"
    );
    assert_eq!(rows[0].host_port, Some(5001), "and the table gets the port");
    assert_eq!(rows[0].last_ping.as_deref(), Some("12s ago"));

    // Its container is up, so the registration is what failed.
    assert_eq!(rows[1].state, ModelState::RunningNotRegistered);
    assert_eq!(rows[1].state.label(), "running, not registered");
    assert_eq!(rows[1].reach, "internal");
    assert_eq!(rows[1].host_port, None);
    assert_eq!(rows[1].last_ping, None);

    // Nothing is running it at all.
    assert_eq!(rows[2].state, ModelState::NotRunning);
    assert_eq!(rows[2].reach, "internal");
}

#[test]
fn a_service_the_project_does_not_enable_is_unmanaged() {
    let rows = model_rows(
        &enabled(),
        &[
            registered("chapkit-ewars-model", 12),
            registered("some-other-service", 3),
        ],
        &running(&["chapkit-ewars-model"]),
        NOW,
    );
    assert_eq!(rows.len(), 4, "the stranger gets a row of its own");
    let stranger = rows.last().unwrap();
    assert_eq!(stranger.id, "some-other-service");
    assert_eq!(stranger.state, ModelState::Unmanaged);
    assert_eq!(stranger.state.label(), "unmanaged");
    // chap-core's own URL is the only handle there is on it.
    assert_eq!(stranger.reach, "http://some-other-service:8000");
    assert_eq!(stranger.last_ping.as_deref(), Some("3s ago"));

    // And it is not counted as one of this deployment's models.
    assert_eq!(closing_line(&rows), "2 of 3 models are not registered.");
}

#[test]
fn the_closing_line_counts_what_the_project_enables() {
    let all_good = model_rows(
        &enabled(),
        &[
            registered("chapkit-ewars-model", 12),
            registered("chapkit-rwanda-malaria-bym-model", 12),
            registered("auto-arima-chapkit", 12),
        ],
        &BTreeSet::new(),
        NOW,
    );
    assert_eq!(closing_line(&all_good), "all 3 models registered");

    let one_missing = model_rows(
        &enabled(),
        &[
            registered("chapkit-ewars-model", 12),
            registered("auto-arima-chapkit", 12),
        ],
        &running(&["chapkit-rwanda-malaria-bym-model"]),
        NOW,
    );
    assert_eq!(
        closing_line(&one_missing),
        "1 of 3 models is not registered."
    );

    // One model, and nothing at all, both read as English.
    let one = model_rows(&enabled()[..1], &[], &BTreeSet::new(), NOW);
    assert_eq!(closing_line(&one), "1 of 1 model is not registered.");
    let one = model_rows(
        &enabled()[..1],
        &[registered("chapkit-ewars-model", 1)],
        &BTreeSet::new(),
        NOW,
    );
    assert_eq!(closing_line(&one), "1 model registered");
    assert_eq!(
        closing_line(&[]),
        "no models enabled; run `chaps models enable ID` to add one"
    );

    // A model run from its checkout is registered and not ours: it is
    // not "no models".
    let host_run = model_rows(&[], &[registered("my-model", 3)], &BTreeSet::new(), NOW);
    assert_eq!(
        closing_line(&host_run),
        "no models enabled here; the unmanaged one above registered from outside this \
             deployment"
    );
}

/// A deployment with OCS enabled, published on `port` when it has one.
fn ocs_project(port: Option<u16>) -> Project {
    let mut state = crate::project::ProjectState::default();
    state.components.ocs.enabled = true;
    state.components.ocs.port = port;
    Project {
        dir: std::path::PathBuf::from("/tmp/chapx"),
        state,
    }
}

/// A deployment with DHIS2 enabled, published on `port` when it has one.
fn dhis2_project(port: Option<u16>) -> Project {
    let mut state = crate::project::ProjectState::default();
    state.components.dhis2.enabled = true;
    state.components.dhis2.port = port;
    Project {
        dir: std::path::PathBuf::from("/tmp/chapx"),
        state,
    }
}

/// Something listening on a component's host port, answering the way that
/// component does and recording what it was asked - so a test can tell "no
/// request was made" from "a request came back empty".
struct StandIn {
    port: u16,
    asked: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl StandIn {
    /// The paths it was asked for, in order.
    fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("the request log").clone()
    }
}

/// A loopback server that logs every path it is asked for and answers each
/// one with whatever `answer` gives back: a status code and a body.
fn stand_in(answer: fn(&str) -> (u16, &'static str)) -> StandIn {
    use std::io::{BufRead, BufReader, Write};
    let listener =
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("a loopback port");
    let port = listener.local_addr().expect("the bound address").port();
    let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = asked.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut request = String::new();
            if BufReader::new(&stream).read_line(&mut request).is_err() {
                continue;
            }
            let path = request
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let (code, body) = answer(&path);
            log.lock().expect("the request log").push(path);
            let reason = if code == 200 { "OK" } else { "Not Found" };
            let _ = write!(
                stream,
                "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    StandIn { port, asked }
}

/// An OCS: `/health` and the dataset list, both 200.
fn stand_in_ocs() -> StandIn {
    stand_in(|path| match path.starts_with("/datasets") {
        true => (200, DATASETS),
        false => (200, r#"{"status":"success","message":"healthy"}"#),
    })
}

/// A DHIS2 whose API layer is alive: `/api/ping` answers, as the container's
/// own healthcheck asks it to.
fn stand_in_dhis2() -> StandIn {
    stand_in(|path| match path == DHIS2_PING_PATH {
        true => (200, "pong"),
        false => (404, r#"{"httpStatusCode":404}"#),
    })
}

/// The falsely healthy DHIS2: Tomcat is up and serving, and every `/api/*`
/// request answers 404 because the Spring context never came up.
fn stand_in_broken_dhis2() -> StandIn {
    stand_in(|_| (404, "<html><body>Not Found</body></html>"))
}

/// A port nothing is listening on: taken to learn a free number, then let
/// go, so a connection to it is refused rather than left hanging.
fn closed_port() -> u16 {
    let listener =
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("a loopback port");
    listener.local_addr().expect("the bound address").port()
}

/// Short, because none of these tests waits for anything: the one request
/// that fails is refused, not timed out.
fn probe_agent() -> ureq::Agent {
    agent(Duration::from_millis(500))
}

/// The row of a deployment that has not been started costs no request at
/// all. Nothing is there to answer, so the only thing a probe could buy is
/// a timeout on every `chaps status` - or an answer from whatever else
/// holds that host port, which is how a stopped OCS came to read as `up`
/// beside another deployment's OCS on the same default 9000.
#[test]
fn a_component_that_is_not_running_is_not_asked_and_still_reports_its_health_url() {
    // Answering, and deliberately not this deployment's.
    let stand_in = stand_in_ocs();
    let rows = component_rows(
        &ocs_project(Some(stand_in.port)),
        &probe_agent(),
        &BTreeSet::new(),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, ComponentState::NotRunning);
    assert!(
        stand_in.asked().is_empty(),
        "no request was worth making: {:?}",
        stand_in.asked()
    );
    // `--json` keeps every field it had: the address is known from the
    // record, so it is reported whether the instance is up or not.
    assert_eq!(
        rows[0].health_url.as_deref(),
        Some(format!("http://localhost:{}/health", stand_in.port).as_str())
    );
    assert_eq!(rows[0].reach, format!("http://localhost:{}", stand_in.port));
    assert!(!rows[0].read_only);
    // Nothing was asked, so there is nothing it holds to report.
    assert_eq!(rows[0].datasets, None);
    assert_eq!(rows[0].data_bytes, None);
}

/// The distinction the probe is there for: a container that is up and
/// answering is `up`, and it is the answer that carries the dataset count.
#[test]
fn a_running_component_is_asked_and_reports_what_it_holds() {
    let stand_in = stand_in_ocs();
    let rows = component_rows(
        &ocs_project(Some(stand_in.port)),
        &probe_agent(),
        &running(&[crate::compose::OCS_SERVICE]),
    );
    assert_eq!(rows[0].state, ComponentState::Up);
    assert_eq!(rows[0].datasets, Some(3));
    assert_eq!(
        stand_in.asked(),
        vec![HEALTH_PATH.to_string(), DATASETS_PATH.to_string()]
    );
    assert_eq!(
        rows[0].health_url.as_deref(),
        Some(format!("http://localhost:{}/health", stand_in.port).as_str())
    );
}

/// The other half of that distinction: the container is up but nothing is
/// answering on its port yet, which is a wait rather than a fault.
#[test]
fn a_running_component_that_does_not_answer_yet_is_starting() {
    let port = closed_port();
    let rows = component_rows(
        &ocs_project(Some(port)),
        &probe_agent(),
        &running(&[crate::compose::OCS_SERVICE]),
    );
    assert_eq!(rows[0].state, ComponentState::Starting);
    // Asked and not answered, so there is still no count to put on the line.
    assert_eq!(rows[0].datasets, None);
    assert_eq!(
        rows[0].health_url.as_deref(),
        Some(format!("http://localhost:{port}/health").as_str())
    );
}

/// An instance that publishes no host port is judged by its container
/// alone, as it always has been: there is no address out here to ask.
#[test]
fn an_instance_with_no_host_port_is_judged_by_its_container() {
    let project = ocs_project(None);
    let rows = component_rows(
        &project,
        &probe_agent(),
        &running(&[crate::compose::OCS_SERVICE]),
    );
    assert_eq!(rows[0].state, ComponentState::Up);
    assert_eq!(rows[0].reach, "internal");
    assert_eq!(rows[0].health_url, None);
    assert_eq!(rows[0].datasets, None);

    let rows = component_rows(&project, &probe_agent(), &BTreeSet::new());
    assert_eq!(rows[0].state, ComponentState::NotRunning);
    assert_eq!(rows[0].health_url, None);
}

/// The DHIS2 row is judged the same way the OCS one is: the container first,
/// then one request - and the request is `/api/ping`, which is the only
/// route an instance this CLI has no login for will answer.
#[test]
fn a_running_dhis2_is_up_only_once_api_ping_answers() {
    let stand_in = stand_in_dhis2();
    let rows = component_rows(
        &dhis2_project(Some(stand_in.port)),
        &probe_agent(),
        &running(&[crate::compose::DHIS2_SERVICE]),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "dhis2");
    assert_eq!(rows[0].state, ComponentState::Up);
    assert_eq!(stand_in.asked(), vec![DHIS2_PING_PATH.to_string()]);
    assert_eq!(
        rows[0].health_url.as_deref(),
        Some(format!("http://localhost:{}/api/ping", stand_in.port).as_str())
    );
    assert_eq!(rows[0].reach, format!("http://localhost:{}", stand_in.port));
    // None of the OCS fields is a DHIS2 fact, and none of them appears.
    assert!(!rows[0].read_only);
    assert_eq!(rows[0].datasets, None);
    assert_eq!(rows[0].data_bytes, None);
}

/// The failure the request exists to catch, and the whole reason the row
/// does not stop at the container: DHIS2 reports itself healthy while every
/// `/api/*` request 404s, which is what a failed Spring context looks like
/// from outside. `up` would be the one wrong answer here.
#[test]
fn a_dhis2_that_serves_pages_but_no_api_is_not_up() {
    let stand_in = stand_in_broken_dhis2();
    let rows = component_rows(
        &dhis2_project(Some(stand_in.port)),
        &probe_agent(),
        &running(&[crate::compose::DHIS2_SERVICE]),
    );
    assert_eq!(rows[0].state, ComponentState::Starting);
    assert_ne!(rows[0].state, ComponentState::Up);
    assert_eq!(stand_in.asked(), vec![DHIS2_PING_PATH.to_string()]);
    // The address is recorded state, so the field says the same thing
    // whatever the instance answered.
    assert_eq!(
        rows[0].health_url.as_deref(),
        Some(format!("http://localhost:{}/api/ping", stand_in.port).as_str())
    );
}

/// A DHIS2 that was never started costs no request, for the reason the OCS
/// row costs none: the only thing a probe could buy is a timeout, or an
/// answer from whatever else holds that host port.
#[test]
fn a_dhis2_that_is_not_running_is_not_asked() {
    let stand_in = stand_in_dhis2();
    let rows = component_rows(
        &dhis2_project(Some(stand_in.port)),
        &probe_agent(),
        &BTreeSet::new(),
    );
    assert_eq!(rows[0].state, ComponentState::NotRunning);
    assert!(
        stand_in.asked().is_empty(),
        "no request was worth making: {:?}",
        stand_in.asked()
    );
    assert_eq!(
        rows[0].health_url.as_deref(),
        Some(format!("http://localhost:{}/api/ping", stand_in.port).as_str())
    );
}

/// An instance behind a reverse proxy publishes no host port, so there is no
/// address out here to ask and the container is the whole answer.
#[test]
fn a_dhis2_with_no_host_port_is_judged_by_its_container() {
    let project = dhis2_project(None);
    let rows = component_rows(
        &project,
        &probe_agent(),
        &running(&[crate::compose::DHIS2_SERVICE]),
    );
    assert_eq!(rows[0].state, ComponentState::Up);
    assert_eq!(rows[0].reach, "internal");
    assert_eq!(rows[0].health_url, None);

    let rows = component_rows(&project, &probe_agent(), &BTreeSet::new());
    assert_eq!(rows[0].state, ComponentState::NotRunning);
    assert_eq!(rows[0].health_url, None);
}

/// The rows come in the order the components are rendered, which is the
/// order the lines are printed in.
#[test]
fn the_component_rows_follow_the_render_order() {
    let mut state = crate::project::ProjectState::default();
    state.components.ocs.enabled = true;
    state.components.ocs.port = None;
    state.components.s3.enabled = true;
    state.components.dhis2.enabled = true;
    state.components.dhis2.port = None;
    let project = Project {
        dir: std::path::PathBuf::from("/tmp/chapx"),
        state,
    };
    let names: Vec<String> = component_rows(&project, &probe_agent(), &BTreeSet::new())
        .into_iter()
        .map(|row| row.name)
        .collect();
    assert_eq!(names, vec!["ocs", "s3", "dhis2"]);
}

/// One component row, for the lines a deployment without chap-core adds up
/// to.
fn component(state: ComponentState) -> ComponentStatus {
    ComponentStatus {
        name: "ocs".to_string(),
        state,
        reach: "http://localhost:9000".to_string(),
        health_url: None,
        read_only: false,
        datasets: None,
        data_bytes: None,
    }
}

/// One report, for the exit-code branches.
fn report(api: ApiHealth, missing: &[&str], components: Vec<ComponentStatus>) -> StatusReport {
    StatusReport {
        project: None,
        api_url: URL.to_string(),
        api_port: 8000,
        api_port_source: ApiPortSource::Project,
        api,
        version: ApiVersion::default(),
        chap_tag: "v2.3.1".to_string(),
        chap_tag_moving: false,
        chap_build: None,
        registered: Vec::new(),
        expected: missing.iter().map(|id| id.to_string()).collect(),
        missing: missing.iter().map(|id| id.to_string()).collect(),
        reach: Default::default(),
        models: Vec::new(),
        unmanaged: Vec::new(),
        auth: false,
        components,
        dhis2_needs_connecting: false,
        chap_core_elsewhere: false,
        unhealthy: Vec::new(),
    }
}

/// `chaps status` is a health gate, so one rule covers every deployment
/// shape: anything this deployment declares and does not have is non-zero.
/// A component that died used to be the exception - the one answer a
/// monitor must never get wrong.
#[test]
fn the_exit_code_is_non_zero_for_anything_the_deployment_is_missing() {
    use ComponentState::{NotRunning, Starting, Up};

    let up = || ApiHealth::Up {
        status: "success".to_string(),
        message: "healthy".to_string(),
    };

    // chap-core, everything registered, no components: the clean run.
    assert!(!exit_failure(&report(up(), &[], Vec::new())));
    // A model that did not register, as before.
    assert!(exit_failure(&report(
        up(),
        &["chapkit-ewars-model"],
        vec![]
    )));
    // An API that is not answering, as before.
    assert!(exit_failure(&report(
        ApiHealth::Down {
            error: "connection refused".to_string()
        },
        &[],
        vec![]
    )));

    // A component that is up changes nothing, on either deployment shape.
    assert!(!exit_failure(&report(up(), &[], vec![component(Up)])));
    assert!(!exit_failure(&report(
        ApiHealth::Off,
        &[],
        vec![component(Up)]
    )));

    // A component that is not up is a failure, on either shape. The
    // chap-core half of this is what used to exit 0: the components were
    // not consulted at all while the API was answering.
    for state in [NotRunning, Starting] {
        assert!(
            exit_failure(&report(up(), &[], vec![component(Up), component(state)])),
            "{state:?} beside a healthy chap-core"
        );
        assert!(
            exit_failure(&report(
                ApiHealth::Off,
                &[],
                vec![component(Up), component(state)]
            )),
            "{state:?} on a deployment without chap-core"
        );
    }

    // A deployment without chap-core and without components: nothing is
    // declared, so nothing is missing.
    assert!(!exit_failure(&report(ApiHealth::Off, &[], Vec::new())));
}

/// A deployment without chap-core has no models to count, so its verdict is
/// its components - and it must never be the models line, which names
/// `chaps models enable`, the one command such a deployment refuses.
/// Without chap-core a model is asked itself: not running, running and
/// silent, or up - and only a running one with a host port is asked.
#[test]
fn standalone_models_are_judged_by_their_own_health() {
    let enabled = vec![
        ("a".to_string(), Some(5001)),
        ("b".to_string(), Some(5002)),
        ("c".to_string(), Some(5003)),
        ("d".to_string(), None),
    ];
    let running: BTreeSet<String> = ["a", "b", "d"].iter().map(|s| s.to_string()).collect();
    let asked = std::cell::RefCell::new(Vec::new());
    let rows = standalone_model_rows(&enabled, &running, &|port| {
        asked.borrow_mut().push(port);
        port == 5001
    });
    let states: Vec<ModelState> = rows.iter().map(|r| r.state).collect();
    assert_eq!(
        states,
        vec![
            ModelState::Up,
            ModelState::RunningNotAnswering,
            ModelState::NotRunning,
            ModelState::Up,
        ]
    );
    assert_eq!(
        *asked.borrow(),
        vec![5001, 5002],
        "a stopped model costs no request"
    );
    assert_eq!(rows[0].reach, "http://localhost:5001");

    let lines = standalone_closing_lines(&rows, &[]);
    assert_eq!(
        lines,
        vec!["1 of 4 models is not running; start them with `chaps up`"]
    );
    let up = standalone_closing_lines(&rows[..1], &[]);
    assert_eq!(up, vec!["1 model up, answering on its own host port"]);
    let silent = standalone_closing_lines(&rows[..2], &[]);
    assert!(silent[0].contains("not answering on /health"), "{silent:?}");
    assert!(silent[0].contains("`chaps logs SERVICE`"), "{silent:?}");
    // Nothing at all falls through to the components' own sentence.
    assert_eq!(standalone_closing_lines(&[], &[]).len(), 1);
}

#[test]
fn the_components_verdict_counts_what_is_not_up_and_never_mentions_models() {
    use ComponentState::{NotRunning, Starting, Up};

    assert_eq!(
        components_closing_line(&[component(Up), component(Up)]),
        "all 2 components are up"
    );
    // One of them is named rather than counted.
    assert_eq!(components_closing_line(&[component(Up)]), "ocs is up");

    // Nothing running at all is the same sentence the "never started"
    // rendering uses, so the two states do not read as different answers.
    assert_eq!(
        components_closing_line(&[component(NotRunning), component(NotRunning)]),
        NOTHING_RUNNING
    );

    assert_eq!(
        components_closing_line(&[component(Up), component(NotRunning)]),
        "1 of 2 components is not running; start it with `chaps up`"
    );
    assert_eq!(
        components_closing_line(&[component(Up), component(NotRunning), component(NotRunning)]),
        "2 of 3 components are not running; start them with `chaps up`"
    );

    // A container that is up but not answering yet is a wait, not a
    // `chaps up`: saying `up` again would recreate nothing.
    let line = components_closing_line(&[component(Up), component(Starting)]);
    assert_eq!(
        line,
        "1 of 2 components is still starting; run `chaps status` again in a moment"
    );

    // The empty deployment names both ways to put something in it.
    assert_eq!(
        components_closing_line(&[]),
        "this deployment has no components and no models; \
             `chaps components enable chap-core` adds CHAP, `chaps models enable ID` a model"
    );

    for rows in [
        vec![component(Up)],
        vec![component(NotRunning)],
        vec![component(Starting)],
    ] {
        let line = components_closing_line(&rows);
        assert!(!line.contains("model"), "{line}");
    }
}

#[test]
fn a_model_that_never_registers_elsewhere_is_told_about_port() {
    let rows = standalone_model_rows(&[("m".to_string(), Some(5001))], &BTreeSet::new(), &|_| {
        true
    });
    assert!(
        external_registration_hints(&rows).is_empty(),
        "not running is another hint"
    );
    let row = ModelStatus {
        state: ModelState::RunningNotRegistered,
        ..rows[0].clone()
    };
    let hints = external_registration_hints(&[row]);
    assert_eq!(hints.len(), 1);
    assert!(hints[0].contains("App never became ready"), "{hints:?}");
    assert!(
        hints[0].contains("`chaps components enable chap-core`"),
        "{hints:?}"
    );
}

#[test]
fn every_problem_row_gets_its_own_hint() {
    let rows = model_rows(
        &enabled(),
        &[registered("chapkit-ewars-model", 12)],
        &running(&["chapkit-rwanda-malaria-bym-model"]),
        NOW,
    );
    assert_eq!(
        hints(&rows, false, None),
        vec![
            "chapkit-rwanda-malaria-bym-model: restart it with \
                 `chaps restart --all chapkit-rwanda-malaria-bym-model`"
                .to_string(),
            "auto-arima-chapkit: start CHAP with `chaps up`, \
                 then `chaps logs auto-arima-chapkit`"
                .to_string(),
        ]
    );

    // Nothing wrong: the one line left is the check `status` cannot make
    // itself, and it does not depend on whether the API is protected.
    let rows = model_rows(
        &enabled()[..1],
        &[registered("chapkit-ewars-model", 12)],
        &BTreeSet::new(),
        NOW,
    );
    assert_eq!(hints(&rows, false, None), vec![TEST_HINT_ONE.to_string()]);
    assert_eq!(hints(&rows, true, None), vec![TEST_HINT_ONE.to_string()]);
    assert!(TEST_HINT.contains("chaps models test --all"));
    assert!(TEST_HINT_ONE.contains("check it can run"));

    // Nothing enabled at all has nothing to test either, and a
    // registration this project does not manage is not a model of ours.
    assert!(hints(&[], false, None).is_empty());
    let stranger = model_rows(
        &[],
        &[registered("some-other-service", 3)],
        &BTreeSet::new(),
        NOW,
    );
    assert!(hints(&stranger, false, None).is_empty());
}

/// A model added by hand that registered under another id gets the two
/// commands that fix it, spelled out with its own id and source.
#[test]
fn a_misnamed_manual_model_is_told_the_exact_commands() {
    let mut rows = model_rows(
        &[("my-model".to_string(), None)],
        &[],
        &running(&["my-model"]),
        NOW,
    );
    rows[0].registered_as = Some("chapkit-minimalist-example-py".to_string());
    rows[0].added_from = Some(("my_model".to_string(), "my-model:dev".to_string()));
    assert_eq!(
        hints(&rows, false, None),
        vec![
            "my-model: its container registered as `chapkit-minimalist-example-py`, the \
                 unmanaged row above; run `chaps models remove my_model`, then `chaps models \
                 add my-model:dev --service-id chapkit-minimalist-example-py`"
                .to_string()
        ]
    );
}

/// A model that started moments ago is waited for, not restarted: the
/// restart would only start its registration over.
#[test]
fn a_model_that_just_started_is_told_to_wait_not_to_restart() {
    let mut rows = model_rows(
        &enabled()[..1],
        &[],
        &running(&["chapkit-ewars-model"]),
        NOW,
    );
    rows[0].young = true;
    let hint = &hints(&rows, true, None)[0];
    assert!(hint.contains("started under two minutes ago"), "{hint}");
    assert!(
        hint.contains("run `chaps status` again in a minute"),
        "{hint}"
    );
    assert!(!hint.contains("restart"), "{hint}");
    assert!(
        external_registration_hints(&rows).is_empty(),
        "a young model gets no `PORT` hint either"
    );
}

#[test]
fn a_protected_deployment_names_the_other_reason_a_model_never_registers() {
    let rows = model_rows(
        &enabled()[..1],
        &[],
        &running(&["chapkit-ewars-model"]),
        NOW,
    );
    let hint = &hints(&rows, true, None)[0];
    assert!(
        hint.starts_with("chapkit-ewars-model: restart it with "),
        "{hint}"
    );
    assert!(
        hint.ends_with(
            "; if its log shows 401, chap-core is missing the registration key: \
                 run `chaps sync`, then `chaps restart`"
        ),
        "{hint}"
    );
    // Without authentication there is no 401 to explain.
    assert!(!hints(&rows, false, None)[0].contains("401"));
}

#[test]
fn relative_times_come_from_the_wire_timestamp() {
    assert_eq!(
        ago(NOW, &crate::backup::timestamp(NOW)).as_deref(),
        Some("0s ago")
    );
    assert_eq!(
        ago(NOW, &crate::backup::timestamp(NOW - 12)).as_deref(),
        Some("12s ago")
    );
    assert_eq!(
        ago(NOW, &crate::backup::timestamp(NOW - 180)).as_deref(),
        Some("3m ago")
    );
    assert_eq!(
        ago(NOW, &crate::backup::timestamp(NOW - 7200)).as_deref(),
        Some("2h ago")
    );
    // A clock that is ahead of ours is not a negative age.
    assert_eq!(
        ago(NOW, &crate::backup::timestamp(NOW + 60)).as_deref(),
        Some("0s ago")
    );
    // Nothing to go on: the table prints a dash instead.
    for text in ["", "-", "soon", "2026-09-22"] {
        assert_eq!(ago(NOW, text), None, "{text:?}");
    }
}

#[test]
fn rfc3339_parsing_round_trips_through_the_formatter() {
    for unix in [0, 1_709_164_800, 1_735_689_599, NOW] {
        let text = crate::backup::timestamp(unix);
        assert_eq!(parse_rfc3339(&text), Some(unix), "{text}");
    }
    // The spellings a JSON API may use.
    assert_eq!(
        parse_rfc3339("2026-09-22T09:04:30.123456Z"),
        parse_rfc3339("2026-09-22T09:04:30Z")
    );
    assert_eq!(
        parse_rfc3339("2026-09-22 09:04:30"),
        parse_rfc3339("2026-09-22T09:04:30Z")
    );
    assert_eq!(
        parse_rfc3339("2026-09-22T11:04:30+02:00"),
        parse_rfc3339("2026-09-22T09:04:30Z")
    );
    assert_eq!(
        parse_rfc3339("2026-09-22T07:04:30-0200"),
        parse_rfc3339("2026-09-22T09:04:30Z")
    );
    for bad in [
        "",
        "not a time",
        "2026-09-22",
        "2026-13-01T00:00:00Z",
        "1969-12-31T23:59:59Z",
    ] {
        assert_eq!(parse_rfc3339(bad), None, "{bad}");
    }
}

#[test]
fn reach_is_the_host_port_or_the_proxy() {
    let project = Project {
        dir: std::path::PathBuf::from("/tmp/chapx"),
        state: crate::project::ProjectState {
            api_port: 8123,
            ..Default::default()
        },
    };
    assert_eq!(
        reach(&project, Some(5001), "chapkit-ewars-model"),
        "http://localhost:5001"
    );
    assert_eq!(
        reach(&project, None, "chapkit-ewars-model"),
        "internal (proxy: http://localhost:8123/v2/services/chapkit-ewars-model/run/)"
    );
}

#[test]
fn report_helpers_describe_the_state() {
    let report = StatusReport {
        project: Some("chapx-1ab2c3".into()),
        api_url: URL.into(),
        api_port: 8000,
        api_port_source: ApiPortSource::Project,
        api: ApiHealth::Down {
            error: "connection refused".into(),
        },
        version: ApiVersion {
            value: "v2.3.1".into(),
            pinned: true,
            revision: None,
        },
        chap_tag: "v2.3.1".into(),
        chap_tag_moving: false,
        chap_build: None,
        registered: Vec::new(),
        expected: vec!["a".into()],
        missing: vec!["a".into()],
        reach: BTreeMap::new(),
        models: Vec::new(),
        unmanaged: Vec::new(),
        auth: false,
        components: Vec::new(),
        dhis2_needs_connecting: false,
        chap_core_elsewhere: false,
        unhealthy: Vec::new(),
    };
    assert!(!report.is_up());
    assert!(!report.api_container_unhealthy());
    assert!(!report.is_complete());

    let report = StatusReport {
        api: ApiHealth::Up {
            status: "success".into(),
            message: String::new(),
        },
        missing: Vec::new(),
        ..report
    };
    assert!(report.is_up());
    assert!(report.is_complete());
}

#[test]
fn a_down_api_serialises_with_its_state_tag() {
    let value = serde_json::to_value(ApiHealth::Down {
        error: "connection refused".into(),
    })
    .unwrap();
    assert_eq!(value["state"], "down");
    assert_eq!(value["error"], "connection refused");
}

#[test]
fn a_row_serialises_with_its_state_as_a_string() {
    let rows = model_rows(
        &enabled()[..1],
        &[registered("chapkit-ewars-model", 12)],
        &BTreeSet::new(),
        NOW,
    );
    let value = serde_json::to_value(&rows).unwrap();
    assert_eq!(value[0]["id"], "chapkit-ewars-model");
    assert_eq!(value[0]["state"], "registered");
    assert_eq!(value[0]["reach"], "http://localhost:5001");
    assert_eq!(value[0]["last_ping"], "12s ago");

    let rows = model_rows(&enabled()[1..2], &[], &running(&["x"]), NOW);
    assert_eq!(
        serde_json::to_value(&rows).unwrap()[0]["state"],
        "not-running"
    );
}

/// Option 10 of the AI page: a model registered as `localhost:5001` with a
/// chap-core that runs in a container. The registration is fine, and every
/// call chap-core makes back to the model is a 502.
#[test]
fn a_registered_model_chap_core_cannot_reach_is_a_problem_with_the_way_out() {
    let registered = [
        registered("chapkit-ewars-model", 5),
        registered("auto-arima-chapkit", 5),
    ];
    let mut rows = model_rows(&enabled(), &registered, &BTreeSet::new(), NOW);
    let asked = std::cell::RefCell::new(Vec::new());
    mark_unreachable(&mut rows, &registered, &|id| {
        asked.borrow_mut().push(id.to_string());
        (id == "chapkit-ewars-model").then(|| "HTTP 502".to_string())
    });
    // Only the registered rows are asked: the others have nothing to proxy to.
    assert_eq!(
        *asked.borrow(),
        vec!["chapkit-ewars-model", "auto-arima-chapkit"]
    );
    assert_eq!(rows[0].state, ModelState::Unreachable);
    assert_eq!(rows[0].state.label(), "registered, unreachable");
    assert!(rows[0].state.is_problem());
    assert_eq!(rows[2].state, ModelState::Registered);

    let elsewhere = hints(&rows, false, Some("http://localhost:8000"));
    assert!(
        elsewhere[0].starts_with(
            "chapkit-ewars-model: chap-core cannot reach it at http://chapkit-ewars-model:8000 \
             (HTTP 502)"
        ),
        "{elsewhere:?}"
    );
    assert!(
        elsewhere[0].contains(
            "`chaps components enable chap-core --url http://localhost:8000 --models-host \
             host.docker.internal`"
        ),
        "{elsewhere:?}"
    );
    let own = hints(&rows, false, None);
    assert!(
        own[0].contains("`chaps logs chapkit-ewars-model`"),
        "{own:?}"
    );
    assert!(!own[0].contains("--models-host"), "{own:?}");
}

#[test]
fn the_closing_line_tells_unreachable_from_not_registered() {
    let mut rows = model_rows(
        &enabled()[..2],
        &[registered("chapkit-ewars-model", 5)],
        &BTreeSet::new(),
        NOW,
    );
    rows[0].state = ModelState::Unreachable;
    assert_eq!(
        closing_line(&rows),
        "1 of 2 models is not registered, and 1 is unreachable from chap-core."
    );
    rows.truncate(1);
    assert_eq!(
        closing_line(&rows),
        "1 of 1 model is registered and unreachable from chap-core."
    );
}
