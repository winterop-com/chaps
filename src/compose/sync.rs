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
    Components, OCS_COMPOSE, OCS_CONFIG_FILE, OCS_DATA_SOURCE_ENV_VARS, OCS_DIR, OCS_PLUGINS_KEY,
    OCS_PLUGINS_TARGET, OCS_READ_ONLY_KEY, OCS_TAG_ENV_VAR, S3_ACCESS_KEY_ENV_VAR, S3_COMPOSE,
    S3_SECRET_KEY_ENV_VAR, S3_TAG_ENV_VAR,
};
use crate::compose::overrides;
use crate::compose::render::{
    NO_TAG_PINS, render_base, render_chaps_overlay, render_ocs, render_ocs_config, render_overlay,
    render_s3, render_umbrella,
};
use crate::compose::spec::{
    BaseSpec, OcsConfigSpec, OcsSpec, OverlaySpec, S3Spec, UpstreamCompose,
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
            render_chaps_overlay(project.state.api_port, project_name.as_deref()),
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
    // Every overlay carries the registration key line when the deployment has
    // a key, because chap-core then requires it from each service that
    // registers. The value stays in `.env`, which compose reads from the
    // project directory, so nothing has to be threaded into the overlays.
    let registration_key = project.state.auth.registration_key;
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
        // The overlay's init container chowns the data volume from busybox,
        // which resolves no account name of its own, so the user has to be
        // expressible as numbers. An unknown one still renders, with the
        // chapkit ids, but the operator should know the guess was taken. A
        // model that runs as root has no init container to warn about.
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
    if components.ocs.enabled
        && !OCS_DATA_SOURCE_ENV_VARS
            .iter()
            .any(|var| mentions_var(body, var))
    {
        let mut section = String::from(
            "# OCS data sources (optional): ERA5-Land needs one of these; \
             WorldPop and CHIRPS3 need none.\n",
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
mod tests {
    use super::*;
    use crate::compose::apply::apply_with;
    use crate::compose::{EnableRequest, Selection};
    use crate::project::{ProjectState, default_compose_files};
    use crate::registry::load_embedded;
    use tempfile::TempDir;

    fn project_with(ids: &[&str]) -> (TempDir, Project, Registry) {
        let dir = tempfile::tempdir().unwrap();
        let registry = load_embedded().unwrap();
        let mut project = Project {
            dir: dir.path().to_path_buf(),
            state: ProjectState::default(),
        };
        let sel = Selection {
            enable: ids.iter().map(|id| EnableRequest::new(*id)).collect(),
            disable: Vec::new(),
        };
        apply_with(
            &mut project,
            &registry,
            &sel,
            &|_| false,
            &crate::compose::resolve::from_table,
        )
        .unwrap();
        (dir, project, registry)
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
    }

    /// Write a cached `compose.ghcr.yml` into `.chaps/` and record it as the
    /// project's compose source, the way `init` does after a fetch.
    fn cache(project: &mut Project, tag: &str, body: &str) {
        let chaps = project.chaps_dir();
        std::fs::create_dir_all(&chaps).unwrap();
        std::fs::write(chaps.join(crate::project::cached_compose_file(tag)), body).unwrap();
        project.state.chap_compose_source = ComposeSource::Fetched {
            url: crate::chapcore::compose_url(tag),
            tag: tag.to_string(),
            sha256: crate::chapcore::sha256_hex(body.as_bytes()),
        };
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_second_sync_changes_nothing() {
        let (_dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let report = sync(&mut project, &registry, false).unwrap();
        assert!(!report.drift, "{report:?}");
        assert!(report.written.is_empty() && report.removed.is_empty());
        assert_eq!(
            names(&report.unchanged),
            vec![
                "compose.yml",
                "compose.chaps.yml",
                "compose.chapkit-ewars-model.yml",
                "compose.marketplace.yml"
            ]
        );
        assert_eq!(
            project.state.rendered_files,
            vec![
                "compose.yml",
                "compose.chaps.yml",
                "compose.chapkit-ewars-model.yml",
                "compose.marketplace.yml"
            ]
        );
        assert_eq!(report.summary(), "0 written, 4 unchanged, 0 removed");
    }

    #[test]
    fn sync_renders_the_chaps_overlay_and_puts_it_in_the_f_list() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let overlay = dir.path().join(CHAPS_COMPOSE);
        assert!(overlay.is_file());
        assert!(read(&overlay).contains("${CHAP_API_PORT:-8000}:8000"));
        assert_eq!(
            project.state.compose_files,
            vec![
                "compose.yml",
                "compose.chaps.yml",
                "compose.marketplace.yml"
            ],
            "the override needs its own -f entry, between the base and the umbrella"
        );

        // The API port lives in .chaps/project.yaml, so moving it is drift.
        project.state.api_port = 8123;
        let report = sync(&mut project, &registry, true).unwrap();
        assert!(report.drift);
        assert_eq!(names(&report.written), vec![CHAPS_COMPOSE]);
        assert!(
            read(&overlay).contains("8000}:8000"),
            "--check writes nothing"
        );

        let report = sync(&mut project, &registry, false).unwrap();
        assert_eq!(names(&report.written), vec![CHAPS_COMPOSE]);
        assert!(read(&overlay).contains("${CHAP_API_PORT:-8123}:8000"));
        assert!(!sync(&mut project, &registry, true).unwrap().drift);

        // It is never removed as if it were a model overlay.
        project.state.models.clear();
        sync(&mut project, &registry, false).unwrap();
        assert!(overlay.is_file());
    }

    #[test]
    fn a_project_from_before_the_chaps_overlay_is_migrated_by_one_sync() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        // What an older chaps wrote: no compose.chaps.yml anywhere.
        std::fs::remove_file(dir.path().join(CHAPS_COMPOSE)).unwrap();
        project.state.compose_files =
            vec![BASE_COMPOSE.to_string(), MARKETPLACE_COMPOSE.to_string()];
        project.state.rendered_files.retain(|f| f != CHAPS_COMPOSE);

        let report = sync(&mut project, &registry, true).unwrap();
        assert!(report.drift, "the new file is missing");
        assert_eq!(names(&report.written), vec![CHAPS_COMPOSE]);
        assert!(
            !dir.path().join(CHAPS_COMPOSE).exists(),
            "--check writes nothing"
        );

        let report = sync(&mut project, &registry, false).unwrap();
        assert_eq!(names(&report.written), vec![CHAPS_COMPOSE]);
        assert!(dir.path().join(CHAPS_COMPOSE).is_file());
        assert_eq!(project.state.compose_files, default_compose_files());
        assert!(
            project
                .state
                .rendered_files
                .contains(&CHAPS_COMPOSE.to_string())
        );
        assert!(!sync(&mut project, &registry, true).unwrap().drift);
    }

    #[test]
    fn check_reports_drift_without_touching_anything() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let overlay = dir.path().join("compose.chapkit-ewars-model.yml");
        std::fs::remove_file(&overlay).unwrap();

        let report = sync(&mut project, &registry, true).unwrap();
        assert!(report.drift);
        assert!(report.check);
        assert_eq!(
            names(&report.written),
            vec!["compose.chapkit-ewars-model.yml"]
        );
        assert!(!overlay.exists(), "--check writes nothing");
        assert_eq!(report.summary(), "1 to write, 3 unchanged, 0 to remove");

        let report = sync(&mut project, &registry, false).unwrap();
        assert!(report.drift);
        assert!(overlay.is_file(), "a real sync re-creates the overlay");
        let again = sync(&mut project, &registry, true).unwrap();
        assert!(!again.drift);
    }

    #[test]
    fn a_model_removed_from_the_state_loses_its_overlay_but_hand_written_files_stay() {
        let (dir, mut project, registry) =
            project_with(&["chapkit_ewars_model", "auto_arima_chapkit"]);
        let custom = dir.path().join("compose.custom.yml");
        std::fs::write(&custom, "services: {}\n").unwrap();

        // Simulate a hand edit of .chaps/models.yaml.
        project.state.models.remove("auto_arima_chapkit");
        let report = sync(&mut project, &registry, false).unwrap();
        assert_eq!(
            names(&report.removed),
            vec!["compose.auto-arima-chapkit.yml"]
        );
        assert!(!dir.path().join("compose.auto-arima-chapkit.yml").exists());
        assert!(custom.is_file(), "hand-written overlays are never removed");
        assert_eq!(names(&report.written), vec!["compose.marketplace.yml"]);

        // A listed name that is not an overlay shape is never removed either.
        std::fs::write(dir.path().join("notes.yml"), "x: 1\n").unwrap();
        project.state.rendered_files.push("notes.yml".into());
        let report = sync(&mut project, &registry, false).unwrap();
        assert!(report.removed.is_empty(), "{report:?}");
        assert!(dir.path().join("notes.yml").is_file());
    }

    #[test]
    fn a_model_missing_from_the_registry_still_renders_with_a_warning() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let mut entry = project.state.models["chapkit_ewars_model"].clone();
        entry.service_id = "gone-model".into();
        entry.compose_file = "compose.gone-model.yml".into();
        project.state.models.insert("gone_model".into(), entry);

        let report = sync(&mut project, &registry, false).unwrap();
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("gone_model"));
        let body = std::fs::read_to_string(dir.path().join("compose.gone-model.yml")).unwrap();
        assert!(body.contains("  gone-model:\n"));
        // The header names the id, and the image where the repository would be.
        assert!(
            body.contains(&format!(
                "# gone_model {} (ghcr.io/chap-models/chapkit_ewars_model)",
                project.state.models["gone_model"].version
            )),
            "{body}"
        );
        assert!(!sync(&mut project, &registry, true).unwrap().drift);
    }

    #[test]
    fn every_overlay_gets_the_registration_line_when_the_project_has_a_key() {
        let (dir, mut project, registry) =
            project_with(&["chapkit_ewars_model", "auto_arima_chapkit"]);
        let overlays = [
            dir.path().join("compose.chapkit-ewars-model.yml"),
            dir.path().join("compose.auto-arima-chapkit.yml"),
        ];
        // Off by default: the line is there, commented, as chap-core's own
        // example overlay ships it.
        for path in &overlays {
            let body = read(path);
            assert!(
                body.contains("      # SERVICEKIT_REGISTRATION_KEY:"),
                "{body}"
            );
        }

        // Turning it on in `.chaps/project.yaml` is drift in every overlay.
        project.state.auth.registration_key = true;
        let report = sync(&mut project, &registry, true).unwrap();
        assert!(report.drift);
        assert_eq!(
            names(&report.written),
            vec![
                "compose.auto-arima-chapkit.yml",
                "compose.chapkit-ewars-model.yml"
            ]
        );

        sync(&mut project, &registry, false).unwrap();
        for path in &overlays {
            let body = read(path);
            assert!(
                body.contains(
                    "      SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}"
                ),
                "{body}"
            );
            assert!(!body.contains("# SERVICEKIT_REGISTRATION_KEY:"), "{body}");
        }
        assert!(!sync(&mut project, &registry, true).unwrap().drift);

        // And off again returns every overlay to the commented form.
        project.state.auth.registration_key = false;
        sync(&mut project, &registry, false).unwrap();
        for path in &overlays {
            assert!(read(path).contains("      # SERVICEKIT_REGISTRATION_KEY:"));
        }
    }

    #[test]
    fn a_user_with_no_known_ids_renders_with_a_warning() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let report = sync(&mut project, &registry, false).unwrap();
        assert!(report.warnings.is_empty(), "{report:?}");

        project
            .state
            .models
            .get_mut("chapkit_ewars_model")
            .unwrap()
            .user = "nobody".into();
        let report = sync(&mut project, &registry, false).unwrap();
        assert_eq!(report.warnings.len(), 1, "{report:?}");
        assert!(report.warnings[0].contains("chapkit_ewars_model runs as `nobody`"));
        assert!(report.warnings[0].contains("1000:1000"));

        // The overlay still renders, with the fallback ids in the chown.
        let body =
            std::fs::read_to_string(dir.path().join("compose.chapkit-ewars-model.yml")).unwrap();
        assert!(body.contains("chown -R 1000:1000 /app/data"), "{body}");
        assert!(body.contains("    user: nobody\n"));

        // A numeric user is understood and warns about nothing.
        project
            .state
            .models
            .get_mut("chapkit_ewars_model")
            .unwrap()
            .user = "1000:1000".into();
        let report = sync(&mut project, &registry, false).unwrap();
        assert!(report.warnings.is_empty(), "{report:?}");
    }

    /// A project with the given components, synced once.
    fn project_with_components(components: Components) -> (TempDir, Project, Registry) {
        let (dir, mut project, registry) = project_with(&[]);
        project.state.components = components;
        sync(&mut project, &registry, false).unwrap();
        (dir, project, registry)
    }

    #[test]
    fn a_component_is_rendered_listed_and_removed_again() {
        let mut components = Components::default();
        components.ocs.enabled = true;
        let (dir, mut project, registry) = project_with_components(components);

        let ocs = dir.path().join(OCS_COMPOSE);
        assert!(ocs.is_file());
        assert_eq!(
            project.state.compose_files,
            vec![
                "compose.yml",
                "compose.chaps.yml",
                "compose.ocs.yml",
                "compose.marketplace.yml"
            ],
            "a component file sits between the override and the umbrella"
        );
        assert!(
            project
                .state
                .rendered_files
                .contains(&OCS_COMPOSE.to_string())
        );
        // The scaffold went in too, and a second sync leaves everything alone.
        assert!(project.ocs_config_path().is_file());
        assert!(!sync(&mut project, &registry, true).unwrap().drift);

        // Turning it off removes the file and takes it out of both lists.
        project.state.components.ocs.enabled = false;
        let report = sync(&mut project, &registry, false).unwrap();
        assert_eq!(names(&report.removed), vec![OCS_COMPOSE]);
        assert!(!ocs.exists());
        assert_eq!(project.state.compose_files, default_compose_files());
        assert!(
            !project
                .state
                .rendered_files
                .contains(&OCS_COMPOSE.to_string())
        );
        // The operator's own config file is not ours to delete.
        assert!(project.ocs_config_path().is_file());
        assert!(!sync(&mut project, &registry, true).unwrap().drift);
    }

    #[test]
    fn the_object_store_reaches_the_ocs_file_as_well() {
        let mut components = Components::default();
        components.ocs.enabled = true;
        components.s3.enabled = true;
        let (dir, mut project, registry) = project_with_components(components);

        assert!(dir.path().join(S3_COMPOSE).is_file());
        assert!(read(&dir.path().join(OCS_COMPOSE)).contains("S3_ENDPOINT: http://s3:9000"));
        assert_eq!(
            project.state.compose_files,
            vec![
                "compose.yml",
                "compose.chaps.yml",
                "compose.ocs.yml",
                "compose.s3.yml",
                "compose.marketplace.yml"
            ]
        );

        // Taking the store away rewrites the OCS file without those lines.
        project.state.components.s3.enabled = false;
        let report = sync(&mut project, &registry, false).unwrap();
        assert_eq!(names(&report.removed), vec![S3_COMPOSE]);
        assert!(names(&report.written).contains(&OCS_COMPOSE.to_string()));
        assert!(!read(&dir.path().join(OCS_COMPOSE)).contains("S3_ENDPOINT"));
    }

    #[test]
    fn chap_core_off_leaves_only_the_component_files() {
        let mut components = Components::default();
        components.ocs.enabled = true;
        let (dir, mut project, registry) = project_with_components(components);
        assert!(dir.path().join(BASE_COMPOSE).is_file());

        project.state.components.chap_core.enabled = false;
        let report = sync(&mut project, &registry, false).unwrap();
        let removed = names(&report.removed);
        assert!(removed.contains(&BASE_COMPOSE.to_string()), "{removed:?}");
        assert!(removed.contains(&CHAPS_COMPOSE.to_string()), "{removed:?}");
        assert!(!dir.path().join(BASE_COMPOSE).exists());
        assert!(!dir.path().join(CHAPS_COMPOSE).exists());
        assert_eq!(
            project.state.compose_files,
            vec!["compose.ocs.yml", "compose.marketplace.yml"]
        );
        assert!(!sync(&mut project, &registry, true).unwrap().drift);
    }

    #[test]
    fn the_component_env_lines_are_appended_once() {
        let (dir, mut project, registry) = project_with(&[]);
        let env = dir.path().join(ENV_FILE);
        std::fs::write(
            &env,
            "POSTGRES_PASSWORD=secret
",
        )
        .unwrap();

        project.state.components.ocs.enabled = true;
        project.state.components.s3.enabled = true;
        let report = sync(&mut project, &registry, true).unwrap();
        assert!(report.drift, "the missing lines are drift");
        assert_eq!(
            std::fs::read_to_string(&env).unwrap(),
            "POSTGRES_PASSWORD=secret\n",
            "--check writes nothing"
        );

        sync(&mut project, &registry, false).unwrap();
        let body = std::fs::read_to_string(&env).unwrap();
        assert!(body.starts_with("POSTGRES_PASSWORD=secret\n"));
        assert!(body.contains("\n# OCS_IMAGE_TAG=main\n"), "{body}");
        assert!(body.contains("\n# S3_IMAGE_TAG=latest\n"), "{body}");
        let access = crate::auth::active_value(&body, S3_ACCESS_KEY_ENV_VAR).expect("a key");
        let secret = crate::auth::active_value(&body, S3_SECRET_KEY_ENV_VAR).expect("a secret");
        assert_eq!(access.len(), 32);
        assert_eq!(secret.len(), 32);
        assert_ne!(access, secret);

        // A second sync adds nothing, and never rewrites the credentials: the
        // volume was created with them.
        assert!(!sync(&mut project, &registry, true).unwrap().drift);
        sync(&mut project, &registry, false).unwrap();
        let again = std::fs::read_to_string(&env).unwrap();
        assert_eq!(again, body);
        assert_eq!(again.matches("OCS_IMAGE_TAG").count(), 1);
    }

    /// The data source section is placeholders, appended once, and never
    /// rewritten: an operator who pasted a Copernicus key into it must not
    /// find it commented out again by the next sync.
    #[test]
    fn the_ocs_data_source_placeholders_are_appended_once_and_never_rewritten() {
        let (dir, mut project, registry) = project_with(&[]);
        let env = dir.path().join(ENV_FILE);
        std::fs::write(&env, "POSTGRES_PASSWORD=secret\n").unwrap();

        project.state.components.ocs.enabled = true;
        sync(&mut project, &registry, false).unwrap();
        let body = std::fs::read_to_string(&env).unwrap();
        assert!(
            body.contains(
                "\n# OCS data sources (optional): ERA5-Land needs one of these; \
                 WorldPop and CHIRPS3 need none.\n"
            ),
            "{body}"
        );
        assert!(
            body.contains("# ECMWF_DATASTORES_URL=https://cds.climate.copernicus.eu/api\n"),
            "the endpoint is the same for everyone, so it is pre-filled: {body}"
        );
        for var in ["ECMWF_DATASTORES_KEY", "EDH_API_KEY", "CDSE_S3_SECRET_KEY"] {
            assert!(body.contains(&format!("# {var}=\n")), "{var}: {body}");
        }
        // Commented, so nothing is set: the compose file's `${VAR:-}` then
        // passes an empty value, which OCS reads as absent.
        for var in OCS_DATA_SOURCE_ENV_VARS {
            assert_eq!(crate::dotenv::non_empty(&body, var), None, "{var}");
        }

        // A second sync adds nothing.
        assert!(!sync(&mut project, &registry, true).unwrap().drift);

        // And a filled-in value keeps the section from being appended again.
        let filled = body.replace("# ECMWF_DATASTORES_KEY=", "ECMWF_DATASTORES_KEY=mine");
        std::fs::write(&env, &filled).unwrap();
        sync(&mut project, &registry, false).unwrap();
        assert_eq!(std::fs::read_to_string(&env).unwrap(), filled);
        assert_eq!(filled.matches("OCS data sources").count(), 1);
    }

    /// The mount and the config key arrive together: a plugin directory that
    /// is mounted and not configured is a mount OCS never looks in.
    #[test]
    fn a_plugin_directory_adds_the_mount_and_the_config_key() {
        let mut components = Components::default();
        components.ocs.enabled = true;
        let (dir, mut project, registry) = project_with_components(components);
        let compose = dir.path().join(OCS_COMPOSE);
        assert!(!read(&compose).contains("/app/plugins"), "nothing to mount");

        std::fs::create_dir_all(project.ocs_plugins_path().join("datasets")).unwrap();
        let report = sync(&mut project, &registry, true).unwrap();
        assert!(report.drift, "the mount and the key are both missing");
        assert!(
            !read(&project.ocs_config_path()).contains(OCS_PLUGINS_KEY),
            "--check writes nothing"
        );

        sync(&mut project, &registry, false).unwrap();
        assert!(
            read(&compose).contains("- ./ocs/plugins:/app/plugins:ro"),
            "{}",
            read(&compose)
        );
        let config = read(&project.ocs_config_path());
        assert!(config.ends_with("plugins_dir: /app/plugins\n"), "{config}");
        assert!(
            config.contains("sierra-leone-climate-service"),
            "the rest of the operator's file is untouched: {config}"
        );

        // Idempotent, and an operator's own value is never moved.
        assert!(!sync(&mut project, &registry, true).unwrap().drift);
        std::fs::write(
            project.ocs_config_path(),
            "id: mine\nplugins_dir: /somewhere/else\n",
        )
        .unwrap();
        sync(&mut project, &registry, false).unwrap();
        assert_eq!(
            read(&project.ocs_config_path()),
            "id: mine\nplugins_dir: /somewhere/else\n"
        );
    }

    /// Only the one key changes, in a file that has it and in one that does
    /// not: everything else in it is the operator's.
    #[test]
    fn the_read_only_switch_edits_one_key_and_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            set_read_only(dir.path(), true).unwrap(),
            None,
            "no file to edit yet"
        );

        let path = dir.path().join(OCS_DIR).join(OCS_CONFIG_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = "# my instance\nid: mine\n\nextent:\n  read_only: true\n  name: Mine\n\n\
                        # a note about ingestion\n# read_only: true\n";
        std::fs::write(&path, original).unwrap();

        // Appended: neither the nested key nor the commented one is this key.
        assert_eq!(
            set_read_only(dir.path(), true).unwrap(),
            Some(KeyEdit::Appended)
        );
        let body = read(&path);
        assert!(body.starts_with(original), "{body}");
        assert!(body.ends_with("\nread_only: true\n"), "{body}");
        assert!(
            body.contains("  read_only: true\n  name: Mine\n"),
            "the nested key is someone else's: {body}"
        );

        // Rewritten in place, and nothing else moves.
        assert_eq!(
            set_read_only(dir.path(), false).unwrap(),
            Some(KeyEdit::Rewritten)
        );
        let flipped = read(&path);
        assert_eq!(
            flipped,
            body.replace("\nread_only: true\n", "\nread_only: false\n")
        );

        // Already that value: no write at all.
        assert_eq!(
            set_read_only(dir.path(), false).unwrap(),
            Some(KeyEdit::Unchanged)
        );
        assert_eq!(read(&path), flipped);
    }

    /// A trailing comment says why the value is what it is, so it survives.
    #[test]
    fn setting_a_key_keeps_its_trailing_comment_and_finds_the_first_one() {
        let (out, edit) = set_config_key("read_only: false  # public demo\n", "read_only", "true");
        assert_eq!(edit, KeyEdit::Rewritten);
        assert_eq!(out, "read_only: true  # public demo\n");

        // Two active assignments is not valid YAML, but the first is the one
        // a parser would report, so it is the one that is edited.
        let (out, _) = set_config_key("read_only: false\nread_only: false\n", "read_only", "true");
        assert_eq!(out, "read_only: true\nread_only: false\n");

        // A key that is a prefix of another is not that other key.
        assert!(!has_config_key("read_only_mode: true\n", "read_only"));
        assert!(has_config_key("read_only: true\n", "read_only"));
        assert!(!has_config_key("  read_only: true\n", "read_only"));
        assert!(!has_config_key("# read_only: true\n", "read_only"));
    }

    #[test]
    fn the_scaffold_is_created_when_missing_and_never_overwritten() {
        let (dir, mut project, registry) = project_with(&[]);
        let path = project.ocs_config_path();

        // Someone else's file is kept byte for byte.
        std::fs::create_dir_all(dir.path().join(OCS_DIR)).unwrap();
        std::fs::write(&path, "id: mine\n").unwrap();
        assert!(
            write_ocs_config(dir.path(), &OcsConfigSpec::default())
                .unwrap()
                .is_none()
        );
        assert_eq!(read(&path), "id: mine\n");

        project.state.components.ocs.enabled = true;
        sync(&mut project, &registry, false).unwrap();
        assert_eq!(read(&path), "id: mine\n", "sync leaves it alone too");

        // A component enabled with no file at all gets the example.
        std::fs::remove_file(&path).unwrap();
        let report = sync(&mut project, &registry, true).unwrap();
        assert!(report.drift);
        assert!(!path.exists(), "--check writes nothing");
        sync(&mut project, &registry, false).unwrap();
        assert!(read(&path).contains("sierra-leone-climate-service"));
    }

    #[test]
    fn refresh_env_pin_moves_only_the_commented_line() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(ENV_FILE);
        std::fs::write(
            &env,
            "POSTGRES_PASSWORD=secret\n\
             # CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-1111111\n\
             AUTO_ARIMA_CHAPKIT_IMAGE_TAG=sha-2222222\n",
        )
        .unwrap();

        assert!(
            refresh_env_pin(dir.path(), "CHAPKIT_EWARS_MODEL_IMAGE_TAG", "sha-3333333").unwrap()
        );
        let body = std::fs::read_to_string(&env).unwrap();
        assert_eq!(
            body,
            "POSTGRES_PASSWORD=secret\n\
             # CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-3333333\n\
             AUTO_ARIMA_CHAPKIT_IMAGE_TAG=sha-2222222\n"
        );

        // Already there: nothing to do. Active line: the operator's, untouched.
        assert!(
            !refresh_env_pin(dir.path(), "CHAPKIT_EWARS_MODEL_IMAGE_TAG", "sha-3333333").unwrap()
        );
        assert!(
            !refresh_env_pin(dir.path(), "AUTO_ARIMA_CHAPKIT_IMAGE_TAG", "sha-4444444").unwrap()
        );
        assert!(!refresh_env_pin(dir.path(), "NOPE_IMAGE_TAG", "sha-4444444").unwrap());
        assert_eq!(std::fs::read_to_string(&env).unwrap(), body);
        assert!(!refresh_env_pin(&dir.path().join("missing"), "X", "y").unwrap());
    }

    #[test]
    fn check_counts_a_missing_env_pin_as_drift() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let env = dir.path().join(ENV_FILE);
        std::fs::write(&env, "POSTGRES_PASSWORD=secret\n").unwrap();
        let report = sync(&mut project, &registry, true).unwrap();
        assert!(report.drift);
        assert_eq!(names(&report.written), vec![".env"]);
        assert_eq!(
            std::fs::read_to_string(&env).unwrap(),
            "POSTGRES_PASSWORD=secret\n"
        );

        sync(&mut project, &registry, false).unwrap();
        let pinned = project.state.models["chapkit_ewars_model"]
            .image_tag
            .clone();
        assert!(
            std::fs::read_to_string(&env)
                .unwrap()
                .contains(&format!("\n# CHAPKIT_EWARS_MODEL_IMAGE_TAG={pinned}\n"))
        );
        assert!(!sync(&mut project, &registry, true).unwrap().drift);
    }

    #[test]
    fn the_base_file_is_rendered_from_the_embedded_copy_by_default() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let base = dir.path().join(BASE_COMPOSE);
        let original = read(&base);
        assert!(
            original.starts_with(
                "# Generated by chaps from .chaps/; edit there and run `chaps sync`.\nservices:\n"
            ),
            "{original}"
        );

        // A hand edit of compose.yml is drift, and a real sync restores it.
        std::fs::write(&base, "services: {}\n").unwrap();
        let report = sync(&mut project, &registry, true).unwrap();
        assert!(report.drift);
        assert_eq!(names(&report.written), vec!["compose.yml"]);
        assert_eq!(read(&base), "services: {}\n", "--check writes nothing");

        let report = sync(&mut project, &registry, false).unwrap();
        assert_eq!(names(&report.written), vec!["compose.yml"]);
        assert_eq!(read(&base), original);
        assert!(!sync(&mut project, &registry, true).unwrap().drift);
    }

    #[test]
    fn a_fetched_source_renders_from_the_cached_copy() {
        let (dir, mut project, registry) = project_with(&["chapkit_ewars_model"]);
        let body = "services:\n  chap:\n    image: ghcr.io/x:${CHAP_IMAGE_TAG:-latest}\n";
        cache(&mut project, "v2.3.1", body);

        let report = sync(&mut project, &registry, false).unwrap();
        assert!(report.warnings.is_empty(), "{report:?}");
        assert_eq!(names(&report.written), vec!["compose.yml"]);
        let base = read(&dir.path().join(BASE_COMPOSE));
        assert_eq!(
            base,
            format!(
                "# Generated by chaps from .chaps/; edit there and run `chaps sync`.\n\
                 # chap-core compose.ghcr.yml at v2.3.1\n\
                 {body}"
            )
        );
        assert!(!sync(&mut project, &registry, true).unwrap().drift);
    }

    #[test]
    fn an_edited_cached_copy_is_followed_but_reported() {
        let (dir, mut project, registry) = project_with(&[]);
        cache(
            &mut project,
            "v2.3.1",
            "services:\n  chap:\n    image: x:${CHAP_IMAGE_TAG:-latest}\n",
        );
        sync(&mut project, &registry, false).unwrap();

        // The recorded checksum no longer matches, but the file on disk is
        // what the operator has: follow it, and say so.
        let cached = project.cached_compose_path().unwrap();
        std::fs::write(
            &cached,
            "services:\n  chap:\n    image: y:${CHAP_IMAGE_TAG:-latest}\n",
        )
        .unwrap();
        let report = sync(&mut project, &registry, false).unwrap();
        assert_eq!(report.warnings.len(), 1, "{report:?}");
        assert!(report.warnings[0].contains("compose.chap-core.v2.3.1.yml"));
        assert!(report.warnings[0].contains("checksum"));
        assert!(read(&dir.path().join(BASE_COMPOSE)).contains("image: y:"));

        // And a cached copy that is gone leaves compose.yml alone.
        let before = read(&dir.path().join(BASE_COMPOSE));
        std::fs::remove_file(&cached).unwrap();
        let report = sync(&mut project, &registry, false).unwrap();
        assert_eq!(report.warnings.len(), 1, "{report:?}");
        assert!(report.warnings[0].contains("is missing"));
        assert!(!names(&report.written).contains(&"compose.yml".to_string()));
        assert!(!names(&report.unchanged).contains(&"compose.yml".to_string()));
        assert_eq!(read(&dir.path().join(BASE_COMPOSE)), before);
        assert!(
            !project
                .state
                .rendered_files
                .contains(&BASE_COMPOSE.to_string())
        );
    }

    #[test]
    fn set_env_chap_tag_moves_the_active_line_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(ENV_FILE);
        std::fs::write(
            &env,
            "POSTGRES_PASSWORD=secret\n\
             CHAP_IMAGE_TAG=v2.3.0\n\
             # CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-1111111\n",
        )
        .unwrap();

        assert_eq!(
            set_env_chap_tag(dir.path(), "v2.3.0", "v2.3.1").unwrap(),
            EnvTag::Updated
        );
        assert_eq!(
            read(&env),
            "POSTGRES_PASSWORD=secret\n\
             CHAP_IMAGE_TAG=v2.3.1\n\
             # CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-1111111\n"
        );
        // Running it again is a no-op, not a second rewrite.
        assert_eq!(
            set_env_chap_tag(dir.path(), "v2.3.0", "v2.3.1").unwrap(),
            EnvTag::Updated
        );
        assert!(read(&env).contains("CHAP_IMAGE_TAG=v2.3.1\n"));
    }

    /// The tag the deployment runs is the one on the last active line, so that
    /// is the one `chaps update` compares against and the one it moves - and
    /// the duplicate that made the answer ambiguous does not survive the
    /// write.
    #[test]
    fn set_env_chap_tag_follows_the_line_compose_reads() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(ENV_FILE);

        // Two active lines: compose runs the second, so the first is not what
        // "the recorded tag" means.
        std::fs::write(
            &env,
            "CHAP_IMAGE_TAG=v1.0.0\nPOSTGRES_DB=chap_core\nCHAP_IMAGE_TAG=v2.3.0\n",
        )
        .unwrap();
        assert_eq!(
            set_env_chap_tag(dir.path(), "v2.3.0", "v2.3.1").unwrap(),
            EnvTag::Updated
        );
        assert_eq!(read(&env), "CHAP_IMAGE_TAG=v2.3.1\nPOSTGRES_DB=chap_core\n");

        // And the other way round: the last line is the operator's own value,
        // whatever an earlier line says, so nothing is touched.
        std::fs::write(&env, "CHAP_IMAGE_TAG=v2.3.0\nCHAP_IMAGE_TAG=v1.0.0\n").unwrap();
        assert_eq!(
            set_env_chap_tag(dir.path(), "v2.3.0", "v2.3.1").unwrap(),
            EnvTag::Foreign("v1.0.0".to_string())
        );
        assert_eq!(read(&env), "CHAP_IMAGE_TAG=v2.3.0\nCHAP_IMAGE_TAG=v1.0.0\n");

        // The shapes compose accepts are the shapes this reads, and an
        // `export ` prefix is the operator's to keep.
        std::fs::write(&env, "export CHAP_IMAGE_TAG=\"v2.3.0\"\n").unwrap();
        assert_eq!(
            set_env_chap_tag(dir.path(), "v2.3.0", "v2.3.1").unwrap(),
            EnvTag::Updated
        );
        assert_eq!(read(&env), "export CHAP_IMAGE_TAG=v2.3.1\n");
    }

    #[test]
    fn set_env_chap_tag_keeps_its_hands_off_the_operators_choices() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join(ENV_FILE);

        // The generated placeholder: commented, so the deployment follows the
        // compose default and uncommenting it is the operator's call.
        std::fs::write(&env, "# CHAP_IMAGE_TAG=latest\n").unwrap();
        assert_eq!(
            set_env_chap_tag(dir.path(), "latest", "v2.3.1").unwrap(),
            EnvTag::Commented
        );
        assert_eq!(read(&env), "# CHAP_IMAGE_TAG=latest\n");

        // An active line with someone else's value.
        std::fs::write(&env, "CHAP_IMAGE_TAG=v1.0.0\n").unwrap();
        assert_eq!(
            set_env_chap_tag(dir.path(), "v2.3.0", "v2.3.1").unwrap(),
            EnvTag::Foreign("v1.0.0".to_string())
        );
        assert_eq!(read(&env), "CHAP_IMAGE_TAG=v1.0.0\n");

        // Never mentioned, and no file at all.
        std::fs::write(&env, "POSTGRES_DB=chap_core\n").unwrap();
        assert_eq!(
            set_env_chap_tag(dir.path(), "v2.3.0", "v2.3.1").unwrap(),
            EnvTag::Absent
        );
        assert_eq!(read(&env), "POSTGRES_DB=chap_core\n");
        assert_eq!(
            set_env_chap_tag(&dir.path().join("elsewhere"), "a", "b").unwrap(),
            EnvTag::NoFile
        );
    }

    #[test]
    fn overlay_name_shape() {
        assert!(is_overlay_name("compose.chapkit-ewars-model.yml"));
        assert!(!is_overlay_name(MARKETPLACE_COMPOSE));
        assert!(!is_overlay_name(BASE_COMPOSE));
        assert!(!is_overlay_name("compose.yaml"));
        assert!(!is_overlay_name("notes.yml"));
    }
}
