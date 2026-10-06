use super::*;
use crate::components::{Dhis2Component, OcsComponent, S3Component};

fn url_of(openable: &Openable) -> Option<&str> {
    match openable {
        Openable::Url { url, .. } => Some(url.as_str()),
        _ => None,
    }
}

/// chap-core's useful page is `/docs`, not the origin: the API answers JSON
/// everywhere else.
#[test]
fn chap_core_opens_its_api_documentation() {
    let components = Components::default();
    assert_eq!(
        resolve(Component::ChapCore, &components, "http://localhost:18000"),
        Openable::Url {
            url: "http://localhost:18000/docs".to_string(),
            what: CHAP_CORE_PAGE,
            proxied: false,
        }
    );
}

/// `CHAP_ROOT_PATH` moves the whole API, `/docs` with it, and a trailing
/// slash on the base must not double up.
#[test]
fn the_root_path_prefixes_the_documentation() {
    let components = Components::default();
    assert_eq!(
        url_of(&resolve(
            Component::ChapCore,
            &components,
            "http://localhost:8000/master"
        )),
        Some("http://localhost:8000/master/docs")
    );
    assert_eq!(
        url_of(&resolve(
            Component::ChapCore,
            &components,
            "http://localhost:8000/"
        )),
        Some("http://localhost:8000/docs")
    );
}

/// A published component opens at its root: that is where both web
/// interfaces live.
#[test]
fn a_published_component_opens_at_its_host_port() {
    let components = Components {
        ocs: OcsComponent {
            enabled: true,
            port: Some(9010),
            ..OcsComponent::default()
        },
        dhis2: Dhis2Component {
            enabled: true,
            port: Some(18080),
            ..Dhis2Component::default()
        },
        ..Components::default()
    };
    let base = "http://localhost:8000";
    assert_eq!(
        url_of(&resolve(Component::Ocs, &components, base)),
        Some("http://localhost:9010")
    );
    assert_eq!(
        url_of(&resolve(Component::Dhis2, &components, base)),
        Some("http://localhost:18080")
    );
}

/// No host port and no recorded origin: reached inside the deployment, so
/// nothing is opened and the internal address is what there is to say.
#[test]
fn a_component_with_no_host_port_is_reached_inside_the_deployment() {
    let components = Components {
        ocs: OcsComponent {
            enabled: true,
            port: None,
            ..OcsComponent::default()
        },
        dhis2: Dhis2Component {
            enabled: true,
            port: None,
            ..Dhis2Component::default()
        },
        ..Components::default()
    };
    let base = "http://localhost:8000";
    assert_eq!(
        resolve(Component::Ocs, &components, base),
        Openable::Internal {
            inside: "http://ocs:9000".to_string()
        }
    );
    assert_eq!(
        resolve(Component::Dhis2, &components, base),
        Openable::Internal {
            inside: "http://dhis2:8080".to_string()
        }
    );
    let reason = internal_reason(Component::Ocs, "http://ocs:9000");
    assert!(reason.contains("http://ocs:9000"), "{reason}");
    assert!(
        reason.contains("`varde components enable ocs --port N`"),
        "{reason}"
    );
}

/// An unpublished instance behind a proxy is reached at the origin it
/// records, which is an address a browser can be pointed at.
#[test]
fn an_unpublished_ocs_opens_the_origin_it_records() {
    let components = Components {
        ocs: OcsComponent {
            enabled: true,
            port: None,
            base_url: Some("https://climate.example.org/".to_string()),
            ..OcsComponent::default()
        },
        ..Components::default()
    };
    assert_eq!(
        resolve(Component::Ocs, &components, "http://localhost:8000"),
        Openable::Url {
            url: "https://climate.example.org".to_string(),
            what: OCS_PAGE,
            proxied: true,
        }
    );
}

/// A component this deployment does not have is refused, and the refusal
/// names the command that adds it.
#[test]
fn a_component_that_is_off_is_refused() {
    let components = Components::default();
    let base = "http://localhost:8000";
    assert_eq!(resolve(Component::Ocs, &components, base), Openable::Off);
    assert_eq!(resolve(Component::Dhis2, &components, base), Openable::Off);

    let mut off = components.clone();
    off.chap_core.enabled = false;
    assert_eq!(resolve(Component::ChapCore, &off, base), Openable::Off);

    let reason = off_reason(Component::Dhis2);
    assert!(
        reason.contains("`varde components enable dhis2`"),
        "{reason}"
    );
}

/// The object store has no web interface, so it says so whether it is on,
/// off, or publishing a port: enabling it would not make it openable.
#[test]
fn the_object_store_is_never_opened() {
    let mut components = Components::default();
    let base = "http://localhost:8000";
    assert_eq!(resolve(Component::S3, &components, base), Openable::NoWeb);
    components.s3 = S3Component {
        enabled: true,
        port: Some(9002),
    };
    assert_eq!(resolve(Component::S3, &components, base), Openable::NoWeb);
    assert!(NO_WEB_INTERFACE.contains("S3 API"));
}

/// Every component has an internal address, and none of them is empty or
/// points at the host.
#[test]
fn every_component_names_where_it_is_reached_inside() {
    for component in Component::ALL {
        let url = internal_url(*component);
        assert!(url.starts_with("http://"), "{url}");
        assert!(!url.contains("localhost"), "{url}");
    }
}

/// Spawning the opener is not tested - it would open a browser on the
/// machine running the suite - but the command it would spawn is.
#[test]
fn every_platform_names_an_opener() {
    let (command, args) = opener();
    assert!(!command.is_empty());
    assert!(args.iter().all(|arg| !arg.contains("://")));
}
