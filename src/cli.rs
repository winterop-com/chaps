//! The `chaps` command tree.
//!
//! Doc comments on the types and fields are the `--help` text. This file and
//! `main.rs` are complete: the command modules only read the arg structs.

use crate::registry::{Channel, DEFAULT_REGISTRY_URL};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// Generate and manage a docker compose deployment of chap-core plus
/// marketplace model services.
#[derive(Debug, Parser)]
#[command(name = "chaps", version, about, long_about = None)]
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

    /// Open the model browser (alias of `chaps models tui`).
    Tui(TuiArgs),

    /// Inspect and refresh the marketplace registry.
    Registry(RegistryArgs),

    /// Render the compose files from .chaps/ (what `up` does first).
    Sync(SyncArgs),

    /// Move channel-following models to the versions the marketplace now
    /// publishes, then pull and restart.
    Update(UpdateArgs),

    /// Sync, then start the stack (docker compose up).
    Up(UpArgs),

    /// Stop the stack (docker compose down).
    Down(DownArgs),

    /// List the stack's containers (docker compose ps).
    Ps(PsArgs),

    /// Show container logs (docker compose logs).
    Logs(LogsArgs),

    /// Pull every image the stack uses (docker compose pull).
    Pull(PullArgs),

    /// Run an arbitrary docker compose command against the project.
    Compose(ComposeArgs),

    /// Check chap-core health and which model services have registered.
    Status(StatusArgs),
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

    /// Image tag for the chap-core services.
    #[arg(long, value_name = "TAG", default_value = "latest")]
    pub chap_tag: String,

    /// Overwrite an existing project in the target directory.
    #[arg(long)]
    pub force: bool,

    /// Lowest host port model overlays may be published on.
    #[arg(long, value_name = "PORT", default_value_t = 5001)]
    pub port_base: u16,

    /// Do not write a .env file.
    #[arg(long)]
    pub no_env: bool,

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

    /// Open the model browser.
    Tui(TuiArgs),
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

    /// Host port to publish the service on; allocated automatically otherwise.
    #[arg(long, value_name = "PORT")]
    pub port: Option<u16>,

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

/// Open the model browser.
#[derive(Debug, Clone, Args)]
pub struct TuiArgs {}

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

/// Move channel-following models to the versions the marketplace now
/// publishes, then pull and restart.
///
/// Always fetches the registry from the network (no cache, no fallback). Models
/// pinned to an exact version are listed but not moved. After updating
/// .chaps/models.yaml and the .env pin comments it runs `chaps sync`,
/// `docker compose pull` and `docker compose up -d`. chap-core itself follows
/// its tag (`latest` by default), so the pull refreshes it too.
#[derive(Debug, Clone, Args)]
pub struct UpdateArgs {
    /// Show what would change and write nothing.
    #[arg(long)]
    pub dry_run: bool,

    /// Update the files and pull the images but do not run `up -d`.
    #[arg(long)]
    pub no_restart: bool,
}

/// Sync, then start the stack (docker compose up).
#[derive(Debug, Clone, Args)]
pub struct UpArgs {
    /// Stay attached to the container output instead of detaching.
    #[arg(long)]
    pub attach: bool,

    /// Pull every image first, including a moving chap-core tag such as
    /// `latest` (docker compose up --pull always).
    #[arg(long)]
    pub pull: bool,

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

/// List the stack's containers (docker compose ps).
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

/// Pull every image the stack uses (docker compose pull).
#[derive(Debug, Clone, Args)]
pub struct PullArgs {}

/// Run an arbitrary docker compose command against the project.
#[derive(Debug, Clone, Args)]
pub struct ComposeArgs {
    /// Arguments passed to docker compose after the project's -f list.
    #[arg(
        value_name = "ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub args: Vec<String>,
}

/// Check chap-core health and which model services have registered.
#[derive(Debug, Clone, Args)]
pub struct StatusArgs {
    /// Base URL of the chap-core API.
    #[arg(long, value_name = "URL", default_value = "http://localhost:8000")]
    pub url: String,

    /// Request timeout in seconds.
    #[arg(long, value_name = "SECONDS", default_value_t = 5)]
    pub timeout: u64,
}

/// The docker compose wrappers, as one value for `commands::docker::run`.
#[derive(Debug, Clone)]
pub enum DockerCmd {
    Up(UpArgs),
    Down(DownArgs),
    Ps(PsArgs),
    Logs(LogsArgs),
    Pull(PullArgs),
    Compose(ComposeArgs),
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
        assert_eq!(args.port_base, 5001);
        assert!(!args.force && !args.no_env && !args.interactive);
        assert!(args.source.is_none());
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
        assert_eq!(args.port, Some(5010));
        assert_eq!(args.data_dir.as_deref(), Some("/app/data"));
        assert_eq!(args.user.as_deref(), Some("chap:chap"));
        assert!(args.allow_template);
        assert_eq!(args.channel, None);
    }

    #[test]
    fn compose_passes_hyphenated_args_through() {
        let cli =
            Cli::try_parse_from(["chap", "compose", "config", "--services", "--quiet"]).unwrap();
        let Command::Compose(args) = cli.command else {
            panic!("expected compose");
        };
        assert_eq!(args.args, vec!["config", "--services", "--quiet"]);
    }

    #[test]
    fn up_takes_a_flag_then_extra_args() {
        let cli = Cli::try_parse_from(["chap", "up", "--attach", "chap"]).unwrap();
        let Command::Up(args) = cli.command else {
            panic!("expected up");
        };
        assert!(args.attach);
        assert!(!args.pull);
        assert_eq!(args.extra, vec!["chap"]);

        let cli = Cli::try_parse_from(["chap", "up", "--pull"]).unwrap();
        let Command::Up(args) = cli.command else {
            panic!("expected up");
        };
        assert!(args.pull && !args.attach);
        assert!(args.extra.is_empty());
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

    #[test]
    fn status_defaults() {
        let cli = Cli::try_parse_from(["chap", "status"]).unwrap();
        let Command::Status(args) = cli.command else {
            panic!("expected status");
        };
        assert_eq!(args.url, "http://localhost:8000");
        assert_eq!(args.timeout, 5);
    }

    #[test]
    fn tui_exists_both_at_the_top_level_and_under_models() {
        assert!(matches!(
            Cli::try_parse_from(["chap", "tui"]).unwrap().command,
            Command::Tui(_)
        ));
        let cli = Cli::try_parse_from(["chap", "models", "tui"]).unwrap();
        let Command::Models(m) = cli.command else {
            panic!("expected models");
        };
        assert!(matches!(m.command, ModelsCmd::Tui(_)));
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
