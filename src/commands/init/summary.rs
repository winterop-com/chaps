//! What `init` prints once the directory is written.

use super::chap_core::ChapCore;
use super::env::{EnvAction, Secrets};
use crate::components::{Components, DHIS2_FIRST_START_NOTE, S3_SOON_NOTE, S3_WITHOUT_OCS_NOTE};
use crate::compose::ApplyReport;
use crate::output::Out;
use crate::project::Project;
use crate::registry::Registry;
use std::path::{Path, PathBuf};

/// What the summary says for a deployment that enables no models.
pub(super) const NO_MODELS: &str = "No models enabled; run `chaps models enable ID` to add one.";

/// The same for a deployment chap-core is not a component of, where a model
/// enabled later runs on its own: it registers nowhere and is published on a
/// host port, which is the one thing worth knowing before enabling one.
pub(super) const NO_MODELS_WITHOUT_CHAP_CORE: &str = "No models enabled; run `chaps models enable ID` to add one. \
     Without chap-core each runs on its own, on a published host port.";

/// Which of the two the summary prints, as a pure function of the component
/// set so the choice is testable without a deployment on disk.
///
/// Neither, for a deployment of components without chap-core - OCS, DHIS2 -
/// where models were not the point and a line about adding one is noise. A
/// deployment of nothing but models (`--only none`) still gets it.
pub(super) fn no_models_line(components: &Components) -> Option<&'static str> {
    // A chap-core elsewhere in front of components of this deployment's own
    // (a DHIS2 for the chap-core being developed) has its models there.
    if components.chap_core_external.is_some() && !components.enabled().is_empty() {
        None
    } else if components.has_chap_core_api() {
        Some(NO_MODELS)
    } else if components.enabled().is_empty() {
        Some(NO_MODELS_WITHOUT_CHAP_CORE)
    } else {
        None
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "one summary line per piece init decided"
)]
pub(super) fn summary(
    out: &Out,
    dir: &Path,
    written: &[PathBuf],
    report: &ApplyReport,
    registry: &Registry,
    env: EnvAction,
    chap_core: &ChapCore,
    project: &Project,
    secrets: Option<&Secrets>,
    dropped_notes: &[String],
) -> String {
    let mut text = format!(
        "{}\n\n{}\n",
        out.heading(&format!("Initialized a chaps project in {}", dir.display())),
        out.heading("Wrote:")
    );
    for path in written {
        text.push_str(&format!("  {}\n", out.dim(&file_label(dir, path))));
    }
    if env == EnvAction::Kept {
        text.push_str(&format!("\n{}\n", out.dim("kept .env (already present)")));
    }
    // The component list stands on its own, above the block of addresses the
    // enabled ones contribute, so the two read as answer and detail.
    let components = &project.state.components;
    text.push_str(&format!(
        "\n{} {}\n",
        out.key("components:"),
        out.value(&components.label())
    ));
    let mut addresses = String::new();
    if components.chap_core.enabled {
        addresses.push_str(&format!(
            "{} {} {}\n",
            out.key("chap-core:"),
            out.value(&chap_core.tag),
            out.dim(&format!("({})", chap_core.source.describe()))
        ));
        addresses.push_str(&format!(
            "{}       {}\n",
            out.key("API:"),
            out.value(&project.api_url())
        ));
    }
    if let Some(external) = &components.chap_core_external {
        addresses.push_str(&format!(
            "{} {} {}\n",
            out.key("chap-core:"),
            out.value(&external.url),
            out.dim(&format!(
                "(elsewhere; models register there, and it calls them back at {}:<port>)",
                external.models_host
            ))
        ));
    }
    if components.ocs.enabled {
        let reach = components.ocs_reach();
        addresses.push_str(&format!(
            "{}       {} {}\n",
            out.key("OCS:"),
            match components.ocs_url() {
                Some(_) => out.value(&reach),
                None => out.dim(&reach),
            },
            out.dim(&format!(
                "(config in {}/{})",
                crate::components::OCS_DIR,
                crate::components::OCS_CONFIG_FILE
            ))
        ));
    }
    if components.dhis2.enabled {
        let reach = components.dhis2_reach();
        addresses.push_str(&format!(
            "{}     {} {}\n",
            out.key("DHIS2:"),
            match components.dhis2.port {
                Some(_) => out.value(&reach),
                None => out.dim(&reach),
            },
            out.dim(&format!(
                "(config in {}/{})",
                crate::components::DHIS2_DIR,
                crate::components::DHIS2_CONFIG_FILE
            ))
        ));
    }
    if components.s3.enabled {
        addresses.push_str(&format!(
            "{}        {}\n",
            out.key("S3:"),
            match components.s3.port {
                Some(port) => out.value(&format!("http://localhost:{port}")),
                None => out.dim(&format!(
                    "internal (http://s3:{} inside the deployment)",
                    crate::components::S3_CONTAINER_PORT
                )),
            }
        ));
    }
    if !addresses.is_empty() {
        text.push('\n');
        text.push_str(&addresses);
    }
    // No part of the token is printed: it stays in `.env`, and
    // `chaps auth show --reveal` is the way back to it.
    if secrets.is_some() {
        text.push_str(&format!(
            "{} {} {}\n",
            out.key("API token:"),
            out.value("generated into .env"),
            out.dim("(chaps auth show --reveal prints it)")
        ));
    } else if project.state.auth.api_token {
        text.push_str(&format!(
            "{} {} {}\n",
            out.key("API token:"),
            out.value("set in .env"),
            out.dim("(chaps auth show --reveal prints it)")
        ));
    }

    if report.enabled.is_empty() {
        if let Some(line) = no_models_line(components) {
            text.push_str(&format!("\n{}\n", out.backticks(line)));
        }
    } else {
        text.push_str(&format!("\n{}\n", out.heading("Enabled:")));
        // A table without headers, so ids of different lengths line up.
        let rows: Vec<Vec<String>> = report
            .enabled
            .iter()
            .map(|(id, model)| {
                let name = registry
                    .get(id)
                    .map(|m| m.display_name.clone())
                    .unwrap_or_else(|| id.clone());
                let reach = match model.host_port {
                    Some(port) => out.value(&format!("http://localhost:{port}")),
                    None => out.dim("internal"),
                };
                vec![
                    id.clone(),
                    format!("{name} {}", out.dim(&format!("v{}", model.version))),
                    reach,
                ]
            })
            .collect();
        for line in out.table(&[], &rows).lines() {
            text.push_str(&format!("  {line}\n"));
        }
        if report.enabled.iter().any(|(_, m)| m.host_port.is_none()) {
            text.push_str(&out.dim(&format!(
                "\nModel services publish no host port: chap-core reaches them over the\n\
                 compose network, and you reach them through it at\n\
                 {}/v2/services/<service_id>/run/. `chaps models expose ID` publishes one.",
                project.api_url()
            )));
            text.push('\n');
        }
    }
    if components.ocs.enabled && !components.s3.enabled {
        text.push_str(&format!("\n{} {S3_SOON_NOTE}\n", out.dim("note:")));
    }
    // The same soft dependency the other way round, in the same words
    // `components enable s3` uses: a store with nothing to put in it.
    if components.s3.enabled && !components.ocs.enabled {
        text.push_str(&format!("\n{} {S3_WITHOUT_OCS_NOTE}\n", out.dim("note:")));
    }
    // What the first start does with the database, and how long it takes. Both
    // are things nothing else on the screen says and the reader has to know
    // before they run `chaps up`.
    if components.dhis2.enabled {
        let mut dhis2 = Vec::new();
        // The warning block below says it with the reason when the pinned minor
        // has no dump, so this line would be the same sentence twice.
        if !components.dhis2_seed_is_unknown() {
            dhis2.push(crate::components::dhis2_seed_note(
                components.dhis2_seed_source(),
            ));
        }
        dhis2.push(DHIS2_FIRST_START_NOTE.to_string());
        // Only with a chap-core to connect it to.
        if components.has_chap_core_api() {
            dhis2.push(crate::components::DHIS2_CONNECT_NOTE.to_string());
        }
        text.push('\n');
        for note in dhis2 {
            text.push_str(&format!("{} {note}\n", out.dim("note:")));
        }
    }
    // What the containers of a dropped component or model did on the way out.
    // The warnings above already said which components are going; this says what
    // was actually stopped, which is the half a reader cannot infer.
    for note in dropped_notes {
        text.push_str(&format!("\n{} {note}\n", out.dim("note:")));
    }
    for warning in &report.warnings {
        text.push_str(&format!("\n{} {warning}\n", out.warn("warning:")));
    }

    // The last thing on the screen is the thing to type next. A deployment
    // with nothing in it has nothing for `up` to start: adding something is
    // the next step there.
    let empty = components.enabled().is_empty()
        && components.chap_core_external.is_none()
        && report.enabled.is_empty();
    let (first, second) = match empty {
        true => (
            format!("cd {} && chaps models add URL", dir.display()),
            "chaps up",
        ),
        false => (format!("cd {} && chaps up", dir.display()), "chaps status"),
    };
    text.push('\n');
    text.push_str(&format!(
        "{}\n  {}\n  {}",
        out.heading("Next:"),
        out.cmd(&first),
        out.cmd(second)
    ));
    text.push('\n');
    text
}

/// Paths are printed relative to the project directory; everything written
/// lives inside it.
pub(super) fn file_label(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir).unwrap_or(path).display().to_string()
}
