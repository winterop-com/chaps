//! `chaps open [NAME]` — a component's web interface in a browser.
//!
//! The command exists because `chaps components list` and `chaps status` both
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
    /// Its container is up. Not "it is answering": that is what `chaps status`
    /// asks, and it costs a request per component.
    Yes,
    /// Docker answered and this component has no running container.
    No,
    /// Docker could not be asked at all, so there is nothing to claim either
    /// way.
    Unknown,
    /// It is not a container of this deployment: an external DHIS2 recorded
    /// by `chaps dhis2 use`, which no docker here can say anything about.
    External,
}

/// What `chaps open NAME` did.
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
    /// none, which is a report and not a failure.
    pub opened: bool,
    pub running: Running,
    /// For a running container: whether the address answered one request.
    /// `null` when the container is not running or docker could not say.
    pub answering: Option<bool>,
    /// Everything worth saying that is not a failure.
    pub notes: Vec<String>,
}

/// One row of the listing a bare `chaps open` prints.
#[derive(Debug, Serialize)]
pub struct OpenRow {
    pub name: String,
    /// The address `chaps open NAME` would open, `null` when it would open
    /// nothing.
    pub url: Option<String>,
    /// What that address is, or why there is none.
    pub what: String,
}

/// What a bare `chaps open` prints.
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
    let answering = (running == Running::Yes)
        .then(|| crate::commands::doctor::probe(&url, std::time::Duration::from_secs(3)).is_ok());

    let report = OpenReport {
        name: component.name().to_string(),
        opened: crate::open::spawn_opener(&url),
        answering,
        url,
        page: what,
        running,
        notes,
    };
    ctx.out.emit(&report, || human(&report, &ctx.out))
}

/// Whether this component's container is up, judged exactly as `chaps status`
/// and `chaps doctor` judge it: by the container, not by a request.
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
             `chaps components enable {} --port N` publishes one on this machine instead",
            component.name(),
            component.name()
        )
    })
}

/// The note about the container, which is the half of the answer the address
/// cannot carry.
///
/// A dead port gives the browser a connection error rather than chaps one, so
/// the reason is said here, before the browser has a chance to be blamed for it.
fn running_note(running: Running, component: Component) -> Option<String> {
    match running {
        Running::Yes | Running::External => None,
        Running::No => Some(format!(
            "no {} container is running, so the page will not load yet; \
             run `chaps up` to start this deployment",
            component.service()
        )),
        Running::Unknown => Some(format!(
            "docker could not be asked whether {} is running, so this is the address and not a \
             promise; run `chaps doctor` to see what docker says",
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
        "this API needs a token, and `/docs` is behind it: `chaps auth show --reveal` prints the \
         token to paste into the page's Authorize button"
            .to_string()
    })
}

/// Every component and what `chaps open NAME` would do with it.
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
                            format!("{what}, external, recorded by `chaps dhis2 use`")
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
    let mut text = if report.opened {
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
                    "{} answered at that address; `chaps status` reports the rest of this \
                     deployment",
                    report.name
                ),
                _ => format!(
                    "the {} container is running and did not answer yet; run `chaps status` in \
                     a moment to see when it does",
                    report.name
                ),
            }));
            text.push('\n');
        }
        Running::External => {
            text.push_str(&out.backticks(
                "this is the external DHIS2 recorded by `chaps dhis2 use`; `chaps dhis2 show` \
                 says whether CHAP is connected to it",
            ));
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
              `chaps components list` says what it is made of"
                .to_string(),
            _ => format!(
                "{openable} of them can be opened: run `chaps open NAME`, or `chaps status` to see \
             what is running first"
            ),
        }),
    );
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{Components, Dhis2Component, OcsComponent};

    /// The three answers about the container each say something different, and
    /// only the one that knows nothing is running names `chaps up`.
    #[test]
    fn the_container_note_says_which_of_the_three_it_is() {
        assert_eq!(running_note(Running::Yes, Component::Dhis2), None);
        let no = running_note(Running::No, Component::Dhis2).expect("a note");
        assert!(no.contains("no dhis2 container is running"), "{no}");
        assert!(no.contains("run `chaps up`"), "{no}");
        let unknown = running_note(Running::Unknown, Component::Ocs).expect("a note");
        assert!(unknown.contains("docker could not be asked"), "{unknown}");
        assert!(unknown.contains("run `chaps doctor`"), "{unknown}");
        assert!(
            !unknown.contains("chaps up"),
            "nothing was learned, so nothing is prescribed: {unknown}"
        );
    }

    /// chap-core's service is `chap`, so the note has to name the container and
    /// not the component.
    #[test]
    fn the_container_note_names_the_service_not_the_component() {
        let note = running_note(Running::No, Component::ChapCore).expect("a note");
        assert!(note.contains("no chap container is running"), "{note}");
    }

    /// The proxy note only appears for the instance that has no port of its own,
    /// and it names the way to publish one.
    #[test]
    fn the_proxy_note_is_only_for_an_unpublished_instance() {
        assert_eq!(proxy_note(false, Component::Ocs), None);
        let note = proxy_note(true, Component::Ocs).expect("a note");
        assert!(
            note.contains("`chaps components enable ocs --port N`"),
            "{note}"
        );
    }

    /// The listing names every component, with an address for the ones that
    /// have one and the reason for the ones that do not.
    #[test]
    fn the_listing_covers_every_component_with_its_reason() {
        let components = Components {
            dhis2: Dhis2Component {
                enabled: true,
                port: Some(18080),
                ..Dhis2Component::default()
            },
            ocs: OcsComponent {
                enabled: true,
                port: None,
                ..OcsComponent::default()
            },
            ..Components::default()
        };
        let rows: Vec<OpenRow> = Component::ALL
            .iter()
            .map(|component| {
                let (url, what) = match resolve(*component, &components, "http://localhost:18000") {
                    Openable::Url { url, what, .. } => (Some(url), what.to_string()),
                    Openable::Internal { inside } => (None, inside),
                    Openable::NoWeb => (None, "no web interface".to_string()),
                    Openable::Off => (None, "off".to_string()),
                };
                OpenRow {
                    name: component.name().to_string(),
                    url,
                    what,
                }
            })
            .collect();
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["chap-core", "ocs", "s3", "dhis2"]);
        assert_eq!(
            rows[0].url.as_deref(),
            Some("http://localhost:18000/docs"),
            "chap-core opens its API documentation"
        );
        assert_eq!(rows[1].url, None, "an unpublished OCS opens nothing");
        assert_eq!(rows[2].url, None, "the object store never opens");
        assert_eq!(rows[3].url.as_deref(), Some("http://localhost:18080"));
    }

    /// The closing line counts what can be opened, and says something else
    /// entirely when nothing can.
    #[test]
    fn the_listing_closes_on_what_can_be_opened() {
        let out = Out::detect(false, true);
        let row = |name: &str, url: Option<&str>| OpenRow {
            name: name.to_string(),
            url: url.map(str::to_string),
            what: String::new(),
        };
        let none = OpenListReport {
            components: vec![row("ocs", None), row("s3", None)],
        };
        let text = human_list(&none, &out);
        assert!(
            text.contains("nothing in this deployment has a web interface"),
            "{text}"
        );
        assert!(text.contains("`chaps components list`"), "{text}");

        let some = OpenListReport {
            components: vec![
                row("chap-core", Some("http://localhost:8000/docs")),
                row("s3", None),
            ],
        };
        let text = human_list(&some, &out);
        assert!(text.contains("1 of them can be opened"), "{text}");
        assert!(text.contains("run `chaps open NAME`"), "{text}");
    }

    /// A machine with no opener is told the address and not that something
    /// failed, and the line names the opener that is missing.
    #[test]
    fn no_opener_reports_the_address_rather_than_a_failure() {
        let out = Out::detect(false, true);
        let report = OpenReport {
            name: "dhis2".to_string(),
            url: "http://localhost:18080".to_string(),
            page: crate::open::DHIS2_PAGE,
            opened: false,
            running: Running::Yes,
            answering: Some(true),
            notes: Vec::new(),
        };
        let text = human(&report, &out);
        assert!(text.contains("http://localhost:18080"), "{text}");
        assert!(
            text.contains("the address above is the whole of it"),
            "{text}"
        );
        assert!(!text.to_lowercase().contains("error"), "{text}");
        let (command, _) = crate::open::opener();
        assert!(text.contains(command), "{text}");
    }

    /// A run that opened something says so first, then whatever is worth
    /// knowing about it, and ends on the command that says whether it answered.
    #[test]
    fn a_successful_open_reports_the_page_then_the_container() {
        let out = Out::detect(false, true);
        let report = OpenReport {
            name: "dhis2".to_string(),
            url: "http://localhost:18080".to_string(),
            page: crate::open::DHIS2_PAGE,
            opened: true,
            running: Running::Yes,
            answering: Some(true),
            notes: Vec::new(),
        };
        let text = human(&report, &out);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "opening the DHIS2 user interface at http://localhost:18080"
        );
        assert_eq!(
            lines[1],
            "dhis2 answered at that address; `chaps status` reports the rest of this deployment"
        );

        // A container that is up and not serving yet is not called answering.
        let starting = OpenReport {
            answering: Some(false),
            ..report
        };
        let text = human(&starting, &out);
        assert!(
            text.contains("the dhis2 container is running and did not answer yet"),
            "{text}"
        );
        assert!(text.contains("run `chaps status` in a moment"), "{text}");
    }

    /// Nothing about the container is claimed when docker said nothing, and the
    /// note is what stands in for the closing line.
    #[test]
    fn an_unknown_container_says_so_and_claims_nothing() {
        let out = Out::detect(false, true);
        let report = OpenReport {
            name: "ocs".to_string(),
            url: "http://localhost:9000".to_string(),
            page: crate::open::OCS_PAGE,
            opened: true,
            running: Running::Unknown,
            answering: None,
            notes: vec![running_note(Running::Unknown, Component::Ocs).expect("a note")],
        };
        let text = human(&report, &out);
        assert!(text.contains("note: docker could not be asked"), "{text}");
        assert!(
            !text.contains("container is running"),
            "nothing is claimed: {text}"
        );
    }
}
