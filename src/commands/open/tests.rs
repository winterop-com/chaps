use super::*;
use crate::components::{Components, Dhis2Component, OcsComponent};

/// A model's `/docs` is on its own port when it publishes one, and through
/// chap-core's proxy when it does not.
#[test]
fn a_model_opens_its_docs_on_its_port_or_through_chap_core() {
    let mut project = Project {
        dir: std::path::PathBuf::from("/tmp/chapx"),
        state: Default::default(),
    };
    let mut model = crate::project::EnabledModel {
        service_id: "chapkit-ewars-model".into(),
        image: String::new(),
        image_tag: String::new(),
        version: String::new(),
        channel: None,
        host_port: Some(5001),
        data_dir: String::new(),
        user: String::new(),
        user_from: Default::default(),
        platform: None,
        compose_file: String::new(),
    };
    let url = |project: &Project, model: &crate::project::EnabledModel| {
        model_docs_url(project, "chapkit_ewars_model", model)
    };
    assert_eq!(url(&project, &model).unwrap(), "http://localhost:5001/docs");
    model.host_port = None;
    project.state.components.chap_core.enabled = true;
    assert!(
        url(&project, &model)
            .unwrap()
            .ends_with("/v2/services/chapkit-ewars-model/run/docs")
    );
    project.state.components.chap_core.enabled = false;
    let err = url(&project, &model).unwrap_err().to_string();
    assert!(
        err.contains("run `chaps models expose chapkit_ewars_model`"),
        "{err}"
    );
}

/// The three answers about the container each say something different, and
/// only the one that knows nothing is running names `chaps up`.
#[test]
fn the_container_note_says_which_of_the_three_it_is() {
    assert_eq!(running_note(Running::Yes, Component::Dhis2), None);
    let no = running_note(Running::No, Component::Dhis2).expect("a note");
    assert!(no.contains("no dhis2 container is running"), "{no}");
    assert!(no.contains("run `chaps up`"), "{no}");
    let unknown = running_note(Running::Unknown, Component::Ocs).expect("a note");
    assert!(unknown.contains("docker could not be asked"), "{unknown}");
    assert!(unknown.contains("run `chaps doctor`"), "{unknown}");
    assert!(
        !unknown.contains("chaps up"),
        "nothing was learned, so nothing is prescribed: {unknown}"
    );
}

/// chap-core's service is `chap`, so the note has to name the container and
/// not the component.
#[test]
fn the_container_note_names_the_service_not_the_component() {
    let note = running_note(Running::No, Component::ChapCore).expect("a note");
    assert!(note.contains("no chap container is running"), "{note}");
}

/// The proxy note only appears for the instance that has no port of its own,
/// and it names the way to publish one.
#[test]
fn the_proxy_note_is_only_for_an_unpublished_instance() {
    assert_eq!(proxy_note(false, Component::Ocs), None);
    let note = proxy_note(true, Component::Ocs).expect("a note");
    assert!(
        note.contains("`chaps components enable ocs --port N`"),
        "{note}"
    );
}

/// The listing names every component, with an address for the ones that
/// have one and the reason for the ones that do not.
#[test]
fn the_listing_covers_every_component_with_its_reason() {
    let components = Components {
        dhis2: Dhis2Component {
            enabled: true,
            port: Some(18080),
            ..Dhis2Component::default()
        },
        ocs: OcsComponent {
            enabled: true,
            port: None,
            ..OcsComponent::default()
        },
        ..Components::default()
    };
    let rows: Vec<OpenRow> = Component::ALL
        .iter()
        .map(|component| {
            let (url, what) = match resolve(*component, &components, "http://localhost:18000") {
                Openable::Url { url, what, .. } => (Some(url), what.to_string()),
                Openable::Internal { inside } => (None, inside),
                Openable::NoWeb => (None, "no web interface".to_string()),
                Openable::Off => (None, "off".to_string()),
            };
            OpenRow {
                name: component.name().to_string(),
                url,
                what,
            }
        })
        .collect();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["chap-core", "ocs", "s3", "dhis2"]);
    assert_eq!(
        rows[0].url.as_deref(),
        Some("http://localhost:18000/docs"),
        "chap-core opens its API documentation"
    );
    assert_eq!(rows[1].url, None, "an unpublished OCS opens nothing");
    assert_eq!(rows[2].url, None, "the object store never opens");
    assert_eq!(rows[3].url.as_deref(), Some("http://localhost:18080"));
}

/// The closing line counts what can be opened, and says something else
/// entirely when nothing can.
#[test]
fn the_listing_closes_on_what_can_be_opened() {
    let out = Out::detect(false, true);
    let row = |name: &str, url: Option<&str>| OpenRow {
        name: name.to_string(),
        url: url.map(str::to_string),
        what: String::new(),
    };
    let none = OpenListReport {
        components: vec![row("ocs", None), row("s3", None)],
    };
    let text = human_list(&none, &out);
    assert!(
        text.contains("nothing in this deployment has a web interface"),
        "{text}"
    );
    assert!(text.contains("`chaps components list`"), "{text}");

    let some = OpenListReport {
        components: vec![
            row("chap-core", Some("http://localhost:8000/docs")),
            row("s3", None),
        ],
    };
    let text = human_list(&some, &out);
    assert!(text.contains("1 of them can be opened"), "{text}");
    assert!(text.contains("run `chaps open NAME`"), "{text}");
}

/// A machine with no opener is told the address and not that something
/// failed, and the line names the opener that is missing.
#[test]
fn no_opener_reports_the_address_rather_than_a_failure() {
    let out = Out::detect(false, true);
    let report = OpenReport {
        name: "dhis2".to_string(),
        url: "http://localhost:18080".to_string(),
        page: crate::open::DHIS2_PAGE,
        opened: false,
        no_browser: false,
        running: Running::Yes,
        answering: Some(true),
        notes: Vec::new(),
    };
    let text = human(&report, &out);
    assert!(text.contains("http://localhost:18080"), "{text}");
    assert!(
        text.contains("the address above is the whole of it"),
        "{text}"
    );
    assert!(!text.to_lowercase().contains("error"), "{text}");
    let (command, _) = crate::open::opener();
    assert!(text.contains(command), "{text}");
}

/// A run that opened something says so first, then whatever is worth
/// knowing about it, and ends on the command that says whether it answered.
#[test]
fn a_successful_open_reports_the_page_then_the_container() {
    let out = Out::detect(false, true);
    let report = OpenReport {
        name: "dhis2".to_string(),
        url: "http://localhost:18080".to_string(),
        page: crate::open::DHIS2_PAGE,
        opened: true,
        no_browser: false,
        running: Running::Yes,
        answering: Some(true),
        notes: Vec::new(),
    };
    let text = human(&report, &out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "opening the DHIS2 user interface at http://localhost:18080"
    );
    assert_eq!(
        lines[1],
        "dhis2 answered at that address; `chaps status` reports the rest of this deployment"
    );

    // A container that is up and not serving yet is not called answering.
    let starting = OpenReport {
        answering: Some(false),
        ..report
    };
    let text = human(&starting, &out);
    assert!(
        text.contains("the dhis2 container is running and did not answer yet"),
        "{text}"
    );
    assert!(text.contains("run `chaps status` in a moment"), "{text}");
}

/// `--no-browser` says where the page is and nothing about an opener: none
/// was asked, so its absence is not worth a line.
#[test]
fn no_browser_names_the_page_and_no_opener() {
    let out = Out::detect(false, true);
    let report = OpenReport {
        name: "ocs".to_string(),
        url: "http://localhost:8790".to_string(),
        page: crate::open::OCS_PAGE,
        opened: false,
        no_browser: true,
        running: Running::Yes,
        answering: Some(true),
        notes: Vec::new(),
    };
    let text = human(&report, &out);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "the OCS web interface is at http://localhost:8790"
    );
    assert!(!text.contains("opening"), "{text}");
    assert!(!text.contains("the whole of it"), "{text}");
}

/// Nothing about the container is claimed when docker said nothing, and the
/// note is what stands in for the closing line.
#[test]
fn an_unknown_container_says_so_and_claims_nothing() {
    let out = Out::detect(false, true);
    let report = OpenReport {
        name: "ocs".to_string(),
        url: "http://localhost:9000".to_string(),
        page: crate::open::OCS_PAGE,
        opened: true,
        no_browser: false,
        running: Running::Unknown,
        answering: None,
        notes: vec![running_note(Running::Unknown, Component::Ocs).expect("a note")],
    };
    let text = human(&report, &out);
    assert!(text.contains("note: docker could not be asked"), "{text}");
    assert!(
        !text.contains("container is running"),
        "nothing is claimed: {text}"
    );
}
