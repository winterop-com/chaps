//! `varde components list|enable|disable` — what a deployment is made of.
//!
//! The three commands do the same small thing: edit `.varde/components.yaml`
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
use crate::output::{Out, Report};
use crate::project::Project;
use serde::Serialize;
use std::path::PathBuf;

/// One row of `varde components list`, and of its `--json`.
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

/// What `varde components list` prints.
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
    pub notes: Vec<Note>,
    /// Data volumes `disable --purge` removed.
    pub purged: Vec<String>,
    /// Data volumes `disable` left in place, which is what it does without
    /// `--purge`.
    pub kept_volumes: Vec<String>,
    /// Set when the run changed nothing but the read mode of an OCS that was
    /// already on: then `varde restart ocs` applies it, and `varde up` does not.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_mode: Option<ReadMode>,
}

/// The read mode of an OCS that was already on, when that is all a run of
/// `components enable ocs --read-only` or `--read-write` touched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadMode {
    /// The instance config now says this; the running instance reads it at
    /// its next start.
    Changed { read_only: bool },
    /// The instance config already said this.
    Unchanged { read_only: bool },
}

/// `read-only` or `read-write`, as the flags spell the two modes.
fn mode_word(read_only: bool) -> &'static str {
    if read_only { "read-only" } else { "read-write" }
}

/// List every component and whether this deployment has it.
pub fn list(ctx: &Ctx, _args: &ComponentsListArgs) -> Result<()> {
    let project = ctx.project()?;
    let report = ComponentsReport {
        components: rows(&project.state.components, project.effective_api_port()),
    };
    if !ctx.out.json {
        print!("{}", human_list(&report, &ctx.out));
    }
    ctx.out.report(&report, list_summary)
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
    out.table(&["COMPONENT", "STATE", "REACH", "WHAT IT IS"], &rows)
}

/// The lines after the `components list` table.
fn list_summary(lines: &mut Report) {
    lines.hint(
        "`varde components enable NAME` adds a component, `disable NAME` removes it; \
         the set is in `.varde/components.yaml`",
    );
}

/// How much one note of a [`ChangeReport`] matters, as a [`Report`] level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteLevel {
    Info,
    Hint,
    Warning,
}

/// One note of a [`ChangeReport`]. `--json` carries the text alone, so the
/// `notes` list stays a list of strings.
#[derive(Debug, Clone)]
pub struct Note {
    pub level: NoteLevel,
    pub text: String,
}

impl Note {
    pub fn info(text: impl Into<String>) -> Note {
        Note {
            level: NoteLevel::Info,
            text: text.into(),
        }
    }

    pub fn hint(text: impl Into<String>) -> Note {
        Note {
            level: NoteLevel::Hint,
            text: text.into(),
        }
    }

    pub fn warning(text: impl Into<String>) -> Note {
        Note {
            level: NoteLevel::Warning,
            text: text.into(),
        }
    }
}

impl Serialize for Note {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

/// Every note, as one level each.
fn notes_of(level: NoteLevel, texts: impl IntoIterator<Item = String>) -> Vec<Note> {
    texts.into_iter().map(|text| Note { level, text }).collect()
}

/// The lines of one `components enable` or `components disable`.
fn change_summary(report: &ChangeReport, project: &Project, lines: &mut Report) {
    if let Some(mode) = report.read_mode {
        return read_mode_summary(report, mode, lines);
    }
    let name = &report.name;
    let headline = match (report.enabled, report.unchanged) {
        (true, false) => format!("enabled {name}"),
        (true, true) => format!("{name} was already enabled"),
        (false, false) => format!("disabled {name}"),
        (false, true) => format!("{name} was already disabled"),
    };
    match (report.enabled, report.port) {
        (true, Some(port)) => lines.info(format!("{headline} on http://localhost:{port}")),
        // A component with no host port is reached somewhere else rather than
        // not at all, so the line names how it is reached.
        (true, None) if name == "chap-core" => {
            match &project.state.components.chap_core_external {
                Some(external) => {
                    lines.info(format!("{headline} at {}", external.url));
                    lines.hint(format!(
                        "chap-core runs elsewhere; it calls the models back at {}:<port>",
                        external.models_host
                    ))
                }
                None => {
                    lines.info(headline);
                    lines.hint("chap-core publishes no host port; it is reached inside the compose network")
                }
            }
        }
        (true, None) => match &report.base_url {
            Some(base) => lines.info(format!("{headline}, reached through the proxy at {base}")),
            None => {
                lines.info(headline);
                lines.hint(format!(
                    "{name} publishes no host port; it is reached inside the compose network"
                ))
            }
        },
        (false, _) => lines.info(headline),
    };
    if report.read_only == Some(true) {
        lines.hint("a read-only ocs refuses ingestion over HTTP");
    }
    note_lines(&report.notes, lines);
    for path in &report.written {
        lines.hint(format!("wrote {}", label(project, path)));
    }
    for path in &report.removed {
        lines.hint(format!("removed {}", label(project, path)));
    }
    // A disable has already stopped the containers; `up` has work only when
    // the sync changed a file that a running service reads.
    match report.enabled || !report.written.is_empty() {
        true => lines.info("run `varde up` to apply"),
        false => lines.hint("`varde status` shows what runs now"),
    };
}

/// Every note, at its own level.
fn note_lines(notes: &[Note], lines: &mut Report) {
    for note in notes {
        match note.level {
            NoteLevel::Info => lines.info(note.text.as_str()),
            NoteLevel::Hint => lines.hint(note.text.as_str()),
            NoteLevel::Warning => lines.warning(note.text.as_str()),
        };
    }
}

/// The lines of a run that changed only the read mode of an OCS that was on.
///
/// The instance config is a bind mount, which compose does not compare, so
/// `varde up` leaves the container as it is; `varde restart ocs` recreates it
/// (see [`crate::commands::docker::edited_configs`]).
fn read_mode_summary(report: &ChangeReport, mode: ReadMode, lines: &mut Report) {
    let name = &report.name;
    match mode {
        ReadMode::Changed { read_only } => {
            lines.info(format!("{name} is now {}", mode_word(read_only)))
        }
        ReadMode::Unchanged { read_only } => {
            lines.info(format!("{name} is already {}", mode_word(read_only)))
        }
    };
    note_lines(&report.notes, lines);
    match mode {
        ReadMode::Changed { .. } => lines.info(format!("run `varde restart {name}` to apply")),
        ReadMode::Unchanged { .. } => lines.hint("`varde status` shows what runs now"),
    };
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
