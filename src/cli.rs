//! The `chaps` command tree.
//!
//! Doc comments on the types and fields are the `--help` text. This file and
//! `main.rs` are complete: the command modules only read the arg structs.

use crate::compose::PortRequest;
use crate::project::DEFAULT_API_PORT;
use crate::registry::{Channel, DEFAULT_REGISTRY_URL};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// The value of `--port`: a number, or `auto` for the lowest free one.
///
/// Parsed here rather than in the command so a typo is a clap error next to
/// the flag, not a failure halfway through a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortArg(pub PortRequest);

impl std::str::FromStr for PortArg {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<PortArg, String> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("auto") {
            return Ok(PortArg(PortRequest::Auto));
        }
        match text.parse::<u16>() {
            Ok(0) => Err("0 is not a host port; pass a number or `auto`".to_string()),
            Ok(port) => Ok(PortArg(PortRequest::Fixed(port))),
            Err(_) => Err(format!("`{text}` is neither a port number nor `auto`")),
        }
    }
}

/// The one-liner `chaps -h` opens with.
const ABOUT: &str = "deploy and manage CHAP, the DHIS2 Climate Health Analytics Platform";

/// The header `chaps --help` opens with. Clap renders `long_about` for the long
/// help and falls back to `about` for the short one, so both spellings of help
/// say what CHAP is.
const LONG_ABOUT: &str = "\
chaps deploys and manages CHAP, the DHIS2 Climate Health Analytics Platform.

It runs chap-core, its worker and database with Docker Compose, and adds
forecasting models from the CHAP model marketplace as pinned overlays.
Start with `chaps init`. Docs: https://chap.dhis2.org";

/// Deploy and manage CHAP, the DHIS2 Climate Health Analytics Platform.
#[derive(Debug, Parser)]
#[command(name = "chaps", version, about = ABOUT, long_about = LONG_ABOUT)]
pub struct Cli {
    /// Emit machine-readable JSON instead of human output.
    #[arg(long, global = true)]
    pub json: bool,

    /// Project directory (or any directory inside one); found like git finds .git.
    #[arg(
        short = 'C',
        long,
        global = true,
        value_name = "DIR",
        default_value = "."
    )]
    pub project_dir: PathBuf,

    /// URL of the marketplace registry index.
    #[arg(long, global = true, value_name = "URL", default_value = DEFAULT_REGISTRY_URL)]
    pub registry_url: String,

    /// Never touch the network; use the cache or the embedded snapshot.
    #[arg(long, global = true)]
    pub offline: bool,

    /// Directory for the cached registry snapshot.
    #[arg(long, global = true, value_name = "DIR")]
    pub cache_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a deployment directory: compose files, .env and .chaps/.
    Init(InitArgs),

    /// Browse and manage marketplace models.
    Models(ModelsArgs),

    /// Open the model browser.
    Ui(UiArgs),

    /// Inspect and refresh the marketplace registry.
    Registry(RegistryArgs),

    /// Render the compose files from .chaps/ (what `up` does first).
    Sync(SyncArgs),

    /// Move the model and chap-core pins to what upstream publishes now, then
    /// pull and restart.
    Update(UpdateArgs),

    /// Sync, then start the stack (docker compose up).
    Up(UpArgs),

    /// Stop the stack (docker compose down).
    Down(DownArgs),

    /// Show container logs (docker compose logs).
    Logs(LogsArgs),

    /// Talk to Docker directly: containers, images, raw compose commands.
    Docker(DockerArgs),

    /// Create or restore a backup of the database, model data and project files.
    Backup(BackupArgs),

    /// Check chap-core health and which model services have registered.
    Status(StatusArgs),

    /// Print the whole command tree as Markdown (writes docs/reference.md).
    #[command(hide = true)]
    DocsMarkdown(DocsMarkdownArgs),
}

/// Create a deployment directory: compose files, .env and .chaps/.
#[derive(Debug, Clone, Args)]
pub struct InitArgs {
    /// Directory to create, resolved against the current working directory.
    #[arg(value_name = "DIR", default_value = ".")]
    pub dir: PathBuf,

    /// Models to enable: `none`, `default`, or a comma-separated list of ids.
    #[arg(long, value_name = "SPEC", default_value = "default")]
    pub models: String,

    /// Pick the models in the browser instead of taking them from --models.
    #[arg(long)]
    pub interactive: bool,

    /// Image tag for the chap-core services: `latest` (the default, resolved
    /// to the newest release's `vX.Y.Z` so the deployment is reproducible),
    /// `master`, `dev`, or an exact `vX.Y.Z`. `chaps` also writes the
    /// `compose.ghcr.yml` that tag publishes; `--offline` keeps the tag as
    /// given and uses the compose file built into this binary.
    #[arg(long, value_name = "TAG", default_value = "latest")]
    pub chap_tag: String,

    /// Overwrite an existing project in the target directory.
    #[arg(long)]
    pub force: bool,

    /// Host port to publish chap-core's API on. It is the only port the
    /// deployment publishes: model services are reached through it.
    #[arg(long, value_name = "PORT", default_value_t = DEFAULT_API_PORT)]
    pub api_port: u16,

    /// Lowest host port `chaps models expose` may publish a model on.
    #[arg(long, value_name = "PORT", default_value_t = 5001)]
    pub port_base: u16,

    /// Do not write a .env file.
    #[arg(long)]
    pub no_env: bool,

    /// Regenerate .env even when the directory already has one, rotating the
    /// database password. An existing database volume keeps the old one.
    #[arg(long, conflicts_with = "no_env")]
    pub fresh_env: bool,

    /// Build chap-core from a local checkout instead of the published images.
    /// Not supported yet; passing it is an error.
    #[arg(long, value_name = "PATH")]
    pub source: Option<PathBuf>,
}

/// Browse and manage marketplace models.
#[derive(Debug, Args)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub command: ModelsCmd,
}

#[derive(Debug, Subcommand)]
pub enum ModelsCmd {
    /// List marketplace models.
    List(ModelsListArgs),

    /// Search the marketplace by id, name or summary.
    Search(ModelsSearchArgs),

    /// Show everything known about one model.
    Info(ModelsInfoArgs),

    /// Enable a model: record it in .chaps/models.yaml and write its overlay.
    Enable(ModelsEnableArgs),

    /// Disable a model: drop it from .chaps/models.yaml and remove its overlay.
    Disable(ModelsDisableArgs),

    /// Publish a host port for an enabled model.
    Expose(ModelsExposeArgs),

    /// Take an enabled model's host port away again.
    Unexpose(ModelsUnexposeArgs),
}

/// List marketplace models.
#[derive(Debug, Clone, Args)]
pub struct ModelsListArgs {
    /// Include every entry, templates as well as models.
    #[arg(long)]
    pub all: bool,

    /// List only templates.
    #[arg(long)]
    pub templates: bool,

    /// List only the models enabled in this project.
    #[arg(long)]
    pub enabled: bool,
}

/// Search the marketplace by id, name or summary.
#[derive(Debug, Clone, Args)]
pub struct ModelsSearchArgs {
    /// Text to look for; matching is case-insensitive.
    #[arg(value_name = "QUERY")]
    pub query: String,
}

/// Show everything known about one model.
#[derive(Debug, Clone, Args)]
pub struct ModelsInfoArgs {
    /// Marketplace id or service id.
    #[arg(value_name = "ID")]
    pub id: String,
}

/// Enable a model: record it in .chaps/models.yaml and write its overlay.
///
/// The model gets no host port: chap-core reaches it over the compose network
/// and a human through `/v2/services/<id>/run/`. `--port` opts in to one.
#[derive(Debug, Clone, Args)]
pub struct ModelsEnableArgs {
    /// Marketplace id or service id.
    #[arg(value_name = "ID")]
    pub id: String,

    /// Follow a release channel instead of pinning a version.
    #[arg(long, value_name = "CHANNEL", conflicts_with = "version")]
    pub channel: Option<Channel>,

    /// Pin an exact version from the model's `versions` list.
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,

    /// Publish the service on this host port, or on the lowest free one with
    /// `auto`. Without it the model publishes nothing.
    #[arg(long, value_name = "PORT|auto")]
    pub port: Option<PortArg>,

    /// Data directory inside the container, for images that differ from the default.
    #[arg(long, value_name = "PATH")]
    pub data_dir: Option<String>,

    /// User the container runs as, as `user:group`.
    #[arg(long, value_name = "USER")]
    pub user: Option<String>,

    /// Enable a template even though it is scaffolding, not a model.
    #[arg(long)]
    pub allow_template: bool,
}

/// Disable a model: drop it from .chaps/models.yaml and remove its overlay.
#[derive(Debug, Clone, Args)]
pub struct ModelsDisableArgs {
    /// Marketplace id or service id.
    #[arg(value_name = "ID")]
    pub id: String,
}

/// Publish a host port for a model this project already enabled.
///
/// The model keeps the version it is pinned to: this only rewrites its
/// overlay's `ports:`, so it is safe on a deployment that is running a build
/// you do not want moved. Run `chaps up` afterwards to apply it.
#[derive(Debug, Clone, Args)]
pub struct ModelsExposeArgs {
    /// Marketplace id or service id.
    #[arg(value_name = "ID")]
    pub id: String,

    /// Host port to publish on; `auto` (the default) takes the lowest free one.
    #[arg(long, value_name = "PORT|auto")]
    pub port: Option<PortArg>,
}

/// Take a model's host port away again.
///
/// It stays registered with chap-core and reachable at
/// `/v2/services/<id>/run/`; only the host mapping goes. Run `chaps up`
/// afterwards to apply it.
#[derive(Debug, Clone, Args)]
pub struct ModelsUnexposeArgs {
    /// Marketplace id or service id.
    #[arg(value_name = "ID")]
    pub id: String,
}

/// Open the model browser.
#[derive(Debug, Clone, Args)]
pub struct UiArgs {}

/// Inspect and refresh the marketplace registry.
#[derive(Debug, Args)]
pub struct RegistryArgs {
    #[command(subcommand)]
    pub command: RegistryCmd,
}

#[derive(Debug, Subcommand)]
pub enum RegistryCmd {
    /// Fetch the registry from the network and refresh the cache.
    Update(RegistryUpdateArgs),

    /// Show where the registry was loaded from and what it contains.
    Show(RegistryShowArgs),
}

/// Fetch the registry from the network and refresh the cache.
#[derive(Debug, Clone, Args)]
pub struct RegistryUpdateArgs {}

/// Show where the registry was loaded from and what it contains.
#[derive(Debug, Clone, Args)]
pub struct RegistryShowArgs {}

/// Render the compose files from .chaps/ (what `up` does first).
///
/// Writes every enabled model's overlay and compose.marketplace.yml, removes
/// overlays of models that are no longer enabled, and appends missing image
/// pin comments to .env. Files that already match are left alone.
#[derive(Debug, Clone, Args)]
pub struct SyncArgs {
    /// Write nothing; exit non-zero if anything would change.
    #[arg(long)]
    pub check: bool,
}

/// Move the model and chap-core pins to what upstream publishes now, then pull
/// and restart.
///
/// Always fetches the registry from the network (no cache, no fallback). Models
/// pinned to an exact version are listed but not moved. After updating
/// .chaps/models.yaml and the .env pin comments it runs `chaps sync`,
/// `docker compose pull` and `docker compose up -d`. A chap-core pin on a
/// release tag moves to the newest release, compose file included; a moving
/// tag (`latest`, `master`, `dev`) is only refreshed by the pull.
#[derive(Debug, Clone, Args)]
pub struct UpdateArgs {
    /// Show what would change and write nothing.
    #[arg(long)]
    pub dry_run: bool,

    /// Update the files and pull the images but do not run `up -d`.
    #[arg(long)]
    pub no_restart: bool,

    /// Turn a moving chap-core tag (`latest`, `master`, `dev`) into a pin on
    /// the newest release, the way `chaps init` does by default.
    #[arg(long)]
    pub pin_chap_core: bool,
}

/// Sync, then start the stack (docker compose up).
///
/// Before invoking Docker it checks that the host ports the stack publishes
/// are free, and says what to do about any that are not; `--no-preflight`
/// skips that.
#[derive(Debug, Clone, Args)]
pub struct UpArgs {
    /// Run in the foreground and stream all logs (Ctrl-C stops the stack).
    #[arg(short = 'a', long, visible_alias = "foreground")]
    pub attach: bool,

    /// Pull every image first, including a moving chap-core tag such as
    /// `latest` (docker compose up --pull always).
    #[arg(long)]
    pub pull: bool,

    /// Do not check the host ports first; let Docker report a conflict.
    #[arg(long)]
    pub no_preflight: bool,

    /// Extra arguments passed through to docker compose up.
    #[arg(
        value_name = "EXTRA",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub extra: Vec<String>,
}

/// Stop the stack (docker compose down).
#[derive(Debug, Clone, Args)]
pub struct DownArgs {
    /// Extra arguments passed through to docker compose down.
    #[arg(
        value_name = "EXTRA",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub extra: Vec<String>,
}

/// Show container logs (docker compose logs).
#[derive(Debug, Clone, Args)]
pub struct LogsArgs {
    /// Keep streaming new output.
    #[arg(short = 'f', long)]
    pub follow: bool,

    /// Services to show logs for; all of them when omitted.
    #[arg(value_name = "SERVICE")]
    pub services: Vec<String>,
}

/// Talk to Docker directly: containers, images, raw compose commands.
///
/// The everyday verbs (`up`, `down`, `logs`) are at the top level; this group
/// holds the plumbing you only reach for when you already know what Docker is
/// doing.
#[derive(Debug, Args)]
#[command(arg_required_else_help = true)]
pub struct DockerArgs {
    #[command(subcommand)]
    pub command: DockerSub,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DockerSub {
    /// List the containers this project is running (docker compose ps).
    Ps(PsArgs),

    /// Download the pinned images into the local Docker daemon (no files change).
    Pull(PullArgs),

    /// Run a command inside one of the running containers.
    Exec(ExecArgs),

    /// Run any docker compose command against this project's files.
    Run(RunArgs),

    /// Print the finished stack: every compose file merged into one document.
    Config(ConfigArgs),
}

/// List the containers this project is running (docker compose ps).
#[derive(Debug, Clone, Args)]
pub struct PsArgs {
    /// Extra arguments passed through to docker compose ps.
    #[arg(
        value_name = "EXTRA",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub extra: Vec<String>,
}

/// Download the pinned images into the local Docker daemon (no files change).
#[derive(Debug, Clone, Args)]
pub struct PullArgs {}

/// Run a command inside one of the running containers.
///
/// The container has to be up already; start the stack with `chaps up` first.
/// Without a CMD you get a shell. When this command's own input is not a
/// terminal - in a script, or behind a pipe - `-T` is passed to compose so it
/// does not try to allocate one.
#[derive(Debug, Clone, Args)]
pub struct ExecArgs {
    /// Service to run the command in, as named in the compose files.
    #[arg(value_name = "SERVICE")]
    pub service: String,

    /// Command and arguments to run; a shell (`sh`) when omitted.
    #[arg(
        value_name = "CMD",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub cmd: Vec<String>,
}

/// Run any docker compose command against this project's files.
///
/// Everything after `run` is handed to `docker compose` unchanged, behind the
/// project's `-f` list: `chaps docker run -- restart chap` becomes
/// `docker compose -f ... -f ... restart chap`.
#[derive(Debug, Clone, Args)]
pub struct RunArgs {
    /// Arguments passed to docker compose after the project's -f list.
    #[arg(
        value_name = "ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub args: Vec<String>,
}

/// Print the finished stack: every compose file merged into one document.
///
/// This is what Docker actually reads after the `-f` list, the `include:` and
/// the `.env` substitutions have been applied - useful when a setting is not
/// taking effect. With `--json` the document is printed as JSON.
#[derive(Debug, Clone, Args)]
pub struct ConfigArgs {
    /// Extra arguments passed through to docker compose config.
    #[arg(
        value_name = "EXTRA",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub extra: Vec<String>,
}

/// Create or restore a backup of the database, model data and project files.
///
/// One archive holds everything a deployment is: `.env`, `.chaps/`, the compose
/// files, a `pg_dump` of the chap-core database and one tar per model data
/// volume. It is a plain `tar.gz` - `tar -tzf` lists it, and the docs say
/// which raw `pg_restore` and `tar` commands put it back without `chaps`.
#[derive(Debug, Args)]
#[command(arg_required_else_help = true)]
pub struct BackupArgs {
    #[command(subcommand)]
    pub command: BackupSub,
}

#[derive(Debug, Clone, Subcommand)]
pub enum BackupSub {
    /// Write a tar.gz of the database, the model data and the project files.
    Create(BackupCreateArgs),

    /// Put a deployment back from an archive.
    Restore(RestoreArgs),
}

/// Write a tar.gz of the database, the model data and the project files.
///
/// The database part needs a running postgres (`chaps up`); each model's data
/// is read straight from its volume by the overlay's one-shot init container,
/// so it works whether or not the model itself is running. A model that has
/// never started has no volume yet and is skipped with a warning.
#[derive(Debug, Clone, Args)]
pub struct BackupCreateArgs {
    /// Where to write the archive: a file, or a directory to name it in.
    /// Default: `chaps-backup-<project>-<YYYYMMDD-HHMMSS>.tar.gz` in the
    /// current directory.
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,

    /// Leave the chap-core database out of the archive.
    #[arg(long)]
    pub no_db: bool,

    /// Leave the model data volumes out of the archive.
    #[arg(long)]
    pub no_models: bool,
}

/// Put a deployment back from an archive `chaps backup create` wrote.
///
/// Prints what it is about to overwrite and asks before touching anything.
/// Then: stop `chap`, `worker` and the model services, write the files back and
/// sync, `pg_restore --clean` the database, refill each model's data volume,
/// and start the stack again.
#[derive(Debug, Clone, Args)]
pub struct RestoreArgs {
    /// The `tar.gz` written by `chaps backup create`.
    #[arg(value_name = "ARCHIVE")]
    pub archive: PathBuf,

    /// Skip the confirmation. Required when there is no terminal to ask at.
    #[arg(long)]
    pub yes: bool,

    /// Restore only `.env`, `.chaps/` and the compose files. Needs no Docker.
    #[arg(long, conflicts_with_all = ["db_only", "no_models", "no_start"])]
    pub files_only: bool,

    /// Restore only the chap-core database.
    #[arg(long, conflicts_with = "no_models")]
    pub db_only: bool,

    /// Leave the model data volumes as they are.
    #[arg(long)]
    pub no_models: bool,

    /// Do not run `docker compose up -d` at the end.
    #[arg(long)]
    pub no_start: bool,
}

/// Check chap-core health and which model services have registered.
#[derive(Debug, Clone, Args)]
pub struct StatusArgs {
    /// Base URL of the chap-core API. Default:
    /// `http://localhost:<api_port>`, the port `.chaps/project.yaml` records.
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,

    /// Request timeout in seconds.
    #[arg(long, value_name = "SECONDS", default_value_t = 5)]
    pub timeout: u64,
}

/// Print the whole command tree as Markdown.
///
/// Hidden: it documents `chaps` rather than doing anything to a deployment,
/// and `make docs-reference` is the only caller. It needs no project and no
/// Docker.
#[derive(Debug, Clone, Args)]
pub struct DocsMarkdownArgs {}

/// The docker compose wrappers, as one value for `commands::docker::run`.
///
/// Flat on purpose: `up`, `down` and `logs` are top-level commands while the
/// rest live under `chaps docker`, but all of them build one argument list and
/// run down the same path.
#[derive(Debug, Clone)]
pub enum DockerCmd {
    Up(UpArgs),
    Down(DownArgs),
    Logs(LogsArgs),
    Ps(PsArgs),
    Pull(PullArgs),
    Exec(ExecArgs),
    Run(RunArgs),
    Config(ConfigArgs),
}

impl From<&DockerSub> for DockerCmd {
    fn from(sub: &DockerSub) -> DockerCmd {
        match sub {
            DockerSub::Ps(args) => DockerCmd::Ps(args.clone()),
            DockerSub::Pull(args) => DockerCmd::Pull(args.clone()),
            DockerSub::Exec(args) => DockerCmd::Exec(args.clone()),
            DockerSub::Run(args) => DockerCmd::Run(args.clone()),
            DockerSub::Config(args) => DockerCmd::Config(args.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_tree_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn global_defaults() {
        let cli = Cli::try_parse_from(["chap", "status"]).unwrap();
        assert!(!cli.json);
        assert!(!cli.offline);
        assert_eq!(cli.project_dir, PathBuf::from("."));
        assert_eq!(cli.registry_url, DEFAULT_REGISTRY_URL);
        assert!(cli.cache_dir.is_none());
    }

    #[test]
    fn globals_are_accepted_after_the_subcommand() {
        let cli = Cli::try_parse_from(["chap", "models", "list", "--json", "--offline"]).unwrap();
        assert!(cli.json);
        assert!(cli.offline);
    }

    #[test]
    fn init_defaults() {
        let cli = Cli::try_parse_from(["chap", "init"]).unwrap();
        let Command::Init(args) = cli.command else {
            panic!("expected init");
        };
        assert_eq!(args.dir, PathBuf::from("."));
        assert_eq!(args.models, "default");
        assert_eq!(args.chap_tag, "latest");
        assert_eq!(args.api_port, 8000);
        assert_eq!(args.port_base, 5001);
        assert!(!args.force && !args.no_env && !args.fresh_env && !args.interactive);
        assert!(args.source.is_none());

        let cli = Cli::try_parse_from(["chap", "init", "--api-port", "8123"]).unwrap();
        let Command::Init(args) = cli.command else {
            panic!("expected init");
        };
        assert_eq!(args.api_port, 8123);
    }

    #[test]
    fn init_fresh_env_and_no_env_conflict() {
        assert!(Cli::try_parse_from(["chap", "init", "--fresh-env"]).is_ok());
        assert!(Cli::try_parse_from(["chap", "init", "--no-env"]).is_ok());
        assert!(Cli::try_parse_from(["chap", "init", "--fresh-env", "--no-env"]).is_err());
    }

    #[test]
    fn enable_channel_and_version_conflict() {
        let ok = Cli::try_parse_from(["chap", "models", "enable", "x", "--channel", "latest"]);
        assert!(ok.is_ok());
        let ok = Cli::try_parse_from(["chap", "models", "enable", "x", "--version", "1.0.0"]);
        assert!(ok.is_ok());
        let bad = Cli::try_parse_from([
            "chap",
            "models",
            "enable",
            "x",
            "--channel",
            "stable",
            "--version",
            "1.0.0",
        ]);
        assert!(bad.is_err(), "--channel and --version must conflict");
    }

    #[test]
    fn enable_parses_every_override() {
        let cli = Cli::try_parse_from([
            "chap",
            "models",
            "enable",
            "chapkit_minimalist_example_r",
            "--port",
            "5010",
            "--data-dir",
            "/app/data",
            "--user",
            "chap:chap",
            "--allow-template",
        ])
        .unwrap();
        let Command::Models(m) = cli.command else {
            panic!("expected models");
        };
        let ModelsCmd::Enable(args) = m.command else {
            panic!("expected enable");
        };
        assert_eq!(args.port, Some(PortArg(PortRequest::Fixed(5010))));
        assert_eq!(args.data_dir.as_deref(), Some("/app/data"));
        assert_eq!(args.user.as_deref(), Some("chap:chap"));
        assert!(args.allow_template);
        assert_eq!(args.channel, None);
    }

    /// The `ModelsCmd` behind `chap models <argv..>`.
    fn models_cmd(argv: &[&str]) -> ModelsCmd {
        let mut args = vec!["chap", "models"];
        args.extend_from_slice(argv);
        let cli = Cli::try_parse_from(args).unwrap();
        let Command::Models(m) = cli.command else {
            panic!("expected models");
        };
        m.command
    }

    #[test]
    fn a_port_is_a_number_or_auto_and_nothing_else() {
        use std::str::FromStr;

        assert_eq!(
            PortArg::from_str("5010").unwrap(),
            PortArg(PortRequest::Fixed(5010))
        );
        assert_eq!(
            PortArg::from_str("auto").unwrap(),
            PortArg(PortRequest::Auto)
        );
        assert_eq!(
            PortArg::from_str("AUTO").unwrap(),
            PortArg(PortRequest::Auto)
        );
        assert_eq!(
            PortArg::from_str(" auto ").unwrap(),
            PortArg(PortRequest::Auto)
        );
        for bad in ["", "none", "0", "-1", "70000", "50a0"] {
            assert!(PortArg::from_str(bad).is_err(), "{bad} should not parse");
        }

        // And clap rejects it next to the flag rather than later.
        assert!(Cli::try_parse_from(["chap", "models", "enable", "x", "--port", "nope"]).is_err());
    }

    #[test]
    fn expose_defaults_to_an_automatic_port_and_unexpose_takes_none() {
        let ModelsCmd::Expose(args) = models_cmd(&["expose", "chapkit_ewars_model"]) else {
            panic!("expected expose");
        };
        assert_eq!(args.id, "chapkit_ewars_model");
        assert_eq!(args.port, None, "the command fills in `auto`");

        let ModelsCmd::Expose(args) =
            models_cmd(&["expose", "chapkit-ewars-model", "--port", "5010"])
        else {
            panic!("expected expose");
        };
        assert_eq!(args.port, Some(PortArg(PortRequest::Fixed(5010))));

        let ModelsCmd::Expose(args) = models_cmd(&["expose", "x", "--port", "auto"]) else {
            panic!("expected expose");
        };
        assert_eq!(args.port, Some(PortArg(PortRequest::Auto)));

        let ModelsCmd::Unexpose(args) = models_cmd(&["unexpose", "chapkit-ewars-model"]) else {
            panic!("expected unexpose");
        };
        assert_eq!(args.id, "chapkit-ewars-model");

        for argv in [
            ["chap", "models", "expose"].as_slice(),
            ["chap", "models", "unexpose"].as_slice(),
        ] {
            assert!(Cli::try_parse_from(argv).is_err(), "the id is required");
        }
    }

    /// The `DockerSub` behind `chap docker <argv..>`.
    fn docker_sub(argv: &[&str]) -> DockerSub {
        let mut args = vec!["chap", "docker"];
        args.extend_from_slice(argv);
        let cli = Cli::try_parse_from(args).unwrap();
        let Command::Docker(d) = cli.command else {
            panic!("expected docker");
        };
        d.command
    }

    #[test]
    fn docker_run_passes_hyphenated_args_through() {
        let DockerSub::Run(args) = docker_sub(&["run", "config", "--services", "--quiet"]) else {
            panic!("expected docker run");
        };
        assert_eq!(args.args, vec!["config", "--services", "--quiet"]);
    }

    #[test]
    fn docker_needs_a_subcommand() {
        let err = Cli::try_parse_from(["chap", "docker"]).expect_err("a subcommand is required");
        // arg_required_else_help prints the whole help, not a one-line error.
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
        assert!(err.to_string().contains("exec"));
    }

    #[test]
    fn the_old_top_level_docker_commands_are_gone() {
        for argv in [
            ["chap", "ps"],
            ["chap", "pull"],
            ["chap", "compose"],
            ["chap", "exec"],
        ] {
            assert!(
                Cli::try_parse_from(argv).is_err(),
                "{argv:?} must live under `chaps docker` now"
            );
        }
    }

    #[test]
    fn docker_ps_and_pull_keep_their_shapes() {
        let DockerSub::Ps(args) = docker_sub(&["ps", "-a"]) else {
            panic!("expected docker ps");
        };
        assert_eq!(args.extra, vec!["-a"]);
        assert!(matches!(docker_sub(&["pull"]), DockerSub::Pull(_)));
    }

    #[test]
    fn docker_exec_takes_a_service_and_an_optional_command() {
        let DockerSub::Exec(args) = docker_sub(&["exec", "chap"]) else {
            panic!("expected docker exec");
        };
        assert_eq!(args.service, "chap");
        assert!(
            args.cmd.is_empty(),
            "the default command is filled in later"
        );

        // A hyphenated command reaches the container rather than clap.
        let DockerSub::Exec(args) = docker_sub(&["exec", "chap", "--", "ls", "-la", "/app"]) else {
            panic!("expected docker exec");
        };
        assert_eq!(args.service, "chap");
        assert_eq!(args.cmd, vec!["ls", "-la", "/app"]);

        assert!(
            Cli::try_parse_from(["chap", "docker", "exec"]).is_err(),
            "the service name is required"
        );
    }

    #[test]
    fn docker_config_takes_extra_arguments() {
        let DockerSub::Config(args) = docker_sub(&["config"]) else {
            panic!("expected docker config");
        };
        assert!(args.extra.is_empty());

        let DockerSub::Config(args) = docker_sub(&["config", "--", "--services"]) else {
            panic!("expected docker config");
        };
        assert_eq!(args.extra, vec!["--services"]);
    }

    #[test]
    fn up_takes_a_flag_then_extra_args() {
        let cli = Cli::try_parse_from(["chap", "up", "--attach", "chap"]).unwrap();
        let Command::Up(args) = cli.command else {
            panic!("expected up");
        };
        assert!(args.attach);
        assert!(!args.pull && !args.no_preflight);
        assert_eq!(args.extra, vec!["chap"]);

        let cli = Cli::try_parse_from(["chap", "up", "--pull"]).unwrap();
        let Command::Up(args) = cli.command else {
            panic!("expected up");
        };
        assert!(args.pull && !args.attach);
        assert!(args.extra.is_empty());

        let cli = Cli::try_parse_from(["chap", "up", "--no-preflight"]).unwrap();
        let Command::Up(args) = cli.command else {
            panic!("expected up");
        };
        assert!(args.no_preflight);
    }

    #[test]
    fn up_attach_answers_to_a_and_to_foreground() {
        for spelling in ["--attach", "-a", "--foreground"] {
            let cli = Cli::try_parse_from(["chap", "up", spelling]).unwrap();
            let Command::Up(args) = cli.command else {
                panic!("expected up");
            };
            assert!(args.attach, "`up {spelling}` runs in the foreground");
            assert!(args.extra.is_empty(), "{spelling} is a flag, not an extra");
        }

        // The alias is in the help, next to the flag it stands for.
        let help = Cli::command()
            .find_subcommand_mut("up")
            .expect("up is a subcommand")
            .render_long_help()
            .to_string();
        assert!(help.contains("--foreground"), "{help}");
        assert!(help.contains("-a"), "{help}");
        assert!(
            help.contains("Run in the foreground and stream all logs (Ctrl-C stops the stack)"),
            "{help}"
        );
    }

    #[test]
    fn sync_and_update_take_their_flags() {
        let cli = Cli::try_parse_from(["chap", "sync"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Sync(SyncArgs { check: false })
        ));
        let cli = Cli::try_parse_from(["chap", "sync", "--check"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Sync(SyncArgs { check: true })
        ));

        let cli = Cli::try_parse_from(["chap", "update"]).unwrap();
        let Command::Update(args) = cli.command else {
            panic!("expected update");
        };
        assert!(!args.dry_run && !args.no_restart);
        let cli = Cli::try_parse_from(["chap", "update", "--dry-run", "--no-restart"]).unwrap();
        let Command::Update(args) = cli.command else {
            panic!("expected update");
        };
        assert!(args.dry_run && args.no_restart);
    }

    #[test]
    fn logs_takes_follow_and_services() {
        let cli =
            Cli::try_parse_from(["chap", "logs", "-f", "chap", "chapkit-ewars-model"]).unwrap();
        let Command::Logs(args) = cli.command else {
            panic!("expected logs");
        };
        assert!(args.follow);
        assert_eq!(args.services.len(), 2);
    }

    /// The `BackupSub` behind `chap backup <argv..>`.
    fn backup_sub(argv: &[&str]) -> BackupSub {
        let mut args = vec!["chap", "backup"];
        args.extend_from_slice(argv);
        let cli = Cli::try_parse_from(args).unwrap();
        let Command::Backup(b) = cli.command else {
            panic!("expected backup");
        };
        b.command
    }

    #[test]
    fn backup_needs_a_subcommand() {
        let err = Cli::try_parse_from(["chap", "backup"]).expect_err("a subcommand is required");
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
        let help = err.to_string();
        assert!(help.contains("create") && help.contains("restore"));
    }

    #[test]
    fn backup_create_defaults_and_flags() {
        let BackupSub::Create(args) = backup_sub(&["create"]) else {
            panic!("expected backup create");
        };
        assert!(args.out.is_none() && !args.no_db && !args.no_models);

        let BackupSub::Create(args) =
            backup_sub(&["create", "--out", "/backups", "--no-db", "--no-models"])
        else {
            panic!("expected backup create");
        };
        assert_eq!(args.out, Some(PathBuf::from("/backups")));
        assert!(args.no_db && args.no_models);
    }

    #[test]
    fn backup_restore_takes_an_archive_and_its_flags() {
        let BackupSub::Restore(args) = backup_sub(&["restore", "x.tar.gz"]) else {
            panic!("expected backup restore");
        };
        assert_eq!(args.archive, PathBuf::from("x.tar.gz"));
        assert!(!args.yes && !args.files_only && !args.db_only);
        assert!(!args.no_models && !args.no_start);

        let BackupSub::Restore(args) =
            backup_sub(&["restore", "x.tar.gz", "--yes", "--db-only", "--no-start"])
        else {
            panic!("expected backup restore");
        };
        assert!(args.yes && args.db_only && args.no_start);

        assert!(
            Cli::try_parse_from(["chap", "backup", "restore"]).is_err(),
            "the archive is required"
        );
    }

    #[test]
    fn restore_scopes_that_contradict_each_other_are_rejected() {
        for argv in [
            vec!["--files-only", "--db-only"],
            vec!["--files-only", "--no-models"],
            vec!["--files-only", "--no-start"],
            vec!["--db-only", "--no-models"],
        ] {
            let mut args = vec!["chap", "backup", "restore", "x.tar.gz"];
            args.extend_from_slice(&argv);
            assert!(Cli::try_parse_from(&args).is_err(), "{argv:?}");
        }
        assert!(
            Cli::try_parse_from(["chap", "backup", "restore", "x.tar.gz", "--files-only"]).is_ok()
        );
        assert!(
            Cli::try_parse_from(["chap", "backup", "restore", "x.tar.gz", "--db-only"]).is_ok()
        );
    }

    #[test]
    fn status_defaults() {
        let cli = Cli::try_parse_from(["chap", "status"]).unwrap();
        let Command::Status(args) = cli.command else {
            panic!("expected status");
        };
        assert_eq!(
            args.url, None,
            "the default follows the project's api_port, so the command fills it in"
        );
        assert_eq!(args.timeout, 5);

        let cli = Cli::try_parse_from(["chap", "status", "--url", "http://host:9000"]).unwrap();
        let Command::Status(args) = cli.command else {
            panic!("expected status");
        };
        assert_eq!(args.url.as_deref(), Some("http://host:9000"));
    }

    #[test]
    fn the_browser_is_the_top_level_ui_command() {
        assert!(matches!(
            Cli::try_parse_from(["chap", "ui"]).unwrap().command,
            Command::Ui(_)
        ));
        // It used to be `tui`, with a `models tui` alias; neither is a command
        // any more, and no alias keeps them alive.
        for argv in [
            ["chap", "tui"].as_slice(),
            ["chap", "models", "tui"].as_slice(),
            ["chap", "models", "ui"].as_slice(),
        ] {
            assert!(Cli::try_parse_from(argv).is_err(), "{argv:?}");
        }
    }

    #[test]
    fn registry_has_update_and_show() {
        for (argv, want_update) in [
            (["chap", "registry", "update"], true),
            (["chap", "registry", "show"], false),
        ] {
            let cli = Cli::try_parse_from(argv).unwrap();
            let Command::Registry(r) = cli.command else {
                panic!("expected registry");
            };
            assert_eq!(matches!(r.command, RegistryCmd::Update(_)), want_update);
        }
    }
}
