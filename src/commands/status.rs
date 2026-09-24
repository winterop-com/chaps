//! `chaps status` — chap-core health and registered services.
//!
//! Owned by agent C.

use crate::cli::StatusArgs;
use crate::commands::Ctx;
use crate::docker;
use crate::error::Result;
use crate::output::{Out, PanelKind};
use crate::status::{ApiHealth, ModelState, StatusReport, closing_line, hints, status};
use std::collections::BTreeSet;
use std::time::Duration;

/// What a project with no containers at all is told, instead of a table of
/// rows that all say the same thing.
const NOT_RUNNING: &str = "CHAP is not running; start it with `chaps up`";

/// The name the chap-core line opens with, and the one the component lines are
/// padded to line up with.
const CHAP_CORE_LABEL: &str = "chap-core";

/// Probe the API and print a [`StatusReport`].
///
/// Exits non-zero when the API is down or a model the project enabled has not
/// registered, so `chaps status` can gate a script or a CI step. The table
/// already names every model that has not registered, so that exit is silent;
/// an API that is not answering gets the one error line it needs.
///
/// A deployment that has never been started is reported as exactly that -
/// unless `--url` names an API explicitly, which means the caller is asking
/// about that API and not about the containers on this machine.
pub fn run(ctx: &Ctx, args: &StatusArgs) -> Result<()> {
    let project = ctx.project()?;
    // Without --url the API is wherever this project publishes it, which is
    // not 8000 for a deployment created with `init --api-port`.
    let url = args.url.clone().unwrap_or_else(|| project.api_url());

    // Docker is asked first: the containers decide both what the STATE column
    // says and whether there is a deployment here to report on at all. It is
    // best-effort, as everywhere - `None` means docker could not be asked,
    // which is not the same as a project with no containers.
    let containers = match docker::all_containers_or_why(&project) {
        Ok(containers) => Some(containers),
        // A docker that is not installed is nothing to say a word about here:
        // `status --url` is a question about an API. A docker that answered
        // and refused is, because the report is about to be silent about the
        // containers and this is the only place that can say why.
        Err(why) => {
            if why.ran() {
                crate::output::warn(&format!("could not ask docker about this project: {why}"));
            }
            None
        }
    };
    let running: BTreeSet<String> = containers
        .as_deref()
        .map(docker::running_of)
        .unwrap_or_default();
    // A protected deployment needs the token for `/v2/services`; `.env` is
    // where it lives, and a deployment without one reads as `None`. `--url`
    // does not change that: the token belongs to this project either way, and
    // an API that does not want it ignores it.
    ctx.out.verbose(&format!("asking chap-core at {url}"));
    let token = crate::auth::token_in(&project.dir);
    let mut report = status(
        &project,
        &url,
        Duration::from_secs(args.timeout),
        &running,
        token.as_deref(),
    );
    // When the API does not answer, its container usually knows why and has
    // been saying so in its log. Only asked for when the API is down: a
    // healthy deployment has nothing to diagnose.
    if !matches!(report.api, ApiHealth::Up { .. })
        && let Some(containers) = containers.as_deref()
    {
        report.unhealthy = crate::diagnose::failing_containers(
            &project,
            containers,
            Some(crate::compose::API_SERVICE),
        );
    }
    let up = matches!(report.api, ApiHealth::Up { .. });

    let never_started = args.url.is_none()
        && !up
        && matches!(containers.as_deref(), Some(containers) if containers.is_empty());
    if never_started {
        // The JSON report is the same document either way; only the human
        // rendering collapses to the one line that matters.
        ctx.out.emit(&report, || not_running(&ctx.out))?;
        std::process::exit(1);
    }

    ctx.out.emit(&report, || human(&report, &ctx.out))?;

    match &report.api {
        // The single error line for an API that is not answering as
        // chap-core. Under --json the report already carries it, and a second
        // document on stdout would break single-document parsers.
        ApiHealth::Down { .. } if ctx.out.json => std::process::exit(1),
        ApiHealth::Down { error } => Err(anyhow::anyhow!(down_message(&report, error))),
        // The table said which models are missing and what to do about each
        // one; repeating that as an error would be the third telling.
        ApiHealth::Up { .. } if !report.missing.is_empty() => std::process::exit(1),
        ApiHealth::Up { .. } => Ok(()),
        // chap-core is not part of this deployment, so its API not answering
        // is the expected state, not a failure. A component that is down says
        // so on its own line.
        ApiHealth::Off if report.components.iter().any(|c| c.state.is_problem()) => {
            std::process::exit(1)
        }
        ApiHealth::Off => Ok(()),
    }
}

/// The error line for an API that is not answering, plus what its own
/// container said about why.
///
/// `chap-core is not responding` is the symptom; the lines under it are the
/// cause, which is otherwise a `chaps logs chap` away and forty lines long.
fn down_message(report: &StatusReport, error: &str) -> String {
    let mut text = format!("chap-core at {} is not responding: {error}", report.api_url);
    for line in crate::diagnose::lines(&report.unhealthy) {
        text.push('\n');
        text.push_str(&line);
    }
    text
}

/// The "nothing here has ever run" line, as a panel on a terminal.
///
/// The box is the whole answer in that case, so it is worth drawing; a pipe
/// still gets the one line it has always parsed.
fn not_running(out: &Out) -> String {
    out.panel_or(
        "Not running",
        &crate::output::hint_lines(NOT_RUNNING)
            .iter()
            .map(|line| out.backticks(line))
            .collect::<Vec<_>>()
            .join("\n"),
        PanelKind::Warning,
        NOT_RUNNING,
    )
}

/// The human rendering: the chap-core line, the model table, then the one
/// line it adds up to and a hint per model that needs something done.
fn human(report: &StatusReport, out: &Out) -> String {
    let up = matches!(report.api, ApiHealth::Up { .. });
    let chap_core = !matches!(report.api, ApiHealth::Off);
    let mut text = String::new();
    // The name column is padded to the widest of the lines that are actually
    // printed, and the padding sits outside the styled span so a coloured name
    // is the same width as a plain one.
    let width = report
        .components
        .iter()
        .map(|c| c.name.chars().count())
        .chain(chap_core.then_some(CHAP_CORE_LABEL.chars().count()))
        .max()
        .unwrap_or(0);
    let pad = |name: &str| " ".repeat(width.saturating_sub(name.chars().count()));

    // A deployment without chap-core has no line for it: there is no API on
    // that port, and "down" would read as a fault rather than as a choice.
    if chap_core {
        text.push_str(&format!(
            "{}{}   {}   {}",
            out.heading(CHAP_CORE_LABEL),
            pad(CHAP_CORE_LABEL),
            api_cell(out, up, report.api_container_unhealthy()),
            out.value(&report.api_url)
        ));
        let version = report.version.label();
        if !version.is_empty() {
            text.push_str(&format!("   {}", out.dim(&version)));
        }
        // Last on the line, always present: "off" is the answer an operator
        // has to be able to see, and a blank space would not say it.
        text.push_str(&format!(
            "   {} {}",
            out.dim("auth:"),
            if report.auth {
                out.ok("on")
            } else {
                out.warn("off")
            }
        ));
        text.push('\n');
    }
    // The components sit directly under chap-core, in the order they are
    // rendered into the compose files.
    for component in &report.components {
        text.push_str(&format!(
            "{}{}   {}   {}\n",
            out.heading(&component.name),
            pad(&component.name),
            component_cell(out, component.state),
            match component.state {
                crate::status::ComponentState::NotRunning => out.dim(&component.reach),
                _ => out.value(&component.reach),
            }
        ));
    }

    if !report.models.is_empty() {
        let rows: Vec<Vec<String>> = report
            .models
            .iter()
            .map(|m| {
                vec![
                    m.id.clone(),
                    state_cell(out, m.state),
                    reach_cell(out, &m.reach),
                    out.dim(&m.last_ping.clone().unwrap_or_else(|| "-".to_string())),
                ]
            })
            .collect();
        text.push('\n');
        text.push_str(&out.table(&["MODEL", "STATE", "REACH", "LAST PING"], &rows));
    }

    // Everything the API cannot be asked about is left out while it is down:
    // one error line beats a table's worth of consequences.
    if !up {
        return text;
    }

    // The REACH column says `internal` for a model with no host port of its
    // own; the way in is printed once, here, rather than in every row.
    if report.models.iter().any(|m| m.reach == INTERNAL) {
        text.push_str(&out.dim(&format!(
            "\ninternal models are reachable through chap-core at {}/v2/services/<id>/run/",
            report.api_url
        )));
        text.push('\n');
    }

    text.push('\n');
    // The verdict is the line someone scanning the screen should land on.
    text.push_str(&out.cmd(&closing_line(&report.models)));
    text.push('\n');
    for hint in hints(&report.models, report.auth) {
        text.push_str(&format!("  {}\n", out.backticks(&hint)));
    }
    text
}

/// The STATE cell of the chap-core line.
///
/// A container that is up and failing its healthcheck is a different answer
/// from a port nobody is listening on, and it is the one that says where to
/// look: the container is there, and it is the container that is wrong.
fn api_cell(out: &Out, up: bool, unhealthy: bool) -> String {
    match (up, unhealthy) {
        (true, _) => out.ok("up"),
        (false, true) => out.bad("down (container unhealthy)"),
        (false, false) => out.bad("down"),
    }
}

/// The state cell of one component line, coloured the way the model rows are.
fn component_cell(out: &Out, state: crate::status::ComponentState) -> String {
    use crate::status::ComponentState;
    let label = state.label();
    match state {
        ComponentState::Up => out.ok(label),
        ComponentState::Starting => out.warn(label),
        ComponentState::NotRunning => out.bad(label),
    }
}

/// The STATE cell, coloured by what the state means for the operator.
fn state_cell(out: &Out, state: ModelState) -> String {
    let label = state.label();
    match state {
        ModelState::Registered => out.ok(label),
        ModelState::RunningNotRegistered => out.warn(label),
        ModelState::NotRunning => out.bad(label),
        ModelState::Unmanaged => out.dim(label),
    }
}

/// The REACH cell: a URL is a thing to click, `internal` is a fact about the
/// deployment rather than an address.
fn reach_cell(out: &Out, reach: &str) -> String {
    let cell = dash(reach);
    if cell == INTERNAL {
        out.dim(&cell)
    } else {
        cell
    }
}

/// The REACH cell of a model that publishes no host port.
const INTERNAL: &str = "internal";

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
    use crate::status::{ApiVersion, ModelState, ModelStatus, RegisteredService, model_rows};

    /// A fixed "now", so the ages in these tests do not move.
    const NOW: u64 = 1_790_147_400;

    fn service(id: &str, version: &str) -> RegisteredService {
        RegisteredService {
            id: id.to_string(),
            url: format!("http://{id}:8000"),
            display_name: id.to_string(),
            version: version.to_string(),
            last_ping_at: crate::backup::timestamp(NOW - 12),
            expires_at: crate::backup::timestamp(NOW + 288),
        }
    }

    /// A report of a healthy API, with `enabled` as the project's models
    /// (service id and host port) and `running` as the containers that are up.
    fn up(
        registered: Vec<RegisteredService>,
        enabled: &[(&str, Option<u16>)],
        running: &[&str],
    ) -> StatusReport {
        let enabled: Vec<(String, Option<u16>)> = enabled
            .iter()
            .map(|(id, port)| (id.to_string(), *port))
            .collect();
        let expected: Vec<String> = enabled.iter().map(|(id, _)| id.clone()).collect();
        let missing = crate::status::missing_ids(&expected, &registered);
        let running: BTreeSet<String> = running.iter().map(|id| id.to_string()).collect();
        let models = model_rows(&enabled, &registered, &running, NOW);
        let unmanaged = models
            .iter()
            .filter(|m| m.state == ModelState::Unmanaged)
            .map(|m| m.id.clone())
            .collect();
        StatusReport {
            project: Some("chapx-1ab2c3".to_string()),
            api_url: "http://localhost:8000".to_string(),
            api_port: 8000,
            api_port_source: crate::project::ApiPortSource::Project,
            api: ApiHealth::Up {
                status: "success".to_string(),
                message: "healthy".to_string(),
            },
            version: ApiVersion {
                value: "2.3.1".to_string(),
                pinned: false,
            },
            registered,
            expected,
            missing,
            reach: Default::default(),
            models,
            unmanaged,
            auth: false,
            components: Vec::new(),
            unhealthy: Vec::new(),
        }
    }

    #[test]
    fn a_healthy_report_is_a_line_a_table_and_a_verdict() {
        let report = up(
            vec![service("chapkit-ewars-model", "1.0.0")],
            &[("chapkit-ewars-model", Some(5001))],
            &["chap", "chapkit-ewars-model"],
        );
        let text = human(&report, &Out::default());
        assert_eq!(
            text,
            "chap-core   up   http://localhost:8000   2.3.1   auth: off\n\
             \n\
             MODEL                STATE       REACH                  LAST PING\n\
             chapkit-ewars-model  registered  http://localhost:5001  12s ago\n\
             \n\
             all 1 model registered\n"
        );
    }

    #[test]
    fn the_chap_core_line_ends_in_whether_the_api_is_protected() {
        let mut report = up(
            vec![service("chapkit-ewars-model", "1.0.0")],
            &[("chapkit-ewars-model", Some(5001))],
            &["chap", "chapkit-ewars-model"],
        );
        assert!(
            human(&report, &Out::default())
                .starts_with("chap-core   up   http://localhost:8000   2.3.1   auth: off\n")
        );

        report.auth = true;
        assert!(
            human(&report, &Out::default())
                .starts_with("chap-core   up   http://localhost:8000   2.3.1   auth: on\n")
        );

        // A chap-core that publishes no version of its own still gets the
        // cell, and it stays last on the line.
        report.version = ApiVersion {
            value: String::new(),
            pinned: true,
        };
        assert!(
            human(&report, &Out::default())
                .starts_with("chap-core   up   http://localhost:8000   auth: on\n")
        );
    }

    #[test]
    fn the_layout_names_every_state_once_and_hints_once_per_problem() {
        let report = up(
            vec![
                service("chapkit-ewars-model", "1.0.0"),
                service("some-other-service", "0.1.0"),
            ],
            &[
                ("chapkit-ewars-model", Some(5001)),
                ("chapkit-rwanda-malaria-bym-model", None),
                ("auto-arima-chapkit", None),
            ],
            &[
                "chap",
                "chapkit-ewars-model",
                "chapkit-rwanda-malaria-bym-model",
            ],
        );
        let text = human(&report, &Out::default());
        assert_eq!(
            text,
            "chap-core   up   http://localhost:8000   2.3.1   auth: off\n\
             \n\
             MODEL                             STATE                    REACH                           LAST PING\n\
             chapkit-ewars-model               registered               http://localhost:5001           12s ago\n\
             chapkit-rwanda-malaria-bym-model  running, not registered  internal                        -\n\
             auto-arima-chapkit                not running              internal                        -\n\
             some-other-service                unmanaged                http://some-other-service:8000  12s ago\n\
             \n\
             internal models are reachable through chap-core at \
             http://localhost:8000/v2/services/<id>/run/\n\
             \n\
             2 of 3 models are not registered.\n\
             \x20 chapkit-rwanda-malaria-bym-model: restart it with \
             `chaps restart --all chapkit-rwanda-malaria-bym-model`\n\
             \x20 auto-arima-chapkit: start CHAP with `chaps up`, \
             then `chaps logs auto-arima-chapkit`\n"
        );
        // The proxy URL appears once, not once per internal row, and the
        // verdict carries no `error:` line of its own.
        assert_eq!(text.matches("/v2/services/<id>/run/").count(), 1);
        assert!(!text.contains("error"), "{text}");
        assert!(!text.contains("missing:"), "{text}");
    }

    #[test]
    fn a_published_model_shows_its_host_port_and_no_proxy_line() {
        let report = up(
            vec![service("chapkit-ewars-model", "1.0.0")],
            &[("chapkit-ewars-model", Some(5001))],
            &[],
        );
        let text = human(&report, &Out::default());
        assert!(text.contains("http://localhost:5001"));
        assert!(
            !text.contains("internal models are reachable"),
            "nothing is internal here:\n{text}"
        );
    }

    #[test]
    fn a_project_with_no_models_still_says_something() {
        let report = up(vec![], &[], &[]);
        let text = human(&report, &Out::default());
        assert!(text.starts_with("chap-core   up   http://localhost:8000"));
        assert!(!text.contains("MODEL"), "no table for no rows:\n{text}");
        assert!(text.ends_with("no models enabled; run `chaps models enable ID` to add one\n"));
    }

    #[test]
    fn a_down_api_prints_the_state_and_leaves_the_verdict_to_the_error_line() {
        let report = StatusReport {
            api: ApiHealth::Down {
                error: "port 8000 answers but it is not chap-core (got text/html)".to_string(),
            },
            version: ApiVersion {
                value: "v2.3.1".to_string(),
                pinned: true,
            },
            ..up(vec![], &[("chapkit-ewars-model", None)], &[])
        };
        let text = human(&report, &Out::default());
        assert_eq!(
            text,
            "chap-core   down   http://localhost:8000   v2.3.1 (pinned)   auth: off\n\
             \n\
             MODEL                STATE        REACH     LAST PING\n\
             chapkit-ewars-model  not running  internal  -\n"
        );
        // The reason is the error line `run` returns, printed once.
        assert!(!text.contains("not chap-core"));
        assert!(!text.contains("registered"));
    }

    #[test]
    fn the_never_started_line_replaces_the_whole_report() {
        assert_eq!(NOT_RUNNING, "CHAP is not running; start it with `chaps up`");
    }

    #[test]
    fn a_row_with_nothing_in_a_cell_prints_a_dash() {
        let mut report = up(
            vec![service("x", "")],
            &[("chapkit-ewars-model", None)],
            &[],
        );
        // An unmanaged service chap-core has no URL for.
        report.models.push(ModelStatus {
            id: "y".to_string(),
            state: ModelState::Unmanaged,
            reach: String::new(),
            last_ping: None,
        });
        let text = human(&report, &Out::default());
        let row = text
            .lines()
            .find(|l| l.starts_with("y "))
            .expect("the row is there");
        assert!(row.contains('-'), "empty cells become dashes: {row}");
    }
}
