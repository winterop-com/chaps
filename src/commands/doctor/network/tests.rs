use super::*;

#[test]
fn a_probe_that_did_not_answer_is_a_warning_and_offline_is_a_skip() {
    let checks = network_checks(None);
    assert_eq!(checks.len(), 3);
    for check in &checks {
        assert_eq!(check.status, Status::Skip);
        assert!(check.detail.contains("offline"), "{}", check.detail);
    }
    assert_eq!(
        checks.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec!["net-ghcr", "net-marketplace", "net-releases"]
    );

    let probed = Probed {
        ghcr: Ok(401),
        marketplace: Err("dns error".to_string()),
        chap_core: Ok("v2.3.1".to_string()),
        chaps: Ok("v0.2.0".to_string()),
        github: Err("not asked here".to_string()),
    };
    let checks = network_checks(Some(&probed));
    assert_eq!(checks[0].status, Status::Ok);
    assert_eq!(checks[0].detail, "reachable (HTTP 401)");
    assert_eq!(checks[1].status, Status::Warn, "unreachable never fails");
    assert!(checks[1].detail.contains("dns error"));
    assert!(checks[1].fix.as_ref().unwrap().contains("cache"));
    assert_eq!(checks[2].status, Status::Ok);
    assert!(checks[2].detail.contains("v2.3.1"));
    assert!(checks[0].fix.is_none());

    // ghcr is the one whose absence stops images being pulled.
    let probed = Probed {
        ghcr: Err("timed out".to_string()),
        ..probed
    };
    let checks = network_checks(Some(&probed));
    assert_eq!(checks[0].status, Status::Warn);
    assert!(checks[0].fix.as_ref().unwrap().contains("--offline"));
}

/// The `github api` line, from an answer handed in: the quota, which of
/// the two limits it is, and what to do when it has run out.
#[test]
fn the_github_line_reports_the_quota_and_which_limit_it_is() {
    let quota = |remaining: u64, limit: u64, token: bool| github::Quota {
        limit,
        remaining,
        // 2026-09-25T13:04:00Z, whatever this machine calls that hour.
        reset: Some(1_758_805_440),
        token,
    };

    // A token: the 5000 an hour, and nothing to advise.
    let check = github_check(Some(&Ok(quota(4990, 5000, true))));
    assert_eq!(check.status, Status::Ok);
    assert_eq!(check.id, "net-github");
    assert_eq!(check.name, "github api");
    assert_eq!(
        check.detail,
        "reachable, 4990 of 5000 requests left this hour (token)"
    );
    assert!(check.fix.is_none());

    // No token: the 60, and the line says what a token would buy.
    let check = github_check(Some(&Ok(quota(43, 60, false))));
    assert_eq!(check.status, Status::Ok);
    assert_eq!(
        check.detail,
        "reachable, 43 of 60 requests left this hour (no token; set GITHUB_TOKEN for 5000)"
    );

    // Used up: a warning with the clock the window resets on, never a
    // failure - every lookup that needs it degrades to a skip.
    let until = github::reset_clock(Some(1_758_805_440));
    let check = github_check(Some(&Ok(quota(0, 60, false))));
    assert_eq!(check.status, Status::Warn);
    assert_eq!(
        check.detail,
        format!(
            "reachable, 0 of 60 requests left this hour \
                 (no token; set GITHUB_TOKEN for 5000), until {until}"
        )
    );
    assert!(check.fix.as_ref().unwrap().contains("GITHUB_TOKEN"));
    assert!(check.fix.as_ref().unwrap().contains(&until));

    // Used up with a token: setting one is no longer the advice.
    let check = github_check(Some(&Ok(quota(0, 5000, true))));
    assert_eq!(check.status, Status::Warn);
    assert!(
        check
            .detail
            .contains("0 of 5000 requests left this hour (token)")
    );
    assert_eq!(
        check.fix.as_deref(),
        Some(format!("wait until {until}; until then the release and pin checks skip").as_str())
    );

    // No reset to name: the sentence still stands.
    let check = github_check(Some(&Ok(github::Quota {
        limit: 60,
        remaining: 0,
        reset: None,
        token: false,
    })));
    assert!(check.detail.ends_with("until -"), "{}", check.detail);

    // A lookup that did not arrive is a warning like every other host -
    // the wording covers both a host that said nothing and one that
    // refused, because a refused quota is not an unreachable GitHub.
    let check = github_check(Some(&Err("dns error".to_string())));
    assert_eq!(check.status, Status::Warn);
    assert_eq!(
        check.detail,
        "could not ask what is left of this hour: dns error"
    );
    assert!(check.fix.is_some());

    let check = github_check(None);
    assert_eq!(check.status, Status::Skip);
    assert!(check.detail.contains("--offline"), "{}", check.detail);
}

#[test]
fn the_pin_check_only_speaks_up_for_a_release_that_moved() {
    let (status, detail, fix) = pin_verdict("latest", ReleaseList::Newest("v2.3.1"), None);
    assert_eq!(status, Status::Ok, "a moving tag is not behind anything");
    assert!(detail.contains("moving tag"), "{detail}");
    assert_eq!(fix, None);

    let (status, detail, fix) = pin_verdict("v2.3.0", ReleaseList::Newest("v2.3.1"), None);
    assert_eq!(status, Status::Warn);
    assert_eq!(detail, "v2.3.0 pinned, v2.3.1 released");
    assert!(fix.unwrap().contains("chaps update --dry-run"));

    assert_eq!(
        pin_verdict("v2.3.1", ReleaseList::Newest("v2.3.1"), None).0,
        Status::Ok
    );
    assert_eq!(
        pin_verdict("v2.4.0", ReleaseList::Newest("v2.3.1"), None).0,
        Status::Ok
    );

    // Without the release list there is nothing to compare against, and
    // the two silent cases each say which one they are.
    let (status, detail, _) = pin_verdict("v2.3.0", ReleaseList::Unreachable, None);
    assert_eq!(status, Status::Skip);
    assert!(detail.contains("v2.3.0"), "{detail}");
    assert!(detail.contains("not reachable"), "{detail}");

    let (status, detail, _) = pin_verdict("v2.3.0", ReleaseList::Offline, None);
    assert_eq!(status, Status::Skip);
    assert!(
        detail.contains("v2.3.0") && detail.contains("offline"),
        "{detail}"
    );

    // A moving tag needs no list at all, offline included.
    assert_eq!(
        pin_verdict("latest", ReleaseList::Offline, None).0,
        Status::Ok
    );
}

#[test]
fn a_moving_pin_says_which_build_it_is_on_and_how_to_leave_it() {
    let (status, detail, fix) =
        pin_verdict("dev", ReleaseList::Newest("v2.3.1"), Some("7f3a1c2e9b4d"));
    assert_eq!(status, Status::Ok);
    assert_eq!(
        detail,
        "dev (moving tag, running 7f3a1c2e9b4d); `chaps update` re-pulls it, \
             `chaps update --pin-chap-core` pins a release"
    );
    assert_eq!(fix, None);

    // Nothing running, or a docker that would not say: the tag is still
    // the answer, without a digest invented for it.
    for unknown in [None, Some("")] {
        let (_, detail, _) = pin_verdict("dev", ReleaseList::Offline, unknown);
        assert!(detail.starts_with("dev (moving tag);"), "{detail}");
        assert!(!detail.contains("running"), "{detail}");
    }
}

/// The reason the image lines give, in the order the reasons rank.
#[test]
fn offline_is_why_the_image_lines_were_skipped_even_without_docker() {
    let reachable = Probed {
        ghcr: Ok(401),
        marketplace: Ok(200),
        chap_core: Ok("v2.3.1".to_string()),
        chaps: Ok("v0.2.0".to_string()),
        github: Err("not asked here".to_string()),
    };

    // `--offline` outranks a missing docker CLI: the flag is the reason
    // the user gave, and the line reads the same on either machine.
    for have_cli in [true, false] {
        let reason = image_skip_reason(None, have_cli).expect("offline skips the lookup");
        assert!(reason.contains("offline"), "{reason}");
    }

    // Online, a machine without docker has nothing to ask through.
    assert_eq!(
        image_skip_reason(Some(&reachable), false).as_deref(),
        Some("no docker CLI to ask")
    );

    // Online with docker: only an unreachable registry holds it back.
    assert_eq!(image_skip_reason(Some(&reachable), true), None);
    let unreachable = Probed {
        ghcr: Err("dns error".to_string()),
        ..reachable
    };
    let reason = image_skip_reason(Some(&unreachable), true).expect("ghcr is down");
    assert!(reason.contains("dns error"), "{reason}");
}
