//! `chaps components list|enable|disable` — what a deployment is made of.
//!
//! The three commands do the same small thing: edit `.chaps/components.yaml`
//! and hand over to [`crate::compose::sync()`], which renders one compose file
//! per enabled component and removes the ones that are no longer wanted. That
//! is the same path `init --with` takes, so a component enabled at init and one
//! enabled a week later end up with byte-identical files.

mod disable;
mod enable;

pub use disable::{disable, disabled_between, stop_disabled_components};
pub use enable::{dhis2_tag_moved, enable, request};

use crate::cli::ComponentsListArgs;
use crate::commands::Ctx;
use crate::components::{Component, Components};
use crate::error::Result;
use crate::output::Out;
use crate::project::Project;
use serde::Serialize;
use std::path::PathBuf;

/// One row of `chaps components list`, and of its `--json`.
#[derive(Debug, Serialize)]
pub struct ComponentRow {
    pub name: String,
    pub enabled: bool,
    /// Host port it publishes, `null` when it publishes none.
    pub port: Option<u16>,
    /// Compose file `sync` renders for it, `null` for chap-core, whose files
    /// are the base stack itself.
    pub compose_file: Option<String>,
    pub summary: String,
    /// For chap-core: the chap-core elsewhere this deployment uses instead of
    /// running one, when it names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_url: Option<String>,
}

/// What `chaps components list` prints.
#[derive(Debug, Serialize)]
pub struct ComponentsReport {
    pub components: Vec<ComponentRow>,
}

/// What `enable` and `disable` changed.
#[derive(Debug, Serialize)]
pub struct ChangeReport {
    pub name: String,
    pub enabled: bool,
    pub port: Option<u16>,
    /// The public origin OCS builds its links from, `null` for every other
    /// component and for an OCS instance that has none.
    pub base_url: Option<String>,
    /// Whether the OCS instance config refuses ingestion over HTTP, `null` for
    /// every other component.
    pub read_only: Option<bool>,
    /// Whether the component was already in this state.
    pub unchanged: bool,
    pub written: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    /// Everything worth saying that is not a failure: a host port that is
    /// already spoken for, the S3 heads-up either way round, a scaffolded config
    /// file, the data source variables that just landed in `.env`.
    pub notes: Vec<String>,
    /// Data volumes `disable --purge` removed.
    pub purged: Vec<String>,
    /// Data volumes `disable` left in place, which is what it does without
    /// `--purge`.
    pub kept_volumes: Vec<String>,
}

/// List every component and whether this deployment has it.
pub fn list(ctx: &Ctx, _args: &ComponentsListArgs) -> Result<()> {
    let project = ctx.project()?;
    let report = ComponentsReport {
        components: rows(&project.state.components, project.effective_api_port()),
    };
    ctx.out.emit(&report, || human_list(&report, &ctx.out))
}

/// One row per component. chap-core's port is the API port, which is not in
/// the component block - it is `CHAP_API_PORT` in `.env` when that line is set
/// and `project.yaml`'s otherwise - so it is passed in.
fn rows(components: &Components, api_port: u16) -> Vec<ComponentRow> {
    Component::ALL
        .iter()
        .map(|component| ComponentRow {
            name: component.name().to_string(),
            enabled: components.is_enabled(*component),
            port: match component {
                Component::ChapCore => components.chap_core.enabled.then_some(api_port),
                other => components.port_of(*other),
            },
            compose_file: component.compose_file().map(str::to_string),
            summary: component.summary().to_string(),
            external_url: match component {
                Component::ChapCore => components
                    .chap_core_external
                    .as_ref()
                    .map(|e| e.url.clone()),
                _ => None,
            },
        })
        .collect()
}

fn human_list(report: &ComponentsReport, out: &Out) -> String {
    let rows: Vec<Vec<String>> = report
        .components
        .iter()
        .map(|row| {
            vec![
                row.name.clone(),
                match (row.enabled, &row.external_url) {
                    (true, _) => out.ok("enabled"),
                    (false, Some(_)) => out.ok("external"),
                    (false, None) => out.dim("off"),
                },
                match (row.enabled, row.port, &row.external_url) {
                    (true, Some(port), _) => out.value(&format!("http://localhost:{port}")),
                    (true, None, _) => out.dim("internal"),
                    (false, _, Some(url)) => out.value(url),
                    (false, _, None) => out.dim("-"),
                },
                out.dim(&row.summary),
            ]
        })
        .collect();
    let mut text = out.table(&["COMPONENT", "STATE", "REACH", "WHAT IT IS"], &rows);
    text.push('\n');
    text.push_str(&out.backticks(
        "`chaps components enable NAME` adds one, `disable NAME` takes it away; \
         the set lives in .chaps/components.yaml",
    ));
    text.push('\n');
    text
}

fn human_change(report: &ChangeReport, project: &Project, out: &Out) -> String {
    let verb = if report.enabled {
        "enabled"
    } else {
        "disabled"
    };
    let painted = if report.enabled {
        out.ok(verb)
    } else {
        out.warn(verb)
    };
    let mut text = match (report.enabled, report.port) {
        (true, Some(port)) => format!(
            "{painted} {} on {}\n",
            report.name,
            out.value(&format!("http://localhost:{port}"))
        ),
        // A component with no host port is reached somewhere else rather than
        // not at all, so the proxy that reaches it is named where the address
        // would have been.
        (true, None) if report.name == "chap-core" => {
            match &project.state.components.chap_core_external {
                Some(external) => format!(
                    "{painted} chap-core at {} {}\n",
                    out.value(&external.url),
                    out.dim(&format!(
                        "(elsewhere; models are called back at {}:<port>)",
                        external.models_host
                    ))
                ),
                None => format!(
                    "{painted} chap-core {}\n",
                    out.dim("(no host port; it is reached inside the compose network)")
                ),
            }
        }
        (true, None) => format!(
            "{painted} {} {}\n",
            report.name,
            match &report.base_url {
                Some(base) => out.dim(&format!(
                    "(no host port; reached through the proxy at {base})"
                )),
                None => out.dim("(no host port; it is reached inside the compose network)"),
            }
        ),
        (false, _) => format!("{painted} {}\n", report.name),
    };
    if report.read_only == Some(true) {
        text.push_str(&out.dim("read-only: ingestion over HTTP is refused\n"));
    }
    if report.unchanged {
        text.push_str(&out.dim("(that is what it was already)"));
        text.push('\n');
    }
    for path in &report.written {
        text.push_str(&format!(
            "{}  {}\n",
            out.ok("written"),
            out.dim(&label(project, path))
        ));
    }
    for path in &report.removed {
        text.push_str(&format!(
            "{} {}\n",
            out.bad("removed"),
            out.dim(&label(project, path))
        ));
    }
    for note in &report.notes {
        text.push_str(&format!("{} {note}\n", out.dim("note:")));
    }
    text.push_str(&out.backticks("run `chaps up` to apply"));
    text
}

/// A written path, relative to the project directory.
fn label(project: &Project, path: &std::path::Path) -> String {
    path.strip_prefix(&project.dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests;
