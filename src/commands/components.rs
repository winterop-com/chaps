//! `chaps components list|enable|disable` — what a deployment is made of.
//!
//! The three commands do the same small thing: edit `.chaps/components.yaml`
//! and hand over to [`crate::compose::sync`], which renders one compose file
//! per enabled component and removes the ones that are no longer wanted. That
//! is the same path `init --with` takes, so a component enabled at init and one
//! enabled a week later end up with byte-identical files.

use crate::cli::{ComponentsDisableArgs, ComponentsEnableArgs, ComponentsListArgs, OcsConfigArgs};
use crate::commands::Ctx;
use crate::components::{Component, Components, S3_SOON_NOTE, models_need_chap_core};
use crate::compose::spec::OcsConfigRequest;
use crate::compose::sync::{sync, write_ocs_config};
use crate::error::Result;
use crate::output::Out;
use crate::project::Project;
use crate::registry;
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
    /// Whether the component was already in this state.
    pub unchanged: bool,
    pub written: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
    /// Everything worth saying that is not a failure: the S3 heads-up, a
    /// scaffolded config file.
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
            compose_file: match component {
                Component::ChapCore => None,
                Component::Ocs => Some(crate::components::OCS_COMPOSE.to_string()),
                Component::S3 => Some(crate::components::S3_COMPOSE.to_string()),
            },
            summary: component.summary().to_string(),
        })
        .collect()
}

/// Turn a component on, or change the settings of one that already is.
pub fn enable(ctx: &Ctx, args: &ComponentsEnableArgs) -> Result<()> {
    let component = Component::from_name(&args.name)?;
    let mut project = ctx.project()?;
    let before = project.state.components.clone();

    if let Some(port) = args.port {
        set_port(&mut project.state.components, component, port)?;
    }
    project.state.components.set_enabled(component, true);
    let after = project.state.components.clone();

    let mut notes = Vec::new();
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
    }
    if component == Component::Ocs && !after.s3.enabled {
        notes.push(S3_SOON_NOTE.to_string());
    }

    let registry = registry::load(&ctx.registry)?;
    let synced = sync(&mut project, &registry, ctx.cli_version, false)?;
    notes.extend(synced.warnings);

    let report = ChangeReport {
        name: component.name().to_string(),
        enabled: true,
        port: after.port_of(component),
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
/// volume, so `chaps docker run -- down -v` no longer reaches it. `--purge`
/// removes it with the component.
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

    // The containers go now, while compose still has the files that define
    // them: a service whose definition has just been removed cannot be
    // stopped by name, and one left running keeps its host port published
    // long after the component was disabled.
    let stopped = super::docker::stop_and_remove(&project, &owns(component));

    // And the volume after them, because docker refuses to remove one a
    // container still has mounted.
    let volume = component
        .volume()
        .and_then(|volume| project.prefixed_volume(volume));
    let mut volume_notes = Vec::new();
    let mut purged = Vec::new();
    let mut kept_volumes = Vec::new();
    match (&volume, args.purge) {
        (Some(name), true) => {
            let (removed, line) = super::docker::purge_volume(name);
            purged.extend(removed);
            volume_notes.push(line);
        }
        (Some(name), false) => {
            volume_notes.push(super::docker::kept_volume_line(
                name,
                &format!("chaps components disable {}", component.name()),
            ));
            kept_volumes.push(name.clone());
        }
        // chap-core keeps its volumes either way, and `--purge` was refused
        // above; a deployment with no compose project name can name none.
        (None, true) => volume_notes.push(super::docker::UNNAMEABLE_VOLUME.to_string()),
        (None, false) => {}
    }

    project.state.components.set_enabled(component, false);
    let after = project.state.components.clone();

    let registry = registry::load(&ctx.registry)?;
    let synced = sync(&mut project, &registry, ctx.cli_version, false)?;

    let mut notes = synced.warnings.clone();
    notes.extend(stopped);
    notes.extend(volume_notes);
    if component == Component::ChapCore {
        notes.push(
            "chap-core's own volumes are left alone; \
             `chaps docker run -- down -v` removes them"
                .to_string(),
        );
    }
    if component == Component::Ocs {
        notes.push("the ocs/ directory is left alone; it is yours".to_string());
    }
    let report = ChangeReport {
        name: component.name().to_string(),
        enabled: false,
        port: None,
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
     `chaps docker run -- down -v` removes every volume of this deployment";

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

/// The scaffold values the `--ocs-*` flags carry.
pub fn request(args: &OcsConfigArgs) -> OcsConfigRequest {
    OcsConfigRequest {
        name: args.ocs_name.clone(),
        country_code: args.ocs_country.clone(),
        bbox: args.ocs_bbox.clone(),
    }
}

/// Record a host port for a component that can publish one.
fn set_port(components: &mut Components, component: Component, port: u16) -> Result<()> {
    match component {
        Component::Ocs => components.ocs.port = port,
        Component::S3 => components.s3.port = Some(port),
        Component::ChapCore => {
            return Err(anyhow::anyhow!(
                "chap-core's host port is the API port; set it with \
                 `chaps init --api-port {port} --force` or CHAP_API_PORT in .env"
            ));
        }
    }
    Ok(())
}

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
        (true, None) => format!(
            "{painted} {} {}\n",
            report.name,
            out.dim("(no host port; it is reached inside the compose network)")
        ),
        (false, _) => format!("{painted} {}\n", report.name),
    };
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
        set_port(&mut components, Component::Ocs, 9010).unwrap();
        assert_eq!(components.ocs.port, 9010);
        set_port(&mut components, Component::S3, 9011).unwrap();
        assert_eq!(components.s3.port, Some(9011));

        let err = set_port(&mut components, Component::ChapCore, 8123)
            .expect_err("chap-core's port is the API port");
        assert!(err.to_string().contains("--api-port 8123"), "{err}");
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
