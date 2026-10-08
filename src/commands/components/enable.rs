//! `varde components enable`: turn a component on, change the settings of
//! one that already is, or point the deployment at a chap-core elsewhere.

use super::{ChangeReport, Note, NoteLevel, ReadMode, change_summary, label, notes_of};
use crate::cli::{ComponentsEnableArgs, OcsConfigArgs};
use crate::commands::Ctx;
use crate::components::{
    Component, Components, DHIS2_COMPOSE, DHIS2_CONNECT_NOTE, DHIS2_FIRST_START_NOTE,
    DHIS2_TAG_ENV_VAR, OCS_DATA_SOURCE_ENV_VARS, OCS_DATA_SOURCE_NOTE, S3_SOON_NOTE,
    S3_WITHOUT_OCS_NOTE, dhis2_seed_note,
};
use crate::compose::spec::{Dhis2ConfigSpec, OcsConfigRequest};
use crate::compose::sync::{KeyEdit, sync, write_dhis2_config, write_ocs_config};
use crate::error::Result;
use crate::project::{ENV_FILE, Project};

/// Turn a component on, or change the settings of one that already is.
pub fn enable(ctx: &Ctx, args: &ComponentsEnableArgs) -> Result<()> {
    let component = Component::from_name(&args.name)?;
    let (mut project, _lock) = ctx.project_mut()?;
    if component == Component::Dhis2
        && let Some(external) = &project.state.components.dhis2_external
    {
        return Err(anyhow::anyhow!(crate::components::dhis2_external_refusal(
            &external.url
        )));
    }
    let before = project.state.components.clone();
    if let Some(url) = &args.url {
        return use_external_chap_core(ctx, project, component, url, args.models_host.as_deref());
    }

    // The port preflight runs on the state as recorded, before anything moves:
    // a component already listening on one port must not be mistaken for the
    // holder of the port it is being moved to. `varde init` warns in exactly
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
    if (args.tag.is_some() || args.image.is_some()) && component != Component::Dhis2 {
        return Err(anyhow::anyhow!(
            "--tag and --image are DHIS2 settings: they pick the DHIS2 version, and no other \
             component's is chosen here"
        ));
    }
    if let Some(tag) = &args.tag {
        project.state.components.dhis2.image_tag = crate::components::dhis2_tag_arg(tag, "--tag")?;
    }
    if let Some(image) = &args.image {
        project.state.components.dhis2.image =
            crate::components::dhis2_image_arg(image, "--image", "--tag")?;
    }
    if let Some(given) = &args.seed {
        project.state.components.dhis2.seed = seed_arg(component, given, ctx.registry.offline)?;
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
    let mut read_edit = None;
    notes.extend(port_warning.map(Note::warning));
    // The scaffold goes in before the sync, so the `--ocs-*` values reach the
    // file rather than the example ones sync would fall back to.
    if component == Component::Ocs {
        let wanted = request(&args.ocs);
        let asked_for = !wanted.is_empty();
        match write_ocs_config(&project.dir, &wanted.into_spec())? {
            Some(path) => notes.push(Note::hint(format!(
                "wrote {}; it is yours to edit, and varde never rewrites it",
                label(&project, &path)
            ))),
            // Values were given for a file that is already there. Silently
            // discarding them would be the worst of the three answers.
            None if asked_for => notes.push(Note::warning(format!(
                "{} is already there, so the --ocs-* values were not used; edit that file \
                 instead, or delete it and enable ocs again",
                label(&project, &project.ocs_config_path())
            ))),
            None => {}
        }
        // And the read-only switch after it, so a component enabled and set
        // read-only in one command edits the file this run just scaffolded.
        if let Some((edit, text)) = set_read_only(&mut project, args)? {
            // A hint for now; the line is settled once the run knows whether
            // the read mode is all it changed.
            read_edit = Some((edit, text.clone(), notes.len()));
            notes.push(Note::hint(text));
        }
    }
    // DHIS2's config is scaffolded the same way and for a harder reason: without
    // `dhis.conf` DHIS2 does not start at all. Nothing is filled in from flags,
    // so there is no "values were not used" case to report - a file that is
    // already there is simply the operator's.
    if component == Component::Dhis2
        && let Some(path) = write_dhis2_config(&project.dir, &Dhis2ConfigSpec::default())?
    {
        notes.push(Note::hint(format!(
            "wrote {}; it is yours to edit, and varde never rewrites it",
            label(&project, &path)
        )));
    }
    let after = project.state.components.clone();
    if component == Component::Ocs && !after.s3.enabled {
        notes.push(Note::hint(S3_SOON_NOTE));
    }
    if component == Component::Dhis2 {
        // The pin first: it is the one line that asks the reader to stop and run
        // something else before `varde up`.
        notes.extend(dhis2_tag_moved(&project, &after).map(Note::warning));
        // The sync below says it with the reason when the pinned minor has no
        // dump, so this line would be the same sentence twice.
        if !after.dhis2_seed_is_unknown() {
            notes.push(Note::hint(dhis2_seed_note(after.dhis2_seed_source())));
        }
        notes.push(Note::hint(DHIS2_FIRST_START_NOTE));
        // Not for a DHIS2 that a connect is recorded for: a new port or tag
        // leaves its route as it is.
        if after.dhis2_needs_connecting() {
            notes.push(Note::info(DHIS2_CONNECT_NOTE));
        }
    }
    // The other half of the same soft dependency: a store with nothing to put
    // in it is worth a line, because the operator may have meant to add OCS too.
    if component == Component::S3 && !after.ocs.enabled {
        notes.push(Note::hint(S3_WITHOUT_OCS_NOTE));
    }
    // A deployment that started without chap-core never asked GitHub for a
    // release, so compose.yml comes from the copy built into this binary.
    if component == Component::ChapCore
        && !before.chap_core.enabled
        && project.state.chap_compose_source == crate::project::ComposeSource::Embedded
    {
        notes.push(Note::hint(format!(
            "compose.yml is rendered from the chap-core compose file built into varde, at tag \
             `{}`; `varde update --pin-chap-core` moves it to the newest release",
            project.state.chap_image_tag
        )));
    }

    // `sync` appends the OCS `.env` sections on the run that first needs them,
    // and its report only says the file was written - which it also says when it
    // appended a model's tag pin. The text before and after is what tells the
    // two apart, and the credentials are the one thing in there an operator has
    // to go and do something about.
    let env_before = env_text(&project);
    let registry = crate::commands::registry_for(ctx, Some(&project))?;
    let synced = sync(&mut project, &registry, false)?;
    notes.extend(notes_of(NoteLevel::Warning, synced.warnings));
    if component == Component::Ocs && data_sources_appended(&env_before, &env_text(&project)) {
        notes.push(Note::hint(OCS_DATA_SOURCE_NOTE));
    }

    let only_the_mode = component == Component::Ocs
        && synced.written.is_empty()
        && synced.removed.is_empty()
        && only_the_read_mode(&before, &after);
    // A running instance reads the file again only when it is recreated. When
    // the read mode is all that changed, the closing line names the restart.
    if let Some((KeyEdit::Rewritten | KeyEdit::Appended, text, at)) = &read_edit
        && before.ocs.enabled
        && !only_the_mode
    {
        notes[*at] = Note::info(format!("{text}; {READ_ONLY_APPLY}"));
    }
    let report = ChangeReport {
        name: component.name().to_string(),
        enabled: true,
        port: match component {
            // chap-core's host port is the API port, which `.env` can move.
            Component::ChapCore => Some(project.api_port_in_effect().0),
            _ => after.port_of(component),
        },
        base_url: ocs_only(component, after.ocs.base_url.clone()),
        read_only: ocs_only(component, Some(after.ocs.read_only)),
        unchanged: before == after,
        written: synced.written,
        removed: synced.removed,
        notes,
        purged: Vec::new(),
        kept_volumes: Vec::new(),
        read_mode: read_edit
            .filter(|_| only_the_mode)
            .map(|(edit, ..)| read_mode(edit, after.ocs.read_only)),
    };
    ctx.out
        .report(&report, |lines| change_summary(&report, &project, lines))
}

/// `varde components enable chap-core --url URL`: record a chap-core that runs
/// elsewhere, and re-render the model overlays so they register with it.
///
/// Refused while this deployment runs its own chap-core: switching would leave
/// its containers running with nothing in the `-f` list that names them, so
/// the operator turns it off first, where `disable` stops them.
fn use_external_chap_core(
    ctx: &Ctx,
    mut project: crate::project::Project,
    component: Component,
    url: &str,
    models_host: Option<&str>,
) -> Result<()> {
    if component != Component::ChapCore {
        return Err(anyhow::anyhow!(
            "--url is a chap-core setting: it names a chap-core elsewhere for the models \
             to register with; run `varde components enable chap-core --url URL`"
        ));
    }
    if project.state.components.chap_core.enabled {
        return Err(anyhow::anyhow!(
            "this deployment runs its own chap-core; turn it off first with \
             `varde components disable chap-core`, then run this again"
        ));
    }
    let before = project.state.components.clone();
    let mut external = crate::components::external_chap_core(url)?;
    let mut detected = None;
    match models_host.map(str::trim).filter(|h| !h.is_empty()) {
        Some(host) => external.models_host = host.to_string(),
        None => {
            detected = crate::components::detect_models_host(
                &mut external,
                &crate::docker::container_publishing,
            )
        }
    }
    project.state.components.chap_core_external = Some(external.clone());
    let after = project.state.components.clone();

    let registry = crate::commands::registry_for(ctx, Some(&project))?;
    let synced = sync(&mut project, &registry, false)?;
    let mut notes = notes_of(NoteLevel::Warning, synced.warnings);
    // A warning, as in `varde init`: it changes where chap-core calls the
    // models back, which the reader did not ask for.
    notes.extend(detected.map(Note::warning));
    notes.push(Note::hint(format!(
        "model services register with the chap-core at {} on the next `varde up`, calling \
         back to them at {}; `varde status` asks it",
        external.url, external.models_host
    )));
    let report = ChangeReport {
        name: component.name().to_string(),
        enabled: true,
        port: None,
        base_url: None,
        read_only: None,
        unchanged: before == after,
        written: synced.written,
        removed: synced.removed,
        notes,
        purged: Vec::new(),
        kept_volumes: Vec::new(),
        read_mode: None,
    };
    ctx.out
        .report(&report, |lines| change_summary(&report, &project, lines))
}

/// The host port a component will publish once this run is done: the `--port`
/// value when the flag was given, otherwise the port already recorded.
///
/// The recorded port is read whether or not the component is on yet, because
/// that is how a re-enable puts an instance back on the port it had.
pub(super) fn wanted_port(
    components: &Components,
    component: Component,
    args: &ComponentsEnableArgs,
) -> Option<u16> {
    match args.port {
        Some(given) => given.0,
        None => match component {
            Component::Ocs => components.ocs.port,
            Component::S3 => components.s3.port,
            Component::Dhis2 => components.dhis2.port,
            Component::ChapCore => None,
        },
    }
}

/// The warning for a run that moves the DHIS2 image pin while `dhis2_db` is
/// already there, or `None` when nothing is moving.
///
/// The tag that is *deployed* is the one the rendered `compose.dhis2.yml` on disk
/// defaults to, not the one the state records: the file is what compose reads, so
/// it is what the existing database was created and migrated by. No file means no
/// deployment yet, and nothing to warn about.
///
/// Best-effort about docker like every other step that needs it: no CLI or no
/// daemon answers "no such volume", which is the quiet answer - a warning about
/// a database that may not exist would be worse than none.
pub fn dhis2_tag_moved(project: &Project, wanted: &Components) -> Option<String> {
    if !wanted.dhis2.enabled {
        return None;
    }
    let deployed = rendered_dhis2_tag(project)?;
    if deployed == wanted.dhis2.image_tag {
        return None;
    }
    let volume = project.prefixed_volume(crate::compose::render::DHIS2_DB_VOLUME)?;
    if !crate::docker::volume_exists(&volume) {
        return None;
    }
    Some(crate::components::dhis2_tag_change_note(
        &deployed,
        &wanted.dhis2.image_tag,
    ))
}

/// The tag the rendered `compose.dhis2.yml` in this deployment defaults to, read
/// out of its `${DHIS2_IMAGE_TAG:-<tag>}`.
///
/// Text rather than a YAML round trip: one variable's default in one image line
/// is all that is wanted, and the file is generated, so its shape is ours.
pub(super) fn rendered_dhis2_tag(project: &Project) -> Option<String> {
    let body = std::fs::read_to_string(project.dir.join(DHIS2_COMPOSE)).ok()?;
    let opening = format!("${{{DHIS2_TAG_ENV_VAR}:-");
    let rest = &body[body.find(&opening)? + opening.len()..];
    Some(rest[..rest.find('}')?].to_string())
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
pub(super) fn data_sources_appended(before: &str, after: &str) -> bool {
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

/// The seed `--seed` names, with the same words and the same `--offline`
/// refusal as `varde init --dhis2-seed`.
///
/// The seed applies only when `dhis2_db` is created, which the seed note of
/// the report says, so a seed set on a database that is already there is
/// recorded and not refused.
pub(super) fn seed_arg(
    component: Component,
    given: &str,
    offline: bool,
) -> Result<crate::components::Dhis2Seed> {
    if component != Component::Dhis2 {
        return Err(anyhow::anyhow!(
            "--seed is a DHIS2 setting: it names the dump a new DHIS2 database is restored \
             from; run `varde components enable dhis2 --seed SPEC`"
        ));
    }
    let seed = crate::components::Dhis2Seed::parse(given);
    if offline && seed.is_url() {
        return Err(anyhow::anyhow!(
            "--offline and `--seed {}` ask for opposite things: the first `varde up` would \
             download that dump; pass a path to a dump you already have, or `--seed none`",
            seed.as_str()
        ));
    }
    Ok(seed)
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
pub(super) fn set_port(
    components: &mut Components,
    component: Component,
    port: Option<u16>,
) -> Result<()> {
    match component {
        Component::Ocs => components.ocs.port = port,
        Component::S3 => components.s3.port = port,
        Component::Dhis2 => components.dhis2.port = port,
        Component::ChapCore => {
            return Err(anyhow::anyhow!(
                "chap-core's host port is the API port; set it with \
                 CHAP_API_PORT={} in `.env`",
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
pub(super) fn base_url(args: &ComponentsEnableArgs) -> Result<Option<Option<String>>> {
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
/// given; otherwise what happened to the file, and the line that says it.
fn set_read_only(
    project: &mut Project,
    args: &ComponentsEnableArgs,
) -> Result<Option<(KeyEdit, String)>> {
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
            "there is no {config} to set {} in; run `varde sync` to scaffold it first",
            crate::components::OCS_READ_ONLY_KEY
        ));
    };
    project.state.components.ocs.read_only = wanted;
    let key = crate::components::OCS_READ_ONLY_KEY;
    let text = match edit {
        KeyEdit::Unchanged => format!("{config} already has {key}: {wanted}"),
        KeyEdit::Rewritten => format!("set {key}: {wanted} in {config}"),
        KeyEdit::Appended => format!("added {key}: {wanted} to {config}"),
    };
    Ok(Some((edit, text)))
}

/// Whether OCS was on before the run and nothing but its read mode differs
/// after it.
pub(super) fn only_the_read_mode(before: &Components, after: &Components) -> bool {
    let mut same = before.clone();
    same.ocs.read_only = after.ocs.read_only;
    before.ocs.enabled && same == *after
}

/// The [`ReadMode`] a run reports, from what it did to the instance config.
pub(super) fn read_mode(edit: KeyEdit, read_only: bool) -> ReadMode {
    match edit {
        KeyEdit::Unchanged => ReadMode::Unchanged { read_only },
        KeyEdit::Rewritten | KeyEdit::Appended => ReadMode::Changed { read_only },
    }
}

/// How a change to `ocs/climate-service.yaml` reaches the running instance.
///
/// The instance config is a bind mount, which compose does not compare, so
/// `varde restart` recreates a service whose mounted config is newer than its
/// container (see [`crate::commands::docker::edited_configs`]). Naming `ocs`
/// leaves chap-core and the models alone.
const READ_ONLY_APPLY: &str =
    "`varde restart ocs` applies it, recreating the instance so it reads the file again";
