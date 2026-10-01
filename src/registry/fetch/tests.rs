use super::*;

/// Port 9 is the discard service and is not listening on a developer
/// machine, so this exercises the transport-error path without a network.
const UNREACHABLE: &str = "http://127.0.0.1:9/registry.yaml";

#[test]
fn a_status_code_becomes_a_typed_http_error() {
    let err = map_error(
        "https://example.test/registry.yaml",
        ureq::Error::StatusCode(404),
    );
    match err.downcast_ref::<ChapError>() {
        Some(ChapError::Http { url, status }) => {
            assert_eq!(url, "https://example.test/registry.yaml");
            assert_eq!(*status, 404);
        }
        other => panic!("wrong error: {other:?}"),
    }
}

#[test]
fn a_transport_failure_keeps_the_url_as_context() {
    let err = map_error(
        "https://example.test/registry.yaml",
        ureq::Error::HostNotFound,
    );
    assert!(err.downcast_ref::<ChapError>().is_none());
    assert!(
        err.to_string()
            .contains("https://example.test/registry.yaml")
    );
}

#[test]
fn an_unreachable_registry_is_an_error_not_a_hang() {
    let opts = RegistryOptions {
        url: UNREACHABLE.to_string(),
        timeout: Duration::from_secs(2),
        ..RegistryOptions::default()
    };
    let err = fetch_registry(&opts).expect_err("nothing is listening on port 9");
    assert!(err.to_string().contains(UNREACHABLE), "{err}");
}
