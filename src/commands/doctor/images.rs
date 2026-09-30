//! Checks of the images the deployment pulls, the users they run as, and whether it is up.

use super::*;

/// One enabled model's image, from what `docker manifest inspect` said.
pub fn image_verdict(
    marketplace_id: &str,
    service_id: &str,
    reference: &str,
    outcome: &Outcome,
) -> Check {
    let id = format!("image-{service_id}");
    let name = format!("image {service_id}");
    match outcome {
        Outcome::Done {
            ok: true, stdout, ..
        } => match manifest_has_amd64(stdout) {
            Some(false) => Check::warn(
                id,
                name,
                format!("{reference} has no linux/amd64 image"),
                format!(
                    "the overlay pins platform: linux/amd64, so this tag cannot run; \
                     re-resolve it with `chaps models enable {marketplace_id}`"
                ),
            ),
            Some(true) => Check::ok(id, name, format!("{reference} (linux/amd64)")),
            None => Check::ok(id, name, format!("{reference} exists")),
        },
        Outcome::TimedOut => Check::skip(
            id,
            name,
            format!(
                "ghcr.io did not answer for {reference} within {}s",
                MANIFEST_TIMEOUT.as_secs()
            ),
        ),
        Outcome::Missing => Check::skip(id, name, "no docker CLI to ask"),
        Outcome::Failed(why) => Check::skip(id, name, why.clone()),
        Outcome::Done { stderr, .. } if stderr.to_ascii_lowercase().contains("experimental") => {
            Check::skip(
                id,
                name,
                "this docker CLI has `docker manifest` behind its experimental flag",
            )
        }
        Outcome::Done { stderr, .. } => Check::fail(
            id,
            name,
            format!("{reference} was not found: {}", first_line(stderr)),
            format!("run `chaps models enable {marketplace_id}` to resolve the pin again"),
        ),
    }
}

/// Why the image manifests were not looked up, or `None` when they were.
///
/// `--offline` is answered first and on its own: it is the reason the user
/// gave, it holds whether or not this machine has a docker CLI, and a
/// checklist that said "no docker CLI to ask" on a runner without Docker but
/// "--offline" on a laptop with it would report the same run two ways. A
/// missing CLI is the reason only for a run that did go looking, where it is
/// what stopped the lookup; `docker-cli` has failed on its own line by then.
pub fn image_skip_reason(probed: Option<&Probed>, have_cli: bool) -> Option<String> {
    let Some(probed) = probed else {
        return Some("--offline: the registry was not asked".to_string());
    };
    if !have_cli {
        return Some("no docker CLI to ask".to_string());
    }
    probed
        .ghcr
        .as_ref()
        .err()
        .map(|why| format!("ghcr.io is unreachable: {why}"))
}

/// One enabled component's image, from what `docker manifest inspect` said.
///
/// Unlike a model overlay, a component pins no platform: both images are
/// multi-arch, so "the tag exists" is the whole question here.
pub fn component_image_verdict(name: &str, reference: &str, outcome: &Outcome) -> Check {
    let id = format!("image-{name}");
    let check_name = format!("image {name}");
    match outcome {
        Outcome::Done { ok: true, .. } => Check::ok(id, check_name, format!("{reference} exists")),
        Outcome::TimedOut => Check::skip(
            id,
            check_name,
            format!(
                "the registry did not answer for {reference} within {}s",
                MANIFEST_TIMEOUT.as_secs()
            ),
        ),
        Outcome::Missing => Check::skip(id, check_name, "no docker CLI to ask"),
        Outcome::Failed(why) => Check::skip(id, check_name, why.clone()),
        Outcome::Done { stderr, .. } if stderr.to_ascii_lowercase().contains("experimental") => {
            Check::skip(
                id,
                check_name,
                "this docker CLI has `docker manifest` behind its experimental flag",
            )
        }
        Outcome::Done { stderr, .. } => Check::fail(
            id,
            check_name,
            format!("{reference} was not found: {}", first_line(stderr)),
            format!(
                "check the tag: `{OCS_TAG_ENV_VAR}`, `{S3_TAG_ENV_VAR}` and \
                 `{DHIS2_TAG_ENV_VAR}` in .env pin the component images"
            ),
        ),
    }
}

/// The images the enabled components pull, as `(component, reference)`.
///
/// One image per component: the one whose tag a deployment can move. A
/// component that is several services pulls more than that - `dhis2` brings a
/// PostGIS database and an alpine one-shot beside its web image - but those two
/// references are exact and compiled into this binary, the same on every
/// deployment, so a check on them would spend a registry round trip per
/// `chaps doctor` to report a chaps bug as the operator's problem, and the
/// `fix` line would name a variable that does not exist. chap-core's own
/// postgres and valkey are left out on the same rule.
pub fn component_images(components: &Components) -> Vec<(String, String)> {
    let mut images = Vec::new();
    if components.ocs.enabled {
        images.push((
            crate::compose::OCS_SERVICE.to_string(),
            format!("{OCS_IMAGE}:{}", components.ocs.image_tag),
        ));
    }
    if components.s3.enabled {
        images.push((
            crate::compose::S3_SERVICE.to_string(),
            format!("{S3_IMAGE}:{S3_DEFAULT_TAG}"),
        ));
    }
    if components.dhis2.enabled {
        images.push((
            crate::compose::DHIS2_SERVICE.to_string(),
            format!("{}:{}", components.dhis2.image, components.dhis2.image_tag),
        ));
    }
    images
}

/// One line per enabled model: does the user its overlay runs it as still
/// match what the image declares.
///
/// The one class of breakage `chaps sync` cannot see: the recorded user is
/// what the overlay renders from, and an image that runs as root under a
/// `user: 1000:1000` starts fine and then fails on its own binaries - the
/// Rwanda BYM model's `inla.run: Permission denied`, up to its 0.1.1 pin. It
/// is also what a model that has moved the other way looks like, which is why
/// the check compares rather than looks for root. Only a locally pulled
/// image can be asked without a network round trip per model, so a model whose
/// amd64 image is not in this machine's image store is skipped rather than
/// guessed at.
pub(super) fn user_checks(project: &Project, have_cli: bool) -> Vec<Check> {
    user_checks_with(project, have_cli, &crate::docker::image_config)
}

/// [`user_checks`] with the daemon injected, so the verdicts can be tested.
fn user_checks_with(
    project: &Project,
    have_cli: bool,
    declared: &dyn Fn(&str) -> Option<(String, String)>,
) -> Vec<Check> {
    project
        .state
        .models
        .iter()
        .map(|(id, model)| {
            let reference = crate::compose::image_ref(&model.image, &model.image_tag);
            let found = have_cli
                .then(|| declared(&reference))
                .flatten()
                .map(|(user, _)| user);
            let asked = have_cli.then_some(reference.as_str());
            user_check(id, &model.service_id, asked, &model.user, found.as_deref())
        })
        .collect()
}

/// The verdict for one model, given what its image declares.
///
/// `declared` is `None` when the image the overlay pins is not in this
/// machine's image store as the `linux/amd64` variant every overlay runs -
/// the usual state before the first `chaps up`, and no reason to say anything
/// is wrong. `reference` is the tag that was asked about, or `None` when
/// there was no docker CLI to ask through.
pub fn user_check(
    id: &str,
    service_id: &str,
    reference: Option<&str>,
    recorded: &str,
    declared: Option<&str>,
) -> Check {
    let check_id = format!("user-{service_id}");
    let name = format!("user {service_id}");
    let Some(declared) = declared else {
        let Some(reference) = reference else {
            return Check::skip(check_id, name, format!("{recorded}; no docker CLI to ask"));
        };
        return Check::skip_with(
            check_id,
            name,
            format!("{recorded}; the amd64 variant of {reference} is not in the local image store"),
            format!("run `chaps docker pull`, or `docker pull --platform linux/amd64 {reference}`"),
        );
    };
    let wanted = crate::compose::resolve::normalize(declared);
    // An account name only the image itself can resolve says nothing about
    // whether the numbers beside it are right.
    if crate::compose::overrides::numeric_pair(&wanted).is_none() {
        return Check::skip(
            check_id,
            name,
            format!("the image runs as `{wanted}`, which only the image can turn into numbers"),
        );
    }
    if crate::compose::resolve::normalize(recorded) == wanted {
        return Check::ok(check_id, name, format!("{recorded}, as the image declares"));
    }
    Check::warn(
        check_id,
        name,
        format!("{recorded}, but the image runs as {wanted}"),
        format!(
            "run `chaps models enable {id}` to read the user off the image again, or `chaps update`"
        ),
    )
}

/// One line per enabled model, asked in parallel.
pub(super) fn image_checks(
    project: &Project,
    probed: Option<&Probed>,
    have_cli: bool,
) -> Vec<Check> {
    // Marketplace id, service id and the exact reference the overlay pins.
    let models: Vec<(String, String, String)> = project
        .state
        .models
        .iter()
        .map(|(id, model)| {
            (
                id.clone(),
                model.service_id.clone(),
                crate::compose::image_ref(&model.image, &model.image_tag),
            )
        })
        .collect();
    let components = component_images(&project.state.components);

    if let Some(reason) = image_skip_reason(probed, have_cli) {
        let model_skips = models.iter().map(|(_, service_id, _)| service_id);
        let component_skips = components.iter().map(|(name, _)| name);
        return model_skips
            .chain(component_skips)
            .map(|name| {
                Check::skip(
                    format!("image-{name}"),
                    format!("image {name}"),
                    reason.clone(),
                )
            })
            .collect();
    }

    // One thread per image: each is a registry round trip, and a deployment
    // with four models would otherwise wait out four of them in a row.
    std::thread::scope(|scope| {
        let references: Vec<String> = models
            .iter()
            .map(|(_, _, reference)| reference.clone())
            .chain(components.iter().map(|(_, reference)| reference.clone()))
            .collect();
        let handles: Vec<_> = references
            .iter()
            .map(|reference| {
                let reference = reference.clone();
                scope.spawn(move || {
                    run_bounded(
                        "docker",
                        &["manifest", "inspect", &reference],
                        MANIFEST_TIMEOUT,
                    )
                })
            })
            .collect();
        let mut outcomes = handles.into_iter().map(|handle| {
            handle
                .join()
                .unwrap_or_else(|_| Outcome::Failed("the probe did not finish".to_string()))
        });
        let mut checks: Vec<Check> = models
            .iter()
            .zip(outcomes.by_ref())
            .map(|((marketplace_id, service_id, reference), outcome)| {
                image_verdict(marketplace_id, service_id, reference, &outcome)
            })
            .collect();
        checks.extend(
            components
                .iter()
                .zip(outcomes)
                .map(|((name, reference), outcome)| {
                    component_image_verdict(name, reference, &outcome)
                }),
        );
        checks
    })
}

/// What a deployment without chap-core adds up to: its components alone.
fn components_only_verdict(report: &StatusReport) -> (Status, String, Option<String>) {
    // Models without chap-core are judged by their own /health, the same rows
    // `chaps status` prints, and count beside the components.
    let models = report
        .models
        .iter()
        .map(|m| (m.id.as_str(), m.state == crate::status::ModelState::Up));
    let (up, down): (Vec<&str>, Vec<&str>) = report
        .components
        .iter()
        .map(|c| (c.name.as_str(), c.state == ComponentState::Up))
        .chain(models)
        .fold(
            (Vec::new(), Vec::new()),
            |(mut up, mut down), (name, is_up)| {
                if is_up {
                    up.push(name);
                } else {
                    down.push(name);
                }
                (up, down)
            },
        );
    if down.is_empty() {
        return (
            Status::Ok,
            format!("no chap-core here; up: {}", up.join(", ")),
            None,
        );
    }
    (
        Status::Warn,
        format!("no chap-core here; not up: {}", down.join(", ")),
        Some("run `chaps status` for what each one is doing".to_string()),
    )
}

/// What the running deployment adds up to.
pub fn stack_verdict(report: &StatusReport) -> (Status, String, Option<String>) {
    match &report.api {
        ApiHealth::Off => components_only_verdict(report),
        // A container that is up and unhealthy is a sharper answer than "the
        // port does not answer", and its own last error line is sharper still.
        ApiHealth::Down { .. } if report.api_container_unhealthy() => {
            let cause = crate::diagnose::headline(&report.unhealthy)
                .map(|line| format!(": {line}"))
                .unwrap_or_default();
            (
                Status::Fail,
                format!(
                    "the chap container is up and unhealthy{}",
                    first_line(&cause)
                ),
                Some(
                    report
                        .unhealthy
                        .iter()
                        .find_map(|entry| entry.hint.clone())
                        .unwrap_or_else(|| "run `chaps logs chap` to see why".to_string()),
                ),
            )
        }
        ApiHealth::Down { error } => (
            Status::Fail,
            // Whatever its container last said about itself beats the
            // connection error out here, which is only the symptom.
            match crate::diagnose::headline(&report.unhealthy) {
                Some(line) => format!(
                    "chap-core at {} is down: {}",
                    report.api_url,
                    first_line(&line)
                ),
                None => format!("chap-core at {} is down: {error}", report.api_url),
            },
            Some(
                report
                    .unhealthy
                    .iter()
                    .find_map(|entry| entry.hint.clone())
                    .unwrap_or_else(|| {
                        "run `chaps logs chap` to see why, and `chaps status` for the detail"
                            .to_string()
                    }),
            ),
        ),
        ApiHealth::Rejected { .. } => (
            Status::Fail,
            format!(
                "chap-core at {} is up and did not accept the API token",
                report.api_url
            ),
            Some(
                "`chaps auth show --reveal` prints the token in .env; after `chaps auth \
                 rotate` or `enable`, `chaps up` hands it to chap-core"
                    .to_string(),
            ),
        ),
        ApiHealth::Up { .. } if report.missing.is_empty() => (
            Status::Ok,
            format!(
                "chap-core up at {}, {} of {} models registered",
                report.api_url,
                report.expected.len(),
                report.expected.len()
            ),
            // Registration is a heartbeat, so a green line here is not proof
            // that any of these models can produce a prediction. There is a
            // check that settles it, and `doctor` cannot make it: it runs the
            // models, which takes minutes.
            (!report.expected.is_empty()).then(|| crate::status::TEST_HINT.to_string()),
        ),
        ApiHealth::Up { .. } => (
            Status::Warn,
            format!(
                "chap-core up at {}, not registered: {}",
                report.api_url,
                report.missing.join(", ")
            ),
            Some("run `chaps status` for what to do about each one".to_string()),
        ),
    }
}

/// The `stack` line, which has nothing to check when nothing is up.
pub(super) fn stack_check(
    project: &Project,
    containers: Option<&[docker::Container]>,
    running: &BTreeSet<String>,
) -> Check {
    const ID: &str = "health";
    const NAME: &str = "health";
    if running.is_empty() {
        return Check::skip_with(
            ID,
            NAME,
            "no container of this project is running",
            "run `chaps up` to start CHAP",
        );
    }
    let url = project.api_url();
    crate::output::verbose(&format!("asking chap-core at {url}"));
    let token = crate::api::token_for(Some(&project.dir));
    let mut report = crate::status::status(project, &url, STACK_TIMEOUT, running, token.as_deref());
    // The same diagnosis `chaps status` makes: when the API does not answer,
    // its container has been saying why in its own log.
    if matches!(report.api, ApiHealth::Down { .. })
        && let Some(containers) = containers
    {
        report.unhealthy =
            crate::diagnose::failing_containers(project, containers, Some(API_SERVICE));
    }
    Check::from_verdict(ID, NAME, stack_verdict(&report))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;
    use crate::status::{ApiVersion, ModelStatus};
    use std::collections::BTreeMap;

    /// The tag the `user` checks are asked about in these tests.
    const IMAGE_REF: &str = "ghcr.io/chap-models/chapkit_ewars_model:sha-fa880a1";

    /// The `user <service>` line: what the overlay runs the model as, against
    /// what the pulled image says it runs as.
    #[test]
    fn a_models_user_is_checked_against_the_image_it_pins() {
        // The image declares `USER chapkit`; the entry records the numbers
        // that resolves to, so the two agree.
        let ok = user_check(
            "chapkit_ewars_model",
            "chapkit-ewars-model",
            Some(IMAGE_REF),
            "1000:1000",
            Some("chapkit"),
        );
        assert_eq!(ok.status, Status::Ok);
        assert_eq!(ok.id, "user-chapkit-ewars-model");
        assert_eq!(ok.name, "user chapkit-ewars-model");
        assert!(ok.detail.contains("as the image declares"), "{ok:?}");
        assert_eq!(ok.fix, None);

        // Root, however it is spelled on either side. An empty `User` is one
        // of those spellings: the parser has already ruled out the empty
        // config that only looks like root.
        for declared in ["root", "", "0:0"] {
            let check = user_check("m", "m", Some(IMAGE_REF), "root", Some(declared));
            assert_eq!(check.status, Status::Ok, "{declared:?} {check:?}");
        }

        // The bug this check exists for: an image that runs as root under an
        // overlay that hands it an unprivileged uid.
        let wrong = user_check(
            "auto_arima_chapkit",
            "auto-arima-chapkit",
            Some(IMAGE_REF),
            "1000:1000",
            Some("root"),
        );
        assert_eq!(wrong.status, Status::Warn);
        assert!(
            wrong
                .detail
                .contains("1000:1000, but the image runs as root"),
            "{wrong:?}"
        );
        assert_eq!(
            wrong.fix.as_deref(),
            Some(
                "run `chaps models enable auto_arima_chapkit` to read the \
                 user off the image again, or `chaps update`"
            )
        );

        // Nothing to ask: the amd64 variant is not in the image store, which
        // is also what an arm64 host that has pulled nothing yet looks like.
        let absent = user_check("m", "m", Some(IMAGE_REF), "1000:1000", None);
        assert_eq!(absent.status, Status::Skip);
        assert!(
            absent
                .detail
                .contains(&format!("the amd64 variant of {IMAGE_REF} is not in the")),
            "{absent:?}"
        );
        assert_eq!(
            absent.fix.as_deref(),
            Some(
                "run `chaps docker pull`, or `docker pull --platform linux/amd64 \
                 ghcr.io/chap-models/chapkit_ewars_model:sha-fa880a1`"
            )
        );

        // No docker CLI at all is a different reason, and no pull would fix
        // it.
        let no_cli = user_check("m", "m", None, "1000:1000", None);
        assert_eq!(no_cli.status, Status::Skip);
        assert!(no_cli.detail.contains("no docker CLI"), "{no_cli:?}");
        assert_eq!(no_cli.fix, None);

        // And an account name only the image can resolve proves nothing about
        // the numbers recorded beside it.
        let opaque = user_check("m", "m", Some(IMAGE_REF), "10001:10001", Some("app"));
        assert_eq!(opaque.status, Status::Skip);
        assert!(opaque.detail.contains("`app`"), "{opaque:?}");
    }

    /// One line per enabled model, and none at all without a docker CLI to
    /// ask.
    #[test]
    fn the_user_checks_cover_every_enabled_model() {
        let entry = crate::project::EnabledModel {
            service_id: "chapkit-ewars-model".into(),
            image: "ghcr.io/chap-models/chapkit_ewars_model".into(),
            image_tag: "sha-fa880a1".into(),
            version: "1.0.0".into(),
            channel: None,
            host_port: None,
            data_dir: "/app/data".into(),
            user: "1000:1000".into(),
            user_from: Default::default(),
            platform: None,
            compose_file: "compose.chapkit-ewars-model.yml".into(),
        };
        let mut project = Project {
            dir: std::path::PathBuf::from("/tmp/chapx"),
            state: crate::project::ProjectState {
                models: std::collections::BTreeMap::from([(
                    "chapkit_ewars_model".to_string(),
                    entry,
                )]),
                ..crate::project::ProjectState::default()
            },
        };
        let checks = user_checks_with(&project, true, &|reference| {
            reference
                .contains("ewars")
                .then(|| ("root".to_string(), "/app".to_string()))
        });
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].id, "user-chapkit-ewars-model");
        assert_eq!(checks[0].status, Status::Warn);

        // Without a docker CLI nothing is asked, but the line is still there.
        let skipped = user_checks_with(&project, false, &|_| panic!("no CLI, no question"));
        assert_eq!(skipped[0].status, Status::Skip);
        assert!(skipped[0].detail.contains("no docker CLI"), "{skipped:?}");

        // A daemon that cannot answer for the amd64 variant - an arm64 host
        // that has pulled nothing, or has pulled only its own architecture -
        // names the pull that would let the line be answered.
        let unknown = user_checks_with(&project, true, &|_| None);
        assert_eq!(unknown[0].status, Status::Skip);
        assert!(
            unknown[0].detail.contains(
                "the amd64 variant of ghcr.io/chap-models/chapkit_ewars_model:sha-fa880a1"
            ),
            "{unknown:?}"
        );
        assert!(
            unknown[0]
                .fix
                .as_deref()
                .is_some_and(|fix| fix.contains("chaps docker pull")),
            "{unknown:?}"
        );

        // A deployment with no models has no such lines.
        project.state.models.clear();
        assert!(user_checks_with(&project, true, &|_| None).is_empty());
    }

    #[test]
    fn the_component_images_are_the_ones_the_enabled_set_pulls() {
        assert!(component_images(&Components::default()).is_empty());

        let mut components = Components::default();
        components.set_enabled(crate::components::Component::Ocs, true);
        components.set_enabled(crate::components::Component::S3, true);
        components.set_enabled(Component::Dhis2, true);
        assert_eq!(
            component_images(&components),
            vec![
                (
                    "ocs".to_string(),
                    "ghcr.io/dhis2/open-climate-service:main".to_string()
                ),
                ("s3".to_string(), "rustfs/rustfs:latest".to_string()),
                // The web image, at the tag the component records. Its PostGIS
                // database and its alpine one-shot are exact references in this
                // binary, so there is no tag of theirs to get wrong.
                ("dhis2".to_string(), "dhis2/core:2.42".to_string()),
            ]
        );
        assert!(
            !component_images(&components)
                .iter()
                .any(|(_, reference)| reference.contains("postgis")),
            "the database image has no tag a deployment can move"
        );
    }

    #[test]
    fn a_component_image_is_checked_for_existence_not_for_amd64() {
        // Both component images are multi-arch and no overlay pins a platform,
        // so a manifest that answers at all is the whole verdict.
        let arm_only = r#"{"manifests":[{"platform":{"os":"linux","architecture":"arm64"}}]}"#;
        let check = component_image_verdict(
            "ocs",
            "ghcr.io/dhis2/open-climate-service:main",
            &Outcome::Done {
                ok: true,
                stdout: arm_only.to_string(),
                stderr: String::new(),
            },
        );
        assert_eq!(check.status, Status::Ok);
        assert_eq!(check.id, "image-ocs");

        let missing = component_image_verdict(
            "ocs",
            "ghcr.io/dhis2/open-climate-service:nope",
            &Outcome::Done {
                ok: false,
                stdout: String::new(),
                stderr: "manifest unknown".to_string(),
            },
        );
        assert_eq!(missing.status, Status::Fail);
        assert!(missing.detail.contains("manifest unknown"));
        assert!(missing.fix.unwrap().contains("OCS_IMAGE_TAG"));

        assert_eq!(
            component_image_verdict("s3", "rustfs/rustfs:latest", &Outcome::Missing).status,
            Status::Skip
        );
    }

    #[test]
    fn the_stack_line_judges_a_deployment_without_chap_core_by_its_components() {
        use crate::status::{ComponentState, ComponentStatus};

        let component = |name: &str, state: ComponentState| ComponentStatus {
            name: name.to_string(),
            state,
            reach: "http://localhost:9000".to_string(),
            health_url: None,
            read_only: false,
            datasets: None,
            data_bytes: None,
        };
        let mut report = status_report(ApiHealth::Off, &[], &[]);
        report.components = vec![component("ocs", ComponentState::Up)];
        let (status, detail, fix) = stack_verdict(&report);
        assert_eq!(status, Status::Ok);
        assert_eq!(detail, "no chap-core here; up: ocs");
        assert_eq!(fix, None);

        report.components = vec![
            component("ocs", ComponentState::Starting),
            component("s3", ComponentState::Up),
        ];
        let (status, detail, fix) = stack_verdict(&report);
        assert_eq!(status, Status::Warn);
        assert_eq!(detail, "no chap-core here; not up: ocs");
        assert!(fix.unwrap().contains("chaps status"));
    }

    #[test]
    fn the_stack_line_says_when_the_container_itself_is_unhealthy() {
        let mut report = status_report(
            ApiHealth::Down {
                error: "connection refused".to_string(),
            },
            &[],
            &[],
        );
        // Without a container to blame it is the port that is not answering.
        let (status, detail, fix) = stack_verdict(&report);
        assert_eq!(status, Status::Fail);
        assert!(detail.contains("is down: connection refused"), "{detail}");
        assert!(fix.unwrap().contains("chaps logs chap"));

        // With one, the check says which half is wrong and what its log said.
        report.unhealthy = vec![crate::diagnose::Unhealthy::of(
            API_SERVICE,
            "FATAL:  password authentication failed for user \"chap\"\n",
        )];
        let (status, detail, fix) = stack_verdict(&report);
        assert_eq!(status, Status::Fail);
        assert!(
            detail.starts_with("the chap container is up and unhealthy:"),
            "{detail}"
        );
        assert!(
            detail.contains("password authentication failed"),
            "{detail}"
        );
        assert_eq!(detail.lines().count(), 1, "one line per check: {detail}");
        assert!(fix.unwrap().contains("chaps down --volumes"));
    }

    #[test]
    fn an_image_that_is_not_there_names_the_model_to_re_resolve() {
        let check = image_verdict(
            "chapkit_ewars_model",
            "chapkit-ewars-model",
            "ghcr.io/chap-models/chapkit_ewars_model:sha-fa880a1",
            &done(false, "", "manifest unknown"),
        );
        assert_eq!(check.status, Status::Fail);
        assert_eq!(check.id, "image-chapkit-ewars-model");
        assert!(
            check.detail.contains("manifest unknown"),
            "{}",
            check.detail
        );
        assert_eq!(
            check.fix.unwrap(),
            "run `chaps models enable chapkit_ewars_model` to resolve the pin again"
        );

        let list = r#"{"manifests":[{"platform":{"architecture":"amd64","os":"linux"}}]}"#;
        let check = image_verdict("m", "svc", "ghcr.io/x:1", &done(true, list, ""));
        assert_eq!(check.status, Status::Ok);
        assert!(check.detail.contains("linux/amd64"));

        let arm = r#"{"manifests":[{"platform":{"architecture":"arm64","os":"linux"}}]}"#;
        let check = image_verdict("m", "svc", "ghcr.io/x:1", &done(true, arm, ""));
        assert_eq!(check.status, Status::Warn);
        assert!(check.fix.unwrap().contains("chaps models enable m"));

        // An old CLI that hides `docker manifest` cannot answer, which is not
        // the same as the tag being gone.
        let check = image_verdict(
            "m",
            "svc",
            "ghcr.io/x:1",
            &done(
                false,
                "",
                "docker manifest is only supported on a Docker cli with \
                  experimental cli features enabled",
            ),
        );
        assert_eq!(check.status, Status::Skip);
        assert_eq!(
            image_verdict("m", "svc", "r", &Outcome::TimedOut).status,
            Status::Skip
        );
        assert_eq!(
            image_verdict("m", "svc", "r", &Outcome::Missing).status,
            Status::Skip
        );
    }

    /// A `StatusReport` with the fields the stack check reads.
    fn status_report(api: ApiHealth, expected: &[&str], missing: &[&str]) -> StatusReport {
        StatusReport {
            project: Some("mychap-1ab2c3".to_string()),
            api_url: "http://localhost:8000".to_string(),
            api_port: 8000,
            api_port_source: ApiPortSource::Project,
            api,
            version: ApiVersion {
                value: "v2.3.1".to_string(),
                pinned: false,
                revision: None,
            },
            chap_tag: "v2.3.1".to_string(),
            chap_tag_moving: false,
            chap_build: None,
            registered: Vec::new(),
            expected: expected.iter().map(|s| s.to_string()).collect(),
            missing: missing.iter().map(|s| s.to_string()).collect(),
            reach: BTreeMap::new(),
            models: Vec::<ModelStatus>::new(),
            unmanaged: Vec::new(),
            auth: false,
            components: Vec::new(),
            dhis2_needs_connecting: false,
            unhealthy: Vec::new(),
        }
    }

    #[test]
    fn the_stack_check_follows_what_status_found() {
        let up = || ApiHealth::Up {
            status: "ok".to_string(),
            message: String::new(),
        };

        let (status, detail, fix) = stack_verdict(&status_report(up(), &["a", "b"], &[]));
        assert_eq!(status, Status::Ok);
        assert_eq!(
            detail,
            "chap-core up at http://localhost:8000, 2 of 2 models registered"
        );
        // Green, and still with something to do: registration is a heartbeat.
        assert_eq!(fix.as_deref(), Some(crate::status::TEST_HINT));

        // A deployment with no models has nothing to test.
        let (status, _, fix) = stack_verdict(&status_report(up(), &[], &[]));
        assert_eq!(status, Status::Ok);
        assert_eq!(fix, None);

        let (status, detail, fix) = stack_verdict(&status_report(up(), &["a", "b"], &["b"]));
        assert_eq!(status, Status::Warn, "a missing model is not a dead stack");
        assert!(detail.ends_with("not registered: b"), "{detail}");
        assert!(fix.unwrap().contains("chaps status"));

        let down = ApiHealth::Down {
            error: "connection refused".to_string(),
        };
        let (status, detail, fix) = stack_verdict(&status_report(down, &["a"], &["a"]));
        assert_eq!(status, Status::Fail);
        assert!(detail.contains("connection refused"), "{detail}");
        assert!(fix.unwrap().contains("chaps logs chap"));
    }
}
