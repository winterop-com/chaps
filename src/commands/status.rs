//! `chaps status` — chap-core health and registered services.
//!
//! Owned by agent C.

use crate::cli::StatusArgs;
use crate::commands::Ctx;
use crate::docker;
use crate::error::Result;
use crate::output::Out;
use crate::status::{ApiHealth, StatusReport, missing_hint, status};
use std::collections::BTreeSet;
use std::time::Duration;

/// Probe the API and print a [`StatusReport`].
///
/// Exits non-zero when the API is down or a model the project enabled has not
/// registered, so `chaps status` can gate a script or a CI step. Under `--json`
/// the report is the only thing printed; the exit code alone signals failure.
pub fn run(ctx: &Ctx, args: &StatusArgs) -> Result<()> {
    let project = ctx.project()?;
    // Without --url the API is wherever this project publishes it, which is
    // not 8000 for a deployment created with `init --api-port`.
    let url = args.url.clone().unwrap_or_else(|| project.api_url());
    let report = status(&project, &url, Duration::from_secs(args.timeout));

    // Which containers are up only sharpens the hint under `missing:`, so
    // docker is asked exactly when there is a hint to print.
    let running = if report.missing.is_empty() || ctx.out.json {
        BTreeSet::new()
    } else {
        docker::running_services(&project)
    };
    ctx.out
        .emit(&report, || human(&report, &ctx.out, &running))?;

    let failure = match &report.api {
        ApiHealth::Down { error } => Some(format!(
            "chap-core at {} is not responding: {error}",
            report.api_url
        )),
        ApiHealth::Up { .. } if !report.missing.is_empty() => Some(format!(
            "{} of {} model service(s) have not registered: {}",
            report.missing.len(),
            report.expected.len(),
            report.missing.join(", ")
        )),
        ApiHealth::Up { .. } => None,
    };

    match failure {
        None => Ok(()),
        // The JSON report already carries `api` and `missing`; a second JSON
        // document on stdout would break single-document parsers, so exit
        // non-zero without printing anything more.
        Some(_) if ctx.out.json => std::process::exit(1),
        Some(msg) => Err(anyhow::anyhow!(msg)),
    }
}

/// The human rendering: an API line, the service table, then what is missing.
fn human(report: &StatusReport, out: &Out, running: &BTreeSet<String>) -> String {
    let mut text = String::new();

    match &report.api {
        ApiHealth::Up { status, message } => {
            text.push_str(&format!("api: up    {} ({status})\n", report.api_url));
            if !message.is_empty() {
                text.push_str(&format!("     {message}\n"));
            }
        }
        ApiHealth::Down { error } => {
            text.push_str(&format!("api: down  {}\n", report.api_url));
            text.push_str(&format!("     {error}\n"));
        }
    }

    text.push('\n');
    if report.registered.is_empty() {
        text.push_str("no services registered\n");
    } else {
        let rows: Vec<Vec<String>> = report
            .registered
            .iter()
            .map(|s| {
                vec![
                    dash(&s.id),
                    dash(&s.version),
                    dash(&s.url),
                    dash(&s.last_ping_at),
                    dash(&s.expires_at),
                ]
            })
            .collect();
        text.push_str(&out.table(&["ID", "VERSION", "URL", "LAST PING", "EXPIRES"], &rows));

        // The URL above is the one the service registered: it resolves on the
        // compose network, not on this machine. This is where a human goes.
        let reachable: Vec<(&str, &String)> = report
            .registered
            .iter()
            .filter_map(|s| report.reach.get(&s.id).map(|url| (s.id.as_str(), url)))
            .collect();
        if !reachable.is_empty() {
            text.push_str("\nreachable at\n");
            let width = reachable.iter().map(|(id, _)| id.len()).max().unwrap_or(0);
            for (id, url) in reachable {
                text.push_str(&format!("  {id:width$}  {url}\n"));
            }
        }
    }

    if !report.missing.is_empty() {
        text.push('\n');
        text.push_str(&format!("missing: {}\n", report.missing.join(", ")));
        if let Some(hint) = missing_hint(&report.missing, running) {
            text.push_str(&format!("  hint: {hint}\n"));
        }
    }
    text
}

/// Empty cells read badly in a table; a dash says "the API did not tell us".
fn dash(value: &str) -> String {
    if value.is_empty() {
        "-".to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::RegisteredService;

    fn service(id: &str, version: &str) -> RegisteredService {
        RegisteredService {
            id: id.to_string(),
            url: format!("http://{id}:8000"),
            display_name: id.to_string(),
            version: version.to_string(),
            last_ping_at: "2026-09-22T09:04:30Z".to_string(),
            expires_at: "2026-09-22T09:09:30Z".to_string(),
        }
    }

    fn up(registered: Vec<RegisteredService>, expected: Vec<&str>) -> StatusReport {
        let expected: Vec<String> = expected.into_iter().map(str::to_string).collect();
        let missing = crate::status::missing_ids(&expected, &registered);
        // Every expected service is internal unless a test says otherwise.
        let reach = expected
            .iter()
            .map(|id| {
                (
                    id.clone(),
                    format!("internal (proxy: http://localhost:8000/v2/services/{id}/run/)"),
                )
            })
            .collect();
        StatusReport {
            api_url: "http://localhost:8000".to_string(),
            api: ApiHealth::Up {
                status: "ok".to_string(),
                message: "CHAP is running".to_string(),
            },
            registered,
            expected,
            missing,
            reach,
        }
    }

    #[test]
    fn a_healthy_report_lists_every_service() {
        let report = up(
            vec![service("chapkit-ewars-model", "1.0.0")],
            vec!["chapkit-ewars-model"],
        );
        let text = human(&report, &Out::default(), &BTreeSet::new());
        assert!(text.starts_with("api: up    http://localhost:8000 (ok)\n"));
        assert!(text.contains("CHAP is running"));
        assert!(text.contains("ID"));
        assert!(text.contains("LAST PING"));
        assert!(text.contains("chapkit-ewars-model"));
        assert!(!text.contains("missing"));
        // A model with no host port is reached through chap-core's proxy.
        assert!(text.contains("reachable at\n"));
        assert!(text.contains(
            "chapkit-ewars-model  internal \
             (proxy: http://localhost:8000/v2/services/chapkit-ewars-model/run/)"
        ));
    }

    #[test]
    fn a_published_model_is_reported_on_its_own_host_port() {
        let mut report = up(
            vec![
                service("chapkit-ewars-model", "1.0.0"),
                service("auto-arima-chapkit", "1.2.0"),
            ],
            vec!["chapkit-ewars-model", "auto-arima-chapkit"],
        );
        report.reach.insert(
            "chapkit-ewars-model".to_string(),
            "http://localhost:5001".to_string(),
        );
        let text = human(&report, &Out::default(), &BTreeSet::new());
        assert!(text.contains("chapkit-ewars-model  http://localhost:5001\n"));
        assert!(text.contains("auto-arima-chapkit   internal (proxy:"));
    }

    #[test]
    fn a_service_the_project_does_not_know_is_left_out_of_the_reach_block() {
        // chap-core may hold a registration from a deployment that is gone; we
        // have nothing to say about how to reach it.
        let report = up(vec![service("stranger", "1.0.0")], vec![]);
        let text = human(&report, &Out::default(), &BTreeSet::new());
        assert!(text.contains("stranger"));
        assert!(!text.contains("reachable at"), "{text}");
    }

    #[test]
    fn missing_services_get_a_hint() {
        let report = up(vec![], vec!["chapkit-ewars-model", "auto-arima-chapkit"]);
        let text = human(&report, &Out::default(), &BTreeSet::new());
        assert!(text.contains("no services registered"));
        assert!(text.contains("missing: chapkit-ewars-model, auto-arima-chapkit"));
        assert!(text.contains("hint: run `chaps logs chapkit-ewars-model`"));
    }

    #[test]
    fn a_down_api_reports_the_reason() {
        let report = StatusReport {
            api_url: "http://localhost:8000".to_string(),
            api: ApiHealth::Down {
                error: "connection refused".to_string(),
            },
            registered: vec![],
            expected: vec!["chapkit-ewars-model".to_string()],
            missing: vec!["chapkit-ewars-model".to_string()],
            reach: Default::default(),
        };
        let text = human(&report, &Out::default(), &BTreeSet::new());
        assert!(text.contains("api: down  http://localhost:8000"));
        assert!(text.contains("connection refused"));
        assert!(text.contains("missing: chapkit-ewars-model"));
    }

    #[test]
    fn unknown_fields_render_as_dashes() {
        let mut svc = service("x", "");
        svc.url = String::new();
        let report = up(vec![svc], vec!["x"]);
        let text = human(&report, &Out::default(), &BTreeSet::new());
        let row = text
            .lines()
            .find(|l| l.starts_with("x "))
            .expect("service row");
        assert!(row.contains('-'), "empty cells become dashes: {row}");
    }
}
