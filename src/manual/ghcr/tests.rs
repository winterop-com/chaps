use super::*;

/// An OCI index as ghcr serves one: the image, and the attestation entry
/// that must not be taken for it.
const INDEX: &str = r#"{
      "schemaVersion": 2,
      "mediaType": "application/vnd.oci.image.index.v1+json",
      "manifests": [
        {
          "mediaType": "application/vnd.oci.image.manifest.v1+json",
          "digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
          "size": 1234,
          "platform": {"architecture": "amd64", "os": "linux"}
        },
        {
          "mediaType": "application/vnd.oci.image.manifest.v1+json",
          "digest": "sha256:2222222222222222222222222222222222222222222222222222222222222222",
          "size": 566,
          "annotations": {"vnd.docker.reference.type": "attestation-manifest"},
          "platform": {"architecture": "unknown", "os": "unknown"}
        }
      ]
    }"#;

const MANIFEST: &str = r#"{
      "schemaVersion": 2,
      "mediaType": "application/vnd.oci.image.manifest.v1+json",
      "config": {
        "mediaType": "application/vnd.oci.image.config.v1+json",
        "digest": "sha256:3333333333333333333333333333333333333333333333333333333333333333",
        "size": 8901
      },
      "layers": []
    }"#;

const BLOB: &str = r#"{
      "architecture": "amd64",
      "os": "linux",
      "config": {
        "User": "app",
        "WorkingDir": "/work",
        "Env": ["PATH=/usr/bin"],
        "Entrypoint": ["/entrypoint.sh"]
      }
    }"#;

#[test]
fn an_index_yields_the_amd64_image_and_skips_the_attestation() {
    let platforms = parse_platforms(INDEX);
    assert_eq!(platforms.len(), 1, "unknown/unknown is not a platform");
    assert_eq!(platforms[0].architecture, AMD64);
    assert_eq!(
        amd64_manifest(INDEX).as_deref(),
        Some("sha256:1111111111111111111111111111111111111111111111111111111111111111")
    );
}

#[test]
fn an_index_of_two_architectures_is_not_amd64_only() {
    let both = INDEX.replace(
        r#""platform": {"architecture": "unknown", "os": "unknown"}"#,
        r#""platform": {"architecture": "arm64", "os": "linux"}"#,
    );
    let platforms = parse_platforms(&both);
    assert_eq!(platforms.len(), 2);
    assert!(platforms.iter().any(|p| p.architecture == "arm64"));
    // The amd64 entry is still the one that is pulled.
    assert!(amd64_manifest(&both).unwrap().contains("1111"));
}

#[test]
fn a_plain_manifest_has_no_platforms_and_one_config() {
    assert!(parse_platforms(MANIFEST).is_empty());
    assert_eq!(amd64_manifest(MANIFEST), None);
    assert_eq!(
        config_digest(MANIFEST).as_deref(),
        Some("sha256:3333333333333333333333333333333333333333333333333333333333333333")
    );
    assert_eq!(config_digest(INDEX), None);
}

#[test]
fn a_config_blob_yields_the_user_and_the_working_directory() {
    let config = parse_config(BLOB);
    assert_eq!(config.user, "app");
    assert_eq!(config.working_dir, "/work");
    assert!(!config.amd64_only, "the index says that, not the blob");

    // An image that declares neither reads as empty rather than failing.
    let bare = parse_config(r#"{"architecture":"amd64","config":{}}"#);
    assert_eq!(bare, ImageConfig::default());
    assert_eq!(parse_config("not json"), ImageConfig::default());
}

#[test]
fn a_token_response_yields_its_token() {
    assert_eq!(parse_token(r#"{"token":"abc"}"#).unwrap(), "abc");
    // Some registries call it access_token.
    assert_eq!(
        parse_token(r#"{"access_token":"xyz","expires_in":300}"#).unwrap(),
        "xyz"
    );
    assert!(parse_token(r#"{"errors":[{"code":"DENIED"}]}"#).is_err());
    assert!(parse_token("<html>").is_err());
}
