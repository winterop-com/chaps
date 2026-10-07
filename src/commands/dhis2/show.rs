//! `varde dhis2 show`: what DHIS2 has, asked and reported, changing nothing.

use super::*;

/// `varde dhis2 show` — what this DHIS2 has, changing nothing.
pub fn show(ctx: &Ctx, args: &Dhis2ShowArgs) -> Result<()> {
    let session = open_session(ctx, &args.common)?;
    let target = session.target();
    let route = dhis2::route_in(&session.dhis2.get_json(&dhis2::routes_query())?);
    let token_needed = crate::api::token_for(Some(&session.project.dir)).is_some();
    let shown = route.as_ref().map(|route| {
        let (verified, answered, token_refused) = match route.url == target {
            // Only worth proxying through a route that points here: one aimed
            // at somebody else's chap-core would answer, and the answer would
            // mean nothing about this deployment.
            true => verify_route(&session.dhis2, token_needed),
            false => (false, "it points at another chap-core".to_string(), false),
        };
        ShownRoute {
            url: route.url.clone(),
            disabled: route.disabled,
            authorised: route
                .authorities
                .iter()
                .any(|authority| authority == dhis2::ROUTE_AUTHORITY),
            ours: route.url == target,
            verified,
            answered,
            token_refused,
        }
    });
    let apps = chap_apps(&session)?;
    let analytics = analytics_evidence(ctx, &session);

    let mut missing = Vec::new();
    match &shown {
        None => missing.push("there is no `chap` route".to_string()),
        Some(route) if !route.ours => missing.push(format!(
            "the `chap` route points at {}, not at this deployment",
            route.url
        )),
        Some(route) if route.disabled => missing.push("the `chap` route is disabled".to_string()),
        Some(route) if !route.authorised => missing.push(format!(
            "the `chap` route is missing the {} authority",
            dhis2::ROUTE_AUTHORITY
        )),
        Some(route) if route.token_refused => missing.push(format!(
            "the `chap` route does not carry chap-core's API token: {}; run `varde dhis2 connect` \
             to put it there",
            route.answered
        )),
        // A row that is right in every field and that nothing answers through
        // is the one case the proxied request exists to catch: the app would
        // be told Chap is reachable and then fail on its first request.
        Some(route) if !route.verified => missing.push(format!(
            "nothing answered through the `chap` route: {}; run `varde status` to see whether \
             chap-core is up",
            route.answered
        )),
        Some(_) => {}
    }
    match analytics {
        dhis2::AnalyticsEvidence::Never => missing.push("analytics has never run".to_string()),
        // Not "analytics has never run": it may well have, and varde cannot
        // see the tables to tell. What it can say is that the timestamp beside
        // it proves nothing, and that one command settles the question.
        dhis2::AnalyticsEvidence::Unconfirmed => missing.push(
            "analytics may never have run on this deployment: a seeded database can carry the \
             dump's timestamp, and no run has finished since DHIS2 started; run \
             `varde dhis2 analytics` to settle it"
                .to_string(),
        ),
        dhis2::AnalyticsEvidence::RanHere | dhis2::AnalyticsEvidence::Recorded => {}
    }
    missing.extend(missing_apps(&apps));

    let report = ShowReport {
        instance: session.instance(),
        target,
        route: shown,
        last_analytics: session.ready.last_analytics.clone(),
        analytics,
        apps,
        next: String::new(),
        missing,
    };
    let report = ShowReport {
        next: match report.missing.is_empty() {
            true => {
                "the Modeling App can reach Chap; open DHIS2 with `varde open dhis2`".to_string()
            }
            false => "run `varde dhis2 connect` to do the rest".to_string(),
        },
        ..report
    };
    if !ctx.out.json {
        print!("{}", show_table(&report, &ctx.out));
    }
    ctx.out
        .report(&report, |lines| show_summary(&report, lines))
}

/// The two apps varde installs, as this instance has them.
///
/// **The two, and not the rest.** `show` exists to say whether DHIS2 has what
/// Chap needs; the climate demo ships 29 bundled apps of its own, the same 29
/// every DHIS2 ships, and printing them turned the one row that answers the
/// question into an inventory with the answer somewhere in the middle of it.
/// A count of the others would be a number that is always about thirty and
/// never means anything, so there is none: `GET /api/apps` is one request away
/// for anybody who wants the list.
///
/// Matched on the App Hub's own name rather than on the label a sentence uses,
/// because the two differ: the app published as `DHIS2 Climate App` is `the
/// Climate App` in prose. [`dhis2::installed_app`] is what copes with the third
/// spelling, the instance's own - it lists the Modeling App as `Modeling`.
pub(super) fn chap_apps(session: &Session) -> Result<Vec<ShownApp>> {
    let listing = session.dhis2.get_json(dhis2::APPS_PATH)?;
    Ok(dhis2::HUB_APPS
        .iter()
        .map(|app| shown_app(app, dhis2::installed_app(&listing, app.name)))
        .collect())
}

/// One [`dhis2::HUB_APPS`] row, found or not.
pub(super) fn shown_app(app: &HubAppRef, found: Option<dhis2::InstalledApp>) -> ShownApp {
    ShownApp {
        name: app.name.to_string(),
        installed: found.is_some(),
        version: found.map(|found| found.version).unwrap_or_default(),
    }
}

/// Which of the two apps this instance does not have.
pub(super) fn missing_apps(apps: &[ShownApp]) -> Vec<String> {
    dhis2::HUB_APPS
        .iter()
        .filter(|app| {
            !apps
                .iter()
                .any(|shown| shown.installed && shown.name == app.name)
        })
        .map(|app| format!("{} is not installed", app.label))
        .collect()
}

/// What this deployment's `lastAnalyticsTableSuccess` is actually worth.
///
/// Two things it is not read against: DHIS2's notifier, which says whether a
/// run has finished on *this* instance since it started, and
/// `.varde/components.yaml`, which says whether the database was restored from
/// a seed dump and so came with somebody else's timestamp in it.
///
/// A notifier that cannot be read is a trace line and not a failure - `show`
/// reaches DHIS2 and stops there - and it leaves the verdict on the cautious
/// side, which is the side that does not claim analytics is done.
fn analytics_evidence(ctx: &Ctx, session: &Session) -> dhis2::AnalyticsEvidence {
    let ran_here = match session.dhis2.get_json(&dhis2::jobs_path()) {
        Ok(tasks) => dhis2::finished_here(&tasks),
        Err(why) => {
            ctx.out.verbose(&format!(
                "{} could not be read ({why}), so no analytics run is counted as seen here",
                dhis2::jobs_path()
            ));
            false
        }
    };
    // A seed dump is something varde restores into its own `dhis2_db`; an
    // external DHIS2 came with nothing of varde', so its timestamp is its own.
    let seeded = session.external().is_none()
        && session
            .project
            .state
            .components
            .dhis2_seed_source()
            .is_some();
    dhis2::analytics_evidence(&session.ready.last_analytics, ran_here, seeded)
}
