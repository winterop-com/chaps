//! `chaps init` — write a deployment directory.
//!
//! Owned by agent B.

use crate::chapcore;
use crate::cli::InitArgs;
use crate::commands::Ctx;
use crate::compose::spec::EnvSpec;
use crate::compose::{ApplyReport, EnableRequest, Selection, apply, render_env};
use crate::error::{ChapError, Result};
use crate::project::{
    CHAPS_DIR, ComposeSource, DEFAULT_PORT_RANGE, ENV_FILE, MODELS_FILE, PROJECT_FILE, Project,
    ProjectState, cached_compose_file,
};
use crate::registry::{self, Registry};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

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

    let registry = registry::load(&ctx.registry)?;
    // Settle the chap-core tag and the compose file that goes with it before
    // anything is written: both need the network, and a failure of either is a
    // warning plus a fallback, never a half-written directory.
    let chap_core = resolve_chap_core(ctx, &dir, &args.chap_tag);
    let state = ProjectState {
        chap_image_tag: chap_core.tag.clone(),
        chap_compose_source: chap_core.source.clone(),
        registry_url: ctx.registry.url.clone(),
        port_range: (args.port_base, DEFAULT_PORT_RANGE.1.max(args.port_base)),
        ..ProjectState::default()
    };
    let mut project = Project {
        dir: dir.clone(),
        state,
    };

    // Decide what to enable before writing anything, so a cancelled browser
    // or a typo in --models leaves no half-written directory behind.
    let selection = if args.interactive {
        // Quitting the browser without saving is "no models", not an error.
        crate::tui::run_tui(ctx, &project, &registry)?.unwrap_or_default()
    } else {
        parse_models(&args.models, &registry)?
    };

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

    if env == EnvAction::Written {
        if env_exists {
            crate::output::warn(
                "--fresh-env rewrote .env with a new POSTGRES_PASSWORD; a database volume \
                 from an earlier `chaps up` still holds the old one - drop it with \
                 `chaps docker run -- down -v` or change the role with ALTER USER",
            );
        }
        std::fs::write(
            &env_path,
            render_env(&EnvSpec {
                postgres_user: POSTGRES_USER.to_string(),
                postgres_password: random_password()?,
                postgres_db: POSTGRES_DB.to_string(),
                chap_image_tag: Some(chap_core.tag.clone()),
                // apply() appends one commented pin per enabled model.
                model_tag_pins: Vec::new(),
                cli_version: ctx.cli_version.to_string(),
            }),
        )
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", env_path.display()))?;
        written.push(env_path.clone());
    }

    // apply() writes the overlays, compose.marketplace.yml (even with no
    // models) and .chaps/.
    let mut report = apply(&mut project, &registry, &selection, ctx.cli_version)?;
    report.removed.extend(stale);
    written.extend(report.written.iter().cloned());
    written.push(dir.join(CHAPS_DIR).join(PROJECT_FILE));
    written.push(dir.join(CHAPS_DIR).join(MODELS_FILE));
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
        "report": report,
    });
    ctx.out.emit(&value, || {
        summary(&dir, &written, &report, &registry, env, &chap_core)
    })
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
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

/// 32 lowercase hex characters, so the generated password is URL-safe inside
/// the postgres URL chap-core composes from POSTGRES_*.
fn random_password() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| anyhow::anyhow!("generating a database password: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn summary(
    dir: &Path,
    written: &[PathBuf],
    report: &ApplyReport,
    registry: &Registry,
    env: EnvAction,
    chap_core: &ChapCore,
) -> String {
    let mut out = format!(
        "Initialized a chaps project in {}\n\nWrote:\n",
        dir.display()
    );
    for path in written {
        out.push_str(&format!("  {}\n", file_label(dir, path)));
    }
    if env == EnvAction::Kept {
        out.push_str("\nkept .env (already present)\n");
    }
    out.push_str(&format!(
        "\nchap-core: {} ({})\n",
        chap_core.tag,
        chap_core.source.describe()
    ));

    if report.enabled.is_empty() {
        out.push_str("\nNo models enabled; run `chaps models enable ID` to add one.\n");
    } else {
        out.push_str("\nEnabled:\n");
        for (id, model) in &report.enabled {
            let name = registry
                .get(id)
                .map(|m| m.display_name.clone())
                .unwrap_or_else(|| id.clone());
            out.push_str(&format!(
                "  {id}  {name} v{}  http://localhost:{}\n",
                model.version, model.host_port
            ));
        }
    }
    for warning in &report.warnings {
        out.push_str(&format!("\nwarning: {warning}\n"));
    }

    out.push_str(&format!(
        "\nNext:\n  cd {} && chaps up\n  chaps status\n",
        dir.display()
    ));
    out
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
    fn resolve_dir_drops_cur_dir_components() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(resolve_dir(Path::new(".")).unwrap(), cwd);
        assert_eq!(
            resolve_dir(Path::new("./chapx")).unwrap(),
            cwd.join("chapx")
        );
        assert_eq!(
            resolve_dir(Path::new("/tmp/chapx")).unwrap(),
            PathBuf::from("/tmp/chapx")
        );
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
