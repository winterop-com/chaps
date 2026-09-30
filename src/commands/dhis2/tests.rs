use super::*;

fn out() -> Out {
    Out::detect(false, true)
}

/// A [`Ctx`] for the one thing in here that writes a file.
fn ctx(dir: &std::path::Path) -> Ctx {
    Ctx {
        out: Out::default(),
        project_dir: dir.to_path_buf(),
        registry: crate::registry::RegistryOptions::default(),
        cli_version: "0.1.0",
    }
}

/// A timestamp in the shape [`record_connect`] writes.
const CONNECTED: &str = "2026-09-27T09:12:33Z";

fn instance() -> Instance {
    Instance {
        url: "http://localhost:18080".to_string(),
        version: "2.42.6".to_string(),
        user: "admin".to_string(),
        external: false,
        auth: AuthKind::Basic,
        credential_from: CredentialSource::Default,
    }
}

fn route_report(outcome: RouteOutcome) -> RouteReport {
    RouteReport {
        outcome,
        code: dhis2::ROUTE_CODE,
        url: "http://chap:8000/**".to_string(),
        previous: None,
        reasons: Vec::new(),
        id: Some("abc123".to_string()),
        verified: true,
        answered: "healthy".to_string(),
    }
}

fn report(route: Option<RouteReport>) -> Dhis2Report {
    Dhis2Report {
        instance: instance(),
        route,
        apps: None,
        analytics: None,
        skipped: Vec::new(),
        record: None,
        next: "run `chaps dhis2 show` to see what is still missing".to_string(),
    }
}

fn apps_report(outcomes: &[AppOutcome]) -> AppsReport {
    AppsReport {
        apps: outcomes
            .iter()
            .zip(dhis2::HUB_APPS)
            .map(|(outcome, app)| AppReport {
                name: app.name.to_string(),
                outcome: *outcome,
                version: "7.1.0".to_string(),
                previous: None,
                reason: None,
            })
            .collect(),
    }
}

/// What a `connect` has to have got through before anything is recorded:
/// the route proved, and both apps in place. Analytics is not part of it -
/// the Modeling App reaches chap-core and works without the tables, and an
/// empty `analytics_*` is a deployment with no data rather than one that
/// cannot talk to itself.
///
/// And the third answer, which is the one that keeps `connect` honest: a
/// run that did not look. `--offline` skips the app install and reports the
/// skip, so it knows nothing about the apps in DHIS2 - very likely the two
/// an earlier run put there - and says so rather than calling the
/// deployment broken over a flag on an unrelated step.
#[test]
fn only_a_proved_route_with_both_apps_counts_as_connected() {
    let both = apps_report(&[AppOutcome::Installed, AppOutcome::Unchanged]);
    let created = route_report(RouteOutcome::Created);
    assert_eq!(judge(&created, Some(&both)), Judgement::Connected);
    // A route that was already right is as good as one just written.
    assert_eq!(
        judge(&route_report(RouteOutcome::Unchanged), Some(&both)),
        Judgement::Connected
    );

    // A row in DHIS2's database proves nothing; the proxied request does.
    let unverified = RouteReport {
        verified: false,
        answered: "HTTP 502 Bad Gateway".to_string(),
        ..route_report(RouteOutcome::Created)
    };
    assert_eq!(judge(&unverified, Some(&both)), Judgement::Broken);
    // A failed request is this run's own evidence, so it counts whether or
    // not the apps were attempted.
    assert_eq!(judge(&unverified, None), Judgement::Broken);

    // One app short is a DHIS2 with no CHAP user interface in it.
    assert_eq!(
        judge(
            &created,
            Some(&apps_report(&[AppOutcome::Installed, AppOutcome::Failed])),
        ),
        Judgement::Broken
    );
    assert_eq!(
        judge(&created, Some(&apps_report(&[AppOutcome::Installed]))),
        Judgement::Broken
    );

    // And `--offline`, which installs none of them and reports the skip:
    // nothing was looked at, so nothing is judged.
    assert_eq!(judge(&created, None), Judgement::Unknown);
}

/// The record moves on evidence, in either direction, and a run with no
/// evidence leaves it alone.
///
/// The transition that matters is the middle one: a deployment that has
/// been connected, then `chaps dhis2 connect --offline`, which proves the
/// route and skips the apps. The record is still there afterwards, because
/// nothing that run saw says it stopped being true. The same deployment
/// with a route nothing answered through loses it, and the hint comes back.
#[test]
fn a_run_that_could_not_look_leaves_the_record_as_it_found_it() {
    let temp = tempfile::tempdir().unwrap();
    let ctx = ctx(temp.path());
    let mut project = Project {
        dir: temp.path().to_path_buf(),
        state: crate::project::ProjectState::default(),
    };
    project.state.components.set_enabled(Component::Dhis2, true);
    project.state.components.dhis2.connected_at = Some(CONNECTED.to_string());
    project.save().unwrap();
    let on_disk = || {
        Project::load(temp.path())
            .unwrap()
            .state
            .components
            .dhis2
            .connected_at
    };

    // The `--offline` shape, on a deployment chaps did connect.
    assert_eq!(
        record_connect(&ctx, &mut project, Judgement::Unknown).unwrap(),
        ConnectRecord::Unchanged
    );
    assert_eq!(on_disk().as_deref(), Some(CONNECTED));

    // Evidence of breakage, on the same deployment.
    assert_eq!(
        record_connect(&ctx, &mut project, Judgement::Broken).unwrap(),
        ConnectRecord::Cleared
    );
    assert_eq!(project.state.components.dhis2.connected_at, None);
    // On disk, not only in memory: the next command reads the file.
    assert_eq!(on_disk(), None);
    assert!(project.state.components.dhis2_needs_connecting());

    // A deployment with no record is given none by a run that judged
    // nothing, and a broken run has nothing left to forget.
    assert_eq!(
        record_connect(&ctx, &mut project, Judgement::Unknown).unwrap(),
        ConnectRecord::Unchanged
    );
    assert_eq!(
        record_connect(&ctx, &mut project, Judgement::Broken).unwrap(),
        ConnectRecord::Unchanged
    );
    assert_eq!(on_disk(), None);

    // And a run that got there stamps the time it ran.
    assert_eq!(
        record_connect(&ctx, &mut project, Judgement::Connected).unwrap(),
        ConnectRecord::Recorded
    );
    let at = on_disk().expect("recorded");
    assert!(at.ends_with('Z'), "a UTC timestamp: {at}");
    assert!(!project.state.components.dhis2_needs_connecting());

    // Which an `--offline` run after it keeps exactly as it is.
    assert_eq!(
        record_connect(&ctx, &mut project, Judgement::Unknown).unwrap(),
        ConnectRecord::Unchanged
    );
    assert_eq!(on_disk().as_deref(), Some(at.as_str()));
}

/// The record says what it is and what it is not, in the place a reader
/// could otherwise take it for a verdict.
#[test]
fn the_record_line_never_claims_the_route_is_right() {
    let recorded = record_lines(ConnectRecord::Recorded, &out());
    assert_eq!(
        recorded,
        "recorded in `.chaps/components.yaml`, so `chaps up` and `chaps status` stop asking\n  \
             a note that this ran, not proof the route is still right; \
             `chaps dhis2 show` asks DHIS2\n"
    );

    let cleared = record_lines(ConnectRecord::Cleared, &out());
    assert!(
        cleared.starts_with("cleared the earlier `chaps dhis2 connect`"),
        "{cleared}"
    );
    assert!(cleared.contains("ask for it again"), "{cleared}");

    // Nothing moved, nothing said: the report is already a full account of
    // what the run did.
    assert_eq!(record_lines(ConnectRecord::Unchanged, &out()), "");
}

/// The opening line says where, what and who - and nothing whatsoever
/// about the password beyond where it was found.
#[test]
fn the_instance_line_never_carries_the_password() {
    let text = instance_line(&instance(), &out());
    assert_eq!(
        text,
        "DHIS2 2.42.6 at http://localhost:18080, as `admin` (the DHIS2 default password)\n"
    );
    assert!(!text.contains("district"), "{text}");

    let from_file = Instance {
        credential_from: CredentialSource::EnvFile,
        ..instance()
    };
    assert!(instance_line(&from_file, &out()).contains("(password from `.env`)"));

    // A token is named by what it is, with the user DHIS2 said owns it,
    // and an external DHIS2 says that it is one.
    let token = Instance {
        user: "ops".to_string(),
        external: true,
        auth: AuthKind::Token,
        credential_from: CredentialSource::Environment,
        ..instance()
    };
    assert_eq!(
        instance_line(&token, &out()),
        "external DHIS2 2.42.6 at http://localhost:18080, as `ops` (API token from \
             CHAPS_DHIS2_TOKEN)\n"
    );

    // An instance that did not say its version still gets a line.
    let bare = Instance {
        version: String::new(),
        ..instance()
    };
    assert!(
        instance_line(&bare, &out()).starts_with("DHIS2 (version unknown) at"),
        "{}",
        instance_line(&bare, &out())
    );
}

/// A repoint says where it pointed before: that is the whole reason the
/// step is not "create if absent".
#[test]
fn a_repointed_route_prints_the_reason_it_was_wrong() {
    let mut route = route_report(RouteOutcome::Repointed);
    route.previous = Some("http://158.39.75.126/stable/**".to_string());
    route.reasons = vec!["it pointed at http://158.39.75.126/stable/**".to_string()];
    let text = human_report(&report(Some(route)), &out());
    assert!(
        text.contains("repointed the `chap` route at http://chap:8000/**"),
        "{text}"
    );
    assert!(
        text.contains("it pointed at http://158.39.75.126/stable/**"),
        "{text}"
    );
    assert!(
        text.contains("verified chap-core answered through it: healthy"),
        "{text}"
    );
}

/// Nothing to do is still said out loud, and named as nothing to do.
#[test]
fn a_route_that_is_already_right_says_there_was_nothing_to_change() {
    let text = human_report(&report(Some(route_report(RouteOutcome::Unchanged))), &out());
    assert!(
        text.contains("already points at http://chap:8000/**; nothing to change"),
        "{text}"
    );
    assert!(text.contains("run `chaps dhis2 show`"), "{text}");
}

/// A route that was written but proved nothing says so rather than
/// claiming the path works.
#[test]
fn an_unverified_route_does_not_claim_chap_core_answered() {
    let mut route = route_report(RouteOutcome::Created);
    route.verified = false;
    route.answered = "HTTP 502 Bad Gateway".to_string();
    let text = human_report(&report(Some(route)), &out());
    assert!(
        text.contains("unverified nothing answered through it: HTTP 502 Bad Gateway"),
        "{text}"
    );
    assert!(!text.contains("chap-core answered through it"), "{text}");
}

#[test]
fn the_app_lines_say_which_of_the_four_happened() {
    let apps = AppsReport {
        apps: vec![
            AppReport {
                name: "Modeling App".to_string(),
                outcome: AppOutcome::Installed,
                version: "7.1.0".to_string(),
                previous: None,
                reason: None,
            },
            AppReport {
                name: "Climate".to_string(),
                outcome: AppOutcome::Moved,
                version: "1.16.2".to_string(),
                previous: Some("1.15.0".to_string()),
                reason: None,
            },
            AppReport {
                name: "Old".to_string(),
                outcome: AppOutcome::Unchanged,
                version: "1.0.0".to_string(),
                previous: None,
                reason: None,
            },
            AppReport {
                name: "Broken".to_string(),
                outcome: AppOutcome::Failed,
                version: String::new(),
                previous: None,
                reason: Some("HTTP 409".to_string()),
            },
        ],
    };
    let text = app_lines(&apps, &out());
    assert!(text.contains("installed Modeling App 7.1.0"), "{text}");
    assert!(
        text.contains("moved Climate from 1.15.0 to 1.16.2"),
        "{text}"
    );
    assert!(text.contains("Old 1.0.0 is already installed"), "{text}");
    assert!(
        text.contains("failed Broken was not installed: HTTP 409"),
        "{text}"
    );

    let why = failed_apps(&apps).expect("one app failed");
    assert!(why.contains("1 of 4 could not be installed"), "{why}");
    assert!(why.contains("HTTP 409"), "{why}");
    assert_eq!(
        failed_apps(&AppsReport { apps: Vec::new() }),
        None,
        "nothing attempted is not a failure"
    );
}

#[test]
fn the_analytics_lines_tell_a_started_run_from_a_finished_one() {
    let running = AnalyticsReport {
        job: "abc".to_string(),
        started: true,
        finished: false,
        seconds: 0,
        message: String::new(),
        last_success: String::new(),
    };
    assert!(
        analytics_lines(&running, &out()).contains("started the analytics run (job abc)"),
        "{}",
        analytics_lines(&running, &out())
    );

    let adopted = AnalyticsReport {
        started: false,
        ..running.clone()
    };
    assert!(
        analytics_lines(&adopted, &out()).contains("an analytics run was already going"),
        "{}",
        analytics_lines(&adopted, &out())
    );

    let done = AnalyticsReport {
        finished: true,
        seconds: 65,
        message: "Analytics tables updated".to_string(),
        last_success: "2026-09-25T10:01:00.000".to_string(),
        ..running
    };
    let text = analytics_lines(&done, &out());
    assert!(text.contains("analytics finished in 1 minute"), "{text}");
    assert!(text.contains("Analytics tables updated"), "{text}");
    assert!(
        text.contains("DHIS2 records its last analytics success as 2026-09-25T10:01:00.000"),
        "{text}"
    );
}

/// A skipped step is printed, never swallowed.
#[test]
fn a_skipped_step_is_reported_with_the_reason() {
    let mut report = report(Some(route_report(RouteOutcome::Created)));
    report.skipped = vec![dhis2::OFFLINE_APPS.to_string()];
    let text = human_report(&report, &out());
    assert!(text.contains("skipped:"), "{text}");
    assert!(text.contains("App Hub"), "{text}");
}

/// The two apps chaps is about, in the order it installs them.
fn shown_apps(versions: &[&str]) -> Vec<ShownApp> {
    dhis2::HUB_APPS
        .iter()
        .zip(versions)
        .map(|(app, version)| ShownApp {
            name: app.name.to_string(),
            installed: !version.is_empty(),
            version: version.to_string(),
        })
        .collect()
}

#[test]
fn show_names_every_missing_piece_and_the_command_that_fixes_them() {
    let report = ShowReport {
        instance: instance(),
        target: "http://chap:8000/**".to_string(),
        route: None,
        last_analytics: String::new(),
        analytics: dhis2::AnalyticsEvidence::Never,
        apps: shown_apps(&["", ""]),
        missing: vec![
            "there is no `chap` route".to_string(),
            "analytics has never run".to_string(),
            "the Modeling App is not installed".to_string(),
        ],
        next: "run `chaps dhis2 connect` to do the rest".to_string(),
    };
    let text = human_show(&report, &out());
    assert!(text.contains("route      none"), "{text}");
    assert!(text.contains("analytics  never run"), "{text}");
    assert!(
        text.contains("apps       Modeling App not installed, DHIS2 Climate App not installed"),
        "{text}"
    );
    assert!(text.contains("missing: there is no `chap` route"), "{text}");
    assert!(text.contains("run `chaps dhis2 connect`"), "{text}");
}

/// The row is the two apps and nothing else. A real instance lists 29
/// bundled apps of its own, and they are not what this command answers for.
#[test]
fn the_apps_row_is_the_two_apps_chaps_installs_and_no_others() {
    let bundled = serde_json::json!([
        {"name": "Reports", "key": "reports", "version": "100.2.4"},
        {"name": "Maintenance app", "key": "maintenance", "version": "32.34.1-v42.0"},
        // The instance's own spelling of the Modeling App, which is not
        // the App Hub's and not chaps'.
        {"name": "Modeling", "key": "modeling", "version": "7.1.0"},
    ]);
    let apps: Vec<ShownApp> = dhis2::HUB_APPS
        .iter()
        .map(|app| shown_app(app, dhis2::installed_app(&bundled, app.name)))
        .collect();
    assert_eq!(apps.len(), 2, "{apps:?}");
    assert_eq!(apps[0].name, "Modeling App");
    assert!(apps[0].installed);
    assert_eq!(apps[0].version, "7.1.0");
    assert!(!apps[1].installed, "{apps:?}");

    let text = human_show(
        &ShowReport {
            instance: instance(),
            target: "http://chap:8000/**".to_string(),
            route: None,
            last_analytics: String::new(),
            analytics: dhis2::AnalyticsEvidence::Never,
            apps,
            missing: Vec::new(),
            next: String::new(),
        },
        &out(),
    );
    assert!(
        text.contains("apps       Modeling App 7.1.0, DHIS2 Climate App not installed"),
        "{text}"
    );
    for other in ["Reports", "Maintenance", "100.2.4"] {
        assert!(!text.contains(other), "{other} is DHIS2's business: {text}");
    }
}

/// The row a seeded deployment gets must not read as "analytics is done",
/// and the one chaps watched finish must not read as more than that.
#[test]
fn the_analytics_row_says_what_the_timestamp_is_worth() {
    let when = "2026-06-16T07:51:00.093";
    let inherited = analytics_cell(dhis2::AnalyticsEvidence::Unconfirmed, when, &out());
    assert_eq!(
        inherited,
        format!("{when} (unconfirmed on a seeded database)")
    );

    let here = analytics_cell(dhis2::AnalyticsEvidence::RanHere, when, &out());
    assert_eq!(here, format!("{when} (a run finished on this deployment)"));
    // What chaps saw is a run finishing, which is not a row count: a run
    // can finish having written nothing, which is why it says neither
    // "done" nor anything about the tables.
    assert!(!here.contains("done"), "{here}");

    assert_eq!(
        analytics_cell(dhis2::AnalyticsEvidence::Recorded, when, &out()),
        when
    );
    assert_eq!(
        analytics_cell(dhis2::AnalyticsEvidence::Never, "", &out()),
        "never run"
    );
}

#[test]
fn show_on_a_connected_instance_says_it_is_connected() {
    let report = ShowReport {
        instance: instance(),
        target: "http://chap:8000/**".to_string(),
        route: Some(ShownRoute {
            url: "http://chap:8000/**".to_string(),
            disabled: false,
            authorised: true,
            ours: true,
            verified: true,
            answered: "healthy".to_string(),
            token_refused: false,
        }),
        last_analytics: "2026-09-25T10:01:00.000".to_string(),
        analytics: dhis2::AnalyticsEvidence::RanHere,
        apps: shown_apps(&["7.1.0", "1.16.2"]),
        missing: Vec::new(),
        next: "the Modeling App can reach CHAP; open DHIS2 with `chaps open dhis2`".to_string(),
    };
    let text = human_show(&report, &out());
    assert!(text.contains("http://chap:8000/** (healthy)"), "{text}");
    assert!(
        text.contains("Modeling App 7.1.0, DHIS2 Climate App 1.16.2"),
        "{text}"
    );
    assert!(
        text.contains("2026-09-25T10:01:00.000 (a run finished on this deployment)"),
        "{text}"
    );
    assert!(!text.contains("missing:"), "{text}");
    assert!(text.contains("`chaps open dhis2`"), "{text}");
}

/// A route aimed at somebody else's chap-core is the trap the whole step
/// exists for, so `show` says so in the row as well as in the list.
#[test]
fn show_marks_a_route_that_points_at_another_chap_core() {
    let report = ShowReport {
        instance: instance(),
        target: "http://chap:8000/**".to_string(),
        route: Some(ShownRoute {
            url: "http://158.39.75.126/stable/**".to_string(),
            disabled: false,
            authorised: true,
            ours: false,
            verified: false,
            answered: "it points at another chap-core".to_string(),
            token_refused: false,
        }),
        last_analytics: "2026-09-25T10:01:00.000".to_string(),
        analytics: dhis2::AnalyticsEvidence::Recorded,
        apps: shown_apps(&["", ""]),
        missing: vec![
            "the `chap` route points at http://158.39.75.126/stable/**, not at this deployment"
                .to_string(),
        ],
        next: "run `chaps dhis2 connect` to do the rest".to_string(),
    };
    let text = human_show(&report, &out());
    assert!(text.contains("(another chap-core)"), "{text}");
    assert!(text.contains("not at this deployment"), "{text}");
}

#[test]
fn the_missing_apps_are_the_ones_the_instance_does_not_have() {
    let missing = missing_apps(&shown_apps(&["", ""]));
    assert_eq!(missing.len(), 2, "{missing:?}");
    assert!(missing[0].contains("the Modeling App"), "{missing:?}");
    assert!(missing[1].contains("the Climate App"), "{missing:?}");

    assert!(missing_apps(&shown_apps(&["7.1.0", "1.16.2"])).is_empty());

    // A row that is there but not installed is a missing app, not a
    // present one: the row exists either way.
    let missing = missing_apps(&shown_apps(&["7.1.0", ""]));
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert!(missing[0].contains("the Climate App"), "{missing:?}");

    // The instance's own spelling is matched, not compared as text: it
    // lists the Modeling App as `Modeling`, and an installed key is the
    // name in a third spelling again.
    let keyed = serde_json::json!([
        {"name": "Modeling", "key": "modeling", "version": "7.1.0"},
        {"name": "", "key": "dhis2-climate-app", "version": "1.16.2"},
    ]);
    let found: Vec<ShownApp> = dhis2::HUB_APPS
        .iter()
        .map(|app| shown_app(app, dhis2::installed_app(&keyed, app.name)))
        .collect();
    assert!(missing_apps(&found).is_empty(), "{found:?}");
}

/// Every refusal names what is true instead and the command that changes
/// it - and none of them says "open", which is the browser's verb and not
/// what these commands do.
#[test]
fn the_refusals_name_what_is_true_instead_and_the_way_out() {
    assert!(NO_CHAP_CORE.contains("`chaps components enable chap-core`"));
    assert!(NOT_RUNNING.contains("`chaps up`"));
    assert!(NO_DHIS2.contains("`chaps components enable dhis2`"));
    let internal = no_host_port("http://dhis2:8080");
    assert!(internal.contains("http://dhis2:8080"), "{internal}");
    assert!(
        internal.contains("`chaps components enable dhis2 --port N`"),
        "{internal}"
    );
    for text in [NO_CHAP_CORE, NOT_RUNNING, NO_DHIS2, &internal] {
        assert!(!text.contains("open"), "{text}");
    }
}

#[test]
fn an_error_is_reported_on_one_line() {
    assert_eq!(first_line("one\ntwo"), "one");
    assert_eq!(first_line("  one  "), "one");
    assert_eq!(first_line(""), "");
}

/// `localhost` as the chap-core address is wrong for a DHIS2 elsewhere and
/// right for one running directly on this machine; a DHIS2 in Docker on this
/// machine needs `host.docker.internal`, which the note names.
#[test]
fn a_loopback_chap_url_is_judged_by_where_dhis2_runs() {
    let remote = loopback_note("https://dhis2.example.org", "http://localhost:8700").unwrap();
    assert!(remote.contains("give `--chap-url` the address"), "{remote}");

    let here = loopback_note("http://localhost:8080", "http://localhost:8700").unwrap();
    assert!(
        here.contains("is right for a DHIS2 running directly on this machine"),
        "{here}"
    );
    assert!(
        here.contains("`http://host.docker.internal:8700`"),
        "{here}"
    );

    assert_eq!(
        loopback_note("http://localhost:8080", "http://host.docker.internal:8700"),
        None
    );
}
