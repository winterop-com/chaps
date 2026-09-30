//! Checks of the hosts CHAP needs: ghcr.io, the marketplace, GitHub and chaps' own releases.

use super::*;

/// The five lookups that need the network, still running.
pub(super) struct Probes<'s> {
    ghcr: std::thread::ScopedJoinHandle<'s, std::result::Result<u16, String>>,
    marketplace: std::thread::ScopedJoinHandle<'s, std::result::Result<u16, String>>,
    chap_core: std::thread::ScopedJoinHandle<'s, std::result::Result<String, String>>,
    chaps: std::thread::ScopedJoinHandle<'s, std::result::Result<String, String>>,
    github: std::thread::ScopedJoinHandle<'s, std::result::Result<github::Quota, String>>,
}

/// The same five, answered.
///
/// Two of them do double duty: the release lookups are what tells the
/// `net-releases` and `chaps` lines that the host answered, and the tags they
/// came back with are what the pin checks compare against.
pub struct Probed {
    pub ghcr: std::result::Result<u16, String>,
    pub marketplace: std::result::Result<u16, String>,
    pub chap_core: std::result::Result<String, String>,
    pub chaps: std::result::Result<String, String>,
    /// What is left of this address's hour on the GitHub API, which is what
    /// the release, pin and `models add` lookups are all spending.
    pub github: std::result::Result<github::Quota, String>,
}

impl<'s> Probes<'s> {
    /// Start all five at once. Five sequential three-second timeouts would be
    /// fifteen seconds of a command that has to stay interactive.
    pub(super) fn spawn(scope: &'s std::thread::Scope<'s, '_>, registry_url: &str) -> Probes<'s> {
        let registry_url = registry_url.to_string();
        Probes {
            ghcr: scope.spawn(|| probe(GHCR_PROBE_URL, NET_TIMEOUT)),
            marketplace: scope.spawn(move || probe(&registry_url, NET_TIMEOUT)),
            chap_core: scope
                .spawn(|| chapcore::latest_release(NET_TIMEOUT).map_err(|e| format!("{e:#}"))),
            chaps: scope.spawn(|| {
                selfupdate::latest_release(NET_TIMEOUT)
                    .map(|release| release.tag)
                    .map_err(|e| format!("{e:#}"))
            }),
            // `/rate_limit` is documented as not counting against the limit
            // it reports, so asking cannot be what uses up the last request.
            github: scope.spawn(|| github::quota(NET_TIMEOUT).map_err(|e| format!("{e:#}"))),
        }
    }

    /// Wait for all five. A panicked probe reads as an unreachable host,
    /// which is the verdict it would have produced anyway.
    pub(super) fn join(self) -> Probed {
        fn joined<T>(
            handle: std::thread::ScopedJoinHandle<'_, std::result::Result<T, String>>,
        ) -> std::result::Result<T, String> {
            handle
                .join()
                .unwrap_or_else(|_| Err("the probe did not finish".to_string()))
        }
        Probed {
            ghcr: joined(self.ghcr),
            marketplace: joined(self.marketplace),
            chap_core: joined(self.chap_core),
            chaps: joined(self.chaps),
            github: joined(self.github),
        }
    }
}

/// What one release lookup came back with, or why it came back with nothing.
///
/// The two empty cases are kept apart because they read differently on the
/// checklist: `--offline` is a flag the user set, and a check it held back
/// says so whatever else is installed on the machine; a lookup that was made
/// and did not arrive is a fact about the network instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseList<'a> {
    /// The newest tag the host named.
    Newest(&'a str),
    /// `--offline`: the list was never asked for.
    Offline,
    /// It was asked for and did not arrive.
    Unreachable,
}

impl<'a> ReleaseList<'a> {
    /// The reason every `--offline` line gives, worded the same way in all of
    /// them so a reader can tell the flag from a broken network at a glance.
    pub const OFFLINE_WHY: &'static str = "--offline: the release list was not asked";

    /// One joined probe, where `None` is `--offline`: that is the only reason
    /// a probe was never started.
    pub fn of(probe: Option<&'a std::result::Result<String, String>>) -> ReleaseList<'a> {
        match probe {
            None => ReleaseList::Offline,
            Some(Ok(tag)) => ReleaseList::Newest(tag),
            Some(Err(_)) => ReleaseList::Unreachable,
        }
    }
}

/// Ask one host whether it is there, within `timeout`.
///
/// A GET whose body is never read, which costs the same as a HEAD and works
/// on the endpoints that answer HEAD with 405. Any HTTP status counts as an
/// answer: ghcr.io replies 401 to an anonymous `/v2/`, and a host that
/// refuses us is still a host this machine can reach.
pub fn probe(url: &str, timeout: Duration) -> std::result::Result<u16, String> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(USER_AGENT)
            .build(),
    );
    let started = Instant::now();
    let answer = match agent.get(url).call() {
        Ok(response) => Ok(response.status().as_u16()),
        Err(ureq::Error::StatusCode(code)) => Ok(code),
        Err(err) => Err(err.to_string()),
    };
    crate::output::verbose(&format!(
        "GET {url} -> {} in {}ms",
        match &answer {
            Ok(code) => code.to_string(),
            Err(err) => err.clone(),
        },
        started.elapsed().as_millis()
    ));
    answer
}

/// One line per host CHAP needs, or three skipped lines under `--offline`.
pub fn network_checks(probed: Option<&Probed>) -> Vec<Check> {
    let hosts: [(&str, &str, &str, &str); 3] = [
        (
            "net-ghcr",
            "network ghcr.io",
            "the image registry",
            "without ghcr.io no image can be pulled; `--offline` keeps chaps itself working \
             from the embedded catalogue",
        ),
        (
            "net-marketplace",
            "network marketplace",
            "the model catalogue",
            "the catalogue falls back to the cache and then to the snapshot built into this \
             binary; `chaps registry show` says which one is in use",
        ),
        (
            "net-releases",
            "network releases",
            "the chap-core release list",
            "`chaps update` needs it; everything else works without it",
        ),
    ];
    let Some(probed) = probed else {
        return hosts
            .iter()
            .map(|(id, name, what, _)| {
                Check::skip(*id, *name, format!("--offline: {what} was not probed"))
            })
            .collect();
    };
    let answers: [&dyn ReachSummary; 3] = [&probed.ghcr, &probed.marketplace, &probed.chap_core];
    hosts
        .iter()
        .zip(answers)
        .map(|((id, name, what, fix), answer)| match answer.reached() {
            Ok(detail) => Check::ok(*id, *name, detail),
            Err(why) => Check::warn(*id, *name, format!("{what} is unreachable: {why}"), *fix),
        })
        .collect()
}

/// The stable id and the printed name of the `github api` line.
pub const GITHUB_ID: &str = "net-github";

pub const GITHUB_NAME: &str = "github api";

/// What the `github api` line says under `--offline`.
pub const GITHUB_OFFLINE: &str = "--offline: the GitHub API rate limit was not asked";

/// What every unreachable GitHub line offers, and what a quota that has run
/// out offers when there is no token to blame.
const GITHUB_FIX: &str = "`chaps update`, the pin checks and `chaps models add` need it; \
     everything else works without it";

/// The `github api` line: does the REST API answer, and how much of this
/// address's hour is left on it.
///
/// Not a failure whatever it says. Every GitHub lookup this CLI makes already
/// degrades to a skip with the reason on its own line; this one is here so an
/// operator can see the quota before it runs out rather than after, and so a
/// morning of skipped pin checks has an explanation at the top of the report.
pub fn github_check(quota: Option<&std::result::Result<github::Quota, String>>) -> Check {
    let Some(quota) = quota else {
        return Check::skip(GITHUB_ID, GITHUB_NAME, GITHUB_OFFLINE);
    };
    let quota = match quota {
        Ok(quota) => quota,
        Err(why) => {
            return Check::warn(
                GITHUB_ID,
                GITHUB_NAME,
                format!("could not ask what is left of this hour: {why}"),
                GITHUB_FIX,
            );
        }
    };
    // The parenthetical says which of the two limits this is, and where there
    // is no token it says what setting one would buy.
    let credential = match quota.token {
        true => "token".to_string(),
        false => format!("no token; set GITHUB_TOKEN for {}", github::TOKEN_LIMIT),
    };
    let left = format!(
        "reachable, {} of {} requests left this hour ({credential})",
        quota.remaining, quota.limit
    );
    if quota.remaining > 0 {
        return Check::ok(GITHUB_ID, GITHUB_NAME, left);
    }
    let until = github::reset_clock(quota.reset);
    Check::warn(
        GITHUB_ID,
        GITHUB_NAME,
        format!("{left}, until {until}"),
        match quota.token {
            true => format!("wait until {until}; until then the release and pin checks skip"),
            false => format!(
                "set GITHUB_TOKEN or GH_TOKEN for {} requests an hour, or wait until {until}; \
                 until then the release and pin checks skip",
                github::TOKEN_LIMIT
            ),
        },
    )
}

/// How a probe result reads on the line for its host.
///
/// Two of the probes fetch a document rather than only touching the host, so
/// each one describes its own success in its own terms.
pub trait ReachSummary {
    fn reached(&self) -> std::result::Result<String, String>;
}

impl ReachSummary for std::result::Result<u16, String> {
    fn reached(&self) -> std::result::Result<String, String> {
        match self {
            Ok(code) => Ok(format!("reachable (HTTP {code})")),
            Err(why) => Err(why.clone()),
        }
    }
}

impl ReachSummary for std::result::Result<String, String> {
    fn reached(&self) -> std::result::Result<String, String> {
        match self {
            Ok(tag) => Ok(format!("reachable, newest chap-core is {tag}")),
            Err(why) => Err(why.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
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
            Some(
                format!("wait until {until}; until then the release and pin checks skip").as_str()
            )
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
}
