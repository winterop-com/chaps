//! Rendering `.chaps/` into the compose files at the project root.
//!
//! `.chaps/` is intent; `compose.yml`, `compose.chaps.yml`,
//! `compose.<service_id>.yml` and `compose.marketplace.yml` are artifacts.
//! [`sync`] is the one place that
//! turns the former into the latter: `apply` ends in it, `chaps sync` calls it
//! directly and `chaps up` runs it before `docker compose up`.
//!
//! Rendering is deterministic and every file is compared before it is
//! written, so a second sync reports everything as unchanged. `compose.yml`
//! is deterministic too because the copy of chap-core's `compose.ghcr.yml` it
//! is rendered from is kept in `.chaps/`; nothing here touches the network.

use crate::components::{
    Components, DHIS2_COMPOSE, DHIS2_CONFIG_FILE, DHIS2_DB_PASSWORD_ENV_VAR,
    DHIS2_DEFAULT_JAVA_OPTIONS, DHIS2_DIR, DHIS2_ENCRYPTION_PASSWORD_ENV_VAR, DHIS2_JAVA_ENV_VAR,
    DHIS2_SEED_ENV_VAR, DHIS2_TAG_ENV_VAR, OCS_COMPOSE, OCS_CONFIG_FILE, OCS_DATA_SOURCE_ENV_VARS,
    OCS_DIR, OCS_PLUGINS_KEY, OCS_PLUGINS_TARGET, OCS_READ_ONLY_KEY, OCS_TAG_ENV_VAR,
    S3_ACCESS_KEY_ENV_VAR, S3_COMPOSE, S3_SECRET_KEY_ENV_VAR, S3_TAG_ENV_VAR,
};
use crate::compose::overrides;
use crate::compose::render::{
    NO_TAG_PINS, render_base, render_chaps_overlay, render_dhis2, render_dhis2_config, render_ocs,
    render_ocs_config, render_overlay, render_s3, render_umbrella,
};
use crate::compose::spec::{
    BaseSpec, Dhis2ConfigSpec, Dhis2Spec, OcsConfigSpec, OcsSpec, OverlaySpec, S3Spec,
    UpstreamCompose,
};
use crate::compose::tag_env_var;
use crate::error::Result;
use crate::project::{
    BASE_COMPOSE, CHAP_TAG_ENV_VAR, CHAPS_COMPOSE, ComposeSource, ENV_FILE, MARKETPLACE_COMPOSE,
    Project, compose_files_for,
};
use crate::registry::Registry;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What [`sync`] did, or with `check` what it would have done.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    /// Files created or whose content changed.
    pub written: Vec<PathBuf>,
    /// Files that already had the rendered content.
    pub unchanged: Vec<PathBuf>,
    /// Overlays of models that are no longer enabled.
    pub removed: Vec<PathBuf>,
    /// Whether anything differed from what `.chaps/` describes.
    pub drift: bool,
    /// Whether this was a dry run.
    pub check: bool,
    pub warnings: Vec<String>,
}

impl SyncReport {
    /// One line: `2 written, 1 unchanged, 1 removed`.
    pub fn summary(&self) -> String {
        let (write, remove) = if self.check {
            ("to write", "to remove")
        } else {
            ("written", "removed")
        };
        format!(
            "{} {write}, {} unchanged, {} {remove}",
            self.written.len(),
            self.unchanged.len(),
            self.removed.len()
        )
    }
}

/// Render every enabled model's overlay and the umbrella from the project
/// state, remove overlays this CLI wrote for models that are no longer
/// enabled, and append missing `.env` pin comments.
///
/// With `check` nothing is written or removed and the state is left alone;
/// the report says what a real run would do. Otherwise
/// `state.rendered_files` is updated and `.chaps/` is saved.
pub fn sync(project: &mut Project, registry: &Registry, check: bool) -> Result<SyncReport> {
    let mut report = SyncReport {
        check,
        ..SyncReport::default()
    };
    let dir = project.dir.clone();
    if !check {
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", dir.display()))?;
    }

    // Desired artifacts: the base stack, the overlays in marketplace-id order,
    // then the umbrella that includes them.
    let mut desired: Vec<(String, String)> = Vec::new();
    // The umbrella includes the overlays and only the overlays; the base file
    // is one of the `-f` list, not something to include.
    let mut overlays: Vec<String> = Vec::new();
    let components = project.state.components.clone();
    // The compose project name is settled before anything is rendered: it goes
    // into every chaps-owned file. A deployment written before the field
    // existed has no recorded name, and takes the one compose has been
    // deriving from its directory all along - so writing it down below renames
    // nothing, and a running deployment keeps every container and volume name
    // it has. Only `chaps init` generates a name with a suffix of its own.
    let project_name = project.compose_project_name();
    // chap-core is a component like the others: with it off, neither the base
    // stack nor the chaps-owned override belongs to this deployment, and both
    // are removed below.
    if components.chap_core.enabled {
        let (base, base_warnings) = base_compose(project);
        report.warnings.extend(base_warnings);
        if let Some(base) = base {
            desired.push((BASE_COMPOSE.to_string(), base));
        }
        // Always rendered, base file or not: it is the only place the API's
        // host port is decided, and it has to be a `-f` entry of its own
        // because a file in `include:` cannot override a service compose.yml
        // defines.
        desired.push((
            CHAPS_COMPOSE.to_string(),
            render_chaps_overlay(
                project.state.api_port,
                project_name.as_deref(),
                match &project.state.chap_compose_source {
                    ComposeSource::Checkout { path } => Some(path.as_str()),
                    _ => None,
                },
            ),
        ));
    }
    // The components sit between the base stack and the model overlays, in
    // the same order as the `-f` list.
    if components.ocs.enabled {
        desired.push((
            OCS_COMPOSE.to_string(),
            render_ocs(&OcsSpec::from_components(
                &components,
                project.ocs_plugins_path().is_dir(),
            )),
        ));
    }
    if components.s3.enabled {
        desired.push((
            S3_COMPOSE.to_string(),
            render_s3(&S3Spec::from_components(&components)),
        ));
    }
    if components.dhis2.enabled {
        let spec = Dhis2Spec::from_components(&components);
        let rendered = render_dhis2(&spec);
        // The seed was left at `default` and the pinned minor line publishes no
        // dump chaps knows the path of, so this deployment starts empty. Said
        // on the sync that writes the DHIS2 compose file - the first, and any
        // that changes it - because nothing else on the screen would show it;
        // every `chaps up` after that would only repeat it.
        let writes_dhis2 = std::fs::read_to_string(dir.join(DHIS2_COMPOSE))
            .map_or(true, |existing| existing != rendered);
        if components.dhis2_seed_is_unknown() && writes_dhis2 {
            report
                .warnings
                .push(crate::components::dhis2_unknown_seed(&spec.image_tag));
        }
        // A dump that is a file is a bind mount, and compose refuses to start a
        // service whose bind source does not exist - so a path that is not there
        // is a `chaps up` that fails, reported now instead.
        if let Some(crate::compose::spec::Dhis2SeedSource::File(path)) = &spec.seed
            && !dir.join(path).exists()
        {
            report.warnings.push(format!(
                "the dhis2 seed names {path}, which is not in {}; copy the dump there before \
                 `chaps up`, or set `seed: none` in .chaps/components.yaml",
                dir.display()
            ));
        }
        desired.push((DHIS2_COMPOSE.to_string(), rendered));
    }
    // Every overlay carries the registration key line when the deployment has
    // a key, because chap-core then requires it from each service that
    // registers. The value stays in `.env`, which compose reads from the
    // project directory, so nothing has to be threaded into the overlays.
    let registration_key = project.state.auth.registration_key;
    let standalone = !project
        .state
        .components
        .is_enabled(crate::components::Component::ChapCore);
    let external = project
        .state
        .components
        .chap_core_external
        .as_ref()
        .map(|external| crate::compose::spec::ExternalRegistration {
            register_url: external
                .url_from_containers()
                .trim_end_matches('/')
                .to_string(),
            models_host: external.models_host.clone(),
        });
    for (id, model) in &project.state.models {
        let mut spec = match registry.get(id) {
            Some(m) => OverlaySpec::from_enabled(id, model, m),
            None => {
                report.warnings.push(format!(
                    "{id} is enabled but not in the registry; its overlay header \
                     names the image in place of the repository"
                ));
                OverlaySpec::from_enabled_without_registry(id, model)
            }
        };
        spec.registration_key = registration_key;
        spec.standalone = standalone;
        spec.external_chap_core = external.clone();
        // The overlay's init container chowns the data volume from busybox,
        // which resolves no account name of its own, so the user has to be
        // expressible as numbers. An unknown one still renders, with the
        // chapkit ids, but the operator should know the guess was taken. A
        // model that runs as root resolves to `0:0`, so it never lands here.
        if overrides::numeric_pair(&spec.user).is_none() {
            report.warnings.push(format!(
                "{id} runs as `{}`, which has no known uid:gid; the volume init \
                 container will chown {} to {} instead - pass `--user <uid>:<gid>` \
                 if that is wrong",
                spec.user,
                spec.data_dir,
                overrides::FALLBACK_UID_GID
            ));
        }
        overlays.push(model.compose_file.clone());
        desired.push((model.compose_file.clone(), render_overlay(&spec)));
    }
    desired.push((
        MARKETPLACE_COMPOSE.to_string(),
        render_umbrella(&overlays, project_name.as_deref()),
    ));

    for (name, content) in &desired {
        let path = dir.join(name);
        let same = std::fs::read_to_string(&path).ok().as_deref() == Some(content.as_str());
        crate::output::verbose(&format!(
            "compared {} ({} rendered bytes): {}",
            path.display(),
            content.len(),
            if same { "unchanged" } else { "differs" }
        ));
        if same {
            report.unchanged.push(path);
            continue;
        }
        if !check {
            std::fs::write(&path, content)
                .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
        }
        report.written.push(path);
    }

    // Only overlays a previous sync wrote are candidates for removal, and
    // only when their service is gone: a hand-written compose.*.yml is never
    // ours to delete. The base stack is the one exception, and a narrow one:
    // it goes when chap-core itself was turned off, never because it failed to
    // render (a base stack we cannot reproduce is not one to delete either).
    let wanted: BTreeSet<&str> = desired.iter().map(|(f, _)| f.as_str()).collect();
    let doomed: BTreeSet<&str> = if components.chap_core.enabled {
        BTreeSet::new()
    } else {
        BTreeSet::from([BASE_COMPOSE, CHAPS_COMPOSE])
    };
    for name in &project.state.rendered_files {
        if wanted.contains(name.as_str()) {
            continue;
        }
        if !is_overlay_name(name) && !doomed.contains(name.as_str()) {
            continue;
        }
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        if !check {
            std::fs::remove_file(&path)
                .map_err(|e| anyhow::anyhow!("removing {}: {e}", path.display()))?;
        }
        report.removed.push(path);
    }

    if let Some(env) = append_env_pins(project, check)? {
        report.written.push(env);
    }
    // The OCS instance config is scaffolded, not rendered: it is the
    // operator's file the moment it exists, so sync only ever creates a
    // missing one.
    if let Some(path) = ensure_ocs_config(project, check)? {
        report.written.push(path);
    }
    // And DHIS2's, on exactly the same terms: DHIS2 will not start without
    // `dhis.conf`, so a missing one is scaffolded, and one that is there is the
    // operator's.
    if let Some(path) = ensure_dhis2_config(project, check)? {
        report.written.push(path);
    }
    // And once it is there, the one key that has to follow the project rather
    // than the operator: a plugin directory that is mounted and not configured
    // is a mount OCS never looks in.
    if let Some(path) = ensure_plugins_key(project, check)?
        && !report.written.contains(&path)
    {
        report.written.push(path);
    }

    report.drift = !report.written.is_empty() || !report.removed.is_empty();
    if check {
        return Ok(report);
    }

    // A project written before compose.chaps.yml existed records a two-entry
    // `-f` list; the first sync after an upgrade puts the new file in it, and
    // the same step puts the component files in the list when one is enabled.
    project.state.compose_files = compose_files_for(&components);
    // The same upgrade step for the compose project name: what was implicit in
    // the directory name becomes explicit in `project.yaml`, unchanged.
    if let Some(name) = project_name {
        project.state.compose_project = name;
    }
    project.state.rendered_files = desired.into_iter().map(|(f, _)| f).collect();
    project.save()?;
    Ok(report)
}

/// The `compose.yml` this project's state describes, plus any warning about
/// the source it is rendered from.
///
/// `None` means the recorded source cannot be replayed - the cached copy of
/// chap-core's `compose.ghcr.yml` is gone - in which case the file on disk is
/// left exactly as it is: a base stack we cannot reproduce is not one to
/// overwrite with a guess.
fn base_compose(project: &Project) -> (Option<String>, Vec<String>) {
    let embedded = || render_base(&BaseSpec { upstream: None });
    if let ComposeSource::Checkout { path } = &project.state.chap_compose_source {
        let file = Path::new(path).join(crate::project::CHECKOUT_COMPOSE);
        return match std::fs::read_to_string(&file) {
            Ok(body) => (
                Some(render_base(&BaseSpec {
                    upstream: Some(UpstreamCompose {
                        tag: format!("the checkout at {path}"),
                        body,
                    }),
                })),
                Vec::new(),
            ),
            Err(_) => (
                None,
                vec![format!(
                    "{} is missing, so {BASE_COMPOSE} is left as it is; check the chap-core \
                     checkout is still at {path}, or re-run `chaps init --force --source PATH`",
                    file.display()
                )],
            ),
        };
    }
    let ComposeSource::Fetched { tag, sha256, .. } = &project.state.chap_compose_source else {
        return (Some(embedded()), Vec::new());
    };

    let Some(path) = project.cached_compose_path() else {
        return (Some(embedded()), Vec::new());
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Ok(body) = std::fs::read_to_string(&path) else {
        return (
            None,
            vec![format!(
                ".chaps/{name} is missing, so {BASE_COMPOSE} is left as it is; \
                 re-run `chaps init --force --chap-tag {tag}` to fetch it again"
            )],
        );
    };

    let mut warnings = Vec::new();
    if crate::chapcore::sha256_hex(body.as_bytes()) != *sha256 {
        warnings.push(format!(
            ".chaps/{name} no longer matches the checksum recorded in .chaps/project.yaml; \
             {BASE_COMPOSE} follows the file as it is now"
        ));
    }
    let text = render_base(&BaseSpec {
        upstream: Some(UpstreamCompose {
            tag: tag.clone(),
            body,
        }),
    });
    (Some(text), warnings)
}

/// `compose.<something>.yml`, the shape of a model overlay.
fn is_overlay_name(name: &str) -> bool {
    name.starts_with("compose.")
        && name.ends_with(".yml")
        && name != MARKETPLACE_COMPOSE
        && name != BASE_COMPOSE
        && name != CHAPS_COMPOSE
}

/// Add a commented image pin to `.env` for every enabled model that has none,
/// and the settings section of every enabled component that has none.
///
/// Only ever appends: the file belongs to the operator once `init` wrote it,
/// and it may hold passwords and tokens we must not rewrite. Returns the path
/// when something was (or with `check`, would be) appended.
fn append_env_pins(project: &Project, check: bool) -> Result<Option<PathBuf>> {
    let path = project.dir.join(ENV_FILE);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    let mut pins = Vec::new();
    for (id, model) in &project.state.models {
        let var = tag_env_var(id);
        if !mentions_var(&body, &var) {
            pins.push(format!("# {var}={}", model.image_tag));
        }
    }
    let components = component_env_sections(&project.state.components, &body)?;
    if pins.is_empty() && components.is_empty() {
        return Ok(None);
    }
    if check {
        return Ok(Some(path));
    }

    // The generated .env carries a placeholder under the pin heading so the
    // section is never a dangling title; the first real pin replaces it. With
    // no pin to put there it stays, or the heading would dangle instead.
    let mut out: String = body
        .lines()
        .filter(|line| pins.is_empty() || line.trim_end() != NO_TAG_PINS)
        .map(|line| format!("{line}\n"))
        .collect();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if !pins.is_empty() {
        out.push_str(&pins.join("\n"));
        out.push('\n');
    }
    for section in &components {
        out.push('\n');
        out.push_str(section);
    }
    std::fs::write(&path, out).map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(Some(path))
}

/// The `.env` sections the enabled components need and `body` does not have.
///
/// Each one is appended whole, with its own heading, so the file reads as
/// sections rather than as a tail of loose variables. The object store's
/// credentials are generated here and never rewritten: like the database
/// password, the volume they created is stamped with them.
fn component_env_sections(components: &Components, body: &str) -> Result<Vec<String>> {
    let mut sections = Vec::new();
    if components.ocs.enabled && !mentions_var(body, OCS_TAG_ENV_VAR) {
        sections.push(format!(
            "# OCS (component). Uncomment to pin a build; the default follows `main`.\n\
             # {OCS_TAG_ENV_VAR}={}\n",
            components.ocs.image_tag
        ));
    }
    // Placeholders only: a credential is the operator's to paste in, and an
    // ERA5-Land key that arrives here would have had to come from somewhere
    // this CLI has no business reading. The section exists so the variable
    // names are in the file an operator already edits rather than in the docs
    // alone, and so `chaps auth show` has something to report on.
    //
    // The heading wraps at 80 columns, unlike the notes this same fact is
    // printed in: a note scrolls past in a terminal, and `.env` is opened in an
    // editor. Neither line is what decides whether the section is already here -
    // that is `mentions_var` over the five variables below - so a deployment
    // whose `.env` carries the one-line heading this replaced is left alone
    // exactly as one carrying this heading is.
    if components.ocs.enabled
        && !OCS_DATA_SOURCE_ENV_VARS
            .iter()
            .any(|var| mentions_var(body, var))
    {
        let mut section = String::from(
            "# OCS data sources (optional): ERA5-Land needs one or both of ECMWF_DATASTORES_*\n\
             # and EDH_API_KEY, per dataset; WorldPop and CHIRPS3 need none.\n",
        );
        for var in OCS_DATA_SOURCE_ENV_VARS {
            let value = if *var == "ECMWF_DATASTORES_URL" {
                crate::components::OCS_ECMWF_URL
            } else {
                ""
            };
            section.push_str(&format!("# {var}={value}\n"));
        }
        sections.push(section);
    }
    if components.s3.enabled
        && !(mentions_var(body, S3_ACCESS_KEY_ENV_VAR) || mentions_var(body, S3_SECRET_KEY_ENV_VAR))
    {
        sections.push(format!(
            "# S3 (component). Root credentials, generated once; see the docs before changing them.\n\
             {S3_ACCESS_KEY_ENV_VAR}={}\n\
             {S3_SECRET_KEY_ENV_VAR}={}\n\
             # {S3_TAG_ENV_VAR}={}\n",
            crate::auth::random_hex(16)?,
            crate::auth::random_hex(16)?,
            crate::components::S3_DEFAULT_TAG,
        ));
    }
    // Two generated secrets, and the same rule the object store's follow: the
    // volume was created with them, so they are written once and never touched
    // again. `random_hex(16)` is 32 characters, comfortably past the 24 DHIS2
    // demands of the encryption password - under that it stops on
    // ENCRYPTION_PASSWORD_TOO_SHORT rather than starting with a weak key.
    //
    // The three commented lines below them are the pins an operator goes looking
    // for: the image, the dump the one-shot fetches and the JVM options the
    // compose file reads, each with the value the rendered file already defaults
    // to, so uncommenting one changes nothing until it is edited.
    if components.dhis2.enabled
        && !(mentions_var(body, DHIS2_DB_PASSWORD_ENV_VAR)
            || mentions_var(body, DHIS2_ENCRYPTION_PASSWORD_ENV_VAR))
    {
        // The value the rendered file already defaults to, which for a dump that
        // is a file is the path *inside* the container: the host path would be
        // one nothing in there can read.
        let spec = Dhis2Spec::from_components(components);
        let seed = spec
            .seed
            .as_ref()
            .map(crate::compose::render::seed_value)
            .unwrap_or_default();
        sections.push(format!(
            "# DHIS2 (component). Generated once; the database volume was created with them.\n\
             {DHIS2_DB_PASSWORD_ENV_VAR}={}\n\
             {DHIS2_ENCRYPTION_PASSWORD_ENV_VAR}={}\n\
             # {DHIS2_TAG_ENV_VAR}={}\n\
             # {DHIS2_SEED_ENV_VAR}={seed}\n\
             # {DHIS2_JAVA_ENV_VAR}={DHIS2_DEFAULT_JAVA_OPTIONS}\n",
            crate::auth::random_hex(16)?,
            crate::auth::random_hex(16)?,
            spec.image_tag,
        ));
    }
    // The credentials `chaps dhis2` authenticates with, as placeholders holding
    // the values it already falls back to - so uncommenting one changes nothing
    // until it is edited, the same rule the pins above follow.
    //
    // The variable names are the whole answer to "where do the credentials
    // come from", and a name nobody can find is a name nobody sets.
    // Nothing here is a secret: every line is commented, `admin` and `district`
    // are what a seeded dump and an empty database both give, and no container
    // is passed any of them. An external DHIS2 gets no `district`: chaps did not
    // create it, so there is no default to write down.
    let deployed = components.dhis2.enabled && components.dhis2_external.is_none();
    let wanted = components.dhis2.enabled || components.dhis2_external.is_some();
    let login_named = mentions_var(body, crate::dhis2::ADMIN_USERNAME_ENV_VAR)
        || mentions_var(body, crate::dhis2::ADMIN_PASSWORD_ENV_VAR);
    if wanted && !login_named {
        sections.push(format!(
            "# DHIS2 login `chaps dhis2` uses. Only chaps reads these - no container is given\n\
             # them - and {}. A personal access token,\n\
             # when set, is used instead of the username and password.\n\
             # {}=\n\
             # {}={}\n\
             # {}={}\n",
            match deployed {
                true => "commented out means the DHIS2 default below",
                false => "chaps has no default for a DHIS2 it did not deploy",
            },
            crate::dhis2::API_TOKEN_ENV_VAR,
            crate::dhis2::ADMIN_USERNAME_ENV_VAR,
            crate::dhis2::DEFAULT_USERNAME,
            crate::dhis2::ADMIN_PASSWORD_ENV_VAR,
            match deployed {
                true => crate::dhis2::DEFAULT_PASSWORD,
                false => "",
            },
        ));
    }
    Ok(sections)
}

/// Scaffold `ocs/climate-service.yaml` when the `ocs` component is on and the
/// file is not there yet.
///
/// Never overwrites: `chaps components enable ocs --ocs-country ...` writes it
/// with the values it was given, and this only covers the case where the
/// component is on and the file has gone missing - a fresh checkout of a
/// deployment whose `ocs/` was never committed, most of all.
fn ensure_ocs_config(project: &Project, check: bool) -> Result<Option<PathBuf>> {
    if !project.state.components.ocs.enabled {
        return Ok(None);
    }
    let path = project.ocs_config_path();
    if path.is_file() {
        return Ok(None);
    }
    if check {
        return Ok(Some(path));
    }
    write_ocs_config(&project.dir, &OcsConfigSpec::default())?;
    Ok(Some(path))
}

/// Write `ocs/climate-service.yaml`, creating `ocs/` around it.
///
/// Returns the path when the file was written and `None` when one was already
/// there: the scaffold is a starting point, not something to put back.
pub fn write_ocs_config(dir: &Path, spec: &OcsConfigSpec) -> Result<Option<PathBuf>> {
    let ocs = dir.join(OCS_DIR);
    let path = ocs.join(OCS_CONFIG_FILE);
    if path.is_file() {
        return Ok(None);
    }
    std::fs::create_dir_all(&ocs)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", ocs.display()))?;
    std::fs::write(&path, render_ocs_config(spec))
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(Some(path))
}

/// Scaffold `dhis2/dhis.conf` when the `dhis2` component is on and the file is
/// not there yet.
///
/// Never overwrites: `chaps components enable dhis2` writes it the first time,
/// and this covers the case where the component is on and the file has gone -
/// a fresh checkout of a deployment whose `dhis2/` was never committed, most of
/// all. Without the file DHIS2 does not start at all, so a missing one is worth
/// putting back.
fn ensure_dhis2_config(project: &Project, check: bool) -> Result<Option<PathBuf>> {
    if !project.state.components.dhis2.enabled {
        return Ok(None);
    }
    let path = dhis2_config_path(&project.dir);
    if path.is_file() {
        return Ok(None);
    }
    if check {
        return Ok(Some(path));
    }
    write_dhis2_config(&project.dir, &Dhis2ConfigSpec::default())?;
    Ok(Some(path))
}

/// Where a deployment keeps its DHIS2 instance config.
pub fn dhis2_config_path(dir: &Path) -> PathBuf {
    dir.join(DHIS2_DIR).join(DHIS2_CONFIG_FILE)
}

/// Write `dhis2/dhis.conf`, creating `dhis2/` around it.
///
/// Returns the path when the file was written and `None` when one was already
/// there: the scaffold is a starting point, not something to put back.
pub fn write_dhis2_config(dir: &Path, spec: &Dhis2ConfigSpec) -> Result<Option<PathBuf>> {
    let dhis2 = dir.join(DHIS2_DIR);
    let path = dhis2.join(DHIS2_CONFIG_FILE);
    if path.is_file() {
        return Ok(None);
    }
    std::fs::create_dir_all(&dhis2)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", dhis2.display()))?;
    std::fs::write(&path, render_dhis2_config(spec))
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(Some(path))
}

/// What [`set_config_key`] found in `ocs/climate-service.yaml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEdit {
    /// The key was already set to this value; nothing changed.
    Unchanged,
    /// Its line was rewritten in place.
    Rewritten,
    /// The file did not have the key, so it was added at the end.
    Appended,
}

/// Set one top-level key of an OCS instance config, and nothing else.
///
/// The file is the operator's: it carries their comments, their dataset list
/// and their scheduler, so this rewrites the one line that assigns `key` and
/// leaves every other byte alone, appending the key when the file does not have
/// it. Text rather than a serde round trip for exactly that reason - parsing
/// and re-emitting the document would drop every comment in it.
///
/// Only a top-level key counts, so a `read_only:` nested inside some other
/// block is not mistaken for this one, and neither is a commented line: the
/// commented `# read_only: true` that a note explains is still only a note.
/// A trailing comment on the line survives the rewrite.
pub fn set_config_key(body: &str, key: &str, value: &str) -> (String, KeyEdit) {
    let wanted = format!("{key}: {value}");
    let mut out = String::with_capacity(body.len() + wanted.len() + 2);
    let mut edit = KeyEdit::Appended;
    for line in body.lines() {
        match top_level_value(line, key) {
            Some(rest) if edit == KeyEdit::Appended => {
                // The operator's comment on this line says why the value is
                // what it is, so it comes across with its own spacing.
                let replacement = format!("{wanted}{}", trailing_comment(rest));
                edit = if replacement == line {
                    KeyEdit::Unchanged
                } else {
                    KeyEdit::Rewritten
                };
                out.push_str(&replacement);
            }
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    if edit != KeyEdit::Appended {
        return (out, edit);
    }
    // A blank line first, so an appended key reads as its own setting rather
    // than as a continuation of whatever the file happened to end on.
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&wanted);
    out.push('\n');
    (out, KeyEdit::Appended)
}

/// Whether an OCS instance config assigns a top-level `key`.
pub fn has_config_key(body: &str, key: &str) -> bool {
    body.lines()
        .any(|line| top_level_value(line, key).is_some())
}

/// The inline comment of a value, whitespace and all, or `""`.
///
/// YAML needs whitespace before an inline `#`, which is also what tells one
/// apart from a `#` inside the value; the whitespace comes along so the
/// rewritten line is aligned exactly as the operator aligned it.
fn trailing_comment(rest: &str) -> &str {
    let Some(at) = rest
        .char_indices()
        .find(|(at, ch)| *ch == '#' && rest[..*at].ends_with([' ', '\t']))
        .map(|(at, _)| at)
    else {
        return "";
    };
    &rest[rest[..at].trim_end_matches([' ', '\t']).len()..]
}

/// The text after `key:` when `line` is that key's top-level assignment.
///
/// Top-level means column zero: YAML nests by indentation, so an indented
/// `read_only:` belongs to some other block and is none of our business.
fn top_level_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    if line.starts_with([' ', '\t', '#']) {
        return None;
    }
    line.strip_prefix(key)?.strip_prefix(':')
}

/// Write `read_only` into `ocs/climate-service.yaml`.
///
/// Returns `None` when there is no file to edit, which is the case a caller
/// reports rather than fails on: the component may not be enabled yet.
pub fn set_read_only(dir: &Path, read_only: bool) -> Result<Option<KeyEdit>> {
    let path = dir.join(OCS_DIR).join(OCS_CONFIG_FILE);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    let (out, edit) = set_config_key(&body, OCS_READ_ONLY_KEY, &read_only.to_string());
    if edit == KeyEdit::Unchanged {
        return Ok(Some(edit));
    }
    std::fs::write(&path, out).map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(Some(edit))
}

/// Point the instance config at the mounted plugin directory when the project
/// has one and the file does not name it yet.
///
/// Only ever adds the key: an operator who pointed `plugins_dir` somewhere else
/// meant it, and the mount is at a fixed path either way. Returns the path when
/// the file was (or with `check`, would be) changed.
fn ensure_plugins_key(project: &Project, check: bool) -> Result<Option<PathBuf>> {
    if !project.state.components.ocs.enabled || !project.ocs_plugins_path().is_dir() {
        return Ok(None);
    }
    let path = project.ocs_config_path();
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(None);
    };
    if has_config_key(&body, OCS_PLUGINS_KEY) {
        return Ok(None);
    }
    if check {
        return Ok(Some(path));
    }
    let (out, _) = set_config_key(&body, OCS_PLUGINS_KEY, OCS_PLUGINS_TARGET);
    std::fs::write(&path, out).map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(Some(path))
}

/// Move the commented `# <VAR>=<tag>` pin line of `var` to `tag`.
///
/// Only that one comment line changes. An active (uncommented) `VAR=` line is
/// the operator's own override and is left alone, as is a `.env` that does
/// not mention the variable at all (the next sync appends it). Returns
/// whether the file changed.
pub fn refresh_env_pin(dir: &Path, var: &str, tag: &str) -> Result<bool> {
    let path = dir.join(ENV_FILE);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(false);
    };
    let wanted = format!("# {var}={tag}");
    let mut changed = false;
    let mut out = String::with_capacity(body.len());
    for line in body.lines() {
        if is_commented_pin(line, var) && line != wanted {
            out.push_str(&wanted);
            changed = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !changed {
        return Ok(false);
    }
    std::fs::write(&path, out).map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(true)
}

/// What [`set_env_chap_tag`] found in `.env`, and therefore what the running
/// deployment will do with the new tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvTag {
    /// There is no `.env` (a project created with `init --no-env`).
    NoFile,
    /// The active `CHAP_IMAGE_TAG=` line now names the new tag.
    Updated,
    /// The only mention is a commented placeholder, so the deployment follows
    /// the compose default. Left alone: uncommenting it is the operator's call.
    Commented,
    /// An active line holds a value this project did not put there.
    Foreign(String),
    /// The file never mentions the variable.
    Absent,
}

/// Move the active `CHAP_IMAGE_TAG=` line of `.env` from `old` to `new`.
///
/// Exactly one line may change, and only when it still says what this project
/// recorded: an operator who pinned something else, or who left the generated
/// placeholder commented out, has made a decision that `chaps update` does not
/// get to undo. The outcome says which of those it was so the caller can warn.
pub fn set_env_chap_tag(dir: &Path, old: &str, new: &str) -> Result<EnvTag> {
    let path = dir.join(ENV_FILE);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return Ok(EnvTag::NoFile);
    };
    // What the deployment is actually running, by compose's rules: the last
    // active assignment, `export ` and quotes included. Reading the first one
    // would let this rewrite a line the deployment is not using and report
    // that the tag had moved. See [`crate::dotenv`].
    let Some(value) = crate::dotenv::value(&body, CHAP_TAG_ENV_VAR) else {
        return Ok(if mentions_var(&body, CHAP_TAG_ENV_VAR) {
            EnvTag::Commented
        } else {
            EnvTag::Absent
        });
    };
    if value != old && value != new {
        return Ok(EnvTag::Foreign(value));
    }
    // The same rules writing: one active line survives, holding the new tag.
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    crate::dotenv::set(&mut lines, CHAP_TAG_ENV_VAR, new);
    let out = crate::dotenv::join(&lines);
    if out != body {
        std::fs::write(&path, &out)
            .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    }
    Ok(EnvTag::Updated)
}

/// Whether any line, commented or not, assigns `var`.
fn mentions_var(body: &str, var: &str) -> bool {
    body.lines().any(|line| {
        let line = line.trim_start().trim_start_matches('#').trim_start();
        line.starts_with(&format!("{var}="))
    })
}

/// `# VAR=...`, with any amount of whitespace around the `#`.
fn is_commented_pin(line: &str, var: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix('#') else {
        return false;
    };
    rest.trim_start().starts_with(&format!("{var}="))
}

#[cfg(test)]
mod tests;
