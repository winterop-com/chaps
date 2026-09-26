//! `chaps init` — write a deployment directory.

use crate::auth;
use crate::chapcore;
use crate::cli::{ComponentPortArg, InitArgs};
use crate::commands::Ctx;
use crate::components::{
    COMPONENTS_FILE, Component, Components, S3_SOON_NOTE, S3_WITHOUT_OCS_NOTE,
};
use crate::compose::spec::EnvSpec;
use crate::compose::sync::write_ocs_config;
use crate::compose::{API_SERVICE, ApplyReport, EnableRequest, Selection, apply, render_env};
use crate::error::{ChapError, Result};
use crate::output::{Out, PanelKind};
use crate::project::{
    API_PORT_ENV_VAR, AuthState, CHAPS_DIR, ComposeSource, DEFAULT_PORT_RANGE, ENV_FILE,
    EnabledModel, MODELS_FILE, PROJECT_FILE, Project, ProjectState, cached_compose_file,
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

/// What the summary says for a deployment that enables no models.
const NO_MODELS: &str = "No models enabled; run `chaps models enable ID` to add one.";

/// The same for a deployment chap-core is not a component of.
///
/// A model service registers with chap-core and is reached through it, so
/// `chaps models enable` there is refused with
/// [`crate::components::MODELS_NEED_CHAP_CORE`]. Naming it as the next step
/// would cost the reader a command to find that out, which is worse than
/// naming none: the line says why there are no models and names the one
/// command that changes it. Where the deployment goes next - its addresses and
/// its OCS config file - the block of addresses above has already said.
const NO_MODELS_WITHOUT_CHAP_CORE: &str = "No models: they register with chap-core, which this deployment leaves out; \
     `chaps components enable chap-core` adds it.";

/// Which of the two the summary prints, as a pure function of the component
/// set so the choice is testable without a deployment on disk.
fn no_models_line(components: &Components) -> &'static str {
    if components.chap_core.enabled {
        NO_MODELS
    } else {
        NO_MODELS_WITHOUT_CHAP_CORE
    }
}

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
    let components = parse_components(&ComponentFlags::from_args(args))?;
    // The deployment this run is writing over, when there is one: `--force`
    // starts the state over, so anything it had and this run does not ask for is
    // going, and the operator hears which before the directory is rewritten.
    let previous = Project::load(&dir).ok();
    let dropped = match &previous {
        Some(previous) => dropped_components(
            &previous.state.components,
            &components,
            &parse_component_list(args.without.as_deref()).unwrap_or_default(),
        ),
        None => Vec::new(),
    };
    for component in &dropped {
        crate::output::warn(&component_dropped(*component));
    }
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
    // The ports the deployment publishes, and a taken one is only a problem at
    // `chaps up`: the process holding one may well be a previous stack this
    // deployment is meant to replace, and a deployment that is down is not
    // holding anything yet. So: a warning with a way out, not a refusal to
    // write the directory.
    let others = crate::ports::other_deployments(&dir, &crate::docker::compose_ls_json);
    let ports = warn_about_ports(&components, args.api_port, &crate::ports::is_busy, &others);
    let api_port_busy = ports.busy.iter().any(|c| c.service == API_SERVICE);
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

    // The containers of everything this run takes away go first, while the
    // compose files that define them are still on disk: a service whose
    // definition has been removed cannot be stopped by name any more, and one
    // left running keeps its host port published long after the deployment
    // stopped asking for it. Read from the previous project, because that is the
    // one whose `-f` list still names those files.
    let mut dropped_notes = Vec::new();
    if let Some(previous) = &previous {
        dropped_notes.extend(stop_dropped(previous, &components, &selection));
    }

    // --force starts the state over, so overlays the previous project owned
    // would otherwise linger: unreferenced by the umbrella, but still holding
    // their host port against the allocator. The compose file of a component
    // that is not coming back is the same problem, and worse: nothing in
    // `.chaps/` would mention it afterwards, so no later `chaps sync` would ever
    // remove it either.
    let mut stale = remove_stale_overlays(&dir, &selection);
    if let Some(previous) = &previous {
        stale.extend(remove_dropped_component_files(
            &dir,
            &previous.state.components,
            &components,
        ));
    }

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
    if components.ocs.enabled {
        if let Some(path) = write_ocs_config(
            &dir,
            &crate::commands::components::request(&args.ocs).into_spec(),
        )? {
            written.push(path);
        }
        // And `--ocs-read-only` after it, on the file this run just scaffolded
        // or on the one that was already there: the scaffold does not carry the
        // key, and one function owns that edit. A file that already says so is
        // left untouched, so it is not reported as written either.
        if components.ocs.read_only
            && let Some(edit) = crate::compose::sync::set_read_only(&dir, true)?
            && edit != crate::compose::sync::KeyEdit::Unchanged
        {
            written.push(project.ocs_config_path());
        }
    }

    // apply() writes the overlays, compose.marketplace.yml (even with no
    // models) and .chaps/.
    let endpoints = crate::manual::Endpoints::from_env(ctx.registry.offline);
    let mut report = apply(&mut project, &registry, &selection, &endpoints)?;
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
        "busy_ports": ports.busy.iter().map(|c| c.port).collect::<Vec<u16>>(),
        "claimed_ports": ports.claimed.iter().map(|c| c.port).collect::<Vec<u16>>(),
        "port_warnings": ports.lines,
        "components": project.state.components,
        // What the previous deployment in this directory lost, for a `--force`
        // that a script is driving rather than a person reading the warnings.
        "dropped_components": dropped.iter().map(|c| c.name()).collect::<Vec<&str>>(),
        "dropped_notes": dropped_notes,
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
            &dropped_notes,
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

/// What `init` found out about the host ports the new deployment wants, and
/// warned about.
#[derive(Debug, Default)]
struct PortWarnings {
    /// Ports something on this machine is listening on right now.
    busy: Vec<crate::ports::PortClaim>,
    /// Ports another deployment on this machine already publishes, and nothing
    /// is listening on.
    claimed: Vec<crate::ports::PortClaim>,
    /// Every line that was printed, in the order it went out, for `--json`.
    lines: Vec<String>,
}

/// The host ports the new deployment is going to publish.
fn wanted_claims(components: &Components, api_port: u16) -> Vec<crate::ports::PortClaim> {
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
    claims
}

/// Warn about every host port the new deployment publishes that is already
/// spoken for, and say what to do about each.
///
/// Two ways a port can be spoken for, and one line either way: something is
/// listening on it now, or one of `others` - a deployment that is down, so
/// holding no socket at all - publishes it too. The second is the one nothing
/// else catches until the `chaps up` that finds the other deployment there
/// first.
///
/// Neither is an error. The listener may be a stack this deployment is meant
/// to replace, two deployments on one port is a perfectly good way to take
/// turns, and the directory is worth writing either way.
fn warn_about_ports(
    components: &Components,
    api_port: u16,
    busy: &dyn Fn(u16) -> bool,
    others: &[crate::ports::Deployment],
) -> PortWarnings {
    // A port another deployment publishes is as good as taken when suggesting
    // one: moving onto it would trade one collision for another.
    let taken = |port: u16| busy(port) || others.iter().any(|other| other.holds(port));
    let mut found = PortWarnings::default();
    for claim in wanted_claims(components, api_port) {
        // Upwards from the requested port: the next free number is the one
        // least likely to collide with something else the operator has in
        // mind.
        let suggestion = crate::ports::first_free(claim.port.saturating_add(1), u16::MAX, &taken);
        // A port that is both listening and claimed gets one line, and the
        // listener is the half that is in the way today.
        if busy(claim.port) {
            let line = crate::ports::busy_line(&claim, suggestion);
            crate::output::warn(&line);
            found.lines.push(line);
            found.busy.push(claim);
            continue;
        }
        let holders: Vec<&crate::ports::Deployment> = others
            .iter()
            .filter(|other| other.holds(claim.port))
            .collect();
        if holders.is_empty() {
            continue;
        }
        let line = crate::ports::claimed_line(&claim, &holders, suggestion);
        crate::output::warn(&line);
        found.lines.push(line);
        found.claimed.push(claim);
    }
    found
}

/// The `init` flags that shape the component set, so [`parse_components`] takes
/// one argument per concern rather than one per flag.
#[derive(Debug, Clone, Default)]
struct ComponentFlags<'a> {
    with: Option<&'a str>,
    without: Option<&'a str>,
    ocs_base_url: Option<&'a str>,
    ocs_port: Option<ComponentPortArg>,
    s3_port: Option<ComponentPortArg>,
    ocs_read_only: bool,
}

impl<'a> ComponentFlags<'a> {
    fn from_args(args: &'a InitArgs) -> ComponentFlags<'a> {
        ComponentFlags {
            with: args.with.as_deref(),
            without: args.without.as_deref(),
            ocs_base_url: args.ocs_base_url.as_deref(),
            ocs_port: args.ocs_port,
            s3_port: args.s3_port,
            ocs_read_only: args.ocs_read_only,
        }
    }
}

/// Expand `--with`, `--without` and the per-component `--ocs-*` / `--s3-*`
/// flags into a component set.
///
/// chap-core is on unless `--without chap-core` says otherwise; everything
/// else is off until `--with` names it. A name in both lists is a
/// contradiction rather than a silent winner, and so is a setting for a
/// component this deployment is not getting: an `--ocs-port` that quietly did
/// nothing would leave the operator waiting for OCS on a port no file mentions.
fn parse_components(flags: &ComponentFlags) -> Result<Components> {
    let on = parse_component_list(flags.with)?;
    let off = parse_component_list(flags.without)?;
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
    if let Some(base_url) = flags
        .ocs_base_url
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        if !components.ocs.enabled {
            return Err(anyhow::anyhow!(
                "--ocs-base-url needs the ocs component; add `--with ocs`"
            ));
        }
        if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
            return Err(anyhow::anyhow!(
                "--ocs-base-url has to be an absolute URL, so `{base_url}` will not do: OCS \
                 builds its STAC and openEO links by appending a path to this value"
            ));
        }
        components.ocs.base_url = Some(base_url.trim_end_matches('/').to_string());
    }
    if let Some(port) = flags.ocs_port {
        if !components.ocs.enabled {
            return Err(anyhow::anyhow!(
                "--ocs-port needs the ocs component; add `--with ocs`"
            ));
        }
        components.ocs.port = port.0;
    }
    if let Some(port) = flags.s3_port {
        if !components.s3.enabled {
            return Err(anyhow::anyhow!(
                "--s3-port needs the s3 component; add `--with s3`"
            ));
        }
        components.s3.port = port.0;
    }
    if flags.ocs_read_only {
        if !components.ocs.enabled {
            return Err(anyhow::anyhow!(
                "--ocs-read-only needs the ocs component; add `--with ocs`"
            ));
        }
        // The record only; the file it records is settled once the scaffold is
        // on disk, by the one function that edits it.
        components.ocs.read_only = true;
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
        // `init` settles the component set through `parse_components` before the
        // selection is built, so there is nothing here for apply() to change.
        components: None,
    })
}

/// The components the previous deployment had that this run does not ask for.
///
/// `init` starts the component set over from the flags, exactly as it starts the
/// model set over from `--models`. That is worth saying out loud, because the
/// default for everything but chap-core is *off*: a directory that had `ocs` and
/// is re-initialised without `--with ocs` loses it without the run mentioning
/// the component at all. One named in `--without` was asked for, so it gets no
/// warning, and chap-core has no compose file of its own to take away.
fn dropped_components(
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
fn component_dropped(component: Component) -> String {
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
fn stop_dropped(previous: &Project, components: &Components, selection: &Selection) -> Vec<String> {
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
fn kept_by(selection: &Selection, id: &str, model: &EnabledModel) -> bool {
    selection
        .enable
        .iter()
        .any(|req| req.id == id || req.id == model.service_id)
}

/// Delete the component compose files the previous deployment had and the new
/// component set does not bring back, and report them.
///
/// Nothing else would: once `.chaps/project.yaml` has been rewritten without
/// them they are in neither `compose_files` nor `rendered_files`, so `chaps
/// sync` does not see them as its own any more and would leave them in the
/// directory for good.
fn remove_dropped_component_files(
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
/// keeps is rewritten by [`apply()`] a moment later, and one it drops would
/// otherwise linger unreferenced while still holding its host port against the
/// allocator. Only files the old `.chaps/models.yaml` lists are touched; a
/// hand-written overlay is none of `init`'s business, and a directory without
/// a readable state file has nothing to clean up.
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
        if !kept_by(selection, id, model) {
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
    dropped_notes: &[String],
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
        let reach = components.ocs_reach();
        addresses.push_str(&format!(
            "{}       {} {}\n",
            out.key("OCS:"),
            match components.ocs_url() {
                Some(_) => out.value(&reach),
                None => out.dim(&reach),
            },
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
            out.backticks(no_models_line(components))
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
    // The same soft dependency the other way round, in the same words
    // `components enable s3` uses: a store with nothing to put in it.
    if components.s3.enabled && !components.ocs.enabled {
        text.push_str(&format!("\n{} {S3_WITHOUT_OCS_NOTE}\n", out.dim("note:")));
    }
    // What the containers of a dropped component or model did on the way out.
    // The warnings above already said which components are going; this says what
    // was actually stopped, which is the half a reader cannot infer.
    for note in dropped_notes {
        text.push_str(&format!("\n{} {note}\n", out.dim("note:")));
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

    /// The next step a summary names has to be one that works. `chaps models
    /// enable` on a deployment without chap-core is refused, so naming it
    /// would cost the reader a command to find that out.
    #[test]
    fn a_deployment_without_chap_core_is_not_told_to_enable_a_model() {
        let mut components = Components::default();
        assert_eq!(no_models_line(&components), NO_MODELS);
        assert!(no_models_line(&components).contains("`chaps models enable ID`"));

        components.set_enabled(Component::ChapCore, false);
        let line = no_models_line(&components);
        assert!(!line.contains("chaps models enable"), "{line}");
        assert!(
            line.contains("`chaps components enable chap-core`"),
            "{line}"
        );
        // The same command the refusal it pre-empts names, so the reader is
        // sent to one place and not two.
        assert!(
            crate::components::MODELS_NEED_CHAP_CORE
                .contains("`chaps components enable chap-core`"),
            "the line and the refusal name the same command"
        );
    }

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

    /// `parse_components` from the two list flags alone, which is what most of
    /// these cases are about.
    fn components_of(with: Option<&str>, without: Option<&str>) -> Result<Components> {
        parse_components(&ComponentFlags {
            with,
            without,
            ..ComponentFlags::default()
        })
    }

    #[test]
    fn the_with_and_without_lists_shape_the_component_set() {
        let plain = components_of(None, None).unwrap();
        assert_eq!(plain, Components::default());
        assert!(plain.chap_core.enabled && !plain.ocs.enabled);

        let both = components_of(Some("ocs,s3"), None).unwrap();
        assert!(both.chap_core.enabled && both.ocs.enabled && both.s3.enabled);
        assert_eq!(both.label(), "chap-core, ocs, s3");
        // Whitespace and case are the shell's, not part of the name.
        assert_eq!(components_of(Some(" OCS , s3 "), None).unwrap(), both);

        let standalone = components_of(Some("ocs"), Some("chap-core")).unwrap();
        assert!(!standalone.chap_core.enabled && standalone.ocs.enabled);
        assert_eq!(standalone.label(), "ocs");

        // An unknown name is a typo to report before anything is written.
        let err = components_of(Some("ocs,nope"), None).expect_err("unknown component");
        assert!(matches!(
            err.downcast_ref::<ChapError>(),
            Some(ChapError::UnknownComponent(name)) if name == "nope"
        ));

        // And a name on both lists is a contradiction, not a silent winner.
        let err = components_of(Some("ocs"), Some("ocs")).expect_err("on and off at once");
        assert!(
            err.to_string().contains("both --with and --without"),
            "{err}"
        );
    }

    #[test]
    fn the_ocs_base_url_needs_the_component_and_has_to_be_absolute() {
        let base_url = |with, url| {
            parse_components(&ComponentFlags {
                with,
                ocs_base_url: Some(url),
                ..ComponentFlags::default()
            })
        };
        let proxied = base_url(Some("ocs"), "https://ocs.example.org/").unwrap();
        assert_eq!(
            proxied.ocs.base_url.as_deref(),
            Some("https://ocs.example.org"),
            "the trailing slash goes: OCS appends a path to this"
        );

        // An empty value is no value, not a refusal.
        assert_eq!(base_url(Some("ocs"), "  ").unwrap().ocs.base_url, None);

        let err = base_url(None, "https://ocs.example.org").expect_err("no component to set it on");
        assert!(err.to_string().contains("--with ocs"), "{err}");

        let err = base_url(Some("ocs"), "ocs.example.org").expect_err("no scheme");
        assert!(err.to_string().contains("absolute URL"), "{err}");
    }

    /// A port for a component this deployment is not getting is the same mistake
    /// as a base URL for one: it has to be a refusal, because a flag that
    /// quietly did nothing leaves the operator waiting for OCS on a port no file
    /// mentions.
    #[test]
    fn the_component_ports_and_the_read_only_switch_need_their_component() {
        let with_ocs = |flags: ComponentFlags| {
            parse_components(&ComponentFlags {
                with: Some("ocs,s3"),
                ..flags
            })
        };

        let moved = with_ocs(ComponentFlags {
            ocs_port: Some(ComponentPortArg(Some(18010))),
            s3_port: Some(ComponentPortArg(Some(18011))),
            ..ComponentFlags::default()
        })
        .unwrap();
        assert_eq!(moved.ocs.port, Some(18010));
        assert_eq!(moved.s3.port, Some(18011));

        // `none` is how a component is reached inside the deployment alone, and
        // it has to survive the flag as well as the record.
        let internal = with_ocs(ComponentFlags {
            ocs_port: Some(ComponentPortArg(None)),
            ..ComponentFlags::default()
        })
        .unwrap();
        assert_eq!(internal.ocs.port, None);
        assert_eq!(internal.ocs_reach(), "internal");

        let read_only = with_ocs(ComponentFlags {
            ocs_read_only: true,
            ..ComponentFlags::default()
        })
        .unwrap();
        assert!(read_only.ocs.read_only);
        // Left alone without the flag: the file says what it says, and this is
        // only a record of it.
        assert!(!with_ocs(ComponentFlags::default()).unwrap().ocs.read_only);

        for (flags, wanted) in [
            (
                ComponentFlags {
                    ocs_port: Some(ComponentPortArg(Some(18010))),
                    ..ComponentFlags::default()
                },
                "--ocs-port needs the ocs component; add `--with ocs`",
            ),
            (
                ComponentFlags {
                    with: Some("ocs"),
                    s3_port: Some(ComponentPortArg(Some(18011))),
                    ..ComponentFlags::default()
                },
                "--s3-port needs the s3 component; add `--with s3`",
            ),
            (
                ComponentFlags {
                    ocs_read_only: true,
                    ..ComponentFlags::default()
                },
                "--ocs-read-only needs the ocs component; add `--with ocs`",
            ),
        ] {
            let err = parse_components(&flags).expect_err("no component to set it on");
            assert_eq!(err.to_string(), wanted);
        }
    }

    /// The new ports have to reach the warning `init` already prints, which they
    /// do by being recorded on the component: `wanted_claims` reads the set, not
    /// the flags.
    #[test]
    fn a_moved_component_port_is_the_one_that_gets_probed() {
        let components = parse_components(&ComponentFlags {
            with: Some("ocs,s3"),
            ocs_port: Some(ComponentPortArg(Some(18010))),
            s3_port: Some(ComponentPortArg(Some(18011))),
            ..ComponentFlags::default()
        })
        .unwrap();

        let ports: Vec<(String, u16)> = wanted_claims(&components, 18000)
            .into_iter()
            .map(|claim| (claim.service, claim.port))
            .collect();
        assert_eq!(
            ports,
            vec![
                (API_SERVICE.to_string(), 18000),
                ("ocs".to_string(), 18010),
                ("s3".to_string(), 18011),
            ]
        );

        let warned = warn_about_ports(&components, 18000, &|port| port == 18010, &[]);
        assert_eq!(warned.busy.len(), 1);
        assert!(warned.lines[0].contains("(needed by ocs)"), "{warned:?}");

        // `--ocs-port none` publishes nothing, so there is nothing to probe.
        let internal = parse_components(&ComponentFlags {
            with: Some("ocs"),
            ocs_port: Some(ComponentPortArg(None)),
            ..ComponentFlags::default()
        })
        .unwrap();
        assert_eq!(
            wanted_claims(&internal, 18000).len(),
            1,
            "the API port only"
        );
    }

    /// A `--force` re-init resets the component set from the flags, the way it
    /// resets the model set from `--models`. The default for everything but
    /// chap-core is off, so the components go without the run mentioning them -
    /// which is exactly why each one has to be said out loud.
    #[test]
    fn a_re_init_says_which_components_it_is_taking_away() {
        let mut previous = Components::default();
        previous.set_enabled(Component::Ocs, true);
        previous.set_enabled(Component::S3, true);

        let both = dropped_components(&previous, &Components::default(), &[]);
        assert_eq!(both, vec![Component::Ocs, Component::S3]);

        // One the operator asked to leave out needs no warning: they typed it.
        assert_eq!(
            dropped_components(&previous, &Components::default(), &[Component::Ocs]),
            vec![Component::S3]
        );
        // And nothing is dropped when the flags bring them back.
        assert!(dropped_components(&previous, &previous, &[]).is_empty());

        // chap-core has no compose file of its own to take away, and can only go
        // off because `--without chap-core` said so.
        let mut core_off = previous.clone();
        core_off.set_enabled(Component::ChapCore, false);
        assert_eq!(dropped_components(&previous, &core_off, &[]), Vec::new());

        let line = component_dropped(Component::Ocs);
        assert!(line.contains("compose.ocs.yml is removed"), "{line}");
        assert!(line.contains("`--with ocs`"), "{line}");
        assert!(!line.contains('\n'), "one line: {line}");
    }

    /// The compose file of a component that is not coming back has to go with
    /// it. Nothing else would ever remove it: the new `project.yaml` lists it in
    /// neither `compose_files` nor `rendered_files`, so `chaps sync` does not see
    /// it as its own any more.
    #[test]
    fn the_compose_file_of_a_dropped_component_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let mut previous = Components::default();
        previous.set_enabled(Component::Ocs, true);
        previous.set_enabled(Component::S3, true);
        let write = |file: &str| {
            std::fs::write(dir.path().join(file), "services: {}\n").unwrap();
            dir.path().join(file)
        };
        let ocs = write(crate::components::OCS_COMPOSE);
        let s3 = write(crate::components::S3_COMPOSE);

        // The store stays, so only OCS's file goes.
        let mut kept = Components::default();
        kept.set_enabled(Component::S3, true);
        assert_eq!(
            remove_dropped_component_files(dir.path(), &previous, &kept),
            vec![ocs.clone()]
        );
        assert!(!ocs.is_file() && s3.is_file());

        // A file that is already gone is not reported twice, and one the new set
        // keeps is left for the sync to re-render.
        assert!(
            remove_dropped_component_files(dir.path(), &previous, &kept).is_empty(),
            "nothing left to remove"
        );
        assert_eq!(
            remove_dropped_component_files(dir.path(), &previous, &Components::default()),
            vec![s3.clone()]
        );
        assert!(!s3.is_file());
    }

    /// Which of the previous project's models the new selection brings back, by
    /// either of the two names one can be asked for. It decides both which
    /// overlays are reported as removed and which containers are stopped, so the
    /// two can never disagree.
    #[test]
    fn a_model_is_kept_by_either_of_its_two_names() {
        let model = crate::project::EnabledModel {
            service_id: "chapkit-ewars-model".to_string(),
            ..model_record()
        };
        let selection = |id: &str| Selection {
            enable: vec![EnableRequest::new(id)],
            ..Selection::default()
        };
        assert!(kept_by(
            &selection("chapkit_ewars_model"),
            "chapkit_ewars_model",
            &model
        ));
        assert!(kept_by(
            &selection("chapkit-ewars-model"),
            "chapkit_ewars_model",
            &model
        ));
        assert!(!kept_by(
            &selection("auto-arima-chapkit"),
            "chapkit_ewars_model",
            &model
        ));
        assert!(!kept_by(
            &Selection::default(),
            "chapkit_ewars_model",
            &model
        ));
    }

    /// An enabled-model record with nothing in it that these tests care about.
    fn model_record() -> crate::project::EnabledModel {
        crate::project::EnabledModel {
            service_id: "x".to_string(),
            image: "ghcr.io/chap-models/x".into(),
            image_tag: "sha-1111111".into(),
            version: "1.0.0".into(),
            channel: None,
            host_port: None,
            data_dir: "/app/data".into(),
            user: "chapkit:chapkit".into(),
            user_from: Default::default(),
            platform: None,
            compose_file: "compose.x.yml".into(),
        }
    }

    #[test]
    fn a_component_port_is_probed_alongside_the_api_port() {
        let mut components = Components::default();
        components.ocs.enabled = true;
        components.ocs.port = Some(9000);

        assert!(
            warn_about_ports(&components, 8000, &|_| false, &[])
                .busy
                .is_empty()
        );
        let busy = warn_about_ports(&components, 8000, &|port| port == 9000, &[]).busy;
        assert_eq!(busy.len(), 1);
        assert_eq!(busy[0].service, "ocs");
        assert_eq!(busy[0].port, 9000);

        // With chap-core off the API port is not one of this deployment's.
        components.chap_core.enabled = false;
        assert!(
            warn_about_ports(&components, 8000, &|port| port == 8000, &[])
                .busy
                .is_empty(),
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
        let quiet = warn_about_ports(&Components::default(), 8000, &|_| false, &[]);
        assert!(quiet.busy.is_empty() && quiet.claimed.is_empty() && quiet.lines.is_empty());
    }

    #[test]
    fn a_busy_api_port_is_a_warning_not_a_refusal() {
        // 8000 and 8001 are taken, 8002 is not: init still writes the
        // directory and says which port to use instead.
        let warned = warn_about_ports(
            &Components::default(),
            8000,
            &|port| (8000..=8001).contains(&port),
            &[],
        );
        assert_eq!(warned.busy.len(), 1);
        assert_eq!(warned.lines.len(), 1);
        assert!(
            warned.lines[0].contains("--api-port 8002"),
            "{:?}",
            warned.lines
        );
        // Even with nothing free above it, the warning goes out.
        let warned = warn_about_ports(&Components::default(), 8000, &|_| true, &[]);
        assert_eq!(warned.busy.len(), 1);
        assert!(
            warned.lines[0].contains("--api-port <free>"),
            "{:?}",
            warned.lines
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
