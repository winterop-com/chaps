//! `chaps components list|enable|disable` — what a deployment is made of.
//!
//! The three commands do the same small thing: edit `.chaps/components.yaml`
//! and hand over to [`crate::compose::sync()`], which renders one compose file
//! per enabled component and removes the ones that are no longer wanted. That
//! is the same path `init --with` takes, so a component enabled at init and one
//! enabled a week later end up with byte-identical files.

use crate::cli::{ComponentsDisableArgs, ComponentsEnableArgs, ComponentsListArgs, OcsConfigArgs};
use crate::commands::Ctx;
use crate::components::{
    Component, Components, OCS_DATA_SOURCE_ENV_VARS, OCS_DATA_SOURCE_NOTE, S3_LEAVES_OCS_NOTE,
    S3_SOON_NOTE, S3_WITHOUT_OCS_NOTE, models_need_chap_core,
};
use crate::compose::spec::OcsConfigRequest;
use crate::compose::sync::{sync, write_ocs_config};
use crate::error::Result;
use crate::output::Out;
use crate::project::{ENV_FILE, Project};
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
        components: rows(&project.state.components, project.state.api_port),
    };
    ctx.out.emit(&report, || human_list(&report, &ctx.out))
}

/// One row per component. chap-core's port is the API port, which lives in
/// `project.yaml` rather than in the component block, so it is passed in.
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
        })
        .collect()
}

/// Turn a component on, or change the settings of one that already is.
pub fn enable(ctx: &Ctx, args: &ComponentsEnableArgs) -> Result<()> {
    let component = Component::from_name(&args.name)?;
    let mut project = ctx.project()?;
    let before = project.state.components.clone();

    // The port preflight runs on the state as recorded, before anything moves:
    // a component already listening on one port must not be mistaken for the
    // holder of the port it is being moved to. `chaps init` warns in exactly
    // this situation and this is the other place the decision is made, so it
    // warns here too - and warns rather than refuses, because the deployment is
    // not up yet and the port is still easy to change.
    let port_warning = wanted_port(&before, component, args).and_then(|port| {
        crate::ports::component_port_warning(&project, component, port, &crate::ports::is_busy)
    });

    if let Some(port) = args.port {
        set_port(&mut project.state.components, component, port.0)?;
    }
    if let Some(base_url) = base_url(args)? {
        if component != Component::Ocs {
            return Err(anyhow::anyhow!(
                "--base-url is an OCS setting: it names the public origin OCS builds its \
                 links from, and no other component composes any"
            ));
        }
        project.state.components.ocs.base_url = base_url;
    }
    if (args.read_only || args.read_write) && component != Component::Ocs {
        return Err(anyhow::anyhow!(
            "--read-only and --read-write are OCS settings: they turn ingestion over HTTP \
             off and on in ocs/{}",
            crate::components::OCS_CONFIG_FILE
        ));
    }
    project.state.components.set_enabled(component, true);

    let mut notes = Vec::new();
    notes.extend(port_warning);
    // The scaffold goes in before the sync, so the `--ocs-*` values reach the
    // file rather than the example ones sync would fall back to.
    if component == Component::Ocs {
        let wanted = request(&args.ocs);
        let asked_for = !wanted.is_empty();
        match write_ocs_config(&project.dir, &wanted.into_spec())? {
            Some(path) => notes.push(format!(
                "wrote {}; it is yours to edit, and chaps never rewrites it",
                label(&project, &path)
            )),
            // Values were given for a file that is already there. Silently
            // discarding them would be the worst of the three answers.
            None if asked_for => notes.push(format!(
                "{} is already there, so the --ocs-* values were not used; edit that file \
                 instead, or delete it and enable ocs again",
                label(&project, &project.ocs_config_path())
            )),
            None => {}
        }
        // And the read-only switch after it, so a component enabled and set
        // read-only in one command edits the file this run just scaffolded.
        if let Some(note) = set_read_only(&mut project, args)? {
            notes.push(note);
        }
    }
    let after = project.state.components.clone();
    if component == Component::Ocs && !after.s3.enabled {
        notes.push(S3_SOON_NOTE.to_string());
    }
    // The other half of the same soft dependency: a store with nothing to put
    // in it is worth a line, because the operator may have meant to add OCS too.
    if component == Component::S3 && !after.ocs.enabled {
        notes.push(S3_WITHOUT_OCS_NOTE.to_string());
    }

    // `sync` appends the OCS `.env` sections on the run that first needs them,
    // and its report only says the file was written - which it also says when it
    // appended a model's tag pin. The text before and after is what tells the
    // two apart, and the credentials are the one thing in there an operator has
    // to go and do something about.
    let env_before = env_text(&project);
    let registry = super::registry_for(ctx, Some(&project))?;
    let synced = sync(&mut project, &registry, false)?;
    notes.extend(synced.warnings);
    if component == Component::Ocs && data_sources_appended(&env_before, &env_text(&project)) {
        notes.push(OCS_DATA_SOURCE_NOTE.to_string());
    }

    let report = ChangeReport {
        name: component.name().to_string(),
        enabled: true,
        port: after.port_of(component),
        base_url: ocs_only(component, after.ocs.base_url.clone()),
        read_only: ocs_only(component, Some(after.ocs.read_only)),
        unchanged: before == after,
        written: synced.written,
        removed: synced.removed,
        notes,
        purged: Vec::new(),
        kept_volumes: Vec::new(),
    };
    ctx.out
        .emit(&report, || human_change(&report, &project, &ctx.out))
}

/// Turn a component off and remove what `sync` rendered for it.
///
/// The component's data volume is kept, exactly as a disabled model's is, and
/// named either way: once the compose file is gone nothing else declares that
/// volume, so `chaps down --volumes` no longer reaches it. `--purge` removes
/// it with the component.
pub fn disable(ctx: &Ctx, args: &ComponentsDisableArgs) -> Result<()> {
    let component = Component::from_name(&args.name)?;
    let mut project = ctx.project()?;
    let before = project.state.components.clone();

    // Models are the one hard dependency between components: a model service
    // registers with chap-core and is reached through it, so a deployment with
    // models and no chap-core would start and do nothing.
    if component == Component::ChapCore && !project.state.models.is_empty() {
        let ids: Vec<String> = project.state.models.keys().cloned().collect();
        return Err(anyhow::anyhow!(models_need_chap_core(&ids)));
    }
    // Said before anything is stopped: a `--purge` that cannot do the one
    // thing it was asked for is a refusal, not a disable with a note.
    if args.purge && component.volume().is_none() {
        return Err(anyhow::anyhow!(CORE_HAS_NO_CHAPS_VOLUME));
    }

    let mut after = before.clone();
    after.set_enabled(component, false);

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
        match component_volume(&project, component) {
            Some(name) => {
                let (removed, line) = super::docker::purge_volume(&name);
                purged.extend(removed);
                stopped.push(line);
            }
            // chap-core keeps its volumes either way, and `--purge` was refused
            // above; a deployment with no compose project name can name none.
            None => stopped.push(super::docker::UNNAMEABLE_VOLUME.to_string()),
        }
    } else {
        stopped.extend(kept_volume_note(&project, component));
        kept_volumes.extend(component_volume(&project, component));
    }

    project.state.components = after.clone();

    let registry = super::registry_for(ctx, Some(&project))?;
    let synced = sync(&mut project, &registry, false)?;

    let mut notes = synced.warnings.clone();
    notes.extend(stopped);
    if component == Component::ChapCore {
        notes.push(
            "chap-core's own volumes are left alone; \
             `chaps down --volumes` removes them"
                .to_string(),
        );
    }
    if component == Component::Ocs {
        notes.push("the ocs/ directory is left alone; it is yours".to_string());
    }
    // The store going takes the `S3_*` block out of `compose.ocs.yml`, which is
    // a change to a service this command was not asked about. Only on the run
    // that took it: a store that was already off left OCS without those
    // variables long ago.
    if disabled_between(&before, &after).contains(&Component::S3) && after.ocs.enabled {
        notes.push(S3_LEAVES_OCS_NOTE.to_string());
    }
    let report = ChangeReport {
        name: component.name().to_string(),
        enabled: false,
        port: None,
        base_url: None,
        read_only: None,
        unchanged: before == after,
        written: synced.written,
        removed: synced.removed,
        notes,
        purged,
        kept_volumes,
    };
    ctx.out
        .emit(&report, || human_change(&report, &project, &ctx.out))
}

/// Why `components disable chap-core --purge` is refused.
///
/// chap-core's volumes - the database, its own data directory - are declared
/// by upstream's `compose.yml`, which this CLI renders but does not author, so
/// there is no one volume `--purge` could mean. The compose command that
/// removes them names the whole deployment's volumes and is the honest way to
/// ask for that.
const CORE_HAS_NO_CHAPS_VOLUME: &str = "chap-core keeps no volume of its own that chaps names, so --purge has nothing to remove; \
     `chaps down --volumes` removes every volume of this deployment";

/// Whether a compose service belongs to a component, for the purpose of
/// stopping its containers when that component is disabled.
///
/// `ocs` and `s3` are the services `sync` renders for them, each with the
/// exited one-shot `-init` companion that prepared it. chap-core's are
/// upstream's - `chap`, its worker, Valkey, PostgreSQL - named in a file this
/// CLI does not write, so they are everything else: `disable` has already
/// refused while any model is enabled, and `ocs` and `s3` are the only other
/// components there are.
fn owns(component: Component) -> impl Fn(&str) -> bool {
    move |service: &str| {
        let base = service.strip_suffix("-init").unwrap_or(service);
        match component {
            Component::Ocs => base == crate::compose::OCS_SERVICE,
            Component::S3 => base == crate::compose::S3_SERVICE,
            Component::ChapCore => {
                base != crate::compose::OCS_SERVICE && base != crate::compose::S3_SERVICE
            }
        }
    }
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
/// components page of `chaps ui`, `init --force` - calls this before it removes
/// or re-renders anything.
///
/// Volumes are named, never removed: `--purge` is a `components disable` flag,
/// and nothing else in chaps deletes a deployment's data unasked. Best-effort
/// about docker like every other step that needs it, so no CLI, no daemon or a
/// compose that refuses is a line in the report rather than a failure.
pub fn stop_disabled_components(
    project: &Project,
    before: &Components,
    after: &Components,
) -> Vec<String> {
    let going = disabled_between(before, after);
    let mut notes = stop_components(project, &going);
    notes.extend(going.iter().filter_map(|c| kept_volume_note(project, *c)));
    notes
}

/// Stop and remove the containers of every component in `going`, in one docker
/// round trip and one line.
///
/// Nothing at all for an empty list, so a caller with nothing going asks docker
/// nothing - which is what keeps `chaps init` over a fresh directory free of
/// docker.
fn stop_components(project: &Project, going: &[Component]) -> Vec<String> {
    if going.is_empty() {
        return Vec::new();
    }
    let owners: Vec<_> = going.iter().map(|component| owns(*component)).collect();
    super::docker::stop_and_remove(project, &|service| owners.iter().any(|owns| owns(service)))
        .into_iter()
        .collect()
}

/// The named data volume `component` keeps in this deployment, as docker spells
/// it.
///
/// `None` for chap-core, whose volumes are upstream's own, and for a directory
/// that records no compose project name and cannot name one.
fn component_volume(project: &Project, component: Component) -> Option<String> {
    component
        .volume()
        .and_then(|volume| project.prefixed_volume(volume))
}

/// The line a disable closes with for the data volume it left in place.
fn kept_volume_note(project: &Project, component: Component) -> Option<String> {
    let name = component_volume(project, component)?;
    Some(super::docker::kept_volume_line(
        &name,
        &format!("chaps components disable {}", component.name()),
    ))
}

/// The host port a component will publish once this run is done: the `--port`
/// value when the flag was given, otherwise the port already recorded.
///
/// The recorded port is read whether or not the component is on yet, because
/// that is how a re-enable puts an instance back on the port it had.
fn wanted_port(
    components: &Components,
    component: Component,
    args: &ComponentsEnableArgs,
) -> Option<u16> {
    match args.port {
        Some(given) => given.0,
        None => match component {
            Component::Ocs => components.ocs.port,
            Component::S3 => components.s3.port,
            Component::ChapCore => None,
        },
    }
}

/// This deployment's `.env`, or an empty string when it has none.
fn env_text(project: &Project) -> String {
    std::fs::read_to_string(project.dir.join(ENV_FILE)).unwrap_or_default()
}

/// Whether the sync between these two readings of `.env` is the one that put the
/// OCS data source variables in it.
///
/// [`crate::compose::sync()`] appends them once, commented out, and says only
/// that it wrote `.env`; re-enabling a component that is already on writes the
/// same file for other reasons. Comparing the two readings is what keeps the
/// note on the run that earned it.
fn data_sources_appended(before: &str, after: &str) -> bool {
    let mentions = |body: &str| {
        OCS_DATA_SOURCE_ENV_VARS
            .iter()
            .any(|var| body.contains(*var))
    };
    !mentions(before) && mentions(after)
}

/// An OCS-only field of the change report, `None` for any other component: the
/// report is one shape for all three, and a base URL on the `s3` line would
/// read as a setting the object store has.
fn ocs_only<T>(component: Component, value: Option<T>) -> Option<T> {
    value.filter(|_| component == Component::Ocs)
}

/// The scaffold values the `--ocs-*` flags carry.
pub fn request(args: &OcsConfigArgs) -> OcsConfigRequest {
    OcsConfigRequest {
        name: args.ocs_name.clone(),
        country_code: args.ocs_country.clone(),
        bbox: args.ocs_bbox.clone(),
    }
}

/// Record a host port for a component that can publish one, or `None` to take
/// the published port away and leave it on the compose network.
fn set_port(components: &mut Components, component: Component, port: Option<u16>) -> Result<()> {
    match component {
        Component::Ocs => components.ocs.port = port,
        Component::S3 => components.s3.port = port,
        Component::ChapCore => {
            return Err(anyhow::anyhow!(
                "chap-core's host port is the API port; set it with \
                 `chaps init --api-port {} --force` or CHAP_API_PORT in .env",
                port.map(|p| p.to_string())
                    .unwrap_or_else(|| "PORT".to_string())
            ));
        }
    }
    Ok(())
}

/// The base URL `--base-url` asks for: `Some(None)` when it was given an empty
/// value, which is how the setting is cleared again.
///
/// `Ok(None)` means the flag was absent and the recorded value stands.
fn base_url(args: &ComponentsEnableArgs) -> Result<Option<Option<String>>> {
    let Some(given) = args.base_url.as_deref().map(str::trim) else {
        return Ok(None);
    };
    if given.is_empty() {
        return Ok(Some(None));
    }
    // A value with no scheme would make OCS log a warning and go on building
    // links from the request, which is the setting silently not working.
    if !given.starts_with("http://") && !given.starts_with("https://") {
        return Err(anyhow::anyhow!(
            "--base-url has to be an absolute URL, so `{given}` will not do: OCS builds its \
             STAC and openEO links by appending a path to this value"
        ));
    }
    Ok(Some(Some(given.trim_end_matches('/').to_string())))
}

/// Apply `--read-only` or `--read-write` to `ocs/climate-service.yaml` and to
/// the component record, and say what changed.
///
/// The file is what OCS reads and the record is only a record of it, so the
/// file is edited first and the record follows. `None` when neither flag was
/// given.
fn set_read_only(project: &mut Project, args: &ComponentsEnableArgs) -> Result<Option<String>> {
    if !args.read_only && !args.read_write {
        return Ok(None);
    }
    let wanted = args.read_only;
    let config = format!(
        "{}/{}",
        crate::components::OCS_DIR,
        crate::components::OCS_CONFIG_FILE
    );
    let Some(edit) = crate::compose::sync::set_read_only(&project.dir, wanted)? else {
        return Err(anyhow::anyhow!(
            "there is no {config} to set {} in; run `chaps sync` to scaffold it first",
            crate::components::OCS_READ_ONLY_KEY
        ));
    };
    project.state.components.ocs.read_only = wanted;
    let key = crate::components::OCS_READ_ONLY_KEY;
    Ok(Some(match edit {
        crate::compose::sync::KeyEdit::Unchanged => {
            format!("{config} already has {key}: {wanted}")
        }
        crate::compose::sync::KeyEdit::Rewritten => {
            format!("set {key}: {wanted} in {config}; {READ_ONLY_APPLY}")
        }
        crate::compose::sync::KeyEdit::Appended => {
            format!("added {key}: {wanted} to {config}; {READ_ONLY_APPLY}")
        }
    }))
}

/// How a change to `ocs/climate-service.yaml` reaches the running instance.
///
/// `chaps restart` on its own is `docker compose up -d`, and compose recreates
/// only what no longer matches the compose files. The instance config is a bind
/// mount, so editing it changes nothing compose compares - the new text is
/// already visible inside the container, and OCS simply read the old one at
/// startup. `--all` with the service named is the force-recreate that makes it
/// read the file again, and it leaves chap-core and the models alone.
const READ_ONLY_APPLY: &str = "`chaps restart --all ocs` applies it (a plain `chaps restart` does not: \
     the config is a bind mount, so compose sees nothing to recreate)";

fn human_list(report: &ComponentsReport, out: &Out) -> String {
    let rows: Vec<Vec<String>> = report
        .components
        .iter()
        .map(|row| {
            vec![
                row.name.clone(),
                if row.enabled {
                    out.ok("enabled")
                } else {
                    out.dim("off")
                },
                match (row.enabled, row.port) {
                    (true, Some(port)) => out.value(&format!("http://localhost:{port}")),
                    (true, None) => out.dim("internal"),
                    (false, _) => out.dim("-"),
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
mod tests {
    use super::*;
    use crate::cli::ComponentPortArg;
    use crate::components::{OCS_DEFAULT_PORT, S3Component};

    #[test]
    fn the_rows_cover_every_component_in_order() {
        let rows = rows(&Components::default(), 8000);
        let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["chap-core", "ocs", "s3"]);
        assert!(rows[0].enabled && !rows[1].enabled && !rows[2].enabled);
        assert!(
            rows[0].compose_file.is_none(),
            "chap-core is the base stack"
        );
        assert_eq!(rows[1].compose_file.as_deref(), Some("compose.ocs.yml"));
        assert_eq!(rows[0].port, Some(8000), "chap-core's port is the API port");
        assert!(rows[1..].iter().all(|r| r.port.is_none()));
    }

    #[test]
    fn an_enabled_component_shows_the_port_it_publishes() {
        let mut components = Components::default();
        components.set_enabled(Component::Ocs, true);
        components.set_enabled(Component::S3, true);
        let listed = rows(&components, 8000);
        assert_eq!(listed[1].port, Some(OCS_DEFAULT_PORT));
        assert_eq!(
            listed[2].port, None,
            "the store publishes nothing by default"
        );

        components.s3 = S3Component {
            enabled: true,
            port: Some(9002),
        };
        assert_eq!(rows(&components, 8000)[2].port, Some(9002));
    }

    #[test]
    fn a_port_is_recorded_on_the_component_that_can_take_one() {
        let mut components = Components::default();
        set_port(&mut components, Component::Ocs, Some(9010)).unwrap();
        assert_eq!(components.ocs.port, Some(9010));
        set_port(&mut components, Component::S3, Some(9011)).unwrap();
        assert_eq!(components.s3.port, Some(9011));

        // `--port none` is how a published component becomes an internal one,
        // and it reads the same on both.
        set_port(&mut components, Component::Ocs, None).unwrap();
        assert_eq!(components.ocs.port, None);
        set_port(&mut components, Component::S3, None).unwrap();
        assert_eq!(components.s3.port, None);

        let err = set_port(&mut components, Component::ChapCore, Some(8123))
            .expect_err("chap-core's port is the API port");
        assert!(err.to_string().contains("--api-port 8123"), "{err}");
    }

    /// `--base-url` has to be a URL OCS can append a path to, and clearing it
    /// has to be possible: an instance that came out from behind a proxy should
    /// not keep advertising the proxy's address.
    #[test]
    fn the_base_url_is_absolute_or_refused() {
        let args = |value: Option<&str>| ComponentsEnableArgs {
            name: "ocs".to_string(),
            port: None,
            base_url: value.map(str::to_string),
            read_only: false,
            read_write: false,
            ocs: OcsConfigArgs::default(),
        };
        assert_eq!(base_url(&args(None)).unwrap(), None, "the flag was absent");
        assert_eq!(
            base_url(&args(Some("https://ocs.example.org/"))).unwrap(),
            Some(Some("https://ocs.example.org".to_string())),
            "the trailing slash goes: OCS appends a path to this"
        );
        assert_eq!(
            base_url(&args(Some("  "))).unwrap(),
            Some(None),
            "an empty value clears it"
        );
        let err = base_url(&args(Some("ocs.example.org"))).expect_err("no scheme");
        assert!(err.to_string().contains("absolute URL"), "{err}");
    }

    /// What `disable` stops before the definition goes away. The one-shot
    /// `-init` companions belong to whatever they prepared, and chap-core's
    /// services are upstream's, so they are everything else.
    #[test]
    fn a_component_owns_its_own_services_and_their_one_shot_companions() {
        let ocs = owns(Component::Ocs);
        assert!(ocs("ocs"));
        assert!(!ocs("s3") && !ocs("chap") && !ocs("chapkit-ewars-model"));

        let s3 = owns(Component::S3);
        assert!(s3("s3"));
        assert!(s3("s3-init"), "the bucket creator goes with it");
        assert!(!s3("ocs"));

        let core = owns(Component::ChapCore);
        assert!(core("chap") && core("chap-worker") && core("postgres"));
        assert!(!core("ocs") && !core("s3") && !core("s3-init"));
    }

    /// What the shared stop path acts on. Only the components that went from on
    /// to off: one that was already off has no containers to stop, and one that
    /// was just turned *on* certainly does not.
    #[test]
    fn only_the_components_that_went_off_are_the_ones_to_stop() {
        let mut before = Components::default();
        before.set_enabled(Component::Ocs, true);
        before.set_enabled(Component::S3, true);

        let mut after = before.clone();
        after.set_enabled(Component::S3, false);
        assert_eq!(disabled_between(&before, &after), vec![Component::S3]);

        let off = Components::default();
        assert_eq!(
            disabled_between(&before, &off),
            vec![Component::Ocs, Component::S3],
            "in the order they are listed and rendered"
        );
        assert!(disabled_between(&off, &before).is_empty(), "both went on");
        assert!(disabled_between(&before, &before).is_empty());

        // chap-core is a component like the others here; `disable` is what
        // refuses to take it away while models are enabled.
        let mut core_off = before.clone();
        core_off.set_enabled(Component::ChapCore, false);
        assert_eq!(
            disabled_between(&before, &core_off),
            vec![Component::ChapCore]
        );
    }

    /// The data volume line a disable closes with, and the two components that
    /// have no volume of their own to name.
    #[test]
    fn a_kept_volume_is_named_with_the_command_that_would_remove_it() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project {
            dir: dir.path().to_path_buf(),
            state: crate::project::ProjectState {
                compose_project: "hello1-abc123".to_string(),
                ..crate::project::ProjectState::default()
            },
        };
        let line = kept_volume_note(&project, Component::Ocs).expect("a volume to keep");
        assert!(line.starts_with("kept volume hello1-abc123_"), "{line}");
        assert!(
            line.contains("`chaps components disable ocs --purge`"),
            "{line}"
        );
        assert!(line.contains("docker volume rm"), "{line}");

        // chap-core's volumes are upstream's own, so there is nothing here to
        // name; `chaps down --volumes` is what removes them.
        assert_eq!(kept_volume_note(&project, Component::ChapCore), None);
        assert_eq!(component_volume(&project, Component::ChapCore), None);
    }

    /// The `.env` note has to go out on the run that appended the section and on
    /// no other, because "written .env" is also what a tag pin looks like.
    #[test]
    fn the_data_source_note_goes_out_only_when_the_section_was_appended() {
        let section = "# OCS data sources (optional)\n# ECMWF_DATASTORES_URL=\n# EDH_API_KEY=\n";
        assert!(data_sources_appended("POSTGRES_DB=chap_core\n", section));
        // A file that was already there keeps quiet, however much else changed.
        assert!(!data_sources_appended(
            section,
            &format!("{section}# CHAP_IMAGE_TAG=v2.3.1\n")
        ));
        assert!(!data_sources_appended("", ""), "--no-env: no file at all");
        assert!(
            !data_sources_appended(
                "POSTGRES_DB=chap_core\n",
                "POSTGRES_DB=chap_core\nS3_KEY=x\n"
            ),
            "some other section is not this one"
        );
    }

    /// The port the preflight asks about: the flag when it was given, and the
    /// recorded port otherwise - which is there while the component is off, since
    /// that is how a re-enable puts an instance back where it was.
    #[test]
    fn the_wanted_port_is_the_flag_or_the_one_already_recorded() {
        let args = |port: Option<ComponentPortArg>| ComponentsEnableArgs {
            name: "ocs".to_string(),
            port,
            base_url: None,
            read_only: false,
            read_write: false,
            ocs: OcsConfigArgs::default(),
        };
        let mut components = Components::default();
        assert_eq!(
            wanted_port(&components, Component::Ocs, &args(None)),
            Some(OCS_DEFAULT_PORT),
            "off, but the port it would come back on"
        );
        assert_eq!(
            wanted_port(
                &components,
                Component::Ocs,
                &args(Some(ComponentPortArg(Some(18010))))
            ),
            Some(18010)
        );
        // `--port none` asks for no host port, so there is nothing to probe.
        assert_eq!(
            wanted_port(
                &components,
                Component::Ocs,
                &args(Some(ComponentPortArg(None)))
            ),
            None
        );

        assert_eq!(
            wanted_port(&components, Component::S3, &args(None)),
            None,
            "the store publishes nothing by default"
        );
        components.s3.port = Some(18011);
        assert_eq!(
            wanted_port(&components, Component::S3, &args(None)),
            Some(18011)
        );
        // chap-core's host port is the API port, which `--api-port` moves.
        assert_eq!(
            wanted_port(
                &components,
                Component::ChapCore,
                &args(Some(ComponentPortArg(Some(18012))))
            ),
            Some(18012),
            "the value is read; set_port is what refuses it"
        );
    }

    #[test]
    fn the_scaffold_request_follows_the_flags() {
        let empty = request(&OcsConfigArgs::default()).into_spec();
        assert!(empty.example);
        assert_eq!(empty.country_code, "SLE");

        let filled = request(&OcsConfigArgs {
            ocs_name: Some("Malawi".into()),
            ocs_country: Some("mwi".into()),
            ocs_bbox: Some("32.6,-17.2,35.9,-9.3".into()),
        })
        .into_spec();
        assert!(!filled.example);
        assert_eq!(filled.id, "malawi-climate-service");
        assert_eq!(filled.name, "Malawi Climate Service");
        assert_eq!(filled.extent_name, "Malawi");
        assert_eq!(filled.country_code, "MWI");
        assert_eq!(filled.bbox, "32.6, -17.2, 35.9, -9.3");
    }
}
