//! `chaps init` — write a deployment directory.
//!
//! Owned by agent B.

use crate::auth;
use crate::chapcore;
use crate::cli::InitArgs;
use crate::commands::Ctx;
use crate::components::{COMPONENTS_FILE, Component, Components, S3_SOON_NOTE};
use crate::compose::spec::EnvSpec;
use crate::compose::sync::write_ocs_config;
use crate::compose::{API_SERVICE, ApplyReport, EnableRequest, Selection, apply, render_env};
use crate::error::{ChapError, Result};
use crate::output::{Out, PanelKind};
use crate::project::{
    API_PORT_ENV_VAR, AuthState, CHAPS_DIR, ComposeSource, DEFAULT_PORT_RANGE, ENV_FILE,
    MODELS_FILE, PROJECT_FILE, Project, ProjectState, cached_compose_file,
};
use crate::registry::{self, Registry};
use serde::Serialize;
use std::path::{Component as PathComponent, Path, PathBuf};

/// The model `--models default` enables.
pub const DEFAULT_MODEL: &str = "chapkit_ewars_model";
/// The `--chap-tag` value that means "whatever the newest release is".
pub const MOVING_DEFAULT_TAG: &str = "latest";
/// Database user and database name of the generated `.env`.
const POSTGRES_USER: &str = "chap";
const POSTGRES_DB: &str = "chap_core";

/// Create compose.yml, compose.marketplace.yml, the model overlays, .env and
/// the `.chaps/` directory in the target directory.
///
/// `args.source` is parsed but must be rejected with "local chap-core build
/// not yet supported"; the flag only reserves the seam.
pub fn run(ctx: &Ctx, args: &InitArgs) -> Result<()> {
    if args.source.is_some() {
        return Err(anyhow::anyhow!("local chap-core build not yet supported"));
    }
    // The positional DIR is relative to the working directory, not to -C:
    // `chaps init foo` is a fresh deployment, not an operation on a project.
    let dir = resolve_dir(&args.dir)?;
    if Project::exists(&dir) && !args.force {
        return Err(ChapError::AlreadyInitialized(dir).into());
    }
    // Nesting is allowed (it is the user's directory), but commands run from
    // in here will find this project, not the outer one, so say so.
    if let Some(parent) = dir.parent().and_then(Project::find_root) {
        crate::output::warn(&format!(
            "{} is inside the chaps project at {}; commands run below it will use the new project",
            dir.display(),
            parent.display()
        ));
    }

    let mut registry = registry::load(&ctx.registry)?;
    // What the deployment is made of is settled first: it decides which
    // compose files are rendered at all, and an unknown name in --with is a
    // typo to report before anything is written.
    let components = parse_components(args.with.as_deref(), args.without.as_deref())?;
    // Settle the chap-core tag and the compose file that goes with it before
    // anything is written: both need the network, and a failure of either is a
    // warning plus a fallback, never a half-written directory.
    let chap_core = resolve_chap_core(ctx, &dir, &args.chap_tag);
    let state = ProjectState {
        chap_image_tag: chap_core.tag.clone(),
        chap_compose_source: chap_core.source.clone(),
        compose_project: compose_project(&dir)?,
        registry_url: ctx.registry.url.clone(),
        api_port: args.api_port,
        port_range: (args.port_base, DEFAULT_PORT_RANGE.1.max(args.port_base)),
        components: components.clone(),
        // A re-init keeps the deployment's own model definitions: `--force`
        // rewrites the directory, and dropping them would leave `--models`
        // naming models nothing has heard of.
        manual: carried_manual(&dir),
        ..ProjectState::default()
    };
    // The ports the deployment publishes, and a busy one is only a problem at
    // `chaps up`: the process holding one may well be a previous stack this
    // deployment is meant to replace. So: a warning with a way out, not a
    // refusal to write the directory.
    let busy_ports = warn_about_busy_ports(&components, args.api_port, &crate::ports::is_busy);
    let api_port_busy = busy_ports.iter().any(|c| c.service == API_SERVICE);
    let mut project = Project {
        dir: dir.clone(),
        state,
    };
    // The carried-over definitions are part of the catalogue from here on,
    // so `--models` and the browser see them like any marketplace entry.
    for warning in registry.with_manual(&project.state.manual) {
        crate::output::warn(&warning);
    }

    // Decide what to enable before writing anything, so a cancelled browser
    // or a typo in --models leaves no half-written directory behind.
    let selection = if args.interactive {
        // Quitting the browser without saving is "no models", not an error.
        crate::tui::run_tui(ctx, &project, &registry)?.unwrap_or_default()
    } else if !components.chap_core.enabled && args.models == "default" {
        // The default model set is a default, not a request: a deployment
        // without chap-core has nowhere to register one, so it starts empty
        // rather than making the operator also type `--models none`.
        Selection::default()
    } else {
        parse_models(&args.models, &registry)?
    };
    // A model service registers with chap-core and is reached through it, so
    // the two cannot be asked for separately.
    if !components.chap_core.enabled && !selection.enable.is_empty() {
        let ids: Vec<String> = selection.enable.iter().map(|r| r.id.clone()).collect();
        return Err(anyhow::anyhow!(
            "--without chap-core leaves nowhere for {} to register: model services \
             register with chap-core and are reached through it. Pass `--models none`, \
             or keep chap-core",
            ids.join(", ")
        ));
    }

    // The whole selection is checked against the registry here, before a file
    // is deleted or written: a template id, a model the marketplace does not
    // list or a version that was yanked must leave an existing deployment
    // exactly as it was, rather than taking its overlays with it on the way
    // out. apply() checks again; this is the run that has something to lose.
    crate::compose::apply::validate(&project, &registry, &selection)?;

    // --force starts the state over, so overlays the previous project owned
    // would otherwise linger: unreferenced by the umbrella, but still holding
    // their host port against the allocator.
    let stale = remove_stale_overlays(&dir, &selection);

    // `.env` is operator-owned state - the database password, the API token,
    // the registration key, the image pins - so `init` decides its fate before
    // it writes anything. Rewriting it under a postgres volume that still
    // holds the old role password leaves chap-core looping on a failed
    // authentication, which is why even `--force` keeps the file.
    let env_path = dir.join(ENV_FILE);
    let env_exists = env_path.is_file();
    let env = env_action(env_exists, args.fresh_env, args.no_env);
    // Compose reads `.env` after the compose files, so a CHAP_API_PORT line in
    // a file this run is keeping wins over `--api-port`. Say so rather than
    // leaving the API on a port nothing in `.chaps/` mentions.
    if env == EnvAction::Kept
        && let Ok(body) = std::fs::read_to_string(&env_path)
        && let Some(pinned) = env_api_port(&body).filter(|p| *p != args.api_port)
    {
        crate::output::warn(&format!(
            ".env already sets {API_PORT_ENV_VAR}={pinned}, and compose reads that after the \
             compose files, so the API stays on {pinned} rather than {}; edit that line to \
             move it",
            args.api_port
        ));
    }

    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", dir.display()))?;
    let mut written = Vec::new();

    // The raw copy of chap-core's compose.ghcr.yml goes in first: `sync`
    // renders compose.yml from it a moment later, and every later sync
    // re-renders from this copy rather than from the network.
    if let Some(body) = &chap_core.cached {
        let chaps = dir.join(CHAPS_DIR);
        std::fs::create_dir_all(&chaps)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", chaps.display()))?;
        let name = chap_core
            .source
            .cached_file()
            .expect("a downloaded copy has a file name");
        let path = chaps.join(name);
        std::fs::write(&path, body)
            .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
        written.push(path);
    }

    // `--api-token` only means something for a `.env` this run writes: the
    // secrets have to be in the file compose reads, and a kept file is the
    // operator's.
    let secrets = match env {
        EnvAction::Written => resolve_secrets(args.api_token.as_ref())?,
        _ => {
            if args.api_token.is_some() {
                crate::output::warn(&format!(
                    "--api-token needs a .env to write to, and this run {}; \
                     run `chaps auth enable` in the project instead",
                    match env {
                        EnvAction::Kept => "is keeping the one already there",
                        _ => "writes none (--no-env)",
                    }
                ));
            }
            None
        }
    };

    if env == EnvAction::Written {
        if env_exists {
            crate::output::warn(
                "--fresh-env rewrote .env with a new POSTGRES_PASSWORD; a database volume \
                 from an earlier `chaps up` still holds the old one - drop it with \
                 `chaps down --volumes` or change the role with ALTER USER",
            );
        }
        std::fs::write(
            &env_path,
            render_env(&EnvSpec {
                postgres_user: POSTGRES_USER.to_string(),
                postgres_password: random_password()?,
                postgres_db: POSTGRES_DB.to_string(),
                chap_image_tag: Some(chap_core.tag.clone()),
                api_port: args.api_port,
                api_token: secrets.as_ref().map(|s| s.api_token.clone()),
                registration_key: secrets.as_ref().map(|s| s.registration_key.clone()),
                // apply() appends one commented pin per enabled model.
                model_tag_pins: Vec::new(),
                cli_version: ctx.cli_version.to_string(),
            }),
        )
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", env_path.display()))?;
        written.push(env_path.clone());
    }

    // `.env` decides: a kept file that already carries the secrets keeps the
    // deployment authenticated, and a rendered one says so itself. The state
    // file only records which of the two are in use, never their values, and
    // `sync` reads it to decide whether each overlay carries the registration
    // key - so it has to be settled before apply().
    project.state.auth = env_auth(env, &env_path);

    // The OCS instance config goes in before the sync inside apply(), so the
    // `--ocs-*` values land in it rather than the example ones sync falls
    // back to. Like `.env`, an existing file is kept: it is the operator's.
    if components.ocs.enabled
        && let Some(path) = write_ocs_config(
            &dir,
            &crate::commands::components::request(&args.ocs).into_spec(),
        )?
    {
        written.push(path);
    }

    // apply() writes the overlays, compose.marketplace.yml (even with no
    // models) and .chaps/.
    let mut report = apply(&mut project, &registry, &selection, ctx.cli_version)?;
    report.removed.extend(stale);
    written.extend(report.written.iter().cloned());
    written.push(dir.join(CHAPS_DIR).join(PROJECT_FILE));
    written.push(dir.join(CHAPS_DIR).join(MODELS_FILE));
    written.push(dir.join(CHAPS_DIR).join(COMPONENTS_FILE));
    // apply() reports .env again when it appends a pin to the file init just
    // wrote; the summary lists each file once.
    let mut seen = std::collections::BTreeSet::new();
    written.retain(|path| seen.insert(path.clone()));
    // apply() may have appended a pin comment to a `.env` this run did not
    // write; a kept file is reported as kept, not as one of init's writes.
    if env != EnvAction::Written {
        written.retain(|path| path != &env_path);
    }

    let value = serde_json::json!({
        "dir": dir,
        "written": written,
        "env": env,
        "chap_image_tag": chap_core.tag,
        "chap_compose_source": chap_core.source,
        "api_port": args.api_port,
        "api_url": project.api_url(),
        "api_port_busy": api_port_busy,
        "busy_ports": busy_ports.iter().map(|c| c.port).collect::<Vec<u16>>(),
        "components": project.state.components,
        // Whether the deployment is protected, never the secrets themselves:
        // `--json` output is the kind of thing that ends up in a log.
        "auth": project.state.auth,
        "report": report,
    });
    ctx.out.emit(&value, || {
        summary(
            &ctx.out,
            &dir,
            &written,
            &report,
            &registry,
            env,
            &chap_core,
            &project,
            secrets.as_ref(),
        )
    })
}

/// The model definitions a re-init carries over, which is all of them: they
/// are the deployment's own, and nothing else records them.
fn carried_manual(dir: &Path) -> crate::project::ManualModels {
    Project::load(dir)
        .map(|existing| existing.state.manual)
        .unwrap_or_default()
}

/// The compose project name this deployment gets: the prefix every container
/// and every named volume of it carries.
///
/// A fresh directory gets `<slug of the directory name>-<6 hex>`, because the
/// directory name on its own is not unique: two deployments in directories
/// both called `demo` would share `demo_chap-db` and every other volume, and a
/// fresh `chaps up` would inherit a database created with a password it has
/// never seen.
///
/// A directory that is already a deployment keeps the name it has, `--force`
/// included: renaming it would leave its containers and its data behind under
/// the old name. For one written before the name was recorded that is the
/// directory name compose has been deriving all along, which is what the next
/// `sync` would write down anyway.
fn compose_project(dir: &Path) -> Result<String> {
    if let Ok(existing) = Project::load(dir) {
        if let Some(name) = existing.compose_project() {
            return Ok(name.to_string());
        }
        if let Some(derived) = crate::project::derived_project_name(dir) {
            return Ok(derived);
        }
    }
    crate::project::new_compose_project_name(dir)
}

/// The port an active (uncommented) `CHAP_API_PORT=` line of a `.env` sets.
///
/// A commented placeholder is not a setting, and a value that is not a port
/// number is the operator's problem to see for themselves - neither yields a
/// warning here.
fn env_api_port(body: &str) -> Option<u16> {
    body.lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix(&format!("{API_PORT_ENV_VAR}=")))
        .find_map(|value| value.trim().parse::<u16>().ok())
}

/// Warn about every host port the new deployment publishes that something is
/// already listening on, and say what to do about each.
///
/// Returns the claims that were busy. Not an error: the process holding one may
/// be a stack this deployment is meant to replace, and the directory is worth
/// writing either way.
fn warn_about_busy_ports(
    components: &Components,
    api_port: u16,
    busy: &dyn Fn(u16) -> bool,
) -> Vec<crate::ports::PortClaim> {
    let mut claims = Vec::new();
    if components.chap_core.enabled {
        claims.push(crate::ports::PortClaim {
            service: API_SERVICE.to_string(),
            port: api_port,
        });
    }
    for (component, service) in [
        (Component::Ocs, crate::compose::OCS_SERVICE),
        (Component::S3, crate::compose::S3_SERVICE),
    ] {
        if let Some(port) = components.port_of(component) {
            claims.push(crate::ports::PortClaim {
                service: service.to_string(),
                port,
            });
        }
    }
    claims.retain(|claim| busy(claim.port));
    // Upwards from the requested port: the next free number is the one least
    // likely to collide with something else the operator has in mind.
    let suggestion = claims
        .first()
        .and_then(|claim| crate::ports::first_free(claim.port.saturating_add(1), u16::MAX, busy));
    for claim in &claims {
        crate::output::warn(&crate::ports::busy_line(claim, suggestion));
    }
    claims
}

/// Expand `--with` and `--without` into a component set.
///
/// chap-core is on unless `--without chap-core` says otherwise; everything
/// else is off until `--with` names it. A name in both lists is a
/// contradiction rather than a silent winner.
fn parse_components(with: Option<&str>, without: Option<&str>) -> Result<Components> {
    let on = parse_component_list(with)?;
    let off = parse_component_list(without)?;
    if let Some(both) = on.iter().find(|c| off.contains(c)) {
        return Err(anyhow::anyhow!(
            "`{}` is in both --with and --without; it cannot be on and off at once",
            both.name()
        ));
    }
    let mut components = Components::default();
    for component in on {
        components.set_enabled(component, true);
    }
    for component in off {
        components.set_enabled(component, false);
    }
    Ok(components)
}

/// A comma-separated list of component names.
fn parse_component_list(spec: Option<&str>) -> Result<Vec<Component>> {
    let Some(spec) = spec else {
        return Ok(Vec::new());
    };
    spec.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(Component::from_name)
        .collect()
}

/// The chap-core tag `init` settled on, and where `compose.yml` comes from.
#[derive(Debug, Clone)]
struct ChapCore {
    /// The tag written to `.env` and `.chaps/project.yaml`.
    tag: String,
    source: ComposeSource,
    /// The downloaded `compose.ghcr.yml`, when one has to be written into
    /// `.chaps/`; `None` when the copy is already there or is the embedded one.
    cached: Option<String>,
}

/// Decide the chap-core tag and the compose file that goes with it.
///
/// `latest` is a moving tag: two `init` runs a month apart would deploy
/// different code from the same state file, and `chaps update` would have
/// nothing to compare. So the default is resolved to the release it points at
/// right now, and the compose file that release publishes is downloaded with
/// it. Every step degrades to a warning: `--offline`, an unreachable GitHub, a
/// rate-limited API or a compose file that does not parse all end with the
/// literal tag and the copy compiled into this binary, which still deploys.
fn resolve_chap_core(ctx: &Ctx, dir: &Path, requested: &str) -> ChapCore {
    let offline = ctx.registry.offline;
    let timeout = ctx.registry.timeout;
    let embedded = |tag: String| ChapCore {
        tag,
        source: ComposeSource::Embedded,
        cached: None,
    };

    let mut tag = requested.trim().to_string();
    if tag == MOVING_DEFAULT_TAG {
        match resolve_latest(offline, timeout) {
            Ok(release) => tag = release,
            Err(reason) => {
                crate::output::warn(&format!(
                    "{reason}; the chap-core tag stays `{MOVING_DEFAULT_TAG}`, so this deployment \
                     follows that moving tag - pass `--chap-tag vX.Y.Z` to pin a release"
                ));
                return embedded(tag);
            }
        }
    }

    // An unresolved `latest` names no fixed document, so there is no copy of
    // compose.ghcr.yml to pin either.
    if tag == MOVING_DEFAULT_TAG {
        return embedded(tag);
    }

    // A copy of this exact tag already in `.chaps/` (an earlier `init`, or an
    // `init --force` over a working deployment) is reused rather than
    // re-downloaded, which is also what makes `--offline` reproducible here.
    let path = dir.join(CHAPS_DIR).join(cached_compose_file(&tag));
    if let Ok(body) = std::fs::read_to_string(&path)
        && chapcore::validate_compose(&body).is_ok()
    {
        return ChapCore {
            source: fetched(&tag, &body),
            tag,
            cached: None,
        };
    }

    if offline {
        crate::output::warn(&format!(
            "--offline: compose.yml is rendered from the copy of chap-core's compose.ghcr.yml \
             built into this binary, not from the one {tag} publishes"
        ));
        return embedded(tag);
    }
    match chapcore::fetch_compose(&tag, timeout) {
        Ok(body) => ChapCore {
            source: fetched(&tag, &body),
            tag,
            cached: Some(body),
        },
        Err(err) => {
            crate::output::warn(&format!(
                "could not fetch chap-core's compose.ghcr.yml at {tag} ({err:#}); compose.yml is \
                 rendered from the copy built into this binary"
            ));
            embedded(tag)
        }
    }
}

/// The newest chap-core release, or why we are not asking.
fn resolve_latest(
    offline: bool,
    timeout: std::time::Duration,
) -> std::result::Result<String, String> {
    if offline {
        return Err("--offline: the newest chap-core release cannot be looked up".to_string());
    }
    chapcore::latest_release(timeout)
        .map_err(|err| format!("could not resolve the newest chap-core release ({err:#})"))
}

/// A [`ComposeSource::Fetched`] for a body that is about to be (or already is)
/// cached under `.chaps/`.
fn fetched(tag: &str, body: &str) -> ComposeSource {
    ComposeSource::Fetched {
        url: chapcore::compose_url(tag),
        tag: tag.to_string(),
        sha256: chapcore::sha256_hex(body.as_bytes()),
    }
}

/// What `init` did with `DIR/.env`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum EnvAction {
    /// A freshly rendered file, with a new database password.
    Written,
    /// The file that was already there, byte for byte.
    Kept,
    /// No `.env` at all (`--no-env`).
    Skipped,
}

/// Decide what to do with `DIR/.env`.
///
/// `--force` is deliberately not an input: it re-renders the compose files and
/// the state, but the credentials in `.env` outlive it. Only `--fresh-env`
/// replaces a file that is already there.
fn env_action(exists: bool, fresh_env: bool, no_env: bool) -> EnvAction {
    match (no_env, exists, fresh_env) {
        (true, _, _) => EnvAction::Skipped,
        (false, true, false) => EnvAction::Kept,
        (false, _, _) => EnvAction::Written,
    }
}

/// Expand `--models` into a selection.
///
/// `none` enables nothing, `default` enables [`DEFAULT_MODEL`], anything else
/// is a comma-separated list of marketplace ids or service ids.
fn parse_models(spec: &str, registry: &Registry) -> Result<Selection> {
    let spec = spec.trim();
    let ids: Vec<String> = match spec {
        "none" | "" => Vec::new(),
        "default" => vec![DEFAULT_MODEL.to_string()],
        list => list
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
    };
    for id in &ids {
        if registry.get(id).is_none() {
            return Err(ChapError::UnknownModel(id.clone()).into());
        }
    }
    Ok(Selection {
        enable: ids.into_iter().map(EnableRequest::new).collect(),
        disable: Vec::new(),
    })
}

/// Delete every overlay a previous project in `dir` owned.
///
/// All of them go: `init` starts the state over, so a model the new selection
/// keeps is rewritten by [`apply`] a moment later, and one it drops would
/// otherwise linger unreferenced while still holding its host port against the
/// allocator. Only files the old `.chaps/models.yaml` lists are touched; a hand-written
/// overlay is none of `init`'s business, and a directory without a readable
/// state file has nothing to clean up.
///
/// Returns the ones that are not coming back, for the report.
fn remove_stale_overlays(dir: &Path, selection: &Selection) -> Vec<PathBuf> {
    let Ok(previous) = Project::load(dir) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for (id, model) in &previous.state.models {
        let path = dir.join(&model.compose_file);
        if !path.is_file() || std::fs::remove_file(&path).is_err() {
            continue;
        }
        let rewritten = selection
            .enable
            .iter()
            .any(|req| req.id == *id || req.id == model.service_id);
        if !rewritten {
            removed.push(path);
        }
    }
    removed
}

/// Resolve the positional DIR against the working directory, dropping `.`
/// components so the printed path stays readable.
fn resolve_dir(arg: &Path) -> Result<PathBuf> {
    let mut out = if arg.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir()
            .map_err(|e| anyhow::anyhow!("resolving the current directory: {e}"))?
    };
    for component in arg.components() {
        match component {
            PathComponent::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

/// 32 lowercase hex characters, so the generated password is URL-safe inside
/// the postgres URL chap-core composes from POSTGRES_*.
fn random_password() -> Result<String> {
    auth::random_hex(16)
}

/// The pair of secrets `--api-token` writes into `.env`.
#[derive(Debug, Clone)]
struct Secrets {
    api_token: String,
    registration_key: String,
}

/// The secrets `--api-token` asks for: both of them, or neither.
///
/// A bare flag generates a token; an explicit value is used verbatim, with a
/// warning when chap-core would call it weak. The registration key is always
/// generated and can never be given on the command line: chap-core rejects an
/// unauthenticated registration once the API is protected, so a deployment with
/// a token needs a key as well, and there is no reason to make anyone type a
/// second secret.
fn resolve_secrets(requested: Option<&Option<String>>) -> Result<Option<Secrets>> {
    let Some(value) = requested else {
        return Ok(None);
    };
    let api_token = match value.as_deref().map(str::trim) {
        Some(given) if !given.is_empty() => {
            let length = given.chars().count();
            if length < auth::MIN_TOKEN_LENGTH {
                crate::output::warn(&auth::weak_token_warning(length));
            }
            given.to_string()
        }
        Some(_) => {
            crate::output::warn("--api-token was given an empty value; generated one instead");
            auth::random_secret()?
        }
        None => auth::random_secret()?,
    };
    Ok(Some(Secrets {
        api_token,
        registration_key: auth::random_secret()?,
    }))
}

/// The auth state `DIR/.env` describes, once `init` has settled its fate.
///
/// The file is the single source of truth, so reading it back covers all three
/// cases at once: a file this run rendered with `--api-token`, one that was
/// kept and already carries secrets from an earlier run or `chaps auth enable`,
/// and `--no-env`, which has no file and therefore no secrets.
fn env_auth(env: EnvAction, path: &Path) -> AuthState {
    if env == EnvAction::Skipped {
        return AuthState::default();
    }
    std::fs::read_to_string(path)
        .map(|body| auth::state_of(&body))
        .unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn summary(
    out: &Out,
    dir: &Path,
    written: &[PathBuf],
    report: &ApplyReport,
    registry: &Registry,
    env: EnvAction,
    chap_core: &ChapCore,
    project: &Project,
    secrets: Option<&Secrets>,
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
    if components.ocs.enabled {
        addresses.push_str(&format!(
            "{}       {} {}\n",
            out.key("OCS:"),
            out.value(&components.ocs_url()),
            out.dim(&format!(
                "(config in {}/{})",
                crate::components::OCS_DIR,
                crate::components::OCS_CONFIG_FILE
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
    // The token is printed once and only in its masked form: the full value
    // stays in `.env`, and `chaps auth show --reveal` is the way back to it.
    if let Some(secrets) = secrets {
        text.push_str(&format!(
            "{} {} {}\n",
            out.key("API token:"),
            out.value(&auth::mask(&secrets.api_token)),
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
        text.push_str(&format!(
            "\n{}\n",
            out.backticks("No models enabled; run `chaps models enable ID` to add one.")
        ));
    } else {
        text.push_str(&format!("\n{}\n", out.heading("Enabled:")));
        for (id, model) in &report.enabled {
            let name = registry
                .get(id)
                .map(|m| m.display_name.clone())
                .unwrap_or_else(|| id.clone());
            let reach = match model.host_port {
                Some(port) => out.value(&format!("http://localhost:{port}")),
                None => out.dim("internal"),
            };
            text.push_str(&format!(
                "  {id}  {name} {}  {reach}\n",
                out.dim(&format!("v{}", model.version))
            ));
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
    for warning in &report.warnings {
        text.push_str(&format!("\n{} {warning}\n", out.warn("warning:")));
    }

    // The last thing on the screen is the thing to type next, so it gets the
    // one box `init` draws.
    let next = format!("cd {} && chaps up\nchaps status", dir.display());
    text.push('\n');
    text.push_str(&out.panel_or(
        "Next",
        &out.backticks(&next),
        PanelKind::Ok,
        &format!("Next:\n  cd {} && chaps up\n  chaps status", dir.display()),
    ));
    text.push('\n');
    text
}

/// Paths are printed relative to the project directory; everything written
/// lives inside it.
fn file_label(dir: &Path, path: &Path) -> String {
    path.strip_prefix(dir).unwrap_or(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::load_embedded;

    #[test]
    fn models_none_enables_nothing() {
        let registry = load_embedded().unwrap();
        assert!(parse_models("none", &registry).unwrap().enable.is_empty());
        assert!(parse_models("  ", &registry).unwrap().enable.is_empty());
    }

    #[test]
    fn models_default_is_ewars() {
        let registry = load_embedded().unwrap();
        let sel = parse_models("default", &registry).unwrap();
        assert_eq!(sel.enable.len(), 1);
        assert_eq!(sel.enable[0].id, DEFAULT_MODEL);
        assert!(!sel.enable[0].allow_template);
    }

    #[test]
    fn models_takes_a_comma_list_of_ids_or_service_ids() {
        let registry = load_embedded().unwrap();
        let sel = parse_models("chapkit_ewars_model, auto-arima-chapkit", &registry).unwrap();
        let ids: Vec<&str> = sel.enable.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["chapkit_ewars_model", "auto-arima-chapkit"]);
    }

    #[test]
    fn an_unknown_model_is_named_in_the_error() {
        let registry = load_embedded().unwrap();
        let err = parse_models("chapkit_ewars_model,nope", &registry).expect_err("unknown model");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownModel(id)) if id == "nope"
        ));
    }

    #[test]
    fn the_with_and_without_lists_shape_the_component_set() {
        let plain = parse_components(None, None).unwrap();
        assert_eq!(plain, Components::default());
        assert!(plain.chap_core.enabled && !plain.ocs.enabled);

        let both = parse_components(Some("ocs,s3"), None).unwrap();
        assert!(both.chap_core.enabled && both.ocs.enabled && both.s3.enabled);
        assert_eq!(both.label(), "chap-core, ocs, s3");
        // Whitespace and case are the shell's, not part of the name.
        assert_eq!(parse_components(Some(" OCS , s3 "), None).unwrap(), both);

        let standalone = parse_components(Some("ocs"), Some("chap-core")).unwrap();
        assert!(!standalone.chap_core.enabled && standalone.ocs.enabled);
        assert_eq!(standalone.label(), "ocs");

        // An unknown name is a typo to report before anything is written.
        let err = parse_components(Some("ocs,nope"), None).expect_err("unknown component");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownComponent(name)) if name == "nope"
        ));

        // And a name on both lists is a contradiction, not a silent winner.
        let err = parse_components(Some("ocs"), Some("ocs")).expect_err("on and off at once");
        assert!(
            err.to_string().contains("both --with and --without"),
            "{err}"
        );
    }

    #[test]
    fn a_component_port_is_probed_alongside_the_api_port() {
        let mut components = Components::default();
        components.ocs.enabled = true;
        components.ocs.port = 9000;

        assert!(warn_about_busy_ports(&components, 8000, &|_| false).is_empty());
        let busy = warn_about_busy_ports(&components, 8000, &|port| port == 9000);
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].service, "ocs");
        assert_eq!(busy[0].port, 9000);

        // With chap-core off the API port is not one of this deployment's.
        components.chap_core.enabled = false;
        assert!(
            warn_about_busy_ports(&components, 8000, &|port| port == 8000).is_empty(),
            "no chap-core, no API port"
        );
    }

    #[test]
    fn resolve_dir_drops_cur_dir_components() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(resolve_dir(Path::new(".")).unwrap(), cwd);
        assert_eq!(
            resolve_dir(Path::new("./chapx")).unwrap(),
            cwd.join("chapx")
        );
        assert_eq!(
            resolve_dir(Path::new("./a/./b")).unwrap(),
            cwd.join("a").join("b")
        );
        // An absolute path is taken as it stands. It is spelled with the
        // platform's own separators and prefix, which is what `current_dir`
        // hands back, so the test says nothing about `/tmp` on a machine
        // where that is not a path at all.
        let absolute = cwd.join("chapx");
        assert!(absolute.is_absolute());
        assert_eq!(resolve_dir(&absolute).unwrap(), absolute);
    }

    #[test]
    fn an_existing_env_is_kept_whatever_force_says() {
        // --force is not an input at all: the only thing that replaces a file
        // that is already there is --fresh-env.
        assert_eq!(env_action(true, false, false), EnvAction::Kept);
        assert_eq!(env_action(true, true, false), EnvAction::Written);
    }

    #[test]
    fn a_missing_env_is_written() {
        assert_eq!(env_action(false, false, false), EnvAction::Written);
        assert_eq!(env_action(false, true, false), EnvAction::Written);
    }

    #[test]
    fn no_env_skips_the_file_either_way() {
        assert_eq!(env_action(false, false, true), EnvAction::Skipped);
        assert_eq!(env_action(true, false, true), EnvAction::Skipped);
    }

    #[test]
    fn the_env_action_serializes_lowercase() {
        let json = |a: EnvAction| serde_json::to_string(&a).unwrap();
        assert_eq!(json(EnvAction::Written), "\"written\"");
        assert_eq!(json(EnvAction::Kept), "\"kept\"");
        assert_eq!(json(EnvAction::Skipped), "\"skipped\"");
    }

    /// A [`Ctx`] that never touches the network.
    fn offline_ctx() -> Ctx {
        Ctx {
            out: crate::output::Out::default(),
            project_dir: PathBuf::from("."),
            registry: crate::registry::RegistryOptions {
                offline: true,
                ..crate::registry::RegistryOptions::default()
            },
            cli_version: "0.1.0",
        }
    }

    #[test]
    fn offline_keeps_the_tag_as_given_and_the_embedded_compose_file() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = offline_ctx();

        let latest = resolve_chap_core(&ctx, dir.path(), "latest");
        assert_eq!(latest.tag, "latest");
        assert_eq!(latest.source, ComposeSource::Embedded);
        assert!(latest.cached.is_none());

        // An explicit release tag is used verbatim; only the compose file
        // falls back, because downloading it needs the network.
        let pinned = resolve_chap_core(&ctx, dir.path(), "v2.3.1");
        assert_eq!(pinned.tag, "v2.3.1");
        assert_eq!(pinned.source, ComposeSource::Embedded);
        assert!(pinned.cached.is_none());
    }

    #[test]
    fn a_cached_compose_file_is_reused_instead_of_downloaded() {
        let dir = tempfile::tempdir().unwrap();
        let chaps = dir.path().join(CHAPS_DIR);
        std::fs::create_dir_all(&chaps).unwrap();
        let body = "services:\n  chap:\n    image: ghcr.io/x:${CHAP_IMAGE_TAG:-latest}\n";
        std::fs::write(chaps.join(cached_compose_file("v2.3.1")), body).unwrap();

        // Offline, and still a fetched source: the copy is right there.
        let found = resolve_chap_core(&offline_ctx(), dir.path(), "v2.3.1");
        assert_eq!(found.tag, "v2.3.1");
        assert!(
            found.cached.is_none(),
            "nothing to write, it is already there"
        );
        assert_eq!(
            found.source,
            ComposeSource::Fetched {
                url: chapcore::compose_url("v2.3.1"),
                tag: "v2.3.1".to_string(),
                sha256: chapcore::sha256_hex(body.as_bytes()),
            }
        );

        // A cached file that is not a usable compose document is ignored.
        std::fs::write(chaps.join(cached_compose_file("v2.2.0")), "nonsense: [").unwrap();
        let ignored = resolve_chap_core(&offline_ctx(), dir.path(), "v2.2.0");
        assert_eq!(ignored.source, ComposeSource::Embedded);
    }

    #[test]
    fn an_active_api_port_line_is_read_out_of_an_env_file() {
        let body = "POSTGRES_DB=chap_core\nCHAP_API_PORT=8123\n# CHAP_IMAGE_TAG=latest\n";
        assert_eq!(env_api_port(body), Some(8123));
        // A commented placeholder is not a setting, and neither is nonsense.
        assert_eq!(env_api_port("# CHAP_API_PORT=8123\n"), None);
        assert_eq!(env_api_port("CHAP_API_PORT=\nCHAP_API_PORT=nope\n"), None);
        assert_eq!(env_api_port("POSTGRES_DB=chap_core\n"), None);
        // The first usable value wins, the way compose reads the file.
        assert_eq!(
            env_api_port("CHAP_API_PORT=8010\nCHAP_API_PORT=8020\n"),
            Some(8010)
        );
    }

    #[test]
    fn a_free_api_port_warns_about_nothing() {
        assert!(warn_about_busy_ports(&Components::default(), 8000, &|_| false).is_empty());
    }

    #[test]
    fn a_busy_api_port_is_a_warning_not_a_refusal() {
        // 8000 and 8001 are taken, 8002 is not: init still writes the
        // directory and says which port to use instead.
        assert_eq!(
            warn_about_busy_ports(&Components::default(), 8000, &|port| (8000..=8001)
                .contains(&port))
            .len(),
            1
        );
        // Even with nothing free above it, the warning goes out.
        assert_eq!(
            warn_about_busy_ports(&Components::default(), 8000, &|_| true).len(),
            1
        );
    }

    #[test]
    fn without_the_flag_there_are_no_secrets_at_all() {
        assert!(resolve_secrets(None).unwrap().is_none());
    }

    #[test]
    fn a_bare_api_token_flag_generates_both_secrets() {
        let secrets = resolve_secrets(Some(&None)).unwrap().expect("both secrets");
        assert_eq!(secrets.api_token.len(), 64);
        assert_eq!(secrets.registration_key.len(), 64);
        assert_ne!(
            secrets.api_token, secrets.registration_key,
            "two independent secrets"
        );
        for secret in [&secrets.api_token, &secrets.registration_key] {
            assert!(
                secret
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                "{secret}"
            );
        }
    }

    #[test]
    fn an_explicit_api_token_is_used_verbatim_and_still_gets_a_key() {
        let given = "mysecrettoken-that-is-long-enough-32ch";
        let secrets = resolve_secrets(Some(&Some(given.to_string())))
            .unwrap()
            .expect("both secrets");
        assert_eq!(secrets.api_token, given);
        assert_eq!(secrets.registration_key.len(), 64);

        // Surrounding whitespace is the shell's, not part of the secret.
        let secrets = resolve_secrets(Some(&Some(format!("  {given}  "))))
            .unwrap()
            .unwrap();
        assert_eq!(secrets.api_token, given);

        // A short token is accepted (chap-core does too) and warned about.
        let secrets = resolve_secrets(Some(&Some("short".to_string())))
            .unwrap()
            .unwrap();
        assert_eq!(secrets.api_token, "short");

        // An empty value cannot be a token, so one is generated instead of
        // silently leaving the API open.
        let secrets = resolve_secrets(Some(&Some("   ".to_string())))
            .unwrap()
            .unwrap();
        assert_eq!(secrets.api_token.len(), 64);
    }

    #[test]
    fn the_recorded_auth_state_follows_the_env_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ENV_FILE);

        // --no-env: no file, and nothing is protected.
        assert_eq!(env_auth(EnvAction::Skipped, &path), AuthState::default());
        // A file that is not there yet reads the same way.
        assert_eq!(env_auth(EnvAction::Written, &path), AuthState::default());

        std::fs::write(&path, "# CHAP_API_TOKEN=\nPOSTGRES_DB=chap_core\n").unwrap();
        assert_eq!(env_auth(EnvAction::Written, &path), AuthState::default());

        std::fs::write(&path, "CHAP_API_TOKEN=t\nSERVICEKIT_REGISTRATION_KEY=k\n").unwrap();
        assert_eq!(
            env_auth(EnvAction::Kept, &path),
            AuthState {
                api_token: true,
                registration_key: true
            },
            "a kept .env keeps the deployment authenticated"
        );
        // ...but --no-env never looks at a file it was told not to write.
        assert_eq!(env_auth(EnvAction::Skipped, &path), AuthState::default());
    }

    #[test]
    fn the_generated_password_is_url_safe_hex() {
        let password = random_password().unwrap();
        assert_eq!(password.len(), 32);
        assert!(
            password
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );
        assert_ne!(password, random_password().unwrap());
    }
}
