//! `chaps ui` — the browser: marketplace models on one page, the components
//! this deployment is made of on the other.
//!
//! The browser returns a [`Selection`](crate::compose::Selection); applying
//! it goes through [`crate::compose::apply()`] outside the terminal, and the
//! resulting [`ApplyReport`] is printed afterwards. The containers of a
//! component the save switches off are stopped in between, while the compose
//! files that name them are still there.

use crate::cli::UiArgs;
use crate::commands::Ctx;
use crate::components::Components;
use crate::compose::ApplyReport;
use crate::error::Result;
use crate::tui::run_tui;

/// Open the browser against the current project and catalogue.
///
/// `--json` is rejected before this is reached.
pub fn run(ctx: &Ctx, _args: &UiArgs) -> Result<()> {
    // The browser edits a deployment, so there has to be one; the error already
    // tells the user to run `chaps init`.
    let project = ctx.project()?;
    let registry = super::registry_for(ctx, Some(&project))?;

    let Some(selection) = run_tui(ctx, &project, &registry)? else {
        println!("left the browser; nothing was written, and `chaps ui` opens it again");
        return Ok(());
    };
    if selection.is_empty() {
        println!("no changes to save; `chaps ui` opens the browser again");
        return Ok(());
    }

    // The browser may have been open for minutes, and another chaps may have
    // changed the deployment meanwhile: the selection is applied to the state
    // as it is now, read under the lock that keeps it so until the save.
    let (project, _lock) = ctx.project_mut()?;
    let registry = super::registry_for(ctx, Some(&project))?;

    // Planned before anything is stopped or written - the checks, the ports,
    // the users - so a selection that was never going to apply leaves the
    // deployment exactly as it was, its components running included.
    let before = project.state.components.clone();
    let wanted = selection
        .components
        .clone()
        .unwrap_or_else(|| before.clone());
    // A component this save switches off is still running while the plan is
    // made, and is stopped before anything starts on its port: that port is
    // free for the plan.
    let freed: Vec<u16> = crate::components::Component::ALL
        .iter()
        .filter(|c| before.is_enabled(**c) && !wanted.is_enabled(**c))
        .filter_map(|c| before.port_of(*c))
        .collect();
    let busy = |port: u16| !freed.contains(&port) && crate::ports::is_busy(port);
    let endpoints = crate::manual::Endpoints::from_env(ctx.registry.offline);
    let mut planned = project.clone();
    let plan =
        crate::compose::apply::plan_with(&mut planned, &registry, &selection, &busy, &|req| {
            crate::compose::resolve::from_image(req, &endpoints)
        })?;

    // The containers of a component being switched off go now, while the
    // compose files that define them are still there: a service whose
    // definition has just been removed cannot be stopped by name, and one left
    // running keeps its host port published long after the component is gone.
    // Volumes are not touched - `--purge` stays a `components disable` flag.
    let after: Components = selection
        .components
        .clone()
        .unwrap_or_else(|| before.clone());
    let stopped = super::components::stop_disabled_components(&project, &before, &after);
    // A DHIS2 version picked in the browser moves the same database the
    // command line would, so it gets the same warning, before anything is
    // written.
    if let Some(note) = super::components::dhis2_tag_moved(&project, &after) {
        crate::output::warn(&note);
    }

    let report = crate::compose::apply::write_planned(&mut planned, &registry, plan)?;
    ctx.out.emit(&report, || human(&report, &stopped))?;
    Ok(())
}

/// The human rendering of what was applied.
fn human(report: &ApplyReport, notes: &[String]) -> String {
    let mut text = String::new();
    for warning in &report.warnings {
        text.push_str(&format!("warning: {warning}\n"));
    }

    for (label, models) in [("enabled", &report.enabled), ("updated", &report.updated)] {
        if models.is_empty() {
            continue;
        }
        text.push_str(&format!("{label}:\n"));
        for (id, model) in models {
            text.push_str(&format!(
                "  {id}  {}  {}\n",
                model.version,
                match model.host_port {
                    Some(port) => format!("port {port}"),
                    None => "internal".to_string(),
                }
            ));
        }
    }

    if !report.disabled.is_empty() {
        text.push_str("disabled:\n");
        for id in &report.disabled {
            text.push_str(&format!("  {id}\n"));
        }
    }

    if !report.components_enabled.is_empty() {
        text.push_str("components on:\n");
        for (name, port) in &report.components_enabled {
            text.push_str(&format!(
                "  {name}  {}\n",
                match port {
                    Some(port) => format!("http://localhost:{port}"),
                    None => "internal".to_string(),
                }
            ));
        }
    }
    if !report.components_disabled.is_empty() {
        text.push_str("components off:\n");
        for name in &report.components_disabled {
            text.push_str(&format!("  {name}\n"));
        }
    }
    // What stopping the containers of a switched-off component did, and which
    // data volumes it left behind.
    for note in notes {
        text.push_str(&format!("note: {note}\n"));
    }

    if report.is_empty() {
        text.push_str("no changes\n");
    } else {
        text.push_str("run `chaps up` to apply the new compose files\n");
    }
    text
}

#[cfg(test)]
mod tests;
