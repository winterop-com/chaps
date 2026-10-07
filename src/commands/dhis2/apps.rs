//! `varde dhis2 apps`: the Modeling App and the Climate App from the App Hub.

use super::*;

/// `varde dhis2 apps` — install the Modeling App and the Climate App.
pub fn apps(ctx: &Ctx, args: &Dhis2AppsArgs) -> Result<()> {
    if ctx.registry.offline {
        return Err(anyhow::anyhow!(dhis2::OFFLINE_APPS));
    }
    let session = open_session(ctx, &args.common)?;
    let apps = install_apps(ctx, &session);
    let failed = failed_apps(&apps);
    let report = Dhis2Report {
        instance: session.instance(),
        next: "open DHIS2 with `varde open dhis2`; the Modeling App is in its apps menu"
            .to_string(),
        route: None,
        apps: Some(apps),
        analytics: None,
        skipped: Vec::new(),
        record: None,
    };
    ctx.out
        .report(&report, |lines| report_summary(&report, lines))?;
    match failed {
        Some(why) => Err(anyhow::anyhow!(why)),
        None => Ok(()),
    }
}

/// Install both apps, taking each one as far as it goes.
///
/// One app's failure does not stop the other: they are independent, and an
/// operator who can get the Modeling App installed while the Climate App is
/// having a bad day is better off than one who gets neither.
pub(super) fn install_apps(ctx: &Ctx, session: &Session) -> AppsReport {
    let listing = session
        .dhis2
        .get_json(dhis2::APPS_PATH)
        .unwrap_or(serde_json::Value::Null);
    let hub = dhis2::app_hub_base();
    AppsReport {
        apps: dhis2::HUB_APPS
            .iter()
            .map(|app| match install_app(ctx, session, &hub, app, &listing) {
                Ok(report) => report,
                Err(why) => AppReport {
                    name: app.name.to_string(),
                    outcome: AppOutcome::Failed,
                    version: String::new(),
                    previous: None,
                    reason: Some(first_line(&why.to_string())),
                },
            })
            .collect(),
    }
}

/// Resolve one app's version on the App Hub and have DHIS2 install it.
fn install_app(
    ctx: &Ctx,
    session: &Session,
    hub: &str,
    app: &HubAppRef,
    listing: &serde_json::Value,
) -> Result<AppReport> {
    let published = dhis2::fetch_hub_app(hub, app.id, dhis2::DEFAULT_TIMEOUT)?;
    // One name in the report, whichever spelling each side uses: the App Hub
    // publishes the Modeling App as `Modeling` and an instance lists it under a
    // third name again, so a report that used all three would read as three
    // apps. The App Hub's own is still what the instance's listing is matched
    // on, because that is the spelling the instance got its name from.
    let name = app.name.to_string();
    let published_as = match published.name.trim().is_empty() {
        true => app.name,
        false => published.name.trim(),
    };
    let Some(version) = dhis2::pick_version(&published.versions, &session.ready.version) else {
        return Err(anyhow::anyhow!(
            "the App Hub publishes no version of {name} that DHIS2 {} can run; install it from \
             DHIS2's own App Management page",
            display_version(&session.ready.version)
        ));
    };
    let installed = dhis2::installed_app(listing, published_as);
    if let Some(installed) = &installed
        && dhis2::compare_versions(&installed.version, &version.version)
            == std::cmp::Ordering::Equal
    {
        return Ok(AppReport {
            name,
            outcome: AppOutcome::Unchanged,
            version: version.version.clone(),
            previous: None,
            reason: None,
        });
    }

    let path = dhis2::install_path(&version.id);
    ctx.out
        .verbose(&format!("installing {name} {}", version.version));
    // The instance downloads the app itself, so this one request is a transfer.
    let client = session.dhis2.with_timeout(dhis2::INSTALL_TIMEOUT);
    let answer = client.send("POST", &path, None)?;
    if !answer.is_success() {
        return Err(client.status_error(&path, &answer));
    }
    Ok(AppReport {
        name,
        outcome: match &installed {
            Some(_) => AppOutcome::Moved,
            None => AppOutcome::Installed,
        },
        version: version.version.clone(),
        previous: installed.map(|app| app.version),
        reason: None,
    })
}

/// The one line the command fails with when an app could not be installed.
pub(super) fn failed_apps(report: &AppsReport) -> Option<String> {
    let failed: Vec<&AppReport> = report
        .apps
        .iter()
        .filter(|app| app.outcome == AppOutcome::Failed)
        .collect();
    let first = failed.first()?;
    Some(format!(
        "{} of {} could not be installed: {}",
        failed.len(),
        report.apps.len(),
        first.reason.clone().unwrap_or_default()
    ))
}
