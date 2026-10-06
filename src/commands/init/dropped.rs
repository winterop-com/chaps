//! What a re-init takes away from the deployment it writes over: components,
//! their compose files, model overlays and the containers behind them.

use crate::components::{COMPONENTS_FILE, Component, Components};
use crate::compose::Selection;
use crate::project::{EnabledModel, Project};
use std::path::{Path, PathBuf};

/// The components the previous deployment had that this run does not ask for.
///
/// `init` starts the component set over from the flags, exactly as it starts the
/// model set over from `--models`. That is worth saying out loud, because the
/// default for everything but chap-core is *off*: a directory that had `ocs` and
/// is re-initialised without `--with ocs` loses it without the run mentioning
/// the component at all. One named in `--without` was asked for, so it gets no
/// warning, and chap-core has no compose file of its own to take away.
pub(super) fn dropped_components(
    previous: &Components,
    now: &Components,
    asked_off: &[Component],
) -> Vec<Component> {
    crate::commands::components::disabled_between(previous, now)
        .into_iter()
        .filter(|component| component.compose_file().is_some())
        .filter(|component| !asked_off.contains(component))
        .collect()
}

/// The warning one dropped component gets: what goes, and how to keep it.
pub(super) fn component_dropped(component: Component) -> String {
    format!(
        "this directory had the {name} component and this run does not ask for it, so {file} \
         is removed and its data volume is left behind; re-run with `--with {name}` to keep it",
        name = component.name(),
        file = component.compose_file().unwrap_or(COMPONENTS_FILE),
    )
}

/// Stop and remove the containers of everything this re-init takes away: the
/// components that are going, and the model services whose overlays are about to
/// be deleted.
///
/// Best-effort about docker like every other step that needs it, and asked
/// nothing at all when there is nothing going, so the common `init` over a fresh
/// directory spawns no docker at all.
pub(super) fn stop_dropped(
    previous: &Project,
    components: &Components,
    selection: &Selection,
) -> Vec<String> {
    let mut notes = crate::commands::components::stop_disabled_components(
        previous,
        &previous.state.components,
        components,
    );
    // The overlays of models that are not coming back are removed a moment
    // later, so their containers are stopped on the same terms a component's
    // are. A model the new selection keeps is rewritten by apply() instead and
    // stays up.
    let going: Vec<String> = previous
        .state
        .models
        .iter()
        .filter(|(id, model)| !kept_by(selection, id, model))
        .map(|(_, model)| model.service_id.clone())
        .collect();
    if going.is_empty() {
        return notes;
    }
    notes.extend(crate::commands::docker::stop_and_remove(
        previous,
        &|service| {
            going
                .iter()
                .any(|id| service == id || service == format!("{id}-init"))
        },
    ));
    notes
}

/// Whether the new selection brings a model the previous project had back, by
/// either of the two names it can be asked for.
pub(super) fn kept_by(selection: &Selection, id: &str, model: &EnabledModel) -> bool {
    selection
        .enable
        .iter()
        .any(|req| req.id == id || req.id == model.service_id)
}

/// Delete the component compose files the previous deployment had and the new
/// component set does not bring back, and report them.
///
/// Nothing else would: once `.varde/project.yaml` has been rewritten without
/// them they are in neither `compose_files` nor `rendered_files`, so `varde
/// sync` does not see them as its own any more and would leave them in the
/// directory for good.
pub(super) fn remove_dropped_component_files(
    dir: &Path,
    previous: &Components,
    now: &Components,
) -> Vec<PathBuf> {
    let keeping = now.compose_files();
    let mut removed = Vec::new();
    for file in previous.compose_files() {
        if keeping.contains(&file) {
            continue;
        }
        let path = dir.join(&file);
        if path.is_file() && std::fs::remove_file(&path).is_ok() {
            removed.push(path);
        }
    }
    removed
}

/// Delete every overlay a previous project in `dir` owned.
///
/// All of them go: `init` starts the state over, so a model the new selection
/// keeps is rewritten by [`apply()`](crate::compose::apply()) a moment later,
/// and one it drops would otherwise linger unreferenced while still holding its
/// host port against the allocator. Only files the old `.varde/models.yaml`
/// lists are touched; a hand-written overlay is none of `init`'s business, and
/// a directory without a readable state file has nothing to clean up.
///
/// Returns the ones that are not coming back, for the report.
pub(super) fn remove_stale_overlays(dir: &Path, selection: &Selection) -> Vec<PathBuf> {
    let Ok(previous) = Project::load(dir) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for (id, model) in &previous.state.models {
        let path = dir.join(&model.compose_file);
        if !path.is_file() || std::fs::remove_file(&path).is_err() {
            continue;
        }
        if !kept_by(selection, id, model) {
            removed.push(path);
        }
    }
    removed
}
