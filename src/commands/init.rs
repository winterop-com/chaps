//! `chaps init` — write a deployment directory.

use crate::auth;
use crate::chapcore;
use crate::cli::{ComponentPortArg, InitArgs};
use crate::commands::Ctx;
use crate::components::{
    COMPONENTS_FILE, Component, Components, DHIS2_FIRST_START_NOTE, S3_SOON_NOTE,
    S3_WITHOUT_OCS_NOTE,
};
use crate::compose::spec::EnvSpec;
use crate::compose::sync::{write_dhis2_config, write_ocs_config};
use crate::compose::{API_SERVICE, ApplyReport, EnableRequest, Selection, apply, render_env};
use crate::error::{ChapError, Result};
use crate::output::Out;
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

/// The same for a deployment chap-core is not a component of, where a model
/// enabled later runs on its own: it registers nowhere and is published on a
/// host port, which is the one thing worth knowing before enabling one.
const NO_MODELS_WITHOUT_CHAP_CORE: &str = "No models enabled; run `chaps models enable ID` to add one. \
     Without chap-core each runs on its own, on a published host port.";

/// Which of the two the summary prints, as a pure function of the component
/// set so the choice is testable without a deployment on disk.
///
/// Neither, for a deployment of components without chap-core - OCS, DHIS2 -
/// where models were not the point and a line about adding one is noise. A
/// deployment of nothing but models (`--only none`) still gets it.
fn no_models_line(components: &Components) -> Option<&'static str> {
    if components.has_chap_core_api() {
        Some(NO_MODELS)
    } else if components.enabled().is_empty() {
        Some(NO_MODELS_WITHOUT_CHAP_CORE)
    } else {
        None
    }
}

/// Create compose.yml, compose.marketplace.yml, the model overlays, .env and
/// the `.chaps/` directory in the target directory.
///
/// `args.source` names a chap-core checkout to build from instead of a
/// release to pull; see [`checkout_source`].
pub fn run(ctx: &Ctx, args: &InitArgs) -> Result<()> {
    let checkout = args.source.as_deref().map(checkout_source).transpose()?;
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
    let mut components = parse_components(&ComponentFlags::from_args(args, ctx.registry.offline))?;
    // A chap-core elsewhere is instead of this deployment's own, never beside it.
    if let Some(url) = &args.chap_core_url {
        components.set_enabled(Component::ChapCore, false);
        components.chap_core_external = Some(crate::components::external_chap_core(url)?);
    }
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
    // A re-init resets the component set from the flags and the defaults, so the
    // DHIS2 pin it records can move while the database the previous one created
    // is still on this machine. DHIS2 migrates a schema forward only, so that is
    // said before anything is written rather than found out at `chaps up`.
    if let Some(previous) = &previous
        && let Some(note) = crate::commands::components::dhis2_tag_moved(previous, &components)
    {
        crate::output::warn(&note);
    }
    // Settle the chap-core tag and the compose file that goes with it before
    // anything is written: both need the network, and a failure of either is a
    // warning plus a fallback, never a half-written directory.
    // A deployment without chap-core asks GitHub nothing: the tag is recorded
    // as given and `compose.yml` would come from the embedded copy, which is
    // what `chaps components enable chap-core` renders and `chaps update`
    // then moves to a release.
    if checkout.is_some() && !components.chap_core.enabled {
        return Err(anyhow::anyhow!(
            "--source builds chap-core, and this run leaves chap-core out; \
             drop --source, or keep chap-core in the component set"
        ));
    }
    let chap_core = if let Some(path) = &checkout {
        // Nothing to look up: the checkout is the version.
        ChapCore {
            tag: CHECKOUT_TAG.to_string(),
            source: ComposeSource::Checkout { path: path.clone() },
            cached: None,
        }
    } else if components.chap_core.enabled {
        resolve_chap_core(ctx, &dir, &args.chap_tag)
    } else {
        ChapCore {
            tag: args.chap_tag.trim().to_string(),
            source: ComposeSource::Embedded,
            cached: None,
        }
    };
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
        // The default model set is a default, not a request: it is the set a
        // CHAP deployment starts with, and a deployment without chap-core asks
        // for its model services by name.
        Selection::default()
    } else {
        parse_models(&args.models, &registry)?
    };
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

    // DHIS2's instance config goes in on the same terms, and before apply() for
    // the same reason: `dhis.conf` is mandatory - DHIS2 does not start without it
    // - and it is the operator's from the moment it exists.
    if components.dhis2.enabled
        && let Some(path) =
            write_dhis2_config(&dir, &crate::compose::spec::Dhis2ConfigSpec::default())?
    {
        written.push(path);
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
        (Component::Dhis2, crate::compose::DHIS2_SERVICE),
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
    /// `--only`: the whole set, chap-core included only when it is named.
    only: Option<&'a str>,
    ocs_base_url: Option<&'a str>,
    ocs_port: Option<ComponentPortArg>,
    s3_port: Option<ComponentPortArg>,
    dhis2_port: Option<ComponentPortArg>,
    dhis2_seed: Option<&'a str>,
    dhis2_tag: Option<&'a str>,
    dhis2_image: Option<&'a str>,
    ocs_read_only: bool,
    /// The global `--offline`, which is not a component flag but decides one
    /// thing here: a seed that has to be downloaded cannot be asked for by a run
    /// that was told not to touch the network.
    offline: bool,
}

impl<'a> ComponentFlags<'a> {
    fn from_args(args: &'a InitArgs, offline: bool) -> ComponentFlags<'a> {
        ComponentFlags {
            with: args.with.as_deref(),
            without: args.without.as_deref(),
            only: args.only.as_deref(),
            ocs_base_url: args.ocs_base_url.as_deref(),
            ocs_port: args.ocs_port,
            s3_port: args.s3_port,
            dhis2_port: args.dhis2_port,
            dhis2_seed: args.dhis2_seed.as_deref(),
            dhis2_tag: args.dhis2_tag.as_deref(),
            dhis2_image: args.dhis2_image.as_deref(),
            ocs_read_only: args.ocs_read_only,
            offline,
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
    let (on, off) = match flags.only {
        Some(only) => only_lists(only)?,
        None => (
            parse_component_list(flags.with)?,
            parse_component_list(flags.without)?,
        ),
    };
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
    if let Some(port) = flags.dhis2_port {
        if !components.dhis2.enabled {
            return Err(anyhow::anyhow!(
                "--dhis2-port needs the dhis2 component; add `--with dhis2`"
            ));
        }
        components.dhis2.port = port.0;
    }
    // Before the seed, which `default` resolves against the version.
    if let Some(tag) = flags.dhis2_tag.map(str::trim) {
        if !components.dhis2.enabled {
            return Err(anyhow::anyhow!(
                "--dhis2-tag needs the dhis2 component; add `--with dhis2`"
            ));
        }
        components.dhis2.image_tag = crate::components::dhis2_tag_arg(tag, "--dhis2-tag")?;
    }
    if let Some(image) = flags.dhis2_image.map(str::trim) {
        if !components.dhis2.enabled {
            return Err(anyhow::anyhow!(
                "--dhis2-image needs the dhis2 component; add `--with dhis2`"
            ));
        }
        components.dhis2.image =
            crate::components::dhis2_image_arg(image, "--dhis2-image", "--dhis2-tag")?;
    }
    if let Some(given) = flags.dhis2_seed {
        if !components.dhis2.enabled {
            return Err(anyhow::anyhow!(
                "--dhis2-seed needs the dhis2 component; add `--with dhis2`"
            ));
        }
        let seed = crate::components::Dhis2Seed::parse(given);
        // The dump is downloaded by the one-shot on the first `chaps up`, not
        // here, so this refusal is about the deployment being written rather than
        // about this run's network: `--offline` is a deployment that does not
        // reach out, and recording a URL would make its first start do exactly
        // that.
        if flags.offline && seed.is_url() {
            return Err(anyhow::anyhow!(
                "--offline and `--dhis2-seed {}` ask for opposite things: the first `chaps up` \
                 would download that dump; pass a path to a dump you already have, or \
                 `--dhis2-seed none`",
                seed.as_str()
            ));
        }
        components.dhis2.seed = seed;
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
/// `--only LIST` as the `--with` and `--without` lists it stands for: the
/// named components on and every other one off. `none` is the empty set, for a
/// deployment of model services alone.
fn only_lists(spec: &str) -> Result<(Vec<Component>, Vec<Component>)> {
    let on = if spec.trim() == "none" {
        Vec::new()
    } else {
        let on = parse_component_list(Some(spec))?;
        if on.is_empty() {
            return Err(anyhow::anyhow!(
                "--only needs a component list, such as `--only ocs,s3`, or `--only none` \
                 for model services alone"
            ));
        }
        on
    };
    let off = Component::ALL
        .iter()
        .copied()
        .filter(|c| !on.contains(c))
        .collect();
    Ok((on, off))
}

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

/// What `.chaps/project.yaml` records as the chap-core tag of a deployment
/// built from a checkout: there is no release behind it.
const CHECKOUT_TAG: &str = "checkout";

/// The absolute path of a chap-core checkout, refused unless it has what a
/// build needs: the API's `Dockerfile`, the worker's `Dockerfile.worker` and
/// the `compose.ghcr.yml` that `compose.yml` is rendered from.
fn checkout_source(path: &Path) -> Result<String> {
    let path = std::path::absolute(path)
        .map_err(|e| anyhow::anyhow!("resolving {}: {e}", path.display()))?;
    let missing: Vec<&str> = [
        "Dockerfile",
        "Dockerfile.worker",
        crate::project::CHECKOUT_COMPOSE,
    ]
    .into_iter()
    .filter(|name| !path.join(name).is_file())
    .collect();
    if !missing.is_empty() {
        return Err(anyhow::anyhow!(
            "{} is not a chap-core checkout: it has no {}; pass the directory you cloned \
             github.com/dhis2-chap/chap-core into",
            path.display(),
            missing.join(", ")
        ));
    }
    Ok(path.to_string_lossy().into_owned())
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
        // Every model the catalogue lists, templates aside: those are
        // scaffolding to copy, not models to run.
        "all" => registry
            .models
            .iter()
            .filter(|model| !model.is_template())
            .map(|model| model.id.clone())
            .collect(),
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
            if let Some(problem) = crate::dotenv::literal_problem(given) {
                return Err(crate::error::ChapError::Usage(format!(
                    "--api-token contains {problem}; choose another, or pass --api-token with no \
                     value and one is generated"
                ))
                .into());
            }
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
    if components.dhis2.enabled {
        let reach = components.dhis2_reach();
        addresses.push_str(&format!(
            "{}     {} {}\n",
            out.key("DHIS2:"),
            match components.dhis2.port {
                Some(_) => out.value(&reach),
                None => out.dim(&reach),
            },
            out.dim(&format!(
                "(config in {}/{})",
                crate::components::DHIS2_DIR,
                crate::components::DHIS2_CONFIG_FILE
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
    // No part of the token is printed: it stays in `.env`, and
    // `chaps auth show --reveal` is the way back to it.
    if secrets.is_some() {
        text.push_str(&format!(
            "{} {} {}\n",
            out.key("API token:"),
            out.value("generated into .env"),
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
        if let Some(line) = no_models_line(components) {
            text.push_str(&format!("\n{}\n", out.backticks(line)));
        }
    } else {
        text.push_str(&format!("\n{}\n", out.heading("Enabled:")));
        // A table without headers, so ids of different lengths line up.
        let rows: Vec<Vec<String>> = report
            .enabled
            .iter()
            .map(|(id, model)| {
                let name = registry
                    .get(id)
                    .map(|m| m.display_name.clone())
                    .unwrap_or_else(|| id.clone());
                let reach = match model.host_port {
                    Some(port) => out.value(&format!("http://localhost:{port}")),
                    None => out.dim("internal"),
                };
                vec![
                    id.clone(),
                    format!("{name} {}", out.dim(&format!("v{}", model.version))),
                    reach,
                ]
            })
            .collect();
        for line in out.table(&[], &rows).lines() {
            text.push_str(&format!("  {line}\n"));
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
    // What the first start does with the database, and how long it takes. Both
    // are things nothing else on the screen says and the reader has to know
    // before they run `chaps up`.
    if components.dhis2.enabled {
        let mut dhis2 = Vec::new();
        // The warning block below says it with the reason when the pinned minor
        // has no dump, so this line would be the same sentence twice.
        if !components.dhis2_seed_is_unknown() {
            dhis2.push(crate::components::dhis2_seed_note(
                components.dhis2_seed_source(),
            ));
        }
        dhis2.push(DHIS2_FIRST_START_NOTE.to_string());
        // Only with a chap-core to connect it to.
        if components.has_chap_core_api() {
            dhis2.push(crate::components::DHIS2_CONNECT_NOTE.to_string());
        }
        text.push('\n');
        for note in dhis2 {
            text.push_str(&format!("{} {note}\n", out.dim("note:")));
        }
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

    // The last thing on the screen is the thing to type next.
    text.push('\n');
    text.push_str(&format!(
        "{}\n  {}\n  {}",
        out.heading("Next:"),
        out.cmd(&format!("cd {} && chaps up", dir.display())),
        out.cmd("chaps status")
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
mod tests;
