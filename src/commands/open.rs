//! `varde open [NAME]` — a component's web interface in a browser.
//!
//! The command exists because `varde components list` and `varde status` both
//! print an address and then leave you to copy it into a browser. What is worth
//! opening differs per component, and [`crate::open::resolve`] is the one place
//! that decides it, so the browser's `o` key lands on the same page.
//!
//! Every answer other than "here it is" is said rather than swallowed: a
//! component that is off, one that publishes no host port, the object store
//! that serves no interface at all, and a machine with no opener on it.

use crate::cli::OpenArgs;
use crate::commands::Ctx;
use crate::components::Component;
use crate::docker;
use crate::error::Result;
use crate::open::{NO_WEB_INTERFACE, Openable, internal_reason, off_reason, resolve};
use crate::output::Out;
use crate::project::Project;
use serde::Serialize;

/// Whether the component's container is running, as far as docker would say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Running {
    /// Its container is up. Not "it is answering": that is what `varde status`
    /// asks, and it costs a request per component.
    Yes,
    /// Docker answered and this component has no running container.
    No,
    /// Docker could not be asked at all, so there is nothing to claim either
    /// way.
    Unknown,
    /// It is not a container of this deployment: an external DHIS2 recorded
    /// by `varde dhis2 use`, which no docker here can say anything about.
    External,
}

/// What `varde open NAME` did.
///
/// Only built for a run that had an address: the three answers that open
/// nothing are failures, so there is no shape here with a `null` URL in it.
#[derive(Debug, Serialize)]
pub struct OpenReport {
    pub name: String,
    /// The address that was handed to the opener.
    pub url: String,
    /// What that address lands on, in the words the first line uses.
    pub page: &'static str,
    /// Whether the platform's opener took it. `false` on a machine that has
    /// none, which is a report and not a failure, and whenever `no_browser` is.
    pub opened: bool,
    /// No browser was asked to open it: `--no-browser` said so, and `--json`
    /// implies it, because a program reading the report has no use for a
    /// browser window appearing on the machine it runs on.
    pub no_browser: bool,
    pub running: Running,
    /// For a running container: whether the address answered one request.
    /// `null` when the container is not running or docker could not say.
    pub answering: Option<bool>,
    /// Everything worth saying that is not a failure.
    pub notes: Vec<String>,
}

/// One row of the listing a bare `varde open` prints.
#[derive(Debug, Serialize)]
pub struct OpenRow {
    pub name: String,
    /// The address `varde open NAME` would open, `null` when it would open
    /// nothing.
    pub url: Option<String>,
    /// What that address is, or why there is none.
    pub what: String,
}

/// What a bare `varde open` prints.
#[derive(Debug, Serialize)]
pub struct OpenListReport {
    pub components: Vec<OpenRow>,
}

/// Open one component's web interface, or say why there is none to open.
pub fn run(ctx: &Ctx, args: &OpenArgs) -> Result<()> {
    let project = ctx.project()?;
    let Some(name) = args.name.as_deref() else {
        return list(ctx, &project);
    };
    if let Some((id, model)) = project
        .state
        .models
        .iter()
        .find(|(id, model)| id.as_str() == name || model.service_id == name)
    {
        return open_model(ctx, &project, id, model, args.no_browser);
    }
    let component = Component::from_name(name)?;
    let resolved = resolve(component, &project.state.components, &project.api_base());

    let (url, what, proxied) = match resolved {
        Openable::Url { url, what, proxied } => (url, what, proxied),
        // The three answers that open nothing are refusals: each names what is
        // true instead and the command that changes it, and none of them leaves
        // a browser at an address this deployment has nobody on.
        Openable::Internal { inside } => {
            return Err(anyhow::anyhow!(internal_reason(component, &inside)));
        }
        Openable::NoWeb => return Err(anyhow::anyhow!(NO_WEB_INTERFACE)),
        Openable::Off => return Err(anyhow::anyhow!(off_reason(component))),
    };

    // Docker is only asked once there is something to open, so a refusal costs
    // no `docker compose ps` - and so the tests for those refusals need no
    // docker at all.
    let external =
        component == Component::Dhis2 && project.state.components.dhis2_external.is_some();
    let running = match external {
        true => Running::External,
        false => running(&project, component),
    };
    let mut notes = Vec::new();
    notes.extend(proxy_note(proxied && !external, component));
    notes.extend(running_note(running, component));
    notes.extend(auth_note(&project, component));

    // One request, to the address being opened: the container being up is not
    // the page loading, and any HTTP status - a 401 from a protected `/docs`
    // included - means something is serving it.
    let answering = matches!(running, Running::Yes | Running::External)
        .then(|| crate::commands::doctor::probe(&url, std::time::Duration::from_secs(3)).is_ok());

    let no_browser = browserless(ctx, args.no_browser);
    let report = OpenReport {
        name: component.name().to_string(),
        opened: !no_browser && crate::open::spawn_opener(&url),
        no_browser,
        answering,
        url,
        page: what,
        running,
        notes,
    };
    ctx.out.emit(&report, || human(&report, &ctx.out))
}

/// Where one model's API documentation is, as this machine reaches it.
fn model_docs_url(
    project: &Project,
    id: &str,
    model: &crate::project::EnabledModel,
) -> Result<String> {
    match model.host_port {
        Some(port) => Ok(format!("http://localhost:{port}/docs")),
        None if project.state.components.has_chap_core_api() => {
            Ok(format!("{}docs", project.proxy_url(&model.service_id)))
        }
        None => Err(anyhow::anyhow!(
            "{id} publishes no host port and there is no chap-core to reach it through; \
             run `varde models expose {id}` to publish one"
        )),
    }
}

/// Open one model's API documentation: on its own host port when it publishes
/// one, else through chap-core's proxy, which reaches every registered model.
fn open_model(
    ctx: &Ctx,
    project: &Project,
    id: &str,
    model: &crate::project::EnabledModel,
    no_browser: bool,
) -> Result<()> {
    let url = model_docs_url(project, id, model)?;
    let running = match docker::running_containers_or_why(project) {
        Ok(containers) if docker::running_of(&containers).contains(&model.service_id) => {
            Running::Yes
        }
        Ok(_) => Running::No,
        Err(_) => Running::Unknown,
    };
    let mut notes = Vec::new();
    if running == Running::No {
        notes.push(format!(
            "no {} container is running, so the page will not load yet; run `varde up` to start \
             this deployment",
            model.service_id
        ));
    }
    let answering = (running == Running::Yes)
        .then(|| crate::commands::doctor::probe(&url, std::time::Duration::from_secs(3)).is_ok());
    let no_browser = browserless(ctx, no_browser);
    let report = OpenReport {
        name: model.service_id.clone(),
        opened: !no_browser && crate::open::spawn_opener(&url),
        no_browser,
        answering,
        url,
        page: "the model's API documentation",
        running,
        notes,
    };
    ctx.out.emit(&report, || human(&report, &ctx.out))
}

/// Whether the address is only printed, never handed to a browser: asked for
/// with `--no-browser`, and always under `--json`.
fn browserless(ctx: &Ctx, no_browser: bool) -> bool {
    no_browser || ctx.out.json
}

/// Whether this component's container is up, judged exactly as `varde status`
/// and `varde doctor` judge it: by the container, not by a request.
///
/// A missing docker CLI is [`Running::Unknown`] rather than "not running",
/// because a probe that could not be made has said nothing - and a command that
/// refused on it would refuse on every machine docker cannot be asked on.
fn running(project: &Project, component: Component) -> Running {
    match docker::running_containers_or_why(project) {
        Ok(containers) => {
            if docker::running_of(&containers).contains(component.service()) {
                Running::Yes
            } else {
                Running::No
            }
        }
        Err(_) => Running::Unknown,
    }
}

/// The note for an instance reached at the public origin it records rather than
/// at a port on this machine.
fn proxy_note(proxied: bool, component: Component) -> Option<String> {
    proxied.then(|| {
        format!(
            "{} publishes no host port, so this is the public origin it records; \
             `varde components enable {} --port N` publishes one on this machine instead",
            component.name(),
            component.name()
        )
    })
}

/// The note about the container, which is the half of the answer the address
/// cannot carry.
///
/// A dead port gives the browser a connection error rather than varde one, so
/// the reason is said here, before the browser has a chance to be blamed for it.
fn running_note(running: Running, component: Component) -> Option<String> {
    match running {
        Running::Yes | Running::External => None,
        Running::No => Some(format!(
            "no {} container is running, so the page will not load yet; \
             run `varde up` to start this deployment",
            component.service()
        )),
        Running::Unknown => Some(format!(
            "docker could not be asked whether {} is running, so this is the address and not a \
             promise; run `varde doctor` to see what docker says",
            component.name()
        )),
    }
}

/// The note for an API behind a token, whose `/docs` is behind it too.
///
/// `/health`, `/health/ready` and `/system/info` are the only open paths, so a
/// browser sent to `/docs` on an authenticated deployment is answered 401 - and
/// that looks like a broken deployment rather than a closed door.
fn auth_note(project: &Project, component: Component) -> Option<String> {
    (component == Component::ChapCore && project.state.auth.api_token).then(|| {
        "this API needs a token, and `/docs` is behind it: `varde auth show --reveal` prints the \
         token to paste into the page's Authorize button"
            .to_string()
    })
}

/// Every component and what `varde open NAME` would do with it.
fn list(ctx: &Ctx, project: &Project) -> Result<()> {
    let base = project.api_base();
    let components = Component::ALL
        .iter()
        .map(|component| {
            let (url, what) = match resolve(*component, &project.state.components, &base) {
                Openable::Url { url, what, proxied } => (
                    Some(url),
                    match proxied {
                        true if *component == Component::Dhis2
                            && project.state.components.dhis2_external.is_some() =>
                        {
                            format!("{what}, external, recorded by `varde dhis2 use`")
                        }
                        true => format!("{what}, at the origin it records"),
                        false => what.to_string(),
                    },
                ),
                Openable::Internal { inside } => (
                    None,
                    format!("no host port; reached at {inside} inside the deployment"),
                ),
                Openable::NoWeb => (None, "an S3 API; no web interface to open".to_string()),
                Openable::Off => (None, "not a component of this deployment".to_string()),
            };
            OpenRow {
                name: component.name().to_string(),
                url,
                what,
            }
        })
        .collect();
    let report = OpenListReport { components };
    ctx.out.emit(&report, || human_list(&report, &ctx.out))
}

fn human(report: &OpenReport, out: &Out) -> String {
    let url = out.value(&report.url);
    let mut text = if report.no_browser {
        format!("{} is at {url}\n", report.page)
    } else if report.opened {
        format!("opening {} at {url}\n", report.page)
    } else {
        // No opener is the honest answer on a server, and the URL is the whole
        // of what the reader needs, so it is not painted as a failure.
        let (command, _) = crate::open::opener();
        format!(
            "{} is at {url}\n{}\n",
            report.page,
            out.backticks(&format!(
                "there is no `{command}` on this machine to open it with, so the address above is \
                 the whole of it"
            ))
        )
    };
    for note in &report.notes {
        text.push_str(&format!("{} {}\n", out.dim("note:"), out.backticks(note)));
    }
    match report.running {
        Running::Yes => {
            text.push_str(&out.backticks(&match report.answering {
                Some(true) => format!(
                    "{} answered at that address; `varde status` reports the rest of this \
                     deployment",
                    report.name
                ),
                _ => format!(
                    "the {} container is running and did not answer yet; run `varde status` in \
                     a moment to see when it does",
                    report.name
                ),
            }));
            text.push('\n');
        }
        Running::External => {
            text.push_str(&out.backticks(match report.answering {
                Some(true) => {
                    "the external DHIS2 recorded by `varde dhis2 use` answered at that address; \
                     `varde dhis2 show` says whether Chap is connected to it"
                }
                _ => {
                    "the external DHIS2 recorded by `varde dhis2 use` did not answer at that \
                     address; check that it is up, or record its URL again with `varde dhis2 use \
                     URL`"
                }
            }));
            text.push('\n');
        }
        Running::No | Running::Unknown => {}
    }
    text
}

fn human_list(report: &OpenListReport, out: &Out) -> String {
    let rows: Vec<Vec<String>> = report
        .components
        .iter()
        .map(|row| {
            vec![
                row.name.clone(),
                match &row.url {
                    Some(url) => out.value(url),
                    None => out.dim("-"),
                },
                out.dim(&row.what),
            ]
        })
        .collect();
    let mut text = out.table(&["COMPONENT", "OPENS", "WHAT IT IS"], &rows);
    text.push('\n');
    let openable = report
        .components
        .iter()
        .filter(|row| row.url.is_some())
        .count();
    text.push_str(
        &out.backticks(&match openable {
            0 => "nothing in this deployment has a web interface on this machine; \
              `varde components list` says what it is made of"
                .to_string(),
            _ => format!(
                "{openable} of them can be opened: run `varde open NAME`, or `varde status` to see \
             what is running first"
            ),
        }),
    );
    text.push('\n');
    text
}

#[cfg(test)]
mod tests;
