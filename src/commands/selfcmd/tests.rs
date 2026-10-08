use super::*;
use crate::output::Out;
use crate::registry::{DEFAULT_TIMEOUT, RegistryOptions};

fn ctx(cache_dir: &Path, out: Out, offline: bool) -> Ctx {
    Ctx {
        out,
        project_dir: PathBuf::from("."),
        registry: RegistryOptions {
            url: "https://example.test/registry.yaml".to_string(),
            offline,
            cache_dir: cache_dir.to_path_buf(),
            timeout: DEFAULT_TIMEOUT,
        },
        cli_version: "0.0.0-test",
    }
}

fn tty() -> Out {
    Out {
        tty: true,
        ..Out::default()
    }
}

#[test]
fn the_version_report_carries_the_build_facts() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx(tmp.path(), Out::default(), true);
    version(&ctx, &SelfVersionArgs {}).expect("version never fails");

    // The report is what --json serialises, so assert on that shape.
    let path = std::env::current_exe().ok();
    let report = VersionReport {
        version: VERSION,
        revision: Some(GIT_REVISION).filter(|r| !r.is_empty()),
        channel: selfupdate::channel().as_str(),
        target: TARGET,
        path: path.clone(),
        install_method: path
            .as_deref()
            .map(selfupdate::install_method)
            .unwrap_or("release archive"),
    };
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["version"], VERSION);
    assert_eq!(value["target"], TARGET);
    assert!(value["install_method"].is_string());
    // The channel is always there, and a plain build is a stable one.
    assert_eq!(value["channel"], "stable");
    assert_eq!(describe_build(), format!("v{VERSION}"));
}

#[test]
fn offline_and_self_update_are_opposite_requests() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = ctx(tmp.path(), Out::default(), true);
    let err = update(
        &ctx,
        &SelfUpdateArgs {
            check: true,
            version: None,
            yes: false,
        },
    )
    .expect_err("--offline refuses before any network call");
    assert!(err.to_string().contains("--offline"), "{err}");
}

#[test]
fn the_notice_is_only_for_a_plain_terminal_run() {
    let tmp = tempfile::tempdir().unwrap();

    assert!(should_notify(&ctx(tmp.path(), tty(), false), false));

    // A pipe, a CI log or a test.
    assert!(!should_notify(
        &ctx(tmp.path(), Out::default(), false),
        false
    ));
    // A parser did not ask for a second stream.
    let json = Out {
        json: true,
        ..tty()
    };
    assert!(!should_notify(&ctx(tmp.path(), json, false), false));
    // --offline means no network, and that includes this.
    assert!(!should_notify(&ctx(tmp.path(), tty(), true), false));
    // VARDE_NO_UPDATE_CHECK=1. Passed in rather than read from the
    // environment, which is process-wide and shared with every other test
    // in this binary.
    assert!(!should_notify(&ctx(tmp.path(), tty(), false), true));
}

/// A check recorded a moment ago is used as it stands: no request is made,
/// which is what makes the notice free on all but one command a day.
#[test]
fn a_fresh_check_is_read_from_the_cache_rather_than_the_network() {
    let tmp = tempfile::tempdir().unwrap();
    selfupdate::write_check(
        tmp.path(),
        &selfupdate::CheckState {
            checked_at_unix: selfupdate::now_unix(),
            latest: "v99.0.0".to_string(),
            ..Default::default()
        },
    );
    let state = selfupdate::read_check(tmp.path()).unwrap();
    assert!(!selfupdate::is_stale(
        Some(&state),
        selfupdate::now_unix(),
        selfupdate::CHECK_INTERVAL
    ));
    assert!(selfupdate::notice_line(&state.latest, VERSION).is_some());
}

/// The rolling release is named by its commit and its day, because its
/// tag is the string `dev` on every build of `main`.
#[test]
fn a_release_is_named_by_its_tag_and_the_rolling_one_by_its_commit() {
    let tagged = selfupdate::Release {
        tag: "v0.3.0".to_string(),
        ..Default::default()
    };
    assert_eq!(describe_release(&tagged), "v0.3.0");

    let rolling = selfupdate::Release {
        tag: selfupdate::DEV_TAG.to_string(),
        prerelease: true,
        published_at: "2026-09-23T17:08:44Z".to_string(),
        body: "built from commit `0b1c2d3e4f50617283940a1b2c3d4e5f60718293` \
                   on 2026-09-23"
            .to_string(),
        ..Default::default()
    };
    assert_eq!(describe_release(&rolling), "dev 0b1c2d3 (2026-09-23)");

    // The day is the day of the build, not the day the release was made:
    // the rolling release keeps its first publish day for every build.
    let rebuilt = selfupdate::Release {
        built_at: "2026-10-07T06:20:00Z".to_string(),
        body: "built from commit `0b1c2d3e4f50617283940a1b2c3d4e5f60718293`".to_string(),
        ..rolling.clone()
    };
    assert_eq!(describe_release(&rebuilt), "dev 0b1c2d3 (2026-10-07)");

    // Notes that name no commit still say which day the build is from.
    let vague = selfupdate::Release {
        tag: selfupdate::DEV_TAG.to_string(),
        published_at: "2026-09-23T17:08:44Z".to_string(),
        built_at: "2026-10-07T06:20:00Z".to_string(),
        ..Default::default()
    };
    assert_eq!(describe_release(&vague), "dev (2026-10-07)");

    // And with nothing that says when it was built, no day at all.
    let undated = selfupdate::Release {
        built_at: String::new(),
        ..vague
    };
    assert_eq!(describe_release(&undated), "dev");
}

/// Crossing channels is never "already up to date", in either direction:
/// a dev build and the stable release carrying the same number are two
/// different binaries, built from two different commits.
#[test]
fn crossing_channels_is_never_already_up_to_date() {
    // The one the review found: a dev build asking for the stable release
    // of its own number has to install it.
    assert!(!already_running(Channel::Dev, "v1.2.3", "1.2.3", false));
    assert!(!already_running(Channel::Dev, "1.2.3", "1.2.3", false));
    // And the other way round: a stable build asking for the rolling tag,
    // even when the rolling release sits on no newer commit.
    assert!(!already_running(
        Channel::Stable,
        selfupdate::DEV_TAG,
        "1.2.3",
        false
    ));

    // Inside one channel the answer is the plain one. A stable build on
    // the release it looked up has nothing to do.
    assert!(already_running(Channel::Stable, "v1.2.3", "1.2.3", false));
    assert!(!already_running(Channel::Stable, "v1.3.0", "1.2.3", false));
    // A dev build is up to date when the rolling release is the commit it
    // already runs, which is the question `dev_newer` answers.
    assert!(already_running(
        Channel::Dev,
        selfupdate::DEV_TAG,
        "1.2.3",
        false
    ));
    assert!(!already_running(
        Channel::Dev,
        selfupdate::DEV_TAG,
        "1.2.3",
        true
    ));
}

/// A lookup that failed still records the attempt, so a machine with no
/// network tries once a day rather than on every command, and the empty
/// tag it stores says nothing rather than something wrong.
#[test]
fn a_failed_check_records_the_attempt_and_stays_quiet() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("cache");

    selfupdate::write_check(
        &cache,
        &selfupdate::CheckState {
            checked_at_unix: selfupdate::now_unix(),
            latest: String::new(),
            ..Default::default()
        },
    );
    let state = selfupdate::read_check(&cache).expect("the attempt was recorded");
    assert!(state.latest.is_empty());
    assert_eq!(selfupdate::notice_line(&state.latest, VERSION), None);
    assert!(
        !selfupdate::is_stale(
            Some(&state),
            selfupdate::now_unix(),
            selfupdate::CHECK_INTERVAL
        ),
        "and the next command does not try again straight away"
    );
}

/// The hint of `--check` names the command that installs what was checked:
/// a plain `varde self update` on a stable build follows releases/latest and
/// does not install `--version dev`.
#[test]
fn the_check_hint_keeps_the_version_it_checked() {
    let args = |version: Option<&str>| SelfUpdateArgs {
        check: true,
        version: version.map(str::to_string),
        yes: false,
    };
    assert_eq!(install_command(&args(None)), "varde self update");
    assert_eq!(
        install_command(&args(Some("dev"))),
        "varde self update --version dev"
    );
    assert_eq!(
        install_command(&args(Some("v0.100.0"))),
        "varde self update --version v0.100.0"
    );
}
