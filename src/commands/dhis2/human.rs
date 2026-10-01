//! The human rendering of every `chaps dhis2` report.

use super::*;

/// The line every verb opens with: where DHIS2 is, what it is, and who it was
/// asked as. Never the password, only where it was found.
pub(super) fn instance_line(instance: &Instance, out: &Out) -> String {
    let what = match instance.external {
        true => format!("external DHIS2 {}", display_version(&instance.version)),
        false => format!("DHIS2 {}", display_version(&instance.version)),
    };
    format!(
        "{} at {}, as {} ({})\n",
        out.value(&what),
        out.value(&instance.url),
        out.value(&format!("`{}`", instance.user)),
        out.dim(&dhis2::describe_credential(
            instance.auth,
            instance.credential_from
        )),
    )
}

/// What `use` prints.
pub(super) fn human_use(report: &UseReport, out: &Out) -> String {
    let mut text = String::new();
    let headline = match (report.outcome, &report.external, &report.previous) {
        (UseOutcome::Cleared, _, Some(previous)) => {
            format!("forgot the external DHIS2 at {}", out.value(&previous.url))
        }
        (UseOutcome::Unchanged, None, _) => {
            "no external DHIS2 is recorded; nothing to clear".to_string()
        }
        (UseOutcome::Recorded, Some(external), _) => format!(
            "recorded the external DHIS2 at {} in `.chaps/components.yaml`",
            out.value(&external.url)
        ),
        (UseOutcome::Changed, Some(external), _) => format!(
            "changed the external DHIS2 to {} in `.chaps/components.yaml`",
            out.value(&external.url)
        ),
        (UseOutcome::Unchanged, Some(external), _) => format!(
            "the external DHIS2 at {} is already recorded; nothing changed",
            out.value(&external.url)
        ),
        (_, Some(external), _) => format!(
            "`chaps dhis2` talks to the external DHIS2 at {}",
            out.value(&external.url)
        ),
        (_, None, _) => match report.component {
            true => "no external DHIS2 is recorded; `chaps dhis2` talks to the `dhis2` component"
                .to_string(),
            false => "no external DHIS2 is recorded, and the `dhis2` component is off".to_string(),
        },
    };
    text.push_str(&out.backticks(&headline));
    text.push('\n');
    if let (Some(external), Some(probe)) = (&report.external, &report.probe) {
        let dhis2 = match probe.answered {
            true => format!(
                "{} {}",
                out.value(&external.url),
                out.ok(&format!("({})", probe.answer))
            ),
            false => format!(
                "{} {}",
                out.value(&external.url),
                out.warn(&format!("({})", probe.answer))
            ),
        };
        let login = match (&probe.credential, probe.accepted) {
            (None, _) => out.warn("none"),
            (Some(credential), Some(true)) => format!("{credential} {}", out.ok("(accepted)")),
            (Some(credential), Some(false)) => format!("{credential} {}", out.warn("(refused)")),
            (Some(credential), None) => format!("{credential} {}", out.dim("(not tried)")),
        };
        let connected = match &external.connected_at {
            Some(at) => out.value(at),
            None => out.dim("never recorded"),
        };
        text.push_str(&crate::output::fields_with(
            0,
            &[
                ("dhis2", dhis2),
                ("chap-url", out.value(&external.chap_url)),
                (
                    "route",
                    out.dim(report.target.as_deref().unwrap_or_default()),
                ),
                ("login", login),
                ("connected", connected),
            ],
            &|label| out.key(label),
        ));
        if let Some(problem) = &probe.problem {
            text.push_str(&format!(
                "{} {}\n",
                out.dim("problem:"),
                out.backticks(problem)
            ));
        }
    }
    for note in &report.notes {
        text.push_str(&format!("{} {}\n", out.dim("note:"), out.backticks(note)));
    }
    text.push_str(&out.backticks(&report.next));
    text.push('\n');
    text
}

/// A version to print, or the words for an instance that did not say.
pub(super) fn display_version(version: &str) -> String {
    match version.trim().is_empty() {
        true => "(version unknown)".to_string(),
        false => version.trim().to_string(),
    }
}

/// What `route`, `analytics`, `apps` and `connect` print.
pub(super) fn human_report(report: &Dhis2Report, out: &Out) -> String {
    let mut text = instance_line(&report.instance, out);
    if let Some(route) = &report.route {
        text.push_str(&route_lines(route, out));
    }
    if let Some(apps) = &report.apps {
        text.push_str(&app_lines(apps, out));
    }
    if let Some(analytics) = &report.analytics {
        text.push_str(&analytics_lines(analytics, out));
    }
    for skip in &report.skipped {
        text.push_str(&format!(
            "{} {}\n",
            out.dim("skipped:"),
            out.backticks(skip)
        ));
    }
    if let Some(record) = report.record {
        text.push_str(&record_lines(record, out));
    }
    text.push_str(&out.backticks(&report.next));
    text.push('\n');
    text
}

/// What `connect` says about the note it left behind, in the shape the route
/// block uses: what happened, then the indented line that says what it is
/// worth.
///
/// The caveat is printed where the record is, rather than left to the chapter,
/// because this is the one moment a reader could take it for a verdict. It is a
/// note that this command ran; `chaps dhis2 show` is what asks DHIS2.
pub(super) fn record_lines(record: ConnectRecord, out: &Out) -> String {
    match record {
        ConnectRecord::Unchanged => String::new(),
        ConnectRecord::Recorded => format!(
            "{} in `.chaps/components.yaml`, so `chaps up` and `chaps status` stop asking\n  {}\n",
            out.ok("recorded"),
            out.dim(
                "a note that this ran, not proof the route is still right; \
                 `chaps dhis2 show` asks DHIS2"
            )
        ),
        ConnectRecord::Cleared => format!(
            "{} the earlier `chaps dhis2 connect` from `.chaps/components.yaml`\n  {}\n",
            out.warn("cleared"),
            out.dim("`chaps up` and `chaps status` ask for it again")
        ),
    }
}

fn route_lines(route: &RouteReport, out: &Out) -> String {
    let where_ = out.value(&route.url);
    let mut text = match route.outcome {
        RouteOutcome::Created => format!(
            "{} the `{}` route at {where_}\n",
            out.ok("created"),
            route.code
        ),
        RouteOutcome::Repointed => format!(
            "{} the `{}` route at {where_}\n",
            out.ok("repointed"),
            route.code
        ),
        RouteOutcome::Unchanged => format!(
            "the `{}` route already points at {where_}; nothing to change\n",
            route.code
        ),
    };
    for reason in &route.reasons {
        text.push_str(&format!("  {}\n", out.dim(reason)));
    }
    text.push_str(&match route.verified {
        true => format!(
            "  {} chap-core answered through it: {}\n",
            out.ok("verified"),
            route.answered
        ),
        false => format!(
            "  {} nothing answered through it: {}\n",
            out.warn("unverified"),
            route.answered
        ),
    });
    text
}

pub(super) fn app_lines(apps: &AppsReport, out: &Out) -> String {
    let mut text = String::new();
    for app in &apps.apps {
        let name = out.value(&app.name);
        text.push_str(&match app.outcome {
            AppOutcome::Installed => {
                format!("{} {name} {}\n", out.ok("installed"), app.version)
            }
            AppOutcome::Moved => format!(
                "{} {name} from {} to {}\n",
                out.ok("moved"),
                app.previous.clone().unwrap_or_default(),
                app.version
            ),
            AppOutcome::Unchanged => {
                format!("{name} {} is already installed\n", app.version)
            }
            AppOutcome::Failed => format!(
                "{} {name} was not installed: {}\n",
                out.bad("failed"),
                app.reason.clone().unwrap_or_default()
            ),
        });
    }
    text
}

pub(super) fn analytics_lines(analytics: &AnalyticsReport, out: &Out) -> String {
    let job = out.dim(&format!("(job {})", analytics.job));
    if !analytics.finished {
        return match analytics.started {
            true => format!("{} the analytics run {job}\n", out.ok("started")),
            false => format!("an analytics run was already going {job}\n"),
        };
    }
    let mut text = format!(
        "{} in {} {job}\n",
        out.ok(match analytics.started {
            true => "analytics finished",
            // The distinction matters: this run watched somebody else's job,
            // so the duration is how long it watched, not how long it took.
            false => "the analytics run that was already going finished",
        }),
        crate::output::human_age(Duration::from_secs(analytics.seconds)),
    );
    if !analytics.message.is_empty() {
        text.push_str(&format!("  {}\n", out.dim(&analytics.message)));
    }
    if !analytics.last_success.is_empty() {
        text.push_str(&format!(
            "  {}\n",
            out.dim(&format!(
                "DHIS2 records its last analytics success as {}",
                analytics.last_success
            ))
        ));
    }
    text
}

/// What `show` prints: three rows and whatever is missing.
pub(super) fn human_show(report: &ShowReport, out: &Out) -> String {
    let mut text = instance_line(&report.instance, out);
    let route = match &report.route {
        None => out.warn("none"),
        Some(route) => {
            let mut cell = out.value(&route.url);
            if !route.ours {
                cell = format!("{cell} {}", out.warn("(another chap-core)"));
            }
            if route.disabled {
                cell = format!("{cell} {}", out.warn("(disabled)"));
            }
            if !route.authorised {
                cell = format!(
                    "{cell} {}",
                    out.warn(&format!("(no {} authority)", dhis2::ROUTE_AUTHORITY))
                );
            }
            if route.verified {
                cell = format!("{cell} {}", out.ok(&format!("({})", route.answered)));
            } else if route.ours {
                cell = format!("{cell} {}", out.warn("(nothing answered through it)"));
            }
            cell
        }
    };
    let apps = match report.apps.is_empty() {
        true => out.warn("none"),
        false => report
            .apps
            .iter()
            .map(|app| app_cell(app, out))
            .collect::<Vec<String>>()
            .join(", "),
    };
    text.push_str(&crate::output::fields_with(
        0,
        &[
            ("route", route),
            ("target", out.dim(&report.target)),
            (
                "analytics",
                analytics_cell(report.analytics, &report.last_analytics, out),
            ),
            ("apps", apps),
        ],
        &|label| out.key(label),
    ));
    for missing in &report.missing {
        text.push_str(&format!("{} {}\n", out.dim("missing:"), missing));
    }
    text.push_str(&out.backticks(&report.next));
    text.push('\n');
    text
}

/// One of the two apps in the `apps` row: the version, or that it is absent.
fn app_cell(app: &ShownApp, out: &Out) -> String {
    match app.installed {
        true => format!("{} {}", app.name, out.value(&app.version)),
        false => format!("{} {}", app.name, out.warn("not installed")),
    }
}

/// The `analytics` row: what DHIS2 records, and what chaps knows it is worth.
///
/// A timestamp on its own reads as "analytics is done", and on a seeded
/// deployment it is not that at all - it is the dump's. So the row says which
/// of the two it is, and never claims more than chaps checked: a run that
/// finished is a run that finished, which is not the same as tables with rows
/// in them. Nothing here can see a row count; `lastYears` is the reason that
/// distinction is worth keeping, and `docs/dhis2.md` carries it.
pub(super) fn analytics_cell(evidence: dhis2::AnalyticsEvidence, last: &str, out: &Out) -> String {
    let when = out.value(last.trim());
    match evidence {
        dhis2::AnalyticsEvidence::Never => out.warn("never run"),
        dhis2::AnalyticsEvidence::Recorded => when,
        dhis2::AnalyticsEvidence::RanHere if last.trim().is_empty() => {
            out.ok("a run finished on this deployment")
        }
        dhis2::AnalyticsEvidence::RanHere => {
            format!("{when} {}", out.ok("(a run finished on this deployment)"))
        }
        dhis2::AnalyticsEvidence::Unconfirmed => {
            format!("{when} {}", out.warn("(unconfirmed on a seeded database)"))
        }
    }
}

/// The first line of an error, for a cell that has one line to say it in.
pub(super) fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_string()
}
