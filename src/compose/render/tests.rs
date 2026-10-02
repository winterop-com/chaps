use super::dhis2::DHIS2_TEMPLATE;
use super::ocs::{OCS_TEMPLATE, S3_TEMPLATE};
use super::overlay::OVERLAY_TEMPLATE;
use super::*;
use crate::compose::spec::{
    ChapsOverlaySpec, Dhis2ConfigSpec, Dhis2SeedSource, Dhis2Spec, OcsConfigSpec, OcsSpec,
    OverlaySpec, S3Spec, UpstreamCompose,
};
use crate::compose::{tag_env_var, volume_name};
use crate::registry::{Channel, VersionSelector, load_embedded};
use serde_yaml_ng::Value;

/// Nothing may leave an `@KEY@` behind; `@` on its own is legal YAML (the
/// base file has one in a postgres URL), so only the token shape counts.
fn assert_no_tokens(text: &str) {
    let re = regex::Regex::new(r"@[A-Z][A-Z0-9_]*@").unwrap();
    assert!(
        !re.is_match(text),
        "unfilled token: {:?}",
        re.find(text).map(|m| m.as_str())
    );
}

/// A spec for a model that publishes no host port: the default.
fn overlay_spec(id: &str) -> OverlaySpec {
    let registry = load_embedded().unwrap();
    let m = registry.get(id).unwrap();
    let v = m
        .resolve(&VersionSelector::Channel(Channel::Stable))
        .unwrap();
    OverlaySpec::from_model(m, v, None, None, None)
}

/// The same, published on `host_port`.
fn published_spec(id: &str, host_port: u16) -> OverlaySpec {
    OverlaySpec {
        host_port: Some(host_port),
        ..overlay_spec(id)
    }
}

/// `compose.chaps.yml` over upstream's services, outside a `chaps run` group.
fn chaps_overlay(api_port: u16, project_name: Option<&str>, checkout: Option<&str>) -> String {
    render_chaps_overlay(&ChapsOverlaySpec {
        project_name: project_name.map(str::to_string),
        checkout: checkout.map(str::to_string),
        ..ChapsOverlaySpec::new(api_port)
    })
}

fn parse(text: &str) -> Value {
    serde_yaml_ng::from_str(text).expect("generated compose parses as YAML")
}

fn service<'a>(doc: &'a Value, name: &str) -> &'a Value {
    &doc["services"][name]
}

#[test]
fn fill_replaces_every_token() {
    let out = fill("a=@A@ b=@B@ a=@A@", &[("A", "1"), ("B", "2")]);
    assert_eq!(out, "a=1 b=2 a=1");
}

#[test]
fn fill_leaves_compose_variables_alone() {
    let out = fill("${TAG:-sha-1234567} @TAG@", &[("TAG", "x")]);
    assert_eq!(out, "${TAG:-sha-1234567} x");
}

#[test]
fn overlay_matches_the_golden_fixture() {
    let text = render_overlay(&overlay_spec("chapkit_ewars_model"));
    let golden = normalize_newlines(include_str!(
        "../../../tests/fixtures/compose.chapkit-ewars-model.yml"
    ));
    assert_eq!(text, golden);
}

/// The same shape under the other WorkingDir: `/work/data`, which is
/// where most of the catalogue keeps its database, rather than the EWARS
/// image's `/app/data`.
#[test]
fn a_work_dir_overlay_matches_its_own_golden_fixture() {
    let text = render_overlay(&overlay_spec("chapkit_rwanda_malaria_bym_model"));
    let golden = normalize_newlines(include_str!(
        "../../../tests/fixtures/compose.chapkit-rwanda-malaria-bym-model.yml"
    ));
    assert_eq!(text, golden);
}

/// The shape a deployment without chap-core renders: no orchestrator URL
/// and no registration key, since the service registers nowhere, and no
/// `chap` to wait for. The rest is byte for byte the usual overlay.
#[test]
fn a_standalone_overlay_matches_its_own_golden_fixture() {
    let mut spec = overlay_spec("chapkit_ewars_model");
    spec.standalone = true;
    let text = render_overlay(&spec);
    assert_no_tokens(&text);
    let golden = normalize_newlines(include_str!(
        "../../../tests/fixtures/compose.chapkit-ewars-model.standalone.yml"
    ));
    assert_eq!(text, golden);
    let doc: Value = serde_yaml_ng::from_str(&text).unwrap();
    let svc = service(&doc, "chapkit-ewars-model");
    assert!(svc.get("environment").is_none());
    assert!(svc["depends_on"].get("chap").is_none());
}

/// The shape for a chap-core elsewhere: the service registers there over
/// the host gateway, under this machine's name and its published port,
/// and waits for no `chap`.
#[test]
fn an_external_chap_core_overlay_matches_its_own_golden_fixture() {
    let mut spec = overlay_spec("chapkit_ewars_model");
    spec.host_port = Some(5001);
    spec.standalone = true;
    spec.external_chap_core = Some(crate::compose::spec::ExternalRegistration {
        register_url: "http://host.docker.internal:8000".to_string(),
        models_host: "localhost".to_string(),
    });
    let text = render_overlay(&spec);
    assert_no_tokens(&text);
    let golden = normalize_newlines(include_str!(
        "../../../tests/fixtures/compose.chapkit-ewars-model.external.yml"
    ));
    assert_eq!(text, golden);
    let doc: Value = serde_yaml_ng::from_str(&text).unwrap();
    let svc = service(&doc, "chapkit-ewars-model");
    assert_eq!(
        svc["environment"]["SERVICEKIT_ORCHESTRATOR_URL"].as_str(),
        Some("http://host.docker.internal:8000/v2/services/$$register")
    );
    assert_eq!(svc["environment"]["SERVICEKIT_PORT"].as_str(), Some("5001"));
    assert!(svc["depends_on"].get("chap").is_none());
}

/// A locally built image has no registry behind it, so compose must never
/// try to pull it; a registry image gets no such line.
#[test]
fn a_local_image_is_never_pulled() {
    let mut spec = overlay_spec("chapkit_ewars_model");
    assert!(!render_overlay(&spec).contains("pull_policy"));
    spec.image = "my-model".to_string();
    spec.image_tag = "dev".to_string();
    let text = render_overlay(&spec);
    let doc: Value = serde_yaml_ng::from_str(&text).unwrap();
    let svc = service(&doc, "chapkit-ewars-model");
    assert_eq!(svc["pull_policy"].as_str(), Some("never"));
    assert!(
        svc["image"].as_str().unwrap().starts_with("my-model:"),
        "{text}"
    );
}

/// A deployment built from a chap-core checkout builds chap and worker
/// from it on every `chaps up`, into images of their own.
#[test]
fn a_checkout_builds_chap_and_the_worker() {
    assert!(!chaps_overlay(8000, None, None).contains("build:"));
    let text = chaps_overlay(8000, None, Some("/src/chap-core"));
    let doc: Value = serde_yaml_ng::from_str(&text).unwrap();
    let chap = service(&doc, "chap");
    assert_eq!(chap["image"].as_str(), Some(CHECKOUT_CHAP_IMAGE));
    assert_eq!(chap["build"]["context"].as_str(), Some("/src/chap-core"));
    assert_eq!(chap["pull_policy"].as_str(), Some("build"));
    let worker = service(&doc, "worker");
    assert_eq!(worker["image"].as_str(), Some(CHECKOUT_WORKER_IMAGE));
    assert_eq!(
        worker["build"]["dockerfile"].as_str(),
        Some("Dockerfile.worker")
    );

    // With a project name the images are that deployment's own.
    let named = chaps_overlay(8000, Some("mychap-1ab2c3"), Some("/src/chap-core"));
    let doc: Value = serde_yaml_ng::from_str(&named).unwrap();
    assert_eq!(
        service(&doc, "chap")["image"].as_str(),
        Some("mychap-1ab2c3-chap:checkout")
    );
    assert_eq!(
        service(&doc, "worker")["image"].as_str(),
        Some("mychap-1ab2c3-worker:checkout")
    );
    assert_eq!(worker["pull_policy"].as_str(), Some("build"));
}

/// The third shape, said as assertions rather than as a golden file: an
/// image that runs as root, which gets no `user:` line and an init
/// container that chowns its volume to `0:0`. Auto-ARIMA is one of the
/// three marketplace images that still end on `USER root`.
#[test]
fn a_root_image_gets_no_user_line_and_an_init_container_that_chowns_to_zero() {
    let text = render_overlay(&overlay_spec("auto_arima_chapkit"));
    assert_no_tokens(&text);
    let doc = parse(&text);
    let svc = service(&doc, "auto-arima-chapkit");
    // The one line a root image does not get: the image's own user
    // applies, and a `user:` here could only take a permission away.
    assert!(svc.get("user").is_none(), "root needs no override");

    // The one it does. A volume docker has just created is root-owned and
    // needs no chown, but one carried over from a deployment that ran
    // this model as 1000:1000 is owned by 1000 - and the overlay drops
    // `CAP_DAC_OVERRIDE` with every other capability, so root cannot
    // write to it. This is what makes that upgrade heal itself.
    let init = service(&doc, "auto-arima-chapkit-init");
    assert_eq!(
        init["command"][2].as_str(),
        Some("chown -R 0:0 /work/data"),
        "{text}"
    );
    assert_eq!(init["user"].as_str(), Some("0:0"));
    assert_eq!(init["restart"].as_str(), Some("no"));
    assert_eq!(init["volumes"][0]["target"].as_str(), Some("/work/data"));

    // The model waits for it, and still waits for chap-core.
    assert_eq!(
        svc["depends_on"]["auto-arima-chapkit-init"]["condition"].as_str(),
        Some("service_completed_successfully")
    );
    assert_eq!(
        svc["depends_on"]["chap"]["condition"].as_str(),
        Some("service_healthy")
    );
    assert_eq!(svc["volumes"][1]["target"].as_str(), Some("/work/data"));
    assert!(doc["volumes"]["ck_auto_arima_chapkit_data"].is_mapping());
    // And the hardening it shares with every other overlay is untouched.
    assert_eq!(svc["read_only"].as_bool(), Some(true));
    assert_eq!(svc["init"].as_bool(), Some(true));

    // Every spelling of root renders the same file, chown included.
    let mut spec = overlay_spec("auto_arima_chapkit");
    for user in ["root", "0", "0:0", "root:root", ""] {
        spec.user = user.to_string();
        assert_eq!(render_overlay(&spec), text, "{user:?}");
    }
}

#[test]
fn an_overlay_publishes_a_port_only_when_one_was_asked_for() {
    // The default: exposed on the compose network, invisible to the host.
    let internal = render_overlay(&overlay_spec("chapkit_ewars_model"));
    assert_no_tokens(&internal);
    assert!(internal.contains("\n    expose:\n      - \"8000\"\n"));
    assert!(!internal.contains("    ports:"));
    // Dropping the ports block must not leave a blank line behind.
    assert!(internal.contains("\n    environment:\n"));
    assert!(!internal.contains("\n\n    environment:"));

    // With a port, both keys: `expose` still documents the container port,
    // and `ports` is the mapping Docker publishes.
    let published = render_overlay(&published_spec("chapkit_ewars_model", 5002));
    assert_no_tokens(&published);
    let svc = &parse(&published)["services"]["chapkit-ewars-model"];
    assert_eq!(
        svc["expose"],
        Value::Sequence(vec![Value::String("8000".into())])
    );
    assert_eq!(
        svc["ports"],
        Value::Sequence(vec![Value::String("5002:8000".into())])
    );
}

#[test]
fn a_bound_port_names_its_host_address() {
    let mut spec = published_spec("chapkit_ewars_model", 5002);
    spec.bind = Some("127.0.0.1".parse().unwrap());
    let svc = &parse(&render_overlay(&spec))["services"]["chapkit-ewars-model"];
    assert_eq!(
        svc["ports"],
        Value::Sequence(vec![Value::String("127.0.0.1:5002:8000".into())])
    );

    spec.bind = Some("::1".parse().unwrap());
    let svc = &parse(&render_overlay(&spec))["services"]["chapkit-ewars-model"];
    assert_eq!(
        svc["ports"],
        Value::Sequence(vec![Value::String("[::1]:5002:8000".into())])
    );
}

#[test]
fn the_chaps_overlay_replaces_the_api_port_rather_than_adding_to_it() {
    let text = chaps_overlay(8000, None, None);
    assert_no_tokens(&text);
    assert!(text.starts_with(&format!("{GENERATED_HEADER}\n")));
    // `!override` is what makes this a replacement: a plain `ports:` list
    // would merge with the one compose.yml already has.
    assert!(text.contains("    ports: !override\n      - \"${CHAP_API_PORT:-8000}:8000\"\n"));
    assert!(text.ends_with('\n'));

    // The recorded port is the variable's default, so the file works
    // without .env and moves when `.chaps/project.yaml` does.
    assert!(chaps_overlay(8123, None, None).contains("${CHAP_API_PORT:-8123}:8000"));

    // It parses, tag and all, and only chap has its ports replaced; the
    // other services are named for their labels alone.
    let doc = parse(&text);
    let services = doc["services"].as_mapping().unwrap();
    assert_eq!(services.len(), 4);
    for (name, svc) in services {
        assert_eq!(
            svc.get("ports").is_some(),
            name.as_str() == Some("chap"),
            "{name:?}"
        );
    }
}

#[test]
fn the_chaps_overlay_names_the_compose_project() {
    let text = chaps_overlay(8000, Some("mychap-1ab2c3"), None);
    assert_no_tokens(&text);
    assert!(text.contains("\nname: mychap-1ab2c3\n"), "{text}");
    assert_eq!(
        parse(&text)["name"].as_str(),
        Some("mychap-1ab2c3"),
        "compose reads it as the project name"
    );
    // Without a name the key is absent rather than empty: compose would
    // reject `name:` with nothing after it.
    assert!(!chaps_overlay(8000, None, None).contains("name:"));
}

#[test]
fn the_chaps_overlay_hands_chap_core_the_registration_key() {
    // Upstream's compose.ghcr.yml passes only CHAP_API_TOKEN into the
    // container, so without this line a protected chap-core has no key to
    // check a model's X-Service-Key against and answers 401.
    let text = chaps_overlay(8000, Some("mychap-1ab2c3"), None);
    let env = &parse(&text)["services"]["chap"]["environment"];
    assert_eq!(
        env["SERVICEKIT_REGISTRATION_KEY"].as_str(),
        Some("${SERVICEKIT_REGISTRATION_KEY:-}")
    );
    // Rendered whether or not the deployment has a key: compose
    // substitutes an empty value, which chap-core reads as no key.
    assert!(chaps_overlay(8000, None, None).contains("SERVICEKIT_REGISTRATION_KEY:"));
}

#[test]
fn the_chaps_overlay_points_gunicorns_control_socket_at_the_tmpfs() {
    // gunicorn 26 falls back to $HOME/.gunicorn/, which the service's
    // read-only root refuses, and logs an error on every start; /tmp is
    // the tmpfs upstream already mounts.
    let text = chaps_overlay(8000, None, None);
    assert_eq!(
        parse(&text)["services"]["chap"]["environment"]["XDG_RUNTIME_DIR"].as_str(),
        Some("/tmp")
    );
    // As a key and nothing else: why it is there is in the docs.
    assert!(!text.contains("# Keeps gunicorn"), "{text}");
    // The worker runs celery, not gunicorn, so it is left alone.
    assert!(
        parse(&text)["services"]["worker"]
            .get("environment")
            .is_none()
    );
}

#[test]
fn overlay_keeps_the_chap_core_deployment_invariants() {
    let text = render_overlay(&overlay_spec("chapkit_ewars_model"));
    assert_no_tokens(&text);
    // `$$register` must survive verbatim: `$register` would expand away.
    assert!(text.contains("http://chap:8000/v2/services/$$register"));

    let doc = parse(&text);
    let svc = service(&doc, "chapkit-ewars-model");
    assert_eq!(svc["restart"].as_str(), Some("unless-stopped"));
    assert_eq!(svc["init"].as_bool(), Some(true));
    assert_eq!(svc["read_only"].as_bool(), Some(true));
    // The numeric form, so this line and the init container's chown are
    // the same two numbers.
    assert_eq!(svc["user"].as_str(), Some("1000:1000"));

    // chap-core/tests/test_compose_deployment.py: a pinned sha build behind
    // an overridable ${<ID>_IMAGE_TAG:-sha-xxxxxxx}, and no pull_policy.
    let image = svc["image"].as_str().unwrap();
    let re =
        regex::Regex::new(r"^ghcr\.io/[a-z0-9._/-]+:\$\{[A-Z0-9_]+_IMAGE_TAG:-sha-[0-9a-f]{7}\}$")
            .unwrap();
    assert!(re.is_match(image), "unexpected image {image:?}");
    assert!(
        svc.get("pull_policy").is_none(),
        "pull_policy defeats the pin"
    );

    // No networks: the default network reaches chap but not redis/postgres.
    assert!(svc.get("networks").is_none());
    // chapkit images ship their own HEALTHCHECK.
    assert!(svc.get("healthcheck").is_none());
    // Both facts live in docs/models.md, not in a comment in every file.
    assert!(!text.contains("# No `networks:`"));
    assert!(!text.contains("# No healthcheck"));

    // No host port by default: only the container port is declared.
    assert_eq!(
        svc["expose"],
        Value::Sequence(vec![Value::String("8000".into())])
    );
    assert!(svc.get("ports").is_none(), "a model publishes nothing");
    assert_eq!(
        svc["depends_on"]["chap"]["condition"].as_str(),
        Some("service_healthy")
    );

    // The named volume is declared as well as mounted.
    let volume = volume_name("chapkit_ewars_model");
    assert_eq!(svc["volumes"][1]["source"].as_str(), Some(volume.as_str()));
    assert_eq!(svc["volumes"][1]["target"].as_str(), Some("/app/data"));
    assert!(doc["volumes"].get(volume.as_str()).is_some());

    // Two services, and only the model carries the restart policy: the
    // init container is a one-shot by design.
    let services = doc["services"].as_mapping().unwrap();
    assert_eq!(services.len(), 2);
}

#[test]
fn overlay_hands_the_data_volume_to_the_model_user_before_it_starts() {
    let text = render_overlay(&overlay_spec("chapkit_ewars_model"));
    let doc = parse(&text);
    let init = service(&doc, "chapkit-ewars-model-init");

    // Docker seeds a fresh named volume from the image, ownership
    // included, and root-owns it when the image has no such directory;
    // the model would then fail to open its SQLite file.
    assert_eq!(init["image"].as_str(), Some("busybox:1.37"));
    assert_eq!(init["user"].as_str(), Some("0:0"));
    // Numeric: busybox knows no `chapkit` account.
    assert_eq!(
        init["command"],
        Value::Sequence(vec![
            "sh".into(),
            "-c".into(),
            "chown -R 1000:1000 /app/data".into()
        ])
    );
    assert!(!text.contains("chown chapkit"));
    // Quoted, or YAML would read it as the boolean false.
    assert!(text.contains("    restart: \"no\"\n"));
    assert_eq!(init["restart"].as_str(), Some("no"));
    // Multi-arch, so no platform pin is needed.
    assert!(init.get("platform").is_none());
    assert!(init.get("ports").is_none());

    // Same volume at the same path as the model service.
    let volume = volume_name("chapkit_ewars_model");
    assert_eq!(init["volumes"][0]["source"].as_str(), Some(volume.as_str()));
    assert_eq!(init["volumes"][0]["target"].as_str(), Some("/app/data"));

    // And the model waits for it to finish.
    let svc = service(&doc, "chapkit-ewars-model");
    assert_eq!(
        svc["depends_on"]["chapkit-ewars-model-init"]["condition"].as_str(),
        Some("service_completed_successfully")
    );
}

#[test]
fn overlay_chowns_to_the_ids_of_the_user_the_model_runs_as() {
    // The simple multistep image runs as `chap`, which is uid/gid 1001.
    let text = render_overlay(&overlay_spec("chapkit_simple_multistep_model"));
    assert!(text.contains("chown -R 1001:1001 /app/data"), "{text}");

    // A numeric --user is passed through, and the data dir follows it.
    let mut spec = overlay_spec("auto_arima_chapkit");
    spec.user = "1500:1600".into();
    spec.data_dir = "/srv/data".into();
    assert!(render_overlay(&spec).contains("chown -R 1500:1600 /srv/data"));

    // An account no image of ours creates falls back to the chapkit ids
    // rather than rendering a chown busybox would reject.
    spec.user = "nobody".into();
    let text = render_overlay(&spec);
    assert!(text.contains("chown -R 1000:1000 /srv/data"), "{text}");
    assert_eq!(
        service(&parse(&text), "auto-arima-chapkit")["user"].as_str(),
        Some("nobody")
    );
}

#[test]
fn an_overlay_pinned_to_a_digest_renders_a_digest_reference() {
    let mut spec = overlay_spec("auto_arima_chapkit");
    let digest = format!("@sha256:{}", "b".repeat(64));
    spec.image_tag = digest.clone();
    spec.version = digest.clone();
    let text = render_overlay(&spec);
    assert_no_tokens(&text);
    assert!(
            text.contains(&format!(
                "    image: ghcr.io/chap-models/auto_arima_chapkit${{AUTO_ARIMA_CHAPKIT_IMAGE_TAG:-{digest}}}\n"
            )),
            "{text}"
        );
    // Compose reads it as one reference, digest and all.
    let image = parse(&text)["services"]["auto-arima-chapkit"]["image"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(image.contains(":-@sha256:"), "{image}");
}

#[test]
fn overlay_pins_the_platform_and_drops_the_line_cleanly_without_one() {
    let inla = render_overlay(&overlay_spec("chapkit_ewars_model"));
    let doc = parse(&inla);
    assert_eq!(
        service(&doc, "chapkit-ewars-model")["platform"].as_str(),
        Some("linux/amd64")
    );

    // The pin is a key and nothing else: why it is there is in the docs.
    assert!(!inla.contains("amd64-only"), "{inla}");

    let mut spec = overlay_spec("chapkit_simple_multistep_model");
    spec.platform = None;
    let portable = render_overlay(&spec);
    let doc = parse(&portable);
    let svc = service(&doc, "chapkit-simple-multistep-model");
    assert!(svc.get("platform").is_none(), "no platform when unset");
    // Dropping the platform line leaves no blank line behind.
    assert!(portable.contains("    image: ghcr.io/chap-models/chapkit_simple_multistep_model:"));
    assert!(!portable.contains("amd64-only"));
    assert!(!portable.contains("\n\n    init: true"));
    assert!(portable.contains("\n    init: true\n"));
    assert_no_tokens(&portable);
}

#[test]
fn overlay_registration_key_is_commented_until_asked_for() {
    let mut spec = overlay_spec("auto_arima_chapkit");
    let commented = render_overlay(&spec);
    assert!(commented.contains("      # Uncomment if chap has SERVICEKIT_REGISTRATION_KEY set:"));
    let doc = parse(&commented);
    assert!(
        service(&doc, "auto-arima-chapkit")["environment"]
            .get("SERVICEKIT_REGISTRATION_KEY")
            .is_none()
    );

    spec.registration_key = true;
    let active = render_overlay(&spec);
    assert_no_tokens(&active);
    assert!(!active.contains("# Uncomment"));
    let doc = parse(&active);
    assert_eq!(
        service(&doc, "auto-arima-chapkit")["environment"]["SERVICEKIT_REGISTRATION_KEY"].as_str(),
        // Left for compose to expand, not resolved at generation time.
        Some("${SERVICEKIT_REGISTRATION_KEY:-}")
    );
}

/// Two lines and no more: the shared one, then the model this file is.
#[test]
fn overlay_header_is_the_shared_line_plus_the_model() {
    let spec = overlay_spec("chapkit_ewars_model");
    let text = render_overlay(&spec);
    let header: Vec<&str> = text.lines().take(3).collect();
    assert_eq!(header[0], GENERATED_HEADER);
    assert_eq!(
        header[1],
        format!(
            "# chapkit_ewars_model {} (https://github.com/chap-models/chapkit_ewars_model)",
            spec.version
        )
    );
    assert_eq!(header[2], "services:", "no third comment line");
    // The tag variable is in the image line, which is the only place it
    // does anything.
    assert!(text.contains(&format!(
        "${{{}:-{}}}",
        tag_env_var("chapkit_ewars_model"),
        spec.image_tag
    )));
}

#[test]
fn base_needs_no_env_file_and_keeps_the_networks_split() {
    let text = render_base(&BaseSpec { upstream: None });
    assert_no_tokens(&text);
    assert_eq!(text.lines().next(), Some(GENERATED_HEADER));
    assert_eq!(text.lines().nth(1), Some("services:"), "one header line");

    // chap-core's test_standalone_compose_needs_no_env_file.
    let bare = regex::Regex::new(r"\$\{[A-Z_]+\}").unwrap();
    assert!(
        !bare.is_match(&text),
        "variable without default: {:?}",
        bare.find(&text).map(|m| m.as_str())
    );

    let doc = parse(&text);
    for name in ["chap", "worker"] {
        let svc = service(&doc, name);
        assert_eq!(
            svc["networks"],
            Value::Sequence(vec!["default".into(), "backend".into()]),
            "{name} must join both networks"
        );
        let image = svc["image"].as_str().unwrap();
        assert!(
            image.ends_with(":${CHAP_IMAGE_TAG:-latest}"),
            "{name} pins {image:?}"
        );
    }
    for name in ["redis", "postgres"] {
        assert_eq!(
            service(&doc, name)["networks"],
            Value::Sequence(vec!["backend".into()]),
            "{name} must stay off the default network"
        );
    }
    let services = doc["services"].as_mapping().unwrap();
    assert_eq!(services.len(), 4);
    for (name, svc) in services {
        assert_eq!(
            svc["restart"].as_str(),
            Some("unless-stopped"),
            "{name:?} has no restart policy"
        );
    }
    assert!(doc["networks"].get("backend").is_some());
}

#[test]
fn base_from_upstream_keeps_the_fetched_body_verbatim() {
    // Only the first line differs between the two renderings; the second
    // is the same note, and everything after it is upstream's.
    let body = "services:\n  chap:\n    image: ghcr.io/x:${CHAP_IMAGE_TAG:-latest}\n";
    let text = render_base(&BaseSpec {
        upstream: Some(UpstreamCompose {
            tag: "v2.3.1".into(),
            body: body.to_string(),
        }),
    });
    assert_no_tokens(&text);
    assert_eq!(
        text,
        format!("{GENERATED_HEADER}\n# chap-core compose.ghcr.yml at v2.3.1\n{body}")
    );
    assert!(text.ends_with('\n'));
    assert_eq!(
        service(&parse(&text), "chap")["image"].as_str(),
        Some("ghcr.io/x:${CHAP_IMAGE_TAG:-latest}")
    );

    // A body without a trailing newline still ends the file cleanly.
    let text = render_base(&BaseSpec {
        upstream: Some(UpstreamCompose {
            tag: "master".into(),
            body: "services: {}".into(),
        }),
    });
    assert!(text.ends_with("services: {}\n"));
    assert!(text.contains("# chap-core compose.ghcr.yml at master\n"));
}

/// Every template carries the same first line as the renderers that write
/// one themselves; if it moves, they all have to move with it.
#[test]
fn every_generated_compose_file_opens_with_the_same_line() {
    for template in [
        &*BASE_TEMPLATE,
        &*OVERLAY_TEMPLATE,
        &*OCS_TEMPLATE,
        &*S3_TEMPLATE,
        &*DHIS2_TEMPLATE,
    ] {
        assert_eq!(template.lines().next(), Some(GENERATED_HEADER));
    }
    for rendered in [
        chaps_overlay(8000, None, None),
        render_umbrella(&[], None),
        render_umbrella(&["compose.a.yml".to_string()], Some("demo-1ab2c3")),
    ] {
        assert_eq!(rendered.lines().next(), Some(GENERATED_HEADER));
    }
}

#[test]
fn umbrella_with_no_models_is_an_empty_service_map() {
    let text = render_umbrella(&[], None);
    let doc = parse(&text);
    assert!(doc.get("include").is_none(), "an empty include is rejected");
    assert_eq!(
        doc["services"],
        Value::Mapping(serde_yaml_ng::Mapping::new())
    );
}

#[test]
fn umbrella_includes_one_line_per_overlay() {
    let one = render_umbrella(&["compose.chapkit-ewars-model.yml".to_string()], None);
    let doc = parse(&one);
    assert_eq!(
        doc["include"],
        Value::Sequence(vec!["compose.chapkit-ewars-model.yml".into()])
    );
    assert!(doc.get("services").is_none());

    let files: Vec<String> = ["a", "b", "c"]
        .iter()
        .map(|n| format!("compose.{n}.yml"))
        .collect();
    let three = render_umbrella(&files, None);
    let doc = parse(&three);
    assert_eq!(
        doc["include"],
        Value::Sequence(files.iter().map(|f| f.as_str().into()).collect())
    );
}

#[test]
fn the_umbrella_names_the_compose_project_too() {
    // It is the one file always in the `-f` list: a deployment with
    // chap-core turned off has no compose.chaps.yml to carry the name.
    for files in [vec![], vec!["compose.chapkit-ewars-model.yml".to_string()]] {
        let text = render_umbrella(&files, Some("mychap-1ab2c3"));
        assert_eq!(
            parse(&text)["name"].as_str(),
            Some("mychap-1ab2c3"),
            "{text}"
        );
    }
    assert!(!render_umbrella(&[], None).contains("name:"));
}

fn ocs_spec() -> OcsSpec {
    OcsSpec {
        host_port: Some(crate::components::OCS_DEFAULT_PORT),
        image_tag: crate::components::OCS_DEFAULT_TAG.to_string(),
        base_url: None,
        s3: false,
        plugins: false,
        group: None,
    }
}

fn s3_spec() -> S3Spec {
    S3Spec {
        host_port: None,
        image_tag: crate::components::S3_DEFAULT_TAG.to_string(),
        group: None,
    }
}

#[test]
fn ocs_publishes_its_port_and_mounts_the_scaffolded_config() {
    let text = render_ocs(&ocs_spec());
    assert_no_tokens(&text);
    assert_eq!(text.lines().next(), Some(GENERATED_HEADER));
    assert_eq!(
        text.lines().nth(1),
        Some("# ocs: Open Climate Service (https://github.com/dhis2/open-climate-service)")
    );

    let doc = parse(&text);
    let svc = service(&doc, "ocs");
    assert_eq!(
        svc["image"].as_str(),
        Some("ghcr.io/dhis2/open-climate-service:${OCS_IMAGE_TAG:-main}")
    );
    // Multi-arch upstream, so no platform pin and no `depends_on` on chap.
    assert!(svc.get("platform").is_none());
    assert!(svc.get("depends_on").is_none());
    assert!(svc.get("networks").is_none());
    // The image ships its own HEALTHCHECK; a second one here would only
    // be a copy to keep in step.
    assert!(svc.get("healthcheck").is_none());
    assert_eq!(svc["restart"].as_str(), Some("unless-stopped"));
    assert_eq!(svc["init"].as_bool(), Some(true));
    assert_eq!(
        svc["expose"],
        Value::Sequence(vec![Value::String("9000".into())])
    );
    assert_eq!(
        svc["ports"],
        Value::Sequence(vec![Value::String("8790:9000".into())])
    );
    // Quoted: an unquoted number is not a string, which is what compose
    // wants of an environment value.
    assert_eq!(svc["environment"]["PORT"].as_str(), Some("9000"));
    assert_eq!(
        svc["environment"]["CLIMATE_SERVICE_CONFIG"].as_str(),
        Some("/app/climate-service.yaml")
    );
    assert!(svc["environment"].get("ROOT_PATH").is_none());
    assert_eq!(
        svc["volumes"][0].as_str(),
        Some("./ocs/climate-service.yaml:/app/climate-service.yaml:ro")
    );
    assert_eq!(svc["volumes"][1]["source"].as_str(), Some(OCS_VOLUME));
    assert_eq!(svc["volumes"][1]["target"].as_str(), Some("/app/data"));
    assert_eq!(
        svc["volumes"].as_sequence().unwrap().len(),
        2,
        "no plugin mount without an ocs/plugins/ to mount"
    );
    assert!(doc["volumes"].get(OCS_VOLUME).is_some());
    // One service: OCS needs no init container of its own.
    assert_eq!(doc["services"].as_mapping().unwrap().len(), 1);
}

/// The five dataset credentials are passed whatever the deployment has:
/// OCS reads each as `os.getenv(...) or <the file>`, so empty is absent.
#[test]
fn ocs_always_gets_the_dataset_credential_variables_as_empty_defaults() {
    let text = render_ocs(&ocs_spec());
    assert_no_tokens(&text);
    let env = &parse(&text)["services"]["ocs"]["environment"];
    for var in crate::components::OCS_DATA_SOURCE_ENV_VARS {
        assert_eq!(
            env[*var].as_str(),
            Some(format!("${{{var}:-}}").as_str()),
            "{var}"
        );
    }
    assert!(text.contains("# Dataset credentials, empty unless .env sets them"));
}

/// The base URL is a `${VAR:-default}` like the image tag: the recorded
/// value is the default, and `.env` can still move it without a sync.
#[test]
fn the_base_url_renders_as_a_default_that_env_can_override() {
    let without = render_ocs(&ocs_spec());
    assert_eq!(
        parse(&without)["services"]["ocs"]["environment"]["CLIMATE_SERVICE_BASE_URL"].as_str(),
        Some("${CLIMATE_SERVICE_BASE_URL:-}"),
        "unset, which OCS reads as `compose the links from the request`"
    );

    let with = render_ocs(&OcsSpec {
        base_url: Some("https://ocs.example.org".to_string()),
        ..ocs_spec()
    });
    assert_no_tokens(&with);
    assert_eq!(
        parse(&with)["services"]["ocs"]["environment"]["CLIMATE_SERVICE_BASE_URL"].as_str(),
        Some("${CLIMATE_SERVICE_BASE_URL:-https://ocs.example.org}")
    );
}

/// The reverse-proxy shape: `expose` and no `ports`, so nothing on the
/// host is bound and the proxy is the only way in.
#[test]
fn ocs_publishes_nothing_when_it_has_no_host_port() {
    let text = render_ocs(&OcsSpec {
        host_port: None,
        base_url: Some("https://ocs.example.org".to_string()),
        ..ocs_spec()
    });
    assert_no_tokens(&text);
    let doc = parse(&text);
    let svc = service(&doc, "ocs");
    assert_eq!(
        svc["expose"],
        Value::Sequence(vec![Value::String("9000".into())])
    );
    assert!(svc.get("ports").is_none(), "{text}");
    // The container port never moves: it is what chap-core and the proxy
    // both reach.
    assert_eq!(svc["environment"]["PORT"].as_str(), Some("9000"));
}

/// The mount only appears when the directory is there: compose refuses to
/// start a service whose bind source does not exist.
#[test]
fn the_plugin_directory_is_mounted_only_when_the_project_has_one() {
    let text = render_ocs(&OcsSpec {
        plugins: true,
        ..ocs_spec()
    });
    assert_no_tokens(&text);
    let doc = parse(&text);
    let volumes = doc["services"]["ocs"]["volumes"].as_sequence().unwrap();
    assert_eq!(
        volumes[1].as_str(),
        Some("./ocs/plugins:/app/plugins:ro"),
        "{text}"
    );
    assert_eq!(volumes.len(), 3, "the config, the plugins, the data volume");
    assert_eq!(volumes[2]["source"].as_str(), Some(OCS_VOLUME));
}

#[test]
fn ocs_gets_the_s3_variables_only_when_the_store_is_enabled() {
    let without = render_ocs(&ocs_spec());
    assert!(!without.contains("S3_ENDPOINT"));
    // Dropping the block must not leave a blank line behind.
    assert!(!without.contains("\n\n    volumes:"));

    let with = render_ocs(&OcsSpec {
        s3: true,
        ..ocs_spec()
    });
    assert_no_tokens(&with);
    assert!(
        with.contains("# Forward-looking: OCS does not read these yet."),
        "{with}"
    );
    let env = &parse(&with)["services"]["ocs"]["environment"];
    assert_eq!(env["S3_ENDPOINT"].as_str(), Some("http://s3:9000"));
    assert_eq!(env["S3_ACCESS_KEY"].as_str(), Some("${S3_ACCESS_KEY:-}"));
    assert_eq!(env["S3_SECRET_KEY"].as_str(), Some("${S3_SECRET_KEY:-}"));
    assert_eq!(env["S3_BUCKET"].as_str(), Some("ocs"));
}

#[test]
fn a_custom_ocs_port_moves_only_the_host_side() {
    let text = render_ocs(&OcsSpec {
        host_port: Some(9010),
        ..ocs_spec()
    });
    assert_eq!(
        parse(&text)["services"]["ocs"]["ports"],
        Value::Sequence(vec![Value::String("9010:9000".into())])
    );
    // The container port is what chap-core reaches, and it never moves.
    assert_eq!(
        parse(&text)["services"]["ocs"]["environment"]["PORT"].as_str(),
        Some("9000")
    );
}

#[test]
fn the_store_is_internal_until_a_port_is_asked_for() {
    let internal = render_s3(&s3_spec());
    assert_no_tokens(&internal);
    let doc = parse(&internal);
    let svc = service(&doc, "s3");
    assert_eq!(
        svc["expose"],
        Value::Sequence(vec![Value::String("9000".into())])
    );
    assert!(svc.get("ports").is_none(), "nothing is published");

    let published = render_s3(&S3Spec {
        host_port: Some(9002),
        ..s3_spec()
    });
    assert_no_tokens(&published);
    let svc = &parse(&published)["services"]["s3"];
    assert_eq!(
        svc["ports"],
        Value::Sequence(vec![Value::String("9002:9000".into())])
    );
    assert_eq!(
        svc["expose"],
        Value::Sequence(vec![Value::String("9000".into())])
    );
}

#[test]
fn the_store_carries_its_credentials_its_health_check_and_a_bucket_init() {
    let text = render_s3(&s3_spec());
    assert_eq!(text.lines().next(), Some(GENERATED_HEADER));
    assert_eq!(
        text.lines().nth(1),
        Some("# s3: RustFS, an S3-compatible object store (https://github.com/rustfs/rustfs)")
    );
    let doc = parse(&text);
    let svc = service(&doc, "s3");
    assert_eq!(
        svc["image"].as_str(),
        Some("rustfs/rustfs:${S3_IMAGE_TAG:-latest}")
    );
    assert!(svc.get("platform").is_none(), "multi-arch upstream");
    assert_eq!(svc["restart"].as_str(), Some("unless-stopped"));
    assert_eq!(svc["init"].as_bool(), Some(true));
    // The image's own variable names, substituted from .env.
    let env = &svc["environment"];
    assert_eq!(
        env["RUSTFS_ACCESS_KEY"].as_str(),
        Some("${S3_ACCESS_KEY:-}")
    );
    assert_eq!(
        env["RUSTFS_SECRET_KEY"].as_str(),
        Some("${S3_SECRET_KEY:-}")
    );
    assert_eq!(env["RUSTFS_VOLUMES"].as_str(), Some("/data"));
    assert_eq!(
        svc["healthcheck"]["test"][1].as_str(),
        Some("curl"),
        "the image ships curl, which is what signs the bucket request too"
    );
    assert_eq!(svc["volumes"][0]["source"].as_str(), Some(S3_VOLUME));
    assert!(doc["volumes"].get(S3_VOLUME).is_some());

    // The one-shot creates the bucket with the same image, so nothing else
    // has to be pulled; `$$` survives for the container's own shell.
    let init = service(&doc, "s3-init");
    assert_eq!(init["restart"].as_str(), Some("no"));
    assert_eq!(
        init["depends_on"]["s3"]["condition"].as_str(),
        Some("service_healthy")
    );
    let command = init["command"][0].as_str().unwrap();
    assert!(
        command.contains("--aws-sigv4 \"aws:amz:us-east-1:s3\""),
        "{command}"
    );
    assert!(
        command.contains("-X PUT \"http://s3:9000/ocs\""),
        "{command}"
    );
    assert!(
        text.contains("$${S3_ACCESS_KEY}"),
        "a literal $ for compose"
    );
    assert_eq!(doc["services"].as_mapping().unwrap().len(), 2);
}

#[test]
fn the_ocs_config_scaffold_says_when_it_is_the_example() {
    let example = render_ocs_config(&OcsConfigSpec::default());
    assert_no_tokens(&example);
    assert!(example.contains(OCS_EXAMPLE_MARKER), "{example}");
    let doc = parse(&example);
    assert_eq!(doc["id"].as_str(), Some("laos-climate-service"));
    assert_eq!(doc["name"].as_str(), Some("Laos Climate Service"));
    assert_eq!(doc["extent"]["name"].as_str(), Some("Laos"));
    assert_eq!(doc["extent"]["country_code"].as_str(), Some("LAO"));
    assert_eq!(doc["extent"]["bbox"][0].as_f64(), Some(100.0));
    assert_eq!(doc["extent"]["bbox"][3].as_f64(), Some(22.5));
    assert_eq!(doc["data_dir"].as_str(), Some("/app/data"));
}

#[test]
fn the_ocs_config_scaffold_fills_the_values_it_is_given() {
    let spec = crate::compose::spec::OcsConfigRequest {
        name: Some("Malawi".into()),
        country_code: Some("mwi".into()),
        bbox: Some("32.6,-17.2,35.9,-9.3".into()),
    }
    .into_spec();
    let text = render_ocs_config(&spec);
    assert_no_tokens(&text);
    assert!(
        !text.contains(OCS_EXAMPLE_MARKER),
        "these are the operator's values"
    );
    let doc = parse(&text);
    assert_eq!(doc["id"].as_str(), Some("malawi-climate-service"));
    assert_eq!(doc["name"].as_str(), Some("Malawi Climate Service"));
    assert_eq!(doc["extent"]["name"].as_str(), Some("Malawi"));
    assert_eq!(doc["extent"]["country_code"].as_str(), Some("MWI"));
    assert_eq!(doc["extent"]["bbox"][1].as_f64(), Some(-17.2));
    assert_eq!(doc["data_dir"].as_str(), Some("/app/data"));
}

/// A seeded deployment publishing the container port on the host: what
/// `init --with dhis2` would ask for.
fn dhis2_spec() -> Dhis2Spec {
    Dhis2Spec {
        host_port: Some(DHIS2_CONTAINER_PORT),
        image_tag: crate::components::DHIS2_DEFAULT_TAG.to_string(),
        image: crate::components::DHIS2_IMAGE.to_string(),
        seed: Some(Dhis2SeedSource::Url(DHIS2_DEFAULT_SEED_URL.to_string())),
        host_gateway: false,
        group: None,
    }
}

/// The same without a seed: an empty DHIS2 that migrates itself.
fn unseeded_dhis2_spec() -> Dhis2Spec {
    Dhis2Spec {
        seed: None,
        ..dhis2_spec()
    }
}

#[test]
fn a_seeded_dhis2_renders_four_services_and_three_volumes() {
    let text = render_dhis2(&dhis2_spec());
    assert_no_tokens(&text);
    assert_eq!(text.lines().next(), Some(GENERATED_HEADER));
    assert!(
        text.lines().nth(1).unwrap().starts_with("# dhis2: DHIS2, "),
        "{text}"
    );
    assert!(text.ends_with('\n'));

    let doc = parse(&text);
    let services = doc["services"].as_mapping().unwrap();
    assert_eq!(services.len(), 4, "{text}");
    for name in ["dhis2", "dhis2-db", "dhis2-dump", "dhis2-prep"] {
        assert!(services.contains_key(name), "no {name}: {text}");
    }
    // Everything but the web service is `dhis2-<something>`, so a container
    // name says which component owns it.
    for name in services.keys() {
        let name = name.as_str().unwrap();
        assert!(
            name == "dhis2" || name.starts_with("dhis2-"),
            "{name} breaks the naming convention"
        );
    }

    let volumes = doc["volumes"].as_mapping().unwrap();
    assert_eq!(volumes.len(), 3, "{text}");
    for volume in [DHIS2_HOME_VOLUME, DHIS2_DB_VOLUME, DHIS2_DUMP_VOLUME] {
        assert!(volumes.contains_key(volume), "{volume} is not declared");
    }

    // Rendering is a pure function of the spec, which is what lets
    // `chaps sync` compare the file byte for byte with what is on disk.
    assert_eq!(render_dhis2(&dhis2_spec()), text);
}

#[test]
fn dhis2_without_a_seed_drops_the_one_shot_the_volume_and_the_wait() {
    let text = render_dhis2(&unseeded_dhis2_spec());
    assert_no_tokens(&text);
    assert!(!text.contains("dhis2-dump"), "{text}");
    assert!(!text.contains("docker-entrypoint-initdb.d"), "{text}");
    assert!(!text.contains(DHIS2_DUMP_VOLUME), "{text}");
    assert!(!text.contains(DHIS2_DEFAULT_SEED_URL), "{text}");

    let doc = parse(&text);
    let services = doc["services"].as_mapping().unwrap();
    assert_eq!(services.len(), 3, "{text}");
    let volumes = doc["volumes"].as_mapping().unwrap();
    assert_eq!(volumes.len(), 2, "{text}");
    // The database waits for nothing once there is no dump to wait for.
    assert!(service(&doc, "dhis2-db").get("depends_on").is_none());
    // And dropping four blocks leaves no blank line and no dangling comment
    // behind.
    assert!(!text.contains("\n\n\n"), "{text}");
    assert!(
        text.ends_with(&format!("  {DHIS2_HOME_VOLUME}: {{}}\n")),
        "{text}"
    );
    assert_eq!(render_dhis2(&unseeded_dhis2_spec()), text);
}

#[test]
fn dhis2_publishes_a_port_only_when_one_was_asked_for() {
    // The reverse-proxy shape: exposed on the compose network, nothing bound
    // on the host.
    let internal = render_dhis2(&Dhis2Spec {
        host_port: None,
        ..dhis2_spec()
    });
    assert_no_tokens(&internal);
    let svc = &parse(&internal)["services"]["dhis2"];
    assert_eq!(
        svc["expose"],
        Value::Sequence(vec![Value::String("8080".into())])
    );
    assert!(svc.get("ports").is_none(), "{internal}");
    // Dropping the ports block must not leave a blank line behind.
    assert!(internal.contains("\n    environment:\n"));
    assert!(!internal.contains("\n\n    environment:"));

    // With a port, both keys: `expose` still documents the container port.
    let published = render_dhis2(&Dhis2Spec {
        host_port: Some(8081),
        ..dhis2_spec()
    });
    assert_no_tokens(&published);
    let svc = &parse(&published)["services"]["dhis2"];
    assert_eq!(
        svc["expose"],
        Value::Sequence(vec![Value::String("8080".into())])
    );
    assert_eq!(
        svc["ports"],
        Value::Sequence(vec![Value::String("8081:8080".into())])
    );
}

/// The container port never moves: 8080 is the one connector the image's
/// `server.xml` declares, and the context path is the root.
#[test]
fn dhis2_pins_no_platform_and_pulls_the_tag_env_can_move() {
    let text = render_dhis2(&dhis2_spec());
    // Both images are multi-arch, so unlike a model overlay nothing here
    // pins an architecture.
    assert!(!text.contains("platform:"), "{text}");

    let doc = parse(&text);
    for name in ["dhis2", "dhis2-db", "dhis2-dump", "dhis2-prep"] {
        assert!(
            service(&doc, name).get("platform").is_none(),
            "{name} pins a platform"
        );
    }
    let svc = service(&doc, "dhis2");
    assert_eq!(
        svc["image"].as_str(),
        Some("dhis2/core:${DHIS2_IMAGE_TAG:-2.42}")
    );
    assert_eq!(svc["restart"].as_str(), Some("unless-stopped"));
    assert_eq!(svc["init"].as_bool(), Some(true));
    // The recorded tag is the variable's default, so the file stands on its
    // own and `.env` still moves the pin without a sync.
    let pinned = render_dhis2(&Dhis2Spec {
        image_tag: "2.43.1.0".to_string(),
        ..dhis2_spec()
    });
    assert!(
        pinned.contains("    image: dhis2/core:${DHIS2_IMAGE_TAG:-2.43.1.0}\n"),
        "{pinned}"
    );
}

/// The config is a file mount inside the volume that covers its directory.
/// Docker orders mounts by destination depth, so `/opt/dhis2` is mounted
/// first and `/opt/dhis2/dhis.conf` lands on top of it; verified against
/// docker itself, not assumed.
#[test]
fn dhis2_mounts_the_config_read_only_inside_the_volume_that_covers_it() {
    let text = render_dhis2(&dhis2_spec());
    let svc = &parse(&text)["services"]["dhis2"];
    let volumes = svc["volumes"].as_sequence().unwrap();
    assert_eq!(volumes.len(), 2, "{text}");
    assert_eq!(
        volumes[0].as_str(),
        Some("./dhis2/dhis.conf:/opt/dhis2/dhis.conf:ro")
    );
    assert_eq!(volumes[1]["source"].as_str(), Some(DHIS2_HOME_VOLUME));
    assert_eq!(volumes[1]["target"].as_str(), Some("/opt/dhis2"));
}

#[test]
fn dhis2_sizes_the_heap_through_java_tool_options() {
    let text = render_dhis2(&dhis2_spec());
    let env = &parse(&text)["services"]["dhis2"]["environment"];
    // JAVA_OPTS is silently ignored by the distroless image variant, which
    // then takes a quarter of the machine's memory as its heap.
    assert!(env.get("JAVA_OPTS").is_none(), "{text}");
    assert_eq!(
        env["JAVA_TOOL_OPTIONS"].as_str(),
        Some("${DHIS2_JAVA_TOOL_OPTIONS:--Xms2g -Xmx4g -XX:+UseG1GC}")
    );
    // And the five variables dhis.conf substitutes are all there, the
    // encryption password included, so no placeholder is left standing as a
    // literal string.
    for var in [
        "DHIS2_DB_HOST",
        "DHIS2_DB_NAME",
        "DHIS2_DB_USER",
        "DHIS2_DB_PASSWORD",
        "DHIS2_ENCRYPTION_PASSWORD",
    ] {
        assert!(env.get(var).is_some(), "{var} is not passed: {text}");
    }
    assert_eq!(
        env["DHIS2_ENCRYPTION_PASSWORD"].as_str(),
        Some("${DHIS2_ENCRYPTION_PASSWORD:-}")
    );
}

#[test]
fn dhis2_waits_for_the_database_and_for_the_prep_one_shot() {
    let doc = parse(&render_dhis2(&dhis2_spec()));
    let depends = &service(&doc, "dhis2")["depends_on"];
    assert_eq!(
        depends["dhis2-db"]["condition"].as_str(),
        Some("service_healthy")
    );
    assert_eq!(
        depends["dhis2-prep"]["condition"].as_str(),
        Some("service_completed_successfully")
    );
    // /api/ping is the one route that answers unauthenticated, and the boot
    // budget is long because a cold start under emulation is minutes.
    let health = &service(&doc, "dhis2")["healthcheck"];
    assert_eq!(
        health["test"],
        Value::Sequence(vec![
            "CMD".into(),
            "curl".into(),
            "-fsS".into(),
            "http://localhost:8080/api/ping".into(),
        ])
    );
    assert_eq!(health["start_period"].as_str(), Some("240s"));
    assert_eq!(health["retries"].as_i64(), Some(60));
}

#[test]
fn the_database_is_postgis_and_answers_over_tcp_once_the_restore_is_done() {
    let doc = parse(&render_dhis2(&dhis2_spec()));
    let svc = service(&doc, "dhis2-db");
    assert_eq!(svc["image"].as_str(), Some(DHIS2_DB_IMAGE));
    assert_eq!(svc["restart"].as_str(), Some("unless-stopped"));
    let env = &svc["environment"];
    assert_eq!(env["POSTGRES_DB"].as_str(), Some("${DHIS2_DB_NAME:-dhis}"));
    assert_eq!(
        env["POSTGRES_USER"].as_str(),
        Some("${DHIS2_DB_USER:-dhis}")
    );
    assert_eq!(
        env["POSTGRES_PASSWORD"].as_str(),
        Some("${DHIS2_DB_PASSWORD:-dhis}")
    );
    // -h makes it a TCP probe. Over the unix socket the entrypoint's own
    // temporary server would answer while the dump was still restoring, and
    // DHIS2 would start against half a database.
    let test = svc["healthcheck"]["test"][1].as_str().unwrap();
    assert!(test.starts_with("pg_isready -h 127.0.0.1 "), "{test}");
    assert!(test.contains("${POSTGRES_USER}"), "{test}");

    // The data directory is a named volume, and the dump volume is mounted
    // where the postgres entrypoint looks for init scripts.
    let volumes = svc["volumes"].as_sequence().unwrap();
    assert_eq!(volumes[0]["source"].as_str(), Some(DHIS2_DB_VOLUME));
    assert_eq!(
        volumes[0]["target"].as_str(),
        Some("/var/lib/postgresql/data")
    );
    assert_eq!(volumes[1]["source"].as_str(), Some(DHIS2_DUMP_VOLUME));
    assert_eq!(
        volumes[1]["target"].as_str(),
        Some("/docker-entrypoint-initdb.d/")
    );
    // And it waits for the dump to be prepared before it starts restoring.
    assert_eq!(
        svc["depends_on"]["dhis2-dump"]["condition"].as_str(),
        Some("service_completed_successfully")
    );
}

#[test]
fn the_dump_one_shot_verifies_the_download_and_retrofits_if_exists() {
    let text = render_dhis2(&dhis2_spec());
    let doc = parse(&text);
    let svc = service(&doc, "dhis2-dump");
    assert_eq!(svc["image"].as_str(), Some(DHIS2_DUMP_IMAGE));
    assert_eq!(svc["restart"].as_str(), Some("no"));
    assert_eq!(
        svc["environment"]["DHIS2_DB_DUMP_URL"].as_str(),
        Some(format!("${{DHIS2_DB_DUMP_URL:-{DHIS2_DEFAULT_SEED_URL}}}").as_str())
    );
    assert_eq!(
        svc["volumes"][0]["source"].as_str(),
        Some(DHIS2_DUMP_VOLUME)
    );
    assert_eq!(svc["volumes"][0]["target"].as_str(), Some("/opt/dump"));

    let script = svc["command"][0].as_str().unwrap();
    // A truncated download must never become the cached dump: verify the
    // gzip, verify the transformed gzip, then publish with one atomic move.
    assert!(script.contains("gzip -t raw.part"), "{script}");
    assert!(script.contains("gzip -t out.part"), "{script}");
    assert!(script.contains("mv out.part dump.sql.gz"), "{script}");
    assert!(
        script.find("gzip -t out.part") < script.find("mv out.part"),
        "the move must come after the check"
    );
    // Published dumps are `pg_dump --clean` without `--if-exists`, and the
    // entrypoint runs init scripts under ON_ERROR_STOP=1.
    assert!(
        script.contains("s/^ALTER TABLE (IF EXISTS )?(ONLY )?/ALTER TABLE IF EXISTS \\2/"),
        "{script}"
    );
    assert!(script.contains("/^DROP EXTENSION /d"), "{script}");
    assert!(script.contains("/^DROP SCHEMA /d"), "{script}");
    assert!(
        script.contains("s/^CREATE SCHEMA (IF NOT EXISTS )?/CREATE SCHEMA IF NOT EXISTS /"),
        "{script}"
    );
    // An already prepared dump is left alone, so a restart re-downloads
    // nothing.
    assert!(script.contains("if [ -f dump.sql.gz ]; then"), "{script}");
    // `$$` in the file is one `$` for the container's shell; a single one
    // would have been expanded away by compose.
    assert!(text.contains("$${DHIS2_DB_DUMP_URL}"), "{text}");
    assert!(text.contains("$$(wc -c < dump.sql.gz)"), "{text}");
}

/// A dump that is a file is bind-mounted and copied; a URL is downloaded.
/// One variable either way, and the script's `[ -f ]` on it is what decides,
/// so both shapes get the same verification and the same rewriting.
#[test]
fn a_file_seed_is_mounted_and_copied_where_a_url_is_downloaded() {
    let text = render_dhis2(&Dhis2Spec {
        seed: Some(Dhis2SeedSource::File("dumps/laos.sql.gz".to_string())),
        ..dhis2_spec()
    });
    assert_no_tokens(&text);
    let doc = parse(&text);
    let svc = service(&doc, "dhis2-dump");
    assert_eq!(
        svc["environment"]["DHIS2_DB_DUMP_URL"].as_str(),
        Some(
            format!(
                "${{{}:-{DHIS2_SEED_MOUNT}}}",
                crate::components::DHIS2_SEED_ENV_VAR
            )
            .as_str()
        ),
        "the value names the mount, not the host path: {text}"
    );
    let volumes = svc["volumes"].as_sequence().unwrap();
    assert_eq!(volumes[0]["source"].as_str(), Some(DHIS2_DUMP_VOLUME));
    assert_eq!(
        volumes[1].as_str(),
        Some(format!("./dumps/laos.sql.gz:{DHIS2_SEED_MOUNT}:ro").as_str()),
        "a relative path is spelled with ./, as compose documents it"
    );
    // The branch that reads it, and the download that is still there for a URL.
    let script = svc["command"][0].as_str().unwrap();
    assert!(
        script.contains("if [ -f \"$${DHIS2_DB_DUMP_URL}\" ]; then"),
        "{script}"
    );
    assert!(
        script.contains("cp \"$${DHIS2_DB_DUMP_URL}\" raw.part"),
        "{script}"
    );
    assert!(
        script.contains("wget -O raw.part \"$${DHIS2_DB_DUMP_URL}\""),
        "{script}"
    );
    // Everything after the fetch is shared, so an operator's own dump is
    // verified and rewritten exactly as a published one is.
    assert!(script.contains("gzip -t raw.part"), "{script}");
    assert!(script.contains("mv out.part dump.sql.gz"), "{script}");

    // An absolute path is passed through, and a URL adds no mount at all.
    let absolute = render_dhis2(&Dhis2Spec {
        seed: Some(Dhis2SeedSource::File("/srv/dumps/laos.sql.gz".to_string())),
        ..dhis2_spec()
    });
    assert!(
        absolute.contains(&format!(
            "      - /srv/dumps/laos.sql.gz:{DHIS2_SEED_MOUNT}:ro\n"
        )),
        "{absolute}"
    );
    // A Windows path with a drive is absolute too, on any host, and comes
    // out with the forward slashes compose reads on every platform.
    let drive = render_dhis2(&Dhis2Spec {
        seed: Some(Dhis2SeedSource::File(r"C:\dumps\laos.sql.gz".to_string())),
        ..dhis2_spec()
    });
    assert!(
        drive.contains(&format!(
            "      - C:/dumps/laos.sql.gz:{DHIS2_SEED_MOUNT}:ro\n"
        )),
        "{drive}"
    );
    let url = render_dhis2(&dhis2_spec());
    assert!(!url.contains(DHIS2_SEED_MOUNT), "{url}");
    assert_eq!(
        parse(&url)["services"]["dhis2-dump"]["volumes"]
            .as_sequence()
            .unwrap()
            .len(),
        1,
        "the dump volume and nothing else"
    );
    // Dropping the mount leaves no blank line in the volume list.
    assert!(
        url.contains("        target: /opt/dump\n    entrypoint:"),
        "{url}"
    );

    // The scheme is the whole of the rule, and nothing touches the disk to
    // apply it: a dump the operator has not copied in yet still renders.
    assert_eq!(
        Dhis2SeedSource::of(DHIS2_DEFAULT_SEED_URL),
        Dhis2SeedSource::Url(DHIS2_DEFAULT_SEED_URL.to_string())
    );
    assert_eq!(
        Dhis2SeedSource::of("dumps/x.sql.gz"),
        Dhis2SeedSource::File("dumps/x.sql.gz".to_string())
    );
}

/// The `.env` block names the variables the rendered file actually reads, so
/// the constants and the template cannot drift apart: a pin an operator
/// uncomments has to be one compose substitutes.
#[test]
fn the_dhis2_env_variables_are_the_ones_the_compose_file_reads() {
    let text = render_dhis2(&dhis2_spec());
    for var in [
        crate::components::DHIS2_TAG_ENV_VAR,
        crate::components::DHIS2_DB_PASSWORD_ENV_VAR,
        crate::components::DHIS2_ENCRYPTION_PASSWORD_ENV_VAR,
        crate::components::DHIS2_SEED_ENV_VAR,
        crate::components::DHIS2_JAVA_ENV_VAR,
    ] {
        assert!(
            text.contains(&format!("${{{var}:-")),
            "{var} is not a default the file reads: {text}"
        );
    }
    // And the value `.env` offers as the starting point is the one the file
    // already defaults to, so uncommenting the line changes nothing.
    assert!(
        text.contains(&format!(
            "${{{}:-{}}}",
            crate::components::DHIS2_JAVA_ENV_VAR,
            crate::components::DHIS2_DEFAULT_JAVA_OPTIONS
        )),
        "{text}"
    );
}

/// Every volume a backup archives has to be read from where the compose file
/// mounts it, so the `data_dir`s in that table come from this file rather
/// than from memory.
#[test]
fn the_archived_dhis2_volumes_are_mounted_where_the_backup_table_says() {
    let doc = parse(&render_dhis2(&dhis2_spec()));
    let target = |service: &str, volume: &str| -> String {
        doc["services"][service]["volumes"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|mount| mount.get("source").and_then(|s| s.as_str()) == Some(volume))
            .unwrap_or_else(|| panic!("{service} mounts {volume}"))["target"]
            .as_str()
            .unwrap()
            .to_string()
    };
    for part in crate::backup::COMPONENT_VOLUMES
        .iter()
        .filter(|part| part.name == "dhis2")
    {
        assert_eq!(
            target(part.service, part.volume),
            part.data_dir,
            "{} is mounted elsewhere",
            part.volume
        );
    }
}

#[test]
fn the_prep_one_shot_analyzes_and_clears_a_job_left_running() {
    let doc = parse(&render_dhis2(&dhis2_spec()));
    let svc = service(&doc, "dhis2-prep");
    // The same image as the database, so nothing extra is pulled.
    assert_eq!(svc["image"].as_str(), Some(DHIS2_DB_IMAGE));
    assert_eq!(svc["restart"].as_str(), Some("no"));
    assert_eq!(svc["entrypoint"], Value::Sequence(vec!["psql".into()]));
    let command: Vec<&str> = svc["command"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    // ON_ERROR_STOP=0: a database DHIS2 has not migrated yet has no
    // jobconfiguration table, and that has to be a no-op.
    assert_eq!(command[0..2], ["-v", "ON_ERROR_STOP=0"]);
    assert!(
        command.contains(
            &"UPDATE jobconfiguration SET jobstatus='SCHEDULED' WHERE jobstatus='RUNNING';"
        ),
        "{command:?}"
    );
    assert!(command.contains(&"ANALYZE;"), "{command:?}");
    assert_eq!(
        svc["depends_on"]["dhis2-db"]["condition"].as_str(),
        Some("service_healthy")
    );
}

#[test]
fn the_dhis2_config_scaffold_keeps_the_placeholders_dhis2_substitutes() {
    let text = render_dhis2_config(&Dhis2ConfigSpec::default());
    assert_no_tokens(&text);
    assert_eq!(
        text.lines().next(),
        Some(
            "# Written once by chaps when the dhis2 component was enabled; \
                 chaps never rewrites it."
        )
    );
    assert!(text.ends_with('\n'));

    // The three mandatory keys and the placeholders DHIS2 replaces from its
    // own environment: filling them here would be filling them twice.
    assert!(
        text.contains("\nconnection.url = jdbc:postgresql://${DHIS2_DB_HOST}/${DHIS2_DB_NAME}\n"),
        "{text}"
    );
    assert!(
        text.contains("\nconnection.username = ${DHIS2_DB_USER}\n"),
        "{text}"
    );
    assert!(
        text.contains("\nconnection.password = ${DHIS2_DB_PASSWORD}\n"),
        "{text}"
    );
    // Not required to boot, but unset DHIS2 encrypts with a password
    // compiled into its own source, and under 24 characters it refuses.
    assert!(
        text.contains("\nencryption.password = ${DHIS2_ENCRYPTION_PASSWORD}\n"),
        "{text}"
    );
    // The driver class already defaults correctly, so it is there as a
    // comment rather than as a value to keep in step.
    assert!(text.contains("\n# connection.driver_class = "), "{text}");
    assert!(text.contains("\nconnection.dialect = "), "{text}");

    // The one value the spec decides: every http and https target, so a
    // chap-core anywhere works without editing the file.
    assert!(
        text.contains("\nroute.remote_servers_allowed = http://*,https://*\n"),
        "{text}"
    );
    assert_eq!(render_dhis2_config(&Dhis2ConfigSpec::default()), text);
}

#[test]
fn the_route_allowlist_is_whatever_the_caller_narrowed_it_to() {
    let text = render_dhis2_config(&Dhis2ConfigSpec {
        route_allowed: "https://chap.example.org".to_string(),
    });
    assert_no_tokens(&text);
    assert!(
        text.contains("\nroute.remote_servers_allowed = https://chap.example.org\n"),
        "{text}"
    );
    assert!(!text.contains("http://chap:8000"));
    // The reason the value is not just left at DHIS2's default, in the file
    // the operator reads, and the one rule it has to obey.
    assert!(text.contains("may carry a path"), "{text}");
}

fn env_spec() -> EnvSpec {
    EnvSpec {
        postgres_user: "chap".into(),
        postgres_password: "0123456789abcdef0123456789abcdef".into(),
        postgres_db: "chap_core".into(),
        chap_image_tag: None,
        api_port: crate::project::DEFAULT_API_PORT,
        api_token: None,
        registration_key: None,
        model_tag_pins: Vec::new(),
    }
}

#[test]
fn env_writes_the_postgres_credentials() {
    let text = render_env(&env_spec());
    assert_no_tokens(&text);
    // Its own first line: `.env` is written once, not rendered from
    // `.chaps/` like the compose files.
    assert_eq!(
        text.lines().next(),
        Some(
            "# Written once by chaps init; chaps never rewrites it. \
                 See the docs for each setting."
        )
    );
    assert!(text.contains("\nPOSTGRES_USER=chap\n"));
    assert!(text.contains("\nPOSTGRES_PASSWORD=0123456789abcdef0123456789abcdef\n"));
    assert!(text.contains("\nPOSTGRES_DB=chap_core\n"));
    assert!(text.contains("\n# CHAP_API_TOKEN=\n"));
    assert!(text.contains("\n# SERVICEKIT_REGISTRATION_KEY=\n"));
    assert!(text.contains("\n# CHAP_DATABASE_URL=\n"));
    assert!(text.ends_with('\n'));
}

#[test]
fn env_carries_an_active_api_port_line_at_every_port() {
    // Active even at the default: the one port the stack publishes has to
    // be discoverable where an operator would change it.
    let text = render_env(&env_spec());
    assert!(text.contains("\nCHAP_API_PORT=8700\n"), "{text}");
    assert!(!text.contains("# CHAP_API_PORT"));

    let text = render_env(&EnvSpec {
        api_port: 8123,
        ..env_spec()
    });
    assert!(text.contains("\nCHAP_API_PORT=8123\n"));
}

#[test]
fn env_activates_the_chap_tag_only_when_it_is_pinned() {
    for tag in [None, Some("latest".to_string())] {
        let text = render_env(&EnvSpec {
            chap_image_tag: tag.clone(),
            ..env_spec()
        });
        assert!(text.contains("\n# CHAP_IMAGE_TAG=latest\n"), "{tag:?}");
    }
    let text = render_env(&EnvSpec {
        chap_image_tag: Some("v1.2.3".into()),
        ..env_spec()
    });
    assert!(text.contains("\nCHAP_IMAGE_TAG=v1.2.3\n"));
    assert!(!text.contains("# CHAP_IMAGE_TAG"));
}

#[test]
fn env_lists_one_commented_pin_per_model() {
    let text = render_env(&EnvSpec {
        model_tag_pins: vec![
            (tag_env_var("chapkit_ewars_model"), "sha-1111111".into()),
            (tag_env_var("auto_arima_chapkit"), "sha-2222222".into()),
        ],
        ..env_spec()
    });
    assert!(text.contains("\n# CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-1111111\n"));
    assert!(text.contains("\n# AUTO_ARIMA_CHAPKIT_IMAGE_TAG=sha-2222222\n"));
    assert!(!text.contains("none yet"));

    // Without models the section keeps a body rather than a blank line.
    let empty = render_env(&env_spec());
    assert!(empty.contains(NO_TAG_PINS));
    assert!(!empty.ends_with("\n\n"));
}

#[test]
fn env_leaves_both_secrets_commented_without_authentication() {
    let text = render_env(&env_spec());
    assert!(text.contains("\n# CHAP_API_TOKEN=\n"), "{text}");
    assert!(
        text.contains("\n# SERVICEKIT_REGISTRATION_KEY=\n"),
        "{text}"
    );
    assert_eq!(
        crate::auth::state_of(&text),
        crate::project::AuthState::default()
    );
}

#[test]
fn env_writes_both_secrets_as_active_lines_when_authentication_is_on() {
    let token = "a".repeat(64);
    let key = "b".repeat(64);
    let text = render_env(&EnvSpec {
        api_token: Some(token.clone()),
        registration_key: Some(key.clone()),
        ..env_spec()
    });
    assert_no_tokens(&text);
    assert!(
        text.contains(&format!("\nCHAP_API_TOKEN={token}\n")),
        "{text}"
    );
    assert!(
        text.contains(&format!("\nSERVICEKIT_REGISTRATION_KEY={key}\n")),
        "{text}"
    );
    // No placeholder left behind for either one.
    assert!(!text.contains("# CHAP_API_TOKEN="));
    assert!(!text.contains("# SERVICEKIT_REGISTRATION_KEY="));
    // And the rendered file reads back as "both on".
    assert_eq!(
        crate::auth::state_of(&text),
        crate::project::AuthState {
            api_token: true,
            registration_key: true
        }
    );
}

#[test]
fn a_secret_line_is_an_assignment_or_a_placeholder() {
    assert_eq!(
        secret_line("CHAP_API_TOKEN", Some("abc")),
        "CHAP_API_TOKEN=abc"
    );
    assert_eq!(secret_line("CHAP_API_TOKEN", None), "# CHAP_API_TOKEN=");
}
