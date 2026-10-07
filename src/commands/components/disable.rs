//! `varde components disable`, and stopping the containers of a component
//! that is going, for every caller that turns one off.

use super::{ChangeReport, Note, NoteLevel, change_summary, notes_of};
use crate::cli::ComponentsDisableArgs;
use crate::commands::Ctx;
use crate::components::{Component, Components, DHIS2_CONNECT_FORGOTTEN, S3_LEAVES_OCS_NOTE};
use crate::compose::sync::sync;
use crate::error::Result;
use crate::project::Project;

/// Turn a component off and remove what `sync` rendered for it.
///
/// The component's data volumes are kept, exactly as a disabled model's volume
/// is, and each of them named: once the compose file is gone nothing else
/// declares them, so `varde down --volumes` no longer reaches them. `--purge`
/// removes them with the component.
pub fn disable(ctx: &Ctx, args: &ComponentsDisableArgs) -> Result<()> {
    let component = Component::from_name(&args.name)?;
    let (mut project, _lock) = ctx.project_mut()?;
    let before = project.state.components.clone();

    // Said before anything is stopped: a `--purge` that cannot do the one
    // thing it was asked for is a refusal, not a disable with a note.
    if args.purge && component.volumes().is_empty() {
        return Err(anyhow::anyhow!(CORE_HAS_NO_VARDE_VOLUME));
    }

    let mut after = before.clone();
    after.set_enabled(component, false);
    // Disabling chap-core also forgets one elsewhere: either way the
    // deployment is left with no chap-core to talk to.
    if component == Component::ChapCore {
        after.chap_core_external = None;
    }

    // The containers go now, while compose still has the files that define
    // them, and the volume after them, because docker refuses to remove one a
    // container still has mounted.
    //
    // The component this was pointed at, on or off: one that is already off can
    // still have a container from before it was, and a volume the operator is
    // asking to purge. [`stop_disabled_components`] is the same work seen as a
    // diff, for the callers that have two component sets and no named component
    // - the browser's save, and `init --force`.
    let going = [component];
    let mut purged = Vec::new();
    let mut kept_volumes = Vec::new();
    let mut stopped = stop_components(&project, &going);
    if args.purge {
        let names = component_volumes(&project, component);
        // chap-core keeps its volumes either way, and `--purge` was refused
        // above; a deployment with no compose project name can name none.
        if names.is_empty() {
            stopped.push(crate::commands::docker::UNNAMEABLE_VOLUME.to_string());
        }
        for name in names {
            let (removed, line) = crate::commands::docker::purge_volume(&name);
            purged.extend(removed);
            stopped.push(line);
        }
    } else {
        let exists = crate::docker::volume_exists;
        stopped.extend(kept_volume_notes(&project, component, &exists));
        kept_volumes.extend(kept_volumes_of(&project, component, &exists));
    }

    let registry = crate::commands::registry_for(ctx, Some(&project))?;
    // Taking chap-core away from models goes through apply(), which publishes
    // a host port for each model that had none: with nothing left to reach
    // them over the compose network, that port is the only way in.
    let (mut notes, written, removed) =
        if component == Component::ChapCore && !project.state.models.is_empty() {
            let sel = crate::compose::apply::Selection {
                enable: Vec::new(),
                disable: Vec::new(),
                components: Some(after.clone()),
            };
            let endpoints = crate::manual::Endpoints::from_env(ctx.registry.offline);
            let applied = crate::compose::apply::apply(&mut project, &registry, &sel, &endpoints)?;
            let mut notes = notes_of(NoteLevel::Warning, applied.warnings);
            notes.extend(applied.updated.iter().filter_map(|(id, e)| {
                e.host_port.map(|port| {
                    Note::info(format!(
                        "{id} now registers nowhere and is published on http://localhost:{port}"
                    ))
                })
            }));
            (notes, applied.written, applied.removed)
        } else {
            project.state.components = after.clone();
            let synced = sync(&mut project, &registry, false)?;
            (
                notes_of(NoteLevel::Warning, synced.warnings),
                synced.written,
                synced.removed,
            )
        };
    notes.extend(notes_of(NoteLevel::Info, stopped));
    if component == Component::ChapCore {
        notes.push(Note::hint(
            "chap-core's own volumes are left alone; `varde down --volumes` removes them",
        ));
    }
    if component == Component::Ocs {
        notes.push(Note::hint("the ocs/ directory is left alone; it is yours"));
    }
    if component == Component::Dhis2 {
        notes.push(Note::hint(
            "the dhis2/ directory is left alone; it is yours",
        ));
        // `set_enabled` forgot it a few lines up, where every caller goes
        // through. Only said on the run that had something to forget: a
        // deployment that was never connected has nothing to report here.
        if before.dhis2.connected_at.is_some() {
            notes.push(Note::hint(DHIS2_CONNECT_FORGOTTEN));
        }
    }
    // The store going takes the `S3_*` block out of `compose.ocs.yml`, which is
    // a change to a service this command was not asked about. Only on the run
    // that took it: a store that was already off left OCS without those
    // variables long ago.
    if disabled_between(&before, &after).contains(&Component::S3) && after.ocs.enabled {
        notes.push(Note::hint(S3_LEAVES_OCS_NOTE));
    }
    let report = ChangeReport {
        name: component.name().to_string(),
        enabled: false,
        port: None,
        base_url: None,
        read_only: None,
        unchanged: before == after,
        written,
        removed,
        notes,
        purged,
        kept_volumes,
        read_mode: None,
    };
    ctx.out
        .report(&report, |lines| change_summary(&report, &project, lines))
}

/// Why `components disable chap-core --purge` is refused.
///
/// chap-core's volumes - the database, its own data directory - are declared
/// by upstream's `compose.yml`, which this CLI renders but does not author, so
/// there is no one volume `--purge` could mean. The compose command that
/// removes them names the whole deployment's volumes and is the honest way to
/// ask for that.
const CORE_HAS_NO_VARDE_VOLUME: &str = "chap-core keeps no volume of its own that varde names, so --purge has nothing to remove; \
     `varde down --volumes` removes every volume of this deployment";

/// Whether a compose service belongs to a component, for the purpose of
/// stopping its containers when that component is disabled.
///
/// [`Component::owns_service`] is the rule; this is the predicate the docker
/// helpers take, so there is one answer and one place it is decided.
pub(super) fn owns(component: Component) -> impl Fn(&str) -> bool {
    move |service: &str| component.owns_service(service)
}

/// The components that were enabled in `before` and are not in `after`, in
/// [`Component::ALL`] order.
pub fn disabled_between(before: &Components, after: &Components) -> Vec<Component> {
    Component::ALL
        .iter()
        .copied()
        .filter(|component| before.is_enabled(*component) && !after.is_enabled(*component))
        .collect()
}

/// Stop and remove the containers of every component that went from enabled in
/// `before` to disabled in `after`, and say what was stopped and which data
/// volumes were kept.
///
/// The containers go while the compose files that define them are still on
/// disk: a service whose definition has just been removed cannot be stopped by
/// name any more, and one left running keeps its host port published long after
/// the component was switched off. So every caller - `components disable`, the
/// components page of `varde ui`, `init --force` - calls this before it removes
/// or re-renders anything.
///
/// Volumes are named, never removed: `--purge` is a `components disable` flag,
/// and nothing else in varde deletes a deployment's data unasked. Best-effort
/// about docker like every other step that needs it, so no CLI, no daemon or a
/// compose that refuses is a line in the report rather than a failure.
pub fn stop_disabled_components(
    project: &Project,
    before: &Components,
    after: &Components,
) -> Vec<String> {
    let going = disabled_between(before, after);
    let mut notes = stop_components(project, &going);
    let exists = crate::docker::volume_exists;
    notes.extend(
        going
            .iter()
            .flat_map(|c| kept_volume_notes(project, *c, &exists)),
    );
    notes
}

/// Stop and remove the containers of every component in `going`, in one docker
/// round trip and one line.
///
/// Nothing at all for an empty list, so a caller with nothing going asks docker
/// nothing - which is what keeps `varde init` over a fresh directory free of
/// docker.
fn stop_components(project: &Project, going: &[Component]) -> Vec<String> {
    if going.is_empty() {
        return Vec::new();
    }
    let owners: Vec<_> = going.iter().map(|component| owns(*component)).collect();
    crate::commands::docker::stop_and_remove(project, &|service| {
        owners.iter().any(|owns| owns(service))
    })
    .into_iter()
    .collect()
}

/// The named data volumes `component` keeps in this deployment, as docker
/// spells them.
///
/// Empty for chap-core, whose volumes are upstream's own, and for a directory
/// that records no compose project name and so can name none of them.
pub(super) fn component_volumes(project: &Project, component: Component) -> Vec<String> {
    component
        .volumes()
        .iter()
        .filter_map(|volume| project.prefixed_volume(volume))
        .collect()
}

/// The lines a disable closes with for the data volumes it left in place: one
/// per volume, because each carries the `docker volume rm` for its own name and
/// an operator removing them by hand needs every one of them.
///
/// Only the ones `exists` confirms: a component that was never started has no
/// volume, and saying one was kept would send the reader to remove nothing.
pub(super) fn kept_volume_notes(
    project: &Project,
    component: Component,
    exists: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    kept_volumes_of(project, component, exists)
        .iter()
        .map(|name| {
            crate::commands::docker::kept_volume_line(
                name,
                Some(&format!("varde components disable {}", component.name())),
            )
        })
        .collect()
}

/// [`component_volumes`] cut to the ones that exist, which are the only ones a
/// disable can be said to have kept.
fn kept_volumes_of(
    project: &Project,
    component: Component,
    exists: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    component_volumes(project, component)
        .into_iter()
        .filter(|name| exists(name))
        .collect()
}
