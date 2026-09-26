//! `chaps status` — chap-core health and registered services.

use crate::cli::StatusArgs;
use crate::commands::Ctx;
use crate::docker;
use crate::error::Result;
use crate::output::{Out, PanelKind};
use crate::status::{
    ApiHealth, ModelState, ModelStatus, StatusReport, closing_line, exit_failure, hints, status,
};
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
/// Exits non-zero when anything this deployment declares is not where it should
/// be - the API down, a model not registered, a component not up - so
/// `chaps status` can gate a script or a CI step. [`exit_failure`] is that one
/// rule for every deployment shape. The rows have already named what is wrong,
/// so the exit is silent; an API that is not answering gets the one error line
/// it needs, because nothing else on the screen says why.
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
    // Which build a moving tag is on is the caller's to fill in too: `dev` is
    // the same name whatever it points at today, and the digest the image was
    // pulled at is the only thing on the line that says which one that is.
    if report.chap_tag_moving
        && let Some(containers) = containers.as_deref()
    {
        report.chap_build = docker::running_build(containers, crate::compose::API_SERVICE);
    }
    // How much data OCS holds is the caller's to fill in, as the unhealthy
    // containers are: it is read from inside the running container, and this
    // is the half that has docker. An instance that is not running is not
    // measured here at all - `chaps doctor` reads its volume instead, where
    // the question is what a `down --volumes` would destroy.
    if running.contains(crate::compose::OCS_SERVICE)
        && let Some(row) = report
            .components
            .iter_mut()
            .find(|c| c.name == crate::compose::OCS_SERVICE)
    {
        row.data_bytes = docker::ocs_data_bytes(&project);
    }
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
        // rendering collapses to what matters when nothing has run.
        ctx.out.emit(&report, || not_running(&report, &ctx.out))?;
        std::process::exit(1);
    }

    ctx.out.emit(&report, || human(&report, &ctx.out))?;

    match &report.api {
        // The single error line for an API that is not answering as
        // chap-core. Under --json the report already carries it, and a second
        // document on stdout would break single-document parsers.
        ApiHealth::Down { .. } if ctx.out.json => std::process::exit(1),
        ApiHealth::Down { error } => Err(anyhow::anyhow!(down_message(&report, error))),
        // Everything else is the one rule, and it is silent: the model table
        // said which models are missing and the component lines said which
        // component is not up, so an error line would be the third telling.
        _ if exit_failure(&report) => std::process::exit(1),
        _ => Ok(()),
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

/// Which "nothing is running" line this deployment gets.
///
/// [`ApiHealth::Off`] is exactly "chap-core is not a component of this
/// deployment", so the report already carries the answer and the choice needs
/// no project: a deployment that has no CHAP in it is never told that CHAP is
/// not running.
fn nothing_running_line(report: &StatusReport) -> &'static str {
    match report.api {
        ApiHealth::Off => crate::status::NOTHING_RUNNING,
        _ => NOT_RUNNING,
    }
}

/// What a deployment with no containers at all is told.
///
/// The box is the whole answer when the deployment is chap-core and nothing
/// else: a table whose every row says "not running" says nothing the one line
/// does not, and a pipe still gets the one line it has always parsed.
///
/// A deployment with components has more than that to show - which components
/// it is made of, and where each of them will answer - and those rows are
/// recorded state rather than an answer docker had to give, so they are known
/// whether anything is up or not. There they are printed and the verdict goes
/// under them, in the shape a running deployment has. The model table stays
/// out: nothing can have registered with a chap-core that has never started.
fn not_running(report: &StatusReport, out: &Out) -> String {
    let line = nothing_running_line(report);
    if report.components.is_empty() {
        return out.panel_or(
            "Not running",
            &crate::output::hint_lines(line)
                .iter()
                .map(|line| out.backticks(line))
                .collect::<Vec<_>>()
                .join("\n"),
            PanelKind::Warning,
            line,
        );
    }
    let mut text = service_lines(report, out);
    text.push('\n');
    text.push_str(&out.cmd(line));
    text.push('\n');
    text
}

/// The chap-core line and one line per component: the part of the report that
/// says what this deployment is made of and where each piece answers.
///
/// Shared with [`not_running`], where these lines are the whole of what there
/// is to say. The component set is recorded in `.chaps/components.yaml`, so
/// these rows exist whether anything is running or not.
fn service_lines(report: &StatusReport, out: &Out) -> String {
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
        // A moving tag names no image, so the line says which one it is on.
        if report.chap_tag_moving {
            let build = crate::status::moving_build_cell(
                &report.chap_tag,
                report.chap_build.as_deref(),
                report.version.revision.as_deref(),
            );
            if !build.is_empty() {
                text.push_str(&format!("   {}", out.dim(&build)));
            }
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
            "{}{}   {}   {}",
            out.heading(&component.name),
            pad(&component.name),
            component_cell(out, component.state),
            match component.state {
                crate::status::ComponentState::NotRunning => out.dim(&component.reach),
                _ => out.value(&component.reach),
            }
        ));
        // What the instance holds, when it could be had: a count and a size
        // are the two questions an operator opens the OCS landing page for.
        for fact in component_facts(component) {
            text.push_str(&format!("   {}", out.dim(&fact)));
        }
        // Last on the line, like chap-core's `auth:`: an instance that refuses
        // every write over HTTP is something an operator has to be able to see
        // without opening a file.
        if component.read_only {
            text.push_str(&format!("   {}", out.dim("read-only")));
        }
        text.push('\n');
    }
    text
}

/// The human rendering: what the deployment is made of, the model table, then
/// the one line it adds up to and a hint per model that needs something done.
fn human(report: &StatusReport, out: &Out) -> String {
    let up = matches!(report.api, ApiHealth::Up { .. });
    let mut text = service_lines(report, out);

    if !report.models.is_empty() {
        let rows: Vec<Vec<String>> = report
            .models
            .iter()
            .map(|m| {
                vec![
                    m.id.clone(),
                    state_cell(out, m.state),
                    reach_cell(out, m),
                    out.dim(&m.last_ping.clone().unwrap_or_else(|| "-".to_string())),
                ]
            })
            .collect();
        text.push('\n');
        text.push_str(&out.table(&["MODEL", "STATE", "REACH", "LAST PING"], &rows));
    }

    // A deployment chap-core is not a component of has no API to be down and
    // no models to register, so its components are its verdict. Without this
    // it would be the one deployment shape `chaps status` said nothing about
    // at the end, and `closing_line` would name `chaps models enable`, which
    // is refused there.
    if matches!(report.api, ApiHealth::Off) {
        text.push('\n');
        text.push_str(&out.cmd(&crate::status::components_closing_line(&report.components)));
        text.push('\n');
        return text;
    }

    // Everything the API cannot be asked about is left out while it is down:
    // one error line beats a table's worth of consequences.
    if !up {
        return text;
    }

    // The REACH column says `via chap-core` for a model with no host port of
    // its own; the way in is printed once, here, rather than in every row.
    if report.models.iter().any(|m| m.reach == INTERNAL) {
        text.push_str(&out.dim(&format!(
            "\nmodels without a host port are reachable through chap-core at \
             {}/v2/services/<id>/run/",
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

/// The facts one component line carries beyond its address: how many datasets
/// OCS holds and how much disk its data takes.
///
/// Both are silent when they could not be had - an instance that is down, a
/// docker that could not be asked, an older OCS without the JSON list - so
/// the line is the one it has always been rather than one with holes in it.
pub fn component_facts(component: &crate::status::ComponentStatus) -> Vec<String> {
    let mut facts = Vec::new();
    if let Some(count) = component.datasets {
        facts.push(format!(
            "{count} dataset{}",
            if count == 1 { "" } else { "s" }
        ));
    }
    if let Some(bytes) = component.data_bytes {
        facts.push(format!("{} data", crate::backup::human_size(bytes)));
    }
    facts
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

/// The REACH cell: the host port this deployment published, or the fact that
/// there is none and chap-core's proxy is the way in.
///
/// `--json` keeps saying `internal` and `http://localhost:5001`, which is
/// what a script already matches on; this is the column a human reads, and it
/// says the same thing as `chaps models list` and the browser.
fn reach_cell(out: &Out, model: &ModelStatus) -> String {
    if let Some(port) = model.host_port {
        return out.key(&format!("port {port}"));
    }
    if model.reach == INTERNAL {
        return out.dim(VIA_CHAP_CORE);
    }
    // An unmanaged service: whatever URL chap-core has for it.
    dash(&model.reach)
}

/// What [`crate::status::ModelStatus::reach`] says for a model that publishes
/// no host port.
const INTERNAL: &str = "internal";

/// What the table says for it.
const VIA_CHAP_CORE: &str = "via chap-core";

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
    use crate::status::{
        ApiVersion, ComponentStatus, ModelState, ModelStatus, RegisteredService, model_rows,
    };

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
                revision: None,
            },
            chap_tag: "v2.3.1".to_string(),
            chap_tag_moving: false,
            chap_build: None,
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

    /// The component line carries the two things that are not in the address:
    /// where an unpublished instance is actually reached, and whether it
    /// refuses every write.
    #[test]
    fn a_component_line_says_read_only_and_names_the_proxy() {
        use crate::status::{ComponentState, ComponentStatus};

        let mut report = up(Vec::new(), &[], &["chap", "ocs"]);
        report.components = vec![ComponentStatus {
            name: "ocs".to_string(),
            state: ComponentState::Up,
            reach: "internal (proxy: https://ocs.example.org)".to_string(),
            health_url: None,
            read_only: true,
            datasets: None,
            data_bytes: None,
        }];
        let text = human(&report, &Out::default());
        assert_eq!(
            text,
            "chap-core   up   http://localhost:8000   2.3.1   auth: off\n\
             ocs         up   internal (proxy: https://ocs.example.org)   read-only\n\
             \n\
             no models enabled; run `chaps models enable ID` to add one\n"
        );

        // A writable instance says nothing, rather than `read-write`: the
        // default is not news.
        report.components[0].read_only = false;
        report.components[0].reach = "http://localhost:9000".to_string();
        let text = human(&report, &Out::default());
        assert!(
            text.contains("ocs         up   http://localhost:9000\n"),
            "{text}"
        );
        assert!(!text.contains("read-only"), "{text}");
    }

    /// What OCS holds goes on its line when it could be had, and nothing takes
    /// its place when it could not.
    #[test]
    fn the_ocs_line_carries_the_dataset_count_and_the_data_size_when_there_are_any() {
        use crate::status::{ComponentState, ComponentStatus};

        let mut report = up(Vec::new(), &[], &["ocs"]);
        report.components = vec![ComponentStatus {
            name: "ocs".to_string(),
            state: ComponentState::Up,
            reach: "http://localhost:9000".to_string(),
            health_url: Some("http://localhost:9000/health".to_string()),
            read_only: false,
            datasets: Some(3),
            data_bytes: Some(212 * 1024 * 1024),
        }];
        let text = human(&report, &Out::default());
        assert!(
            text.contains("ocs         up   http://localhost:9000   3 datasets   212.0 MB data\n"),
            "{text}"
        );

        // One dataset is singular, and `read-only` stays last on the line.
        report.components[0].datasets = Some(1);
        report.components[0].read_only = true;
        let text = human(&report, &Out::default());
        assert!(
            text.contains(
                "ocs         up   http://localhost:9000   1 dataset   212.0 MB data   read-only\n"
            ),
            "{text}"
        );

        // Neither fact is guaranteed: an instance that did not answer, and a
        // docker that could not be asked, each cost one cell and nothing else.
        report.components[0].datasets = None;
        report.components[0].read_only = false;
        let text = human(&report, &Out::default());
        assert!(
            text.contains("ocs         up   http://localhost:9000   212.0 MB data\n"),
            "{text}"
        );
        report.components[0].data_bytes = None;
        let text = human(&report, &Out::default());
        assert!(
            text.contains("ocs         up   http://localhost:9000\n"),
            "{text}"
        );
        assert!(!text.contains("dataset"), "{text}");
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
             MODEL                STATE       REACH      LAST PING\n\
             chapkit-ewars-model  registered  port 5001  12s ago\n\
             \n\
             all 1 model registered\n\
             \u{20}\u{20}run `chaps models test --all` to check they can run\n"
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
            revision: None,
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
             chapkit-ewars-model               registered               port 5001                       12s ago\n\
             chapkit-rwanda-malaria-bym-model  running, not registered  via chap-core                   -\n\
             auto-arima-chapkit                not running              via chap-core                   -\n\
             some-other-service                unmanaged                http://some-other-service:8000  12s ago\n\
             \n\
             models without a host port are reachable through chap-core at \
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
        assert!(text.contains("port 5001"), "{text}");
        assert!(
            !text.contains("reachable through chap-core"),
            "every model here has a port of its own:\n{text}"
        );
        // The URL is still what `--json` carries, untouched by the column.
        assert_eq!(report.models[0].reach, "http://localhost:5001");
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
                revision: None,
            },
            ..up(vec![], &[("chapkit-ewars-model", None)], &[])
        };
        let text = human(&report, &Out::default());
        assert_eq!(
            text,
            "chap-core   down   http://localhost:8000   v2.3.1 (pinned)   auth: off\n\
             \n\
             MODEL                STATE        REACH          LAST PING\n\
             chapkit-ewars-model  not running  via chap-core  -\n"
        );
        // The reason is the error line `run` returns, printed once.
        assert!(!text.contains("not chap-core"));
        assert!(!text.contains("registered"));
    }

    #[test]
    fn the_never_started_line_replaces_the_whole_report() {
        assert_eq!(NOT_RUNNING, "CHAP is not running; start it with `chaps up`");
    }

    /// One component row.
    fn component(name: &str, state: crate::status::ComponentState, reach: &str) -> ComponentStatus {
        ComponentStatus {
            name: name.to_string(),
            state,
            reach: reach.to_string(),
            health_url: None,
            read_only: false,
            datasets: None,
            data_bytes: None,
        }
    }

    /// A report for a deployment chap-core is not a component of.
    fn without_chap_core(components: Vec<ComponentStatus>) -> StatusReport {
        StatusReport {
            api: ApiHealth::Off,
            components,
            ..up(Vec::new(), &[], &[])
        }
    }

    /// The one command whose job is to say what is up must not name a product
    /// this deployment does not contain, and must not go silent about the
    /// components it does.
    #[test]
    fn a_deployment_without_chap_core_is_never_told_that_chap_is_not_running() {
        use crate::status::ComponentState::NotRunning;

        let report = without_chap_core(vec![
            component("ocs", NotRunning, "http://localhost:9000"),
            component("s3", NotRunning, "internal"),
        ]);
        let line = nothing_running_line(&report);
        assert_eq!(line, crate::status::NOTHING_RUNNING);
        assert!(!line.contains("CHAP"), "{line}");
        assert!(line.contains("`chaps up`"), "{line}");

        // The rows are recorded state, not something docker had to answer, so
        // they are printed even with nothing up: which components this
        // deployment is made of, and where each of them will answer.
        assert_eq!(
            not_running(&report, &Out::default()),
            "ocs   not running   http://localhost:9000\n\
             s3    not running   internal\n\
             \n\
             nothing in this deployment is running; start it with `chaps up`\n"
        );

        // chap-core in the set: the one line is the whole answer again, and it
        // names CHAP because there is one to name.
        let bare = up(Vec::new(), &[], &[]);
        assert_eq!(nothing_running_line(&bare), NOT_RUNNING);
        assert_eq!(not_running(&bare, &Out::default()), NOT_RUNNING);
    }

    /// A deployment that has components and chap-core shows both when nothing
    /// is running: the rows say what it is made of, which the one line cannot.
    #[test]
    fn nothing_running_still_lists_the_components_a_deployment_has() {
        use crate::status::ComponentState::NotRunning;

        let report = StatusReport {
            api: ApiHealth::Down {
                error: "connection refused".to_string(),
            },
            components: vec![component("ocs", NotRunning, "http://localhost:9000")],
            ..up(Vec::new(), &[], &[])
        };
        let text = not_running(&report, &Out::default());
        assert!(
            text.starts_with("chap-core   down   http://localhost:8000"),
            "{text}"
        );
        assert!(
            text.contains("ocs         not running   http://localhost:9000\n"),
            "{text}"
        );
        assert!(text.ends_with(&format!("{NOT_RUNNING}\n")), "{text}");
        // The models are left out: nothing can have registered with a
        // chap-core that has never started.
        assert!(!text.contains("MODEL"), "{text}");
    }

    /// The verdict of a deployment without chap-core is its components. Without
    /// it this is the one deployment shape `chaps status` ended on nothing, and
    /// `closing_line` would name `chaps models enable`, which it refuses.
    #[test]
    fn a_components_only_report_ends_on_a_verdict_about_its_components() {
        use crate::status::ComponentState::{NotRunning, Up};

        let mut report = without_chap_core(vec![
            component("ocs", Up, "http://localhost:9000"),
            component("s3", Up, "internal"),
        ]);
        let text = human(&report, &Out::default());
        assert_eq!(
            text,
            "ocs   up   http://localhost:9000\n\
             s3    up   internal\n\
             \n\
             all 2 components are up\n"
        );
        assert!(
            !text.contains("chap-core"),
            "no line for a component this deployment does not have:\n{text}"
        );
        assert!(!text.contains("model"), "{text}");

        report.components[1].state = NotRunning;
        let text = human(&report, &Out::default());
        assert!(
            text.ends_with("1 of 2 components is not running; start it with `chaps up`\n"),
            "{text}"
        );
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
            host_port: None,
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
