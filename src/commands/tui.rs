//! `varde ui` — the browser: marketplace models on one page, the components
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
use crate::output::Report;
use crate::tui::run_tui;

/// Open the browser against the current project and catalogue.
///
/// `--json` is rejected before this is reached.
pub fn run(ctx: &Ctx, _args: &UiArgs) -> Result<()> {
    // The browser edits a deployment, so there has to be one; the error already
    // tells the user to run `varde init`.
    let project = ctx.project()?;
    let registry = super::registry_for(ctx, Some(&project))?;

    let Some(selection) = run_tui(ctx, &project, &registry)? else {
        return ctx.out.report(&serde_json::json!({}), |lines| {
            lines
                .info("left the browser; nothing was written")
                .hint("`varde ui` opens it again");
        });
    };
    if selection.is_empty() {
        return ctx.out.report(&serde_json::json!({}), |lines| {
            lines
                .info("no changes to save")
                .hint("`varde ui` opens the browser again");
        });
    }

    // The browser may have been open for minutes, and another varde may have
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
    let moved = super::components::dhis2_tag_moved(&project, &after);

    let report = crate::compose::apply::write_planned(&mut planned, &registry, plan)?;
    ctx.out.report(&report, |lines| {
        if let Some(note) = moved {
            lines.warning(note);
        }
        summary(&report, &stopped, lines);
    })
}

/// The lines of what was applied.
///
/// `notes` are what stopping the containers of a switched-off component did,
/// and which data volumes it left behind. They are info, because they say
/// what happened to the data.
fn summary(report: &ApplyReport, notes: &[String], lines: &mut Report) {
    for warning in &report.warnings {
        lines.warning(warning.as_str());
    }
    for (verb, models) in [("enabled", &report.enabled), ("updated", &report.updated)] {
        for (id, model) in models {
            // A manually added model's version is its image tag, so `v` in
            // front of it would read as a version number it does not have.
            let version = match model.version == model.image_tag {
                true => model.image_tag.clone(),
                false => format!("v{}", model.version),
            };
            let place = match model.host_port {
                Some(port) => format!(" on http://localhost:{port}"),
                None => String::new(),
            };
            lines.info(format!("{verb} {id} {version}{place}"));
        }
    }
    for id in &report.disabled {
        lines.info(format!("disabled {id}"));
    }
    for (name, port) in &report.components_enabled {
        lines.info(match port {
            Some(port) => format!("enabled the {name} component on http://localhost:{port}"),
            None => format!("enabled the {name} component"),
        });
    }
    for name in &report.components_disabled {
        lines.info(format!("disabled the {name} component"));
    }
    for note in notes {
        lines.info(note.as_str());
    }
    match report.is_empty() {
        true => lines.info("no changes"),
        false => lines.info("run `varde up` to apply"),
    };
}

#[cfg(test)]
mod tests;
