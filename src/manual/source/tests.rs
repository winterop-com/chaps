use super::*;

fn repo_of(text: &str) -> Repo {
    match Source::parse(text).unwrap_or_else(|e| panic!("{text}: {e}")) {
        Source::Repo(repo) => repo,
        other => panic!("{text} parsed as {other:?}"),
    }
}

/// A reference with no registry host is an image in the local store,
/// pinned as given; one with a host is still a registry image.
#[test]
fn a_reference_without_a_registry_is_a_local_image() {
    let local = |text: &str| match Source::parse(text).unwrap_or_else(|e| panic!("{text}: {e}")) {
        Source::Local(image) => image,
        other => panic!("{text} parsed as {other:?}"),
    };
    let image = local("My-Model:dev");
    assert_eq!(image.image, "my-model");
    assert_eq!(image.tag, "dev");
    assert_eq!(Source::Local(image).default_id(), "my_model");

    let image = local("me/my-model:dev");
    assert_eq!(image.image, "me/my-model");
    let digest = format!("sha256:{}", "a".repeat(64));
    assert_eq!(
        local(&format!("my-model@{digest}")).tag,
        format!("@{digest}")
    );

    let err = Source::parse("my-model").unwrap_err().to_string();
    assert!(err.contains("docker build -t my-model:dev ."), "{err}");
    assert!(matches!(
        Source::parse("ghcr.io/chap-models/chapkit_ghr_model:sha-1eb8cf1").unwrap(),
        Source::Image(_)
    ));
}

fn image_of(text: &str) -> Image {
    match Source::parse(text).unwrap_or_else(|e| panic!("{text}: {e}")) {
        Source::Image(image) => image,
        other => panic!("{text} parsed as {other:?}"),
    }
}

#[test]
fn every_spelling_of_a_repository_url_is_the_same_repository() {
    for text in [
        "https://github.com/chap-models/chapkit_ghr_model",
        "https://github.com/chap-models/chapkit_ghr_model/",
        "https://github.com/chap-models/chapkit_ghr_model.git",
        "http://github.com/chap-models/chapkit_ghr_model",
        "https://www.github.com/chap-models/chapkit_ghr_model",
        "github.com/chap-models/chapkit_ghr_model",
        "git@github.com:chap-models/chapkit_ghr_model.git",
        // A URL copied out of a browser, tree view and all.
        "https://github.com/chap-models/chapkit_ghr_model/tree/main",
    ] {
        let repo = repo_of(text);
        assert_eq!(repo.owner, "chap-models", "{text}");
        assert_eq!(repo.repo, "chapkit_ghr_model", "{text}");
        assert_eq!(
            repo.url(),
            "https://github.com/chap-models/chapkit_ghr_model"
        );
        assert_eq!(
            repo.image(),
            "ghcr.io/chap-models/chapkit_ghr_model",
            "{text}"
        );
    }
}

#[test]
fn a_repository_derives_the_id_and_the_service_id() {
    let source = Source::parse("https://github.com/chap-models/chapkit_ghr_model").unwrap();
    assert_eq!(source.default_id(), "chapkit_ghr_model");
    assert_eq!(
        default_service_id(&source.default_id()),
        "chapkit-ghr-model"
    );
    assert_eq!(
        source.repository().as_deref(),
        Some("https://github.com/chap-models/chapkit_ghr_model")
    );

    // A repository nobody named in snake_case still yields a usable id.
    let source = Source::parse("https://github.com/someone/GHR-Model.v2").unwrap();
    assert_eq!(source.default_id(), "ghr_model_v2");
    check_id(&source.default_id()).unwrap();
    check_service_id(&default_service_id(&source.default_id())).unwrap();
}

#[test]
fn an_image_reference_splits_into_the_image_and_the_pin() {
    let image = image_of("ghcr.io/chap-models/chapkit_ghr_model:sha-1eb8cf1");
    assert_eq!(image.image, "ghcr.io/chap-models/chapkit_ghr_model");
    assert_eq!(image.tag, "sha-1eb8cf1");
    assert_eq!(image.repository(), "chap-models/chapkit_ghr_model");
    assert_eq!(
        image.reference(),
        "ghcr.io/chap-models/chapkit_ghr_model:sha-1eb8cf1"
    );

    // The image name is lowercased; the tag is left as it was typed.
    let image = image_of("ghcr.io/Chap-Models/Chapkit_GHR_Model:Sha-1eb8cf1");
    assert_eq!(image.image, "ghcr.io/chap-models/chapkit_ghr_model");
    assert_eq!(image.tag, "Sha-1eb8cf1");
}

#[test]
fn a_digest_keeps_its_own_separator() {
    let digest = "a".repeat(64);
    let image = image_of(&format!(
        "ghcr.io/chap-models/chapkit_ghr_model@sha256:{digest}"
    ));
    assert_eq!(image.image, "ghcr.io/chap-models/chapkit_ghr_model");
    assert_eq!(image.tag, format!("@sha256:{digest}"));
    assert_eq!(
        image.reference(),
        format!("ghcr.io/chap-models/chapkit_ghr_model@sha256:{digest}")
    );
    assert_eq!(
        Source::Image(image).default_id(),
        "chapkit_ghr_model",
        "the id comes from the image path, not the pin"
    );
}

#[test]
fn what_cannot_be_added_says_why() {
    for (text, needle) in [
        ("", "GitHub repository URL"),
        (
            "chapkit_ghr_model",
            "docker build -t chapkit_ghr_model:dev .",
        ),
        ("docker.io/library/nginx:1", "varde models add` reads"),
        ("ghcr.io/chap-models/chapkit_ghr_model", "names no tag"),
        ("ghcr.io/chap-models/chapkit_ghr_model:", "empty tag"),
        ("ghcr.io/x/y@sha256:nope", "not a digest"),
        ("https://github.com/chap-models", "names no repository"),
    ] {
        let err = Source::parse(text).expect_err(text);
        assert!(err.to_string().contains(needle), "{text}: {err}");
    }
}

#[test]
fn an_id_and_a_service_id_are_checked_against_what_names_a_volume() {
    check_id("chapkit_ghr_model").unwrap();
    check_id("m2").unwrap();
    for bad in ["", "Chapkit", "chapkit-ghr", "chap kit", "chap.kit"] {
        assert!(check_id(bad).is_err(), "{bad}");
    }

    check_service_id("chapkit-ghr-model").unwrap();
    for bad in ["", "-model", "Chapkit", "chapkit_ghr", "chap model"] {
        assert!(check_service_id(bad).is_err(), "{bad}");
    }
}

/// Every service chap-core or a component runs as is a name a model may not
/// take; a template that gains a service and not a line here fails this.
#[test]
fn the_reserved_service_ids_cover_every_service_the_templates_define() {
    let templates = [
        include_str!("../../compose/templates/compose.base.yml"),
        include_str!("../../compose/templates/compose.ocs.yml"),
        include_str!("../../compose/templates/compose.s3.yml"),
        include_str!("../../compose/templates/compose.dhis2.yml"),
    ];
    // The templates carry placeholders and are not YAML until rendered, so
    // the service names are read off the lines: two spaces in, under
    // `services:`.
    for template in templates {
        let mut in_services = false;
        let mut found = 0;
        for line in template.lines() {
            if !line.starts_with(' ') && !line.trim().is_empty() {
                in_services = line.trim_end() == "services:";
                continue;
            }
            let Some(name) = line
                .strip_prefix("  ")
                .filter(|rest| !rest.starts_with(' ') && !rest.starts_with('#'))
                .and_then(|rest| rest.trim_end().strip_suffix(':'))
            else {
                continue;
            };
            if !in_services {
                continue;
            }
            found += 1;
            assert!(
                RESERVED_SERVICE_IDS.contains(&name),
                "`{name}` is a service varde runs and is not reserved"
            );
        }
        assert!(found > 0, "no service found in a template");
    }
    for stem in ["varde", "marketplace"] {
        assert!(check_service_id(stem).is_err(), "{stem}");
    }
    assert!(check_service_id("chapkit-ewars-model").is_ok());
}
