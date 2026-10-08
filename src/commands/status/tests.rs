use super::*;
use crate::status::{
    ApiVersion, ComponentStatus, ModelState, ModelStatus, RegisteredService, model_rows,
};

/// A fixed "now", so the ages in these tests do not move.
const NOW: u64 = 1_790_147_400;

/// The rows and the closing lines, with the levels as [`Report::text`] shows
/// them.
fn human(report: &StatusReport, out: &Out) -> String {
    let mut lines = Report::default();
    closing(report, false, &mut lines);
    format!("{}{}", rows(report, out), lines.text())
}

/// [`human`] for a deployment with no containers at all.
fn not_running(report: &StatusReport, out: &Out) -> String {
    let mut lines = Report::default();
    not_running_lines(report, &mut lines);
    format!("{}{}", not_running_rows(report, out), lines.text())
}

fn service(id: &str, version: &str) -> RegisteredService {
    RegisteredService {
        id: id.to_string(),
        url: format!("http://{id}:8000"),
        display_name: id.to_string(),
        version: version.to_string(),
        last_ping_at: crate::backup::timestamp(NOW - 12),
        expires_at: crate::backup::timestamp(NOW + 288),
        git_revision: None,
    }
}

/// A report of a healthy API, with `enabled` as the project's models
/// (service id and host port) and `running` as the containers that are up.
fn up(
    registered: Vec<RegisteredService>,
    enabled: &[(&str, Option<u16>)],
    running: &[&str],
) -> StatusReport {
    let enabled: Vec<(String, Option<u16>)> = enabled
        .iter()
        .map(|(id, port)| (id.to_string(), *port))
        .collect();
    let expected: Vec<String> = enabled.iter().map(|(id, _)| id.clone()).collect();
    let missing = crate::status::missing_ids(&expected, &registered);
    let running: BTreeSet<String> = running.iter().map(|id| id.to_string()).collect();
    let models = model_rows(&enabled, &registered, &running, NOW);
    let unmanaged = models
        .iter()
        .filter(|m| m.state == ModelState::Unmanaged)
        .map(|m| m.id.clone())
        .collect();
    StatusReport {
        project: Some("chapx-1ab2c3".to_string()),
        api_url: "http://localhost:8000".to_string(),
        api_port: 8000,
        api_port_source: crate::project::ApiPortSource::Project,
        api: ApiHealth::Up {
            status: "success".to_string(),
            message: "healthy".to_string(),
        },
        version: ApiVersion {
            value: "2.3.1".to_string(),
            pinned: false,
            revision: None,
        },
        chap_tag: "v2.3.1".to_string(),
        chap_tag_moving: false,
        chap_build: None,
        registered,
        expected,
        missing,
        reach: Default::default(),
        models,
        unmanaged,
        revision_warnings: Vec::new(),
        auth: false,
        components: Vec::new(),
        dhis2_needs_connecting: false,
        chap_core_elsewhere: false,
        api_starting: false,
        dhis2_external: None,
        unhealthy: Vec::new(),
        api_elsewhere: None,
    }
}

/// A chap-core that is not answering is waited for when its container is
/// still starting, and read in its log when nothing else says why.
#[test]
fn a_chap_core_that_is_not_answering_names_the_way_out() {
    let report = up(Vec::new(), &[], &[]);
    let starting = down_message(&report, "io: Connection reset by peer", true, false);
    assert!(
        starting.contains("still starting, so run `varde status` again in a moment"),
        "{starting}"
    );
    let down = down_message(&report, "io: Connection refused", false, false);
    assert!(down.ends_with("; `varde logs chap` says why"), "{down}");
    // A chap that never started has no log to read.
    let absent = down_message(&report, "io: Connection refused", false, true);
    assert!(
        absent.ends_with(
            "; this deployment has no `chap` container yet, so start it with `varde up`"
        ),
        "{absent}"
    );
}

/// A chap-core elsewhere has no `chap` service here, so the way out is the
/// chap-core itself and the URL varde has for it.
#[test]
fn a_chap_core_elsewhere_that_is_not_answering_names_no_service() {
    let mut report = up(Vec::new(), &[], &[]);
    report.chap_core_elsewhere = true;
    let down = down_message(&report, "io: Connection refused", false, false);
    assert!(!down.contains("varde logs"), "{down}");
    assert!(
        down.ends_with(
            "; this deployment does not run it, so start it there, or set another URL with \
             `varde components enable chap-core --url URL`"
        ),
        "{down}"
    );
}

/// The component line carries the two things that are not in the address:
/// where an unpublished instance is actually reached, and whether it
/// refuses every write.
#[test]
fn a_component_line_says_read_only_and_names_the_proxy() {
    use crate::status::{ComponentState, ComponentStatus};

    let mut report = up(Vec::new(), &[], &["chap", "ocs"]);
    report.components = vec![ComponentStatus {
        name: "ocs".to_string(),
        state: ComponentState::Up,
        reach: "internal (proxy: https://ocs.example.org)".to_string(),
        health_url: None,
        read_only: true,
        datasets: None,
        data_bytes: None,
    }];
    let text = human(&report, &Out::default());
    assert_eq!(
        text,
        "chap-core   up   http://localhost:8000   2.3.1   auth: off\n\
             ocs         up   internal (proxy: https://ocs.example.org)   read-only\n\
             \n\
             no models enabled; run `varde models enable ID` to add one\n"
    );

    // A writable instance says nothing, rather than `read-write`: the
    // default is not news.
    report.components[0].read_only = false;
    report.components[0].reach = "http://localhost:9000".to_string();
    let text = human(&report, &Out::default());
    assert!(
        text.contains("ocs         up   http://localhost:9000\n"),
        "{text}"
    );
    assert!(!text.contains("read-only"), "{text}");
}

/// The `dhis2` row a `varde dhis2 connect` has never been recorded for
/// carries the hint under the verdict - but only while the row says `up`,
/// because a DHIS2 that is not answering cannot be connected to anything.
#[test]
fn the_verdict_names_the_connect_a_running_dhis2_still_needs() {
    use crate::status::{ComponentState, ComponentStatus};

    let dhis2 = |state| ComponentStatus {
        name: "dhis2".to_string(),
        state,
        reach: "http://localhost:8080".to_string(),
        health_url: Some("http://localhost:8080/api/ping".to_string()),
        read_only: false,
        datasets: None,
        data_bytes: None,
    };
    let mut report = up(Vec::new(), &[], &["chap", "dhis2"]);
    report.dhis2_needs_connecting = true;
    report.components = vec![dhis2(ComponentState::Up)];

    let text = human(&report, &Out::default());
    assert_eq!(
        text,
        "chap-core   up   http://localhost:8000   2.3.1   auth: off\n\
             dhis2       up   http://localhost:8080\n\
             \n\
             no models enabled; run `varde models enable ID` to add one\n\
             varde has not connected this DHIS2 to Chap; run `varde dhis2 connect`\n"
    );

    // Still starting is still not answering, which is the wait the command
    // would sit in; the line waits for the run where it can be acted on.
    report.components = vec![dhis2(ComponentState::Starting)];
    let starting = human(&report, &Out::default());
    assert!(!starting.contains("varde dhis2 connect"), "{starting}");

    report.components = vec![dhis2(ComponentState::NotRunning)];
    let down = human(&report, &Out::default());
    assert!(!down.contains("varde dhis2 connect"), "{down}");

    // And a deployment that has been through a connect is never asked
    // again, however the row reads.
    report.dhis2_needs_connecting = false;
    report.components = vec![dhis2(ComponentState::Up)];
    let recorded = human(&report, &Out::default());
    assert!(!recorded.contains("varde dhis2 connect"), "{recorded}");
}

/// What OCS holds goes on its line when it could be had, and nothing takes
/// its place when it could not.
#[test]
fn the_ocs_line_carries_the_dataset_count_and_the_data_size_when_there_are_any() {
    use crate::status::{ComponentState, ComponentStatus};

    let mut report = up(Vec::new(), &[], &["ocs"]);
    report.components = vec![ComponentStatus {
        name: "ocs".to_string(),
        state: ComponentState::Up,
        reach: "http://localhost:9000".to_string(),
        health_url: Some("http://localhost:9000/health".to_string()),
        read_only: false,
        datasets: Some(3),
        data_bytes: Some(212 * 1024 * 1024),
    }];
    let text = human(&report, &Out::default());
    assert!(
        text.contains("ocs         up   http://localhost:9000   3 datasets   212.0 MB data\n"),
        "{text}"
    );

    // One dataset is singular, and `read-only` stays last on the line.
    report.components[0].datasets = Some(1);
    report.components[0].read_only = true;
    let text = human(&report, &Out::default());
    assert!(
        text.contains(
            "ocs         up   http://localhost:9000   1 dataset   212.0 MB data   read-only\n"
        ),
        "{text}"
    );

    // Neither fact is guaranteed: an instance that did not answer, and a
    // docker that could not be asked, each cost one cell and nothing else.
    report.components[0].datasets = None;
    report.components[0].read_only = false;
    let text = human(&report, &Out::default());
    assert!(
        text.contains("ocs         up   http://localhost:9000   212.0 MB data\n"),
        "{text}"
    );
    report.components[0].data_bytes = None;
    let text = human(&report, &Out::default());
    assert!(
        text.contains("ocs         up   http://localhost:9000\n"),
        "{text}"
    );
    assert!(!text.contains("dataset"), "{text}");
}

#[test]
fn an_external_dhis2_is_named_with_the_command_that_asks_it() {
    let mut report = up(
        vec![service("chapkit-ewars-model", "1.0.0")],
        &[("chapkit-ewars-model", Some(5001))],
        &["chap", "chapkit-ewars-model"],
    );
    report.dhis2_external = Some("https://dhis2.example.org".to_string());
    let text = human(&report, &Out::default());
    assert!(
        text.contains(
            "\ndhis2       elsewhere   https://dhis2.example.org   `varde dhis2 show` asks it\n"
        ),
        "{text}"
    );
}

#[test]
fn a_healthy_report_is_a_line_a_table_and_a_verdict() {
    let report = up(
        vec![service("chapkit-ewars-model", "1.0.0")],
        &[("chapkit-ewars-model", Some(5001))],
        &["chap", "chapkit-ewars-model"],
    );
    let text = human(&report, &Out::default());
    assert_eq!(
        text,
        "chap-core   up   http://localhost:8000   2.3.1   auth: off\n\
             \n\
             MODEL                STATE       REACH      LAST PING\n\
             chapkit-ewars-model  registered  port 5001  12s ago\n\
             \n\
             1 model registered\n\
             hint: run `varde models test --all` to check it can run\n"
    );
}

#[test]
fn the_chap_core_line_ends_in_whether_the_api_is_protected() {
    let mut report = up(
        vec![service("chapkit-ewars-model", "1.0.0")],
        &[("chapkit-ewars-model", Some(5001))],
        &["chap", "chapkit-ewars-model"],
    );
    assert!(
        human(&report, &Out::default())
            .starts_with("chap-core   up   http://localhost:8000   2.3.1   auth: off\n")
    );

    report.auth = true;
    assert!(
        human(&report, &Out::default())
            .starts_with("chap-core   up   http://localhost:8000   2.3.1   auth: on\n")
    );

    // A chap-core that publishes no version of its own still gets the
    // cell, and it stays last on the line.
    report.version = ApiVersion {
        value: String::new(),
        pinned: true,
        revision: None,
    };
    assert!(
        human(&report, &Out::default())
            .starts_with("chap-core   up   http://localhost:8000   auth: on\n")
    );
}

#[test]
fn the_layout_names_every_state_once_and_hints_once_per_problem() {
    let report = up(
        vec![
            service("chapkit-ewars-model", "1.0.0"),
            service("some-other-service", "0.1.0"),
        ],
        &[
            ("chapkit-ewars-model", Some(5001)),
            ("chapkit-rwanda-malaria-bym-model", None),
            ("auto-arima-chapkit", None),
        ],
        &[
            "chap",
            "chapkit-ewars-model",
            "chapkit-rwanda-malaria-bym-model",
        ],
    );
    let text = human(&report, &Out::default());
    assert_eq!(
        text,
        "chap-core   up   http://localhost:8000   2.3.1   auth: off\n\
             \n\
             MODEL                             STATE                    REACH                           LAST PING\n\
             chapkit-ewars-model               registered               port 5001                       12s ago\n\
             chapkit-rwanda-malaria-bym-model  running, not registered  via chap-core                   -\n\
             auto-arima-chapkit                not running              via chap-core                   -\n\
             some-other-service                unmanaged                http://some-other-service:8000  12s ago\n\
             \n\
             2 of 3 models are not registered.\n\
             chapkit-rwanda-malaria-bym-model: restart it with \
             `varde restart --all chapkit-rwanda-malaria-bym-model`\n\
             auto-arima-chapkit: start Chap with `varde up`, \
             then `varde logs auto-arima-chapkit`\n\
             hint: models without a host port are reachable through chap-core at \
             http://localhost:8000/v2/services/<id>/run/\n"
    );
    // The proxy URL appears once, not once per internal row, and the
    // verdict carries no `error:` line of its own.
    assert_eq!(text.matches("/v2/services/<id>/run/").count(), 1);
    assert!(!text.contains("error"), "{text}");
    assert!(!text.contains("missing:"), "{text}");
}

#[test]
fn a_published_model_shows_its_host_port_and_no_proxy_line() {
    let report = up(
        vec![service("chapkit-ewars-model", "1.0.0")],
        &[("chapkit-ewars-model", Some(5001))],
        &[],
    );
    let text = human(&report, &Out::default());
    assert!(text.contains("port 5001"), "{text}");
    assert!(
        !text.contains("reachable through chap-core"),
        "every model here has a port of its own:\n{text}"
    );
    // The URL is still what `--json` carries, untouched by the column.
    assert_eq!(report.models[0].reach, "http://localhost:5001");
}

#[test]
fn a_project_with_no_models_still_says_something() {
    let report = up(vec![], &[], &[]);
    let text = human(&report, &Out::default());
    assert!(text.starts_with("chap-core   up   http://localhost:8000"));
    assert!(!text.contains("MODEL"), "no table for no rows:\n{text}");
    assert!(text.ends_with("no models enabled; run `varde models enable ID` to add one\n"));
}

#[test]
fn a_down_api_prints_the_state_and_leaves_the_verdict_to_the_error_line() {
    let report = StatusReport {
        api: ApiHealth::Down {
            error: "port 8000 answers but it is not chap-core (got text/html)".to_string(),
        },
        version: ApiVersion {
            value: "v2.3.1".to_string(),
            pinned: true,
            revision: None,
        },
        ..up(vec![], &[("chapkit-ewars-model", None)], &[])
    };
    let text = human(&report, &Out::default());
    assert_eq!(
        text,
        "chap-core   down   http://localhost:8000   v2.3.1 (pinned)   auth: off\n\
             \n\
             MODEL                STATE        REACH          LAST PING\n\
             chapkit-ewars-model  not running  via chap-core  -\n"
    );
    // The reason is the error line `run` returns, printed once.
    assert!(!text.contains("not chap-core"));
    assert!(!text.contains("registered"));
}

#[test]
fn the_never_started_line_replaces_the_whole_report() {
    assert_eq!(NOT_RUNNING, "Chap is not running; start it with `varde up`");
}

/// One component row.
fn component(name: &str, state: crate::status::ComponentState, reach: &str) -> ComponentStatus {
    ComponentStatus {
        name: name.to_string(),
        state,
        reach: reach.to_string(),
        health_url: None,
        read_only: false,
        datasets: None,
        data_bytes: None,
    }
}

/// A report for a deployment chap-core is not a component of.
fn without_chap_core(components: Vec<ComponentStatus>) -> StatusReport {
    StatusReport {
        api: ApiHealth::Off,
        components,
        ..up(Vec::new(), &[], &[])
    }
}

/// The one command whose job is to say what is up must not name a product
/// this deployment does not contain, and must not go silent about the
/// components it does.
#[test]
fn a_deployment_without_chap_core_is_never_told_that_chap_is_not_running() {
    use crate::status::ComponentState::NotRunning;

    let report = without_chap_core(vec![
        component("ocs", NotRunning, "http://localhost:9000"),
        component("s3", NotRunning, "internal"),
    ]);
    let line = nothing_running_line(&report);
    assert_eq!(line, crate::status::NOTHING_RUNNING);
    assert!(!line.contains("Chap"), "{line}");
    assert!(line.contains("`varde up`"), "{line}");

    // The rows are recorded state, not something docker had to answer, so
    // they are printed even with nothing up: which components this
    // deployment is made of, and where each of them will answer.
    assert_eq!(
        not_running(&report, &Out::default()),
        "ocs   not running   http://localhost:9000\n\
             s3    not running   internal\n\
             \n\
             nothing in this deployment is running; start it with `varde up`\n"
    );

    // chap-core in the set: the one line is the whole answer again, and it
    // names Chap because there is one to name.
    let bare = up(Vec::new(), &[], &[]);
    assert_eq!(nothing_running_line(&bare), NOT_RUNNING);
    assert_eq!(
        not_running(&bare, &Out::default()),
        format!("{NOT_RUNNING}\n")
    );
}

/// Another deployment on the same port: its line is the whole answer, without
/// a `start it with varde up` line that a held port makes fail.
#[test]
fn a_port_held_elsewhere_replaces_the_start_line() {
    let why = crate::status::answered_elsewhere("http://localhost:8700", 8700, None);
    let report = StatusReport {
        api: ApiHealth::Down { error: why.clone() },
        api_elsewhere: Some(why.clone()),
        ..up(Vec::new(), &[], &[])
    };
    let text = not_running(&report, &Out::default());
    assert!(!text.contains(NOT_RUNNING), "{text}");
    assert!(text.ends_with(&format!("{why}\n")), "{text}");
}

/// A deployment that has components and chap-core shows both when nothing
/// is running: the rows say what it is made of, which the one line cannot.
#[test]
fn nothing_running_still_lists_the_components_a_deployment_has() {
    use crate::status::ComponentState::NotRunning;

    let report = StatusReport {
        api: ApiHealth::Down {
            error: "connection refused".to_string(),
        },
        components: vec![component("ocs", NotRunning, "http://localhost:9000")],
        ..up(Vec::new(), &[], &[])
    };
    let text = not_running(&report, &Out::default());
    // The STATE column is as wide as its widest cell, so the addresses
    // line up under each other.
    assert!(
        text.starts_with("chap-core   down          http://localhost:8000"),
        "{text}"
    );
    assert!(
        text.contains("ocs         not running   http://localhost:9000\n"),
        "{text}"
    );
    assert!(text.ends_with(&format!("{NOT_RUNNING}\n")), "{text}");
    // The models are left out: nothing can have registered with a
    // chap-core that has never started.
    assert!(!text.contains("MODEL"), "{text}");
}

/// The verdict of a deployment without chap-core is its components. Without
/// it this is the one deployment shape `varde status` ended on nothing, and
/// `closing_line` would name `varde models enable`, which it refuses.
#[test]
fn a_components_only_report_ends_on_a_verdict_about_its_components() {
    use crate::status::ComponentState::{NotRunning, Up};

    let mut report = without_chap_core(vec![
        component("ocs", Up, "http://localhost:9000"),
        component("s3", Up, "internal"),
    ]);
    let text = human(&report, &Out::default());
    assert_eq!(
        text,
        "ocs   up   http://localhost:9000\n\
             s3    up   internal\n\
             \n\
             both components are up\n\
             hint: `varde open ocs` opens it\n"
    );
    assert!(
        !text.contains("chap-core"),
        "no line for a component this deployment does not have:\n{text}"
    );
    assert!(!text.contains("model"), "{text}");

    report.components[1].state = NotRunning;
    let text = human(&report, &Out::default());
    assert!(
        text.ends_with("1 of 2 components is not running; start it with `varde up`\n"),
        "{text}"
    );
}

/// A deployment of models alone has no service lines, so the model table is
/// the first line: no blank line comes before it.
#[test]
fn a_models_only_report_starts_on_the_table() {
    let running: BTreeSet<String> = ["chapkit-ewars-model".to_string()].into();
    let report = StatusReport {
        models: crate::status::standalone_model_rows(
            &[("chapkit-ewars-model".to_string(), Some(5001))],
            &running,
            &|_| true,
        ),
        ..without_chap_core(Vec::new())
    };
    let text = human(&report, &Out::default());
    assert!(text.starts_with("MODEL "), "{text:?}");
}

#[test]
fn a_row_with_nothing_in_a_cell_prints_a_dash() {
    let mut report = up(
        vec![service("x", "")],
        &[("chapkit-ewars-model", None)],
        &[],
    );
    // An unmanaged service chap-core has no URL for.
    report.models.push(ModelStatus {
        id: "y".to_string(),
        state: ModelState::Unmanaged,
        reach: String::new(),
        host_port: None,
        last_ping: None,
        registered_as: None,
        young: false,
        added_from: None,
        unreachable: None,
    });
    let text = human(&report, &Out::default());
    let row = text
        .lines()
        .find(|l| l.starts_with("y "))
        .expect("the row is there");
    assert!(row.contains('-'), "empty cells become dashes: {row}");
}

/// With a chap-core elsewhere, a model that does not register gets a fix at
/// info level and what its log lines mean as a hint.
#[test]
fn an_external_chap_core_names_the_log_and_hints_what_it_means() {
    let report = up(
        vec![],
        &[("chapkit-ewars-model", Some(5001))],
        &["chap", "chapkit-ewars-model"],
    );
    let mut lines = Report::default();
    closing(&report, true, &mut lines);
    let text = lines.text();
    assert!(
        text.contains(
            "\nchapkit-ewars-model: if `varde logs chapkit-ewars-model` shows `App never became \
             ready`, enable it again with a network: `varde models enable chapkit-ewars-model`"
        ),
        "{text}"
    );
    assert!(
        text.ends_with(&format!(
            "hint: {}\n",
            crate::status::EXTERNAL_REGISTRATION_LOG
        )),
        "{text}"
    );

    // Its own chap-core has no such lines.
    let mut lines = Report::default();
    closing(&report, false, &mut lines);
    assert!(!lines.text().contains("App never became ready"));
}

/// The read-only mark comes from the file, so a file newer than its running
/// container is named, with the command that applies it.
#[test]
fn an_instance_config_newer_than_its_container_is_named() {
    assert_eq!(
        edited_config_line("ocs"),
        "`ocs/climate-service.yaml` changed after the ocs container started, so ocs may still \
         use the old settings; run `varde restart ocs` to apply it"
    );
    assert!(edited_config_line("dhis2").starts_with("`dhis2/dhis.conf` changed"));
}
