use super::super::test_support::*;
use super::*;

#[test]
fn a_missing_docker_binary_says_where_to_get_one() {
    let check = docker_cli_check(&Outcome::Missing);
    assert_eq!(check.status, Status::Fail);
    assert!(check.detail.contains("not on PATH"), "{}", check.detail);
    assert!(
        check
            .fix
            .unwrap()
            .contains("https://docs.docker.com/get-docker/"),
        "the fix has to carry the link"
    );

    let check = docker_cli_check(&done(true, "29.8.1", ""));
    assert_eq!(check.status, Status::Ok);
    assert_eq!(check.detail, "docker 29.8.1");

    assert_eq!(docker_cli_check(&Outcome::TimedOut).status, Status::Fail);
}

#[test]
fn the_daemon_error_is_told_apart_by_what_docker_printed() {
    // The two messages a real docker prints, verbatim.
    assert_eq!(
        daemon_problem(
            "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. \
                 Is the docker daemon running?"
        ),
        DaemonProblem::NotRunning
    );
    assert_eq!(
        daemon_problem(
            "Got permission denied while trying to connect to the Docker daemon socket at \
                 unix:///var/run/docker.sock: Get \"http://%2Fvar%2Frun%2Fdocker.sock/v1.24/info\": \
                 dial unix /var/run/docker.sock: connect: permission denied"
        ),
        DaemonProblem::PermissionDenied,
        "permission denied is not `the daemon is not there`"
    );
    // Windows spells the same absence differently.
    assert_eq!(
        daemon_problem(
            "error during connect: Get \"http://%2F%2F.%2Fpipe%2FdockerDesktopLinuxEngine\
                 /v1.51/info\": open //./pipe/dockerDesktopLinuxEngine: The system cannot find \
                 the file specified."
        ),
        DaemonProblem::NotRunning
    );
    assert_eq!(
        daemon_problem("something else entirely"),
        DaemonProblem::Unknown
    );

    assert!(daemon_fix(DaemonProblem::NotRunning).contains("systemctl start docker"));
    assert!(daemon_fix(DaemonProblem::PermissionDenied).contains("docker` group"));
}

#[test]
fn the_daemon_check_reports_the_cause_it_found() {
    let check = docker_daemon_check(
        &done(
            false,
            "",
            "Cannot connect to the Docker daemon at unix:///var/run/docker.sock",
        ),
        true,
    );
    assert_eq!(check.status, Status::Fail);
    assert_eq!(check.detail, "the Docker daemon is not running");

    let check = docker_daemon_check(&done(false, "", "permission denied"), true);
    assert_eq!(check.detail, "permission denied on the Docker socket");

    let check = docker_daemon_check(&done(true, "29.8.0", ""), true);
    assert_eq!(check.status, Status::Ok);
    assert_eq!(check.detail, "Docker Engine 29.8.0");

    // Nothing to ask when the CLI itself is missing; the line above said so.
    assert_eq!(
        docker_daemon_check(&Outcome::Missing, false).status,
        Status::Skip
    );
}

/// The line follows [`docker::MIN_COMPOSE_VERSION`], which is the release
/// `!override` arrived in: 2.24.3 has `include:` and still cannot run the
/// port override every deployment gets, so it is a warning and 2.24.4 is
/// not.
#[test]
fn compose_warns_below_the_version_the_override_tag_needs() {
    let (status, detail, fix) = compose_verdict(Some((2, 24, 3)));
    assert_eq!(status, Status::Warn);
    assert_eq!(detail, "v2.24.3, older than 2.24.4");
    let fix = fix.expect("a warning says what to do");
    assert!(fix.contains("`!override`"), "{fix}");
    assert!(fix.contains("include:"), "{fix}");

    let (status, detail, fix) = compose_verdict(Some((2, 24, 4)));
    assert_eq!(status, Status::Ok);
    assert_eq!(detail, "v2.24.4");
    assert_eq!(fix, None);

    assert_eq!(compose_verdict(Some((5, 5, 1))).0, Status::Ok);
    assert_eq!(compose_verdict(Some((5, 5, 1))).1, "v5.5.1");
    // Old enough to lose the model overlays as well as the port override.
    assert_eq!(compose_verdict(Some((2, 20, 0))).0, Status::Warn);

    let (status, detail, fix) = compose_verdict(Some((2, 19, 9)));
    assert_eq!(status, Status::Warn);
    assert_eq!(detail, "v2.19.9, older than 2.24.4");
    assert!(fix.unwrap().contains("include:"));

    let (status, detail, fix) = compose_verdict(None);
    assert_eq!(status, Status::Fail);
    assert_eq!(detail, "docker compose is not installed");
    assert!(fix.unwrap().contains("2.24.4 or newer"));

    // And with no docker at all there is nothing to report.
    assert_eq!(compose_check(false, None).status, Status::Skip);
    assert_eq!(compose_check(true, Some((2, 24, 6))).status, Status::Ok);
}

#[test]
fn arm_hosts_are_warned_that_the_images_are_amd64() {
    let check = arch_check("linux", "x86_64");
    assert_eq!(check.status, Status::Ok);
    assert_eq!(check.detail, "linux/x86_64");
    assert_eq!(check.fix, None);

    let check = arch_check("macos", "aarch64");
    assert_eq!(check.status, Status::Warn);
    assert!(check.detail.contains("linux/amd64"), "{}", check.detail);
    assert!(check.fix.unwrap().contains("Rosetta"));

    let check = arch_check("linux", "aarch64");
    assert_eq!(check.status, Status::Warn);
    assert!(check.fix.unwrap().contains("binfmt"));

    // Windows spells the same architecture differently.
    assert_eq!(arch_check("windows", "arm64").status, Status::Warn);
}

#[test]
fn docker_info_is_asked_once_and_split_afterwards() {
    assert_eq!(
        info_fields("29.8.0\t/var/lib/docker\t33598197760"),
        DockerInfo {
            engine: "29.8.0".to_string(),
            root_dir: Some("/var/lib/docker".to_string()),
            memory: Some(33_598_197_760),
        }
    );
    // A field the engine does not know comes back empty, not absent - and
    // an engine older than the format asked of it prints fewer of them.
    assert_eq!(
        info_fields("29.8.0\t\t"),
        DockerInfo {
            engine: "29.8.0".to_string(),
            ..DockerInfo::default()
        }
    );
    assert_eq!(
        info_fields("29.8.0"),
        DockerInfo {
            engine: "29.8.0".to_string(),
            ..DockerInfo::default()
        }
    );
    assert_eq!(info_fields(""), DockerInfo::default());
    // A memory of zero is an engine that would not say, not a machine with
    // no memory, which is not a thing.
    assert_eq!(info_fields("29.8.0\t/var/lib/docker\t0").memory, None);
    // Nor is a figure that does not read as a number.
    assert_eq!(info_fields("29.8.0\t/var/lib/docker\t<nil>").memory, None);
}

#[test]
fn the_measured_filesystem_falls_back_when_the_root_is_not_out_here() {
    let here = std::env::current_dir().expect("a working directory");
    // Docker Desktop's root lives inside its VM, so it is not this host's.
    let inside_the_vm = if cfg!(windows) {
        "C:\\definitely\\not\\a\\directory"
    } else {
        "/definitely/not/a/directory"
    };
    assert_eq!(disk_path(Some(inside_the_vm)), here);
    assert_eq!(disk_path(None), here);

    // A root this host can see is the one to measure.
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().to_string_lossy().into_owned();
    assert_eq!(disk_path(Some(&path)), PathBuf::from(&path));
}

#[test]
fn the_disk_thresholds_are_ten_and_three_gigabytes() {
    assert_eq!(disk_verdict(Some(64 * GB)).0, Status::Ok);
    assert_eq!(disk_verdict(Some(DISK_WARN)).0, Status::Ok);
    assert_eq!(disk_verdict(Some(DISK_WARN - 1)).0, Status::Warn);
    assert_eq!(disk_verdict(Some(DISK_FAIL)).0, Status::Warn);
    assert_eq!(disk_verdict(Some(DISK_FAIL - 1)).0, Status::Fail);
    assert_eq!(disk_verdict(Some(0)).0, Status::Fail);
    // Unparsable is not "no space": it is a question this host did not
    // answer, and a failure would be a lie.
    assert_eq!(disk_verdict(None).0, Status::Skip);

    assert_eq!(disk_verdict(Some(4 * GB)).1, "4.0 GB free");
    let check = disk_check(Path::new("/var/lib/docker"), Some(4 * GB));
    assert_eq!(check.detail, "4.0 GB free on /var/lib/docker");
    // A skipped check must not claim to have measured anything.
    assert!(!disk_check(Path::new("/x"), None).detail.contains("/x"));
}

#[test]
fn free_space_is_read_out_of_what_each_platform_prints() {
    // macOS `df -Pk /`, which is the same shape as Linux's.
    let macos = "Filesystem 1024-blocks      Used Available Capacity  Mounted on\n\
                     /dev/disk3s1s1 971350180 21495384 663797352     4%    /\n";
    assert_eq!(parse_df(macos), Some(663_797_352 * 1024));

    // Linux, where a long device name is what `-P` keeps on one line.
    let linux = "Filesystem     1024-blocks     Used Available Capacity Mounted on\n\
                     /dev/mapper/vg0-root  103081248 40209032  57596756      42% /\n";
    assert_eq!(parse_df(linux), Some(57_596_756 * 1024));

    assert_eq!(parse_df(""), None);
    assert_eq!(parse_df("df: /nope: No such file or directory\n"), None);

    // PowerShell's `(Get-PSDrive -Name C).Free` is the number alone.
    assert_eq!(parse_free_bytes("\r\n123456789\r\n"), Some(123_456_789));
    assert_eq!(parse_free_bytes(""), None);
    assert_eq!(parse_free_bytes("Get-PSDrive : Cannot find drive"), None);
}

#[test]
fn the_chaps_line_offers_an_update_only_when_there_is_one() {
    // Offline the lookup never ran, so the line is a skip that says why
    // and still describes the build.
    let check = chaps_check(ReleaseList::Offline);
    assert_eq!(check.status, Status::Skip);
    assert!(check.detail.contains(VERSION) && check.detail.contains(TARGET));
    assert!(check.detail.contains("offline"), "{}", check.detail);

    // The release feed was asked and said nothing useful: still just a
    // description, and `net-releases` is where the network is reported.
    let check = chaps_check(ReleaseList::Unreachable);
    assert_eq!(check.status, Status::Ok);
    assert!(check.detail.contains(VERSION) && check.detail.contains(TARGET));
    let current = format!("v{VERSION}");
    assert_eq!(
        chaps_check(ReleaseList::Newest(&current)).status,
        Status::Ok,
        "the running version is not an update"
    );

    let check = chaps_check(ReleaseList::Newest("v999.0.0"));
    assert_eq!(check.status, Status::Warn);
    assert!(check.detail.contains("v999.0.0"));
    assert_eq!(check.fix.unwrap(), "run `chaps self update`");
}

/// The memory line exists for one failure: DHIS2's JVM being killed part-way
/// through the analytics populate phase, which looks like anything except
/// memory. The figure is docker's own, because on macOS and Windows the
/// containers live in a VM whose size the host's free memory says nothing
/// about.
#[test]
fn the_memory_thresholds_are_six_and_four_gigabytes() {
    assert_eq!(memory_verdict(Some(32 * GB)).0, Status::Ok);
    assert_eq!(memory_verdict(Some(DHIS2_MEMORY_WARN)).0, Status::Ok);
    assert_eq!(memory_verdict(Some(DHIS2_MEMORY_WARN - 1)).0, Status::Warn);
    assert_eq!(memory_verdict(Some(DHIS2_MEMORY_FAIL)).0, Status::Warn);
    assert_eq!(memory_verdict(Some(DHIS2_MEMORY_FAIL - 1)).0, Status::Fail);

    // The detail says what was measured and whose figure it is.
    assert_eq!(memory_verdict(Some(8 * GB)).1, "8.0 GB available to docker");
    // Both arms name the one file an operator can change without touching
    // Docker's own settings.
    for memory in [DHIS2_MEMORY_FAIL - 1, DHIS2_MEMORY_WARN - 1] {
        let fix = memory_verdict(Some(memory)).2.expect("something to do");
        assert!(fix.contains(DHIS2_JAVA_ENV_VAR), "{fix}");
        assert!(fix.contains("`.env`"), "{fix}");
    }

    // A platform, or an engine, that would not say is a reported skip
    // rather than a guess: a wrong answer here sends an operator after a
    // memory problem they do not have.
    let (status, detail, fix) = memory_verdict(None);
    assert_eq!(status, Status::Skip);
    assert!(detail.contains("docker did not say"), "{detail}");
    assert_eq!(fix, None);
    // And no docker at all is the same kind of skip.
    assert_eq!(memory_check(false, Some(32 * GB)).status, Status::Skip);
    assert_eq!(memory_check(true, Some(32 * GB)).status, Status::Ok);
}
