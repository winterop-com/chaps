//! `varde status` — chap-core health and registered services.

use crate::cli::StatusArgs;
use crate::commands::Ctx;
use crate::docker;
use crate::error::Result;
use crate::output::{Out, Report};
use crate::status::{ApiHealth, ModelState, ModelStatus, StatusReport, exit_failure, status};
use std::collections::BTreeSet;
use std::time::Duration;

/// What a project with no containers at all is told, instead of a table of
/// rows that all say the same thing.
const NOT_RUNNING: &str = "Chap is not running; start it with `varde up`";

/// The name the chap-core line opens with, and the one the component lines are
/// padded to line up with.
const CHAP_CORE_LABEL: &str = "chap-core";

/// Probe the API and print a [`StatusReport`].
///
/// Exits non-zero when anything this deployment declares is not where it should
/// be - the API down, a model not registered, a component not up - so
/// `varde status` can gate a script or a CI step. [`exit_failure`] is that one
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
    let mut warnings = Vec::new();
    let containers = match docker::all_containers_or_why(&project) {
        Ok(containers) => Some(containers),
        // A docker that is not installed is nothing to say a word about here:
        // `status --url` is a question about an API. A docker that answered
        // and refused is, because the report is about to be silent about the
        // containers and this is the only place that can say why.
        Err(why) => {
            if why.ran() {
                warnings.push(format!("could not ask docker about this deployment: {why}"));
            }
            None
        }
    };
    let running: BTreeSet<String> = containers
        .as_deref()
        .map(docker::running_of)
        .unwrap_or_default();
    // The read-only mark and the rest of an instance config are read from the
    // file, which a running container read only when it started.
    if let Some(containers) = containers.as_deref() {
        let up: Vec<docker::Container> = containers
            .iter()
            .filter(|c| c.is_running())
            .cloned()
            .collect();
        warnings.extend(crate::commands::docker::edited_config_warnings(
            &project, &up,
        ));
    }
    // A protected deployment needs the token for `/v2/services`; `.env` is
    // where it lives, `CHAP_API_TOKEN` the fallback every request to chap-core
    // shares (`api::token_for`), and a deployment without either reads as
    // `None`. `--url`
    // does not change that: the token belongs to this project either way, and
    // an API that does not want it ignores it.
    ctx.out.verbose(&format!("asking chap-core at {url}"));
    let token = crate::api::token_for(Some(&project.dir));
    let mut report = status(
        &project,
        &url,
        Duration::from_secs(args.timeout),
        &running,
        token.as_deref(),
        args.url.is_none() && containers.is_some(),
    );
    if let Some(containers) = containers.as_deref() {
        let unhealthy: BTreeSet<String> = containers
            .iter()
            .filter(|c| c.is_unhealthy())
            .map(|c| c.service.clone())
            .collect();
        crate::status::mark_unhealthy(&mut report.components, &unhealthy);
        let paused: BTreeSet<String> = containers
            .iter()
            .filter(|c| c.state.eq_ignore_ascii_case("paused"))
            .map(|c| c.service.clone())
            .collect();
        crate::status::mark_paused(&mut report, &paused);
        if let Some(line) = crate::status::paused_line(&paused) {
            warnings.push(line);
        }
    }
    if report.api_elsewhere.is_some() {
        let port = report.api_port;
        let holder = crate::ports::other_deployments(&project.dir, &docker::compose_ls_json)
            .into_iter()
            .find(|other| other.holds(port));
        crate::status::name_elsewhere(&mut report, holder.as_ref());
    }
    // Which unmanaged row is one of our own models under another name is the
    // caller's to settle as well: it takes the container ids.
    if let Some(containers) = containers.as_deref() {
        let ids: Vec<(String, String)> = containers
            .iter()
            .map(|c| (c.service.clone(), c.id.clone()))
            .collect();
        crate::status::link_strays(&mut report.models, &ids);
        for row in &mut report.models {
            row.added_from = project
                .state
                .manual
                .iter()
                .find(|(_, model)| model.service_id == row.id)
                .map(|(id, model)| (id.clone(), model.source()));
            row.young = containers
                .iter()
                .any(|c| c.service == row.id && c.is_running() && c.is_young());
        }
    }
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
    // measured here at all - `varde doctor` reads its volume instead, where
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
    // A refused token is not a container fault: it answered.
    if matches!(report.api, ApiHealth::Down { .. })
        && let Some(containers) = containers.as_deref()
    {
        report.unhealthy = crate::diagnose::failing_containers(
            &project,
            containers,
            Some(crate::compose::API_SERVICE),
        );
    }
    let up = matches!(report.api, ApiHealth::Up { .. });
    // A chap-core that started moments ago is not down, it is starting: with no
    // model depending on its healthcheck, `varde up` returns before it answers.
    let chap_starting = containers.as_deref().is_some_and(|containers| {
        containers.iter().any(|c| {
            c.service == crate::compose::API_SERVICE
                && c.is_running()
                && (c.is_young() || c.health.eq_ignore_ascii_case("starting"))
        })
    });
    report.api_starting = chap_starting;
    // A chap-core of this deployment that has no container at all has no
    // log to read either, so the error names `varde up` instead.
    let chap_absent = args.url.is_none()
        && project.state.components.chap_core_external.is_none()
        && containers.as_deref().is_some_and(|containers| {
            !containers
                .iter()
                .any(|c| c.service == crate::compose::API_SERVICE)
        });

    let never_started = args.url.is_none()
        && !up
        && matches!(containers.as_deref(), Some(containers) if containers.is_empty());
    let warn = |lines: &mut Report| {
        for warning in &warnings {
            lines.warning(warning.as_str());
        }
    };
    if never_started {
        // The JSON report is the same document either way; only the human
        // rendering collapses to what matters when nothing has run.
        if !ctx.out.json {
            print!("{}", not_running_rows(&report, &ctx.out));
        }
        ctx.out.report(&report, |lines| {
            warn(lines);
            not_running_lines(&report, lines);
        })?;
        std::process::exit(1);
    }

    if !ctx.out.json {
        print!("{}", rows(&report, &ctx.out));
    }
    let external = project.state.components.chap_core_external.is_some();
    ctx.out.report(&report, |lines| {
        warn(lines);
        closing(&report, external, lines);
    })?;

    match &report.api {
        // The single error line for an API that is not answering as
        // chap-core. Under --json the report already carries it, and a second
        // document on stdout would break single-document parsers.
        ApiHealth::Down { .. } if ctx.out.json => std::process::exit(1),
        // Not "not responding": something did respond, and the sentence says
        // whose it was and how to swap.
        ApiHealth::Down { .. } if report.api_elsewhere.is_some() => Err(anyhow::anyhow!(
            report.api_elsewhere.clone().unwrap_or_default()
        )),
        ApiHealth::Down { error } => Err(anyhow::anyhow!(down_message(
            &report,
            error,
            chap_starting,
            chap_absent
        ))),
        ApiHealth::Rejected { .. } if ctx.out.json => std::process::exit(1),
        ApiHealth::Rejected { error } => Err(anyhow::anyhow!(
            "chap-core at {} is up and did not accept the API token: {error}",
            report.api_url
        )),
        // Everything else is the one rule, and it is silent: the model table
        // said which models are missing and the component lines said which
        // component is not up, so an error line would be the third telling.
        _ if exit_failure(&report) => std::process::exit(1),
        _ => Ok(()),
    }
}

/// The warning for a running service whose instance config changed after its
/// container started. The service may still run on the old settings.
pub(crate) fn edited_config_line(service: &str) -> String {
    let file = crate::commands::docker::MOUNTED_CONFIGS
        .iter()
        .find(|(name, ..)| *name == service)
        .map(|(_, dir, file)| format!("{dir}/{file}"))
        .unwrap_or_else(|| "its config".to_string());
    format!(
        "`{file}` changed after the {service} container started, so {service} may still use \
         the old settings; run `varde restart {service}` to apply it"
    )
}

/// The error line for an API that is not answering, plus what its own
/// container said about why.
///
/// `chap-core is not responding` is the symptom; the lines under it are the
/// cause, which is otherwise a `varde logs chap` away and forty lines long.
/// When there are none, the line itself carries the way out: a chap-core that
/// is still `starting` is waited for, anything else is read in its log.
fn down_message(report: &StatusReport, error: &str, starting: bool, absent: bool) -> String {
    let mut text = format!("chap-core at {} is not responding: {error}", report.api_url);
    let lines = crate::diagnose::lines(&report.unhealthy);
    if report.chap_core_elsewhere {
        // No container of this deployment runs it, so no `varde logs` can say
        // why; the way out is where it does run.
        text.push_str(
            "; this deployment does not run it, so start it there, or set another URL with \
             `varde components enable chap-core --url URL`",
        );
    } else if starting {
        text.push_str(
            "; its container started moments ago and is still starting, so run `varde status` \
             again in a moment",
        );
    } else if absent {
        text.push_str(&format!(
            "; this deployment has no `{}` container yet, so start it with `varde up`",
            crate::compose::API_SERVICE
        ));
    } else if lines.is_empty() {
        text.push_str(&format!(
            "; `varde logs {}` says why",
            crate::compose::API_SERVICE
        ));
    }
    for line in lines {
        text.push('\n');
        text.push_str(&line);
    }
    text
}

/// The rows a deployment with no containers at all still prints, above the
/// lines from [`not_running_lines`].
///
/// A deployment that is chap-core and nothing else prints no rows: a table
/// whose every row says "not running" says nothing the one line does not, and
/// a pipe still gets the one line it has always parsed.
///
/// A deployment with components has more than that to show - which components
/// it is made of, and where each of them will answer - and those rows are
/// recorded state rather than an answer docker had to give, so they are known
/// whether anything is up or not. The model table stays out: nothing can have
/// registered with a chap-core that has never started.
fn not_running_rows(report: &StatusReport, out: &Out) -> String {
    if report.components.is_empty() {
        return String::new();
    }
    let mut text = service_lines(report, out);
    text.push('\n');
    text
}

/// The chap-core line and one line per component: the part of the report that
/// says what this deployment is made of and where each piece answers.
///
/// Shared with [`not_running`], where these lines are the whole of what there
/// is to say. The component set is recorded in `.varde/components.yaml`, so
/// these rows exist whether anything is running or not.
fn service_lines(report: &StatusReport, out: &Out) -> String {
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
    // The STATE column the same way: measured on what is printed, so `up`
    // under `starting` leaves the addresses in one column.
    let api_state = api_cell(
        out,
        &report.api,
        report.api_container_unhealthy(),
        report.api_starting,
    );
    let states: Vec<String> = report
        .components
        .iter()
        .map(|component| component_cell(out, component.state))
        .collect();
    let elsewhere = out.dim("elsewhere");
    let state_width = states
        .iter()
        .chain(chap_core.then_some(&api_state))
        .chain(report.dhis2_external.is_some().then_some(&elsewhere))
        .map(|cell| console::measure_text_width(cell))
        .max()
        .unwrap_or(0);
    let state_pad =
        |cell: &str| " ".repeat(state_width.saturating_sub(console::measure_text_width(cell)));

    // A deployment without chap-core has no line for it: there is no API on
    // that port, and "down" would read as a fault rather than as a choice.
    if chap_core {
        text.push_str(&format!(
            "{}{}   {}{}   {}",
            out.heading(CHAP_CORE_LABEL),
            pad(CHAP_CORE_LABEL),
            api_state,
            state_pad(&api_state),
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
    for (component, state) in report.components.iter().zip(&states) {
        text.push_str(&format!(
            "{}{}   {state}{}   {}",
            out.heading(&component.name),
            pad(&component.name),
            state_pad(state),
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
    // A DHIS2 recorded with `varde dhis2 use` is part of the picture without
    // being this deployment's to run or to ask: it is named, and the command
    // that asks it is named with it.
    if let Some(url) = &report.dhis2_external {
        let name = crate::compose::DHIS2_SERVICE;
        text.push_str(&format!(
            "{}{}   {elsewhere}{}   {}   {}\n",
            out.heading(name),
            pad(name),
            state_pad(&elsewhere),
            out.value(url),
            out.dim(&out.backticks("`varde dhis2 show` asks it"))
        ));
    }
    text
}

/// The human rendering of the rows: what the deployment is made of and the
/// model table. The lines under them come from [`closing`].
fn rows(report: &StatusReport, out: &Out) -> String {
    let mut text = service_lines(report, out);

    // A refused token hides the registry, so every row would be a guess.
    let registry_unread = matches!(report.api, ApiHealth::Rejected { .. });
    if !report.models.is_empty() && !registry_unread {
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
    // The blank line above the closing lines. An API that is down or refused
    // the token has none: the error line says why.
    if matches!(report.api, ApiHealth::Off | ApiHealth::Up { .. }) {
        text.push('\n');
    }
    text
}

/// The STATE cell of the chap-core line.
///
/// A container that is up and failing its healthcheck is a different answer
/// from a port nobody is listening on, and it is the one that says where to
/// look: the container is there, and it is the container that is wrong.
fn api_cell(out: &Out, api: &ApiHealth, unhealthy: bool, starting: bool) -> String {
    match (api, unhealthy) {
        (ApiHealth::Up { .. }, _) => out.ok("up"),
        (ApiHealth::Rejected { .. }, _) => out.bad("up, token rejected"),
        (ApiHealth::Down { .. }, false) if starting => out.warn("starting"),
        (_, true) => out.bad("down (container unhealthy)"),
        (_, false) => out.bad("down"),
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
        ComponentState::Starting | ComponentState::Paused => out.warn(label),
        ComponentState::Unhealthy | ComponentState::NotRunning => out.bad(label),
    }
}

/// The STATE cell, coloured by what the state means for the operator.
fn state_cell(out: &Out, state: ModelState) -> String {
    let label = state.label();
    match state {
        ModelState::Registered | ModelState::Up => out.ok(label),
        ModelState::RunningNotRegistered
        | ModelState::RunningNotAnswering
        | ModelState::NotConfigured
        | ModelState::Paused => out.warn(label),
        ModelState::NotRunning | ModelState::Unreachable => out.bad(label),
        ModelState::Unmanaged => out.dim(label),
    }
}

/// The REACH cell: the host port this deployment published, or the fact that
/// there is none and chap-core's proxy is the way in.
///
/// `--json` keeps saying `internal` and `http://localhost:5001`, which is
/// what a script already matches on; this is the column a human reads, and it
/// says the same thing as `varde models list` and the browser.
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

mod closing;
#[cfg(test)]
mod tests;

#[cfg(test)]
use closing::nothing_running_line;
use closing::{closing, not_running_lines};
