//! The `chaps` command tree.
//!
//! Doc comments on the types and fields are the `--help` text, and every one of
//! them is a single clause: asking for help is not asking for the manual, so the
//! explanations live in the book (`docs/`) and the reference chapter is
//! generated from these one-liners. A subcommand's own help comes from its
//! variant in the enum, never from the argument struct the variant names. This
//! file and `main.rs` are complete: the command modules only read the arg
//! structs.

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

/// The one-liner both `chaps -h` and `chaps --help` open with. There is no
/// `long_about`: the two spellings of help say the same thing.
const ABOUT: &str = "deploy and manage CHAP, the Climate Health Analytics Platform";

/// The last line of `chaps --help`. Anything longer than a clause belongs in
/// the book, so the help points at it instead of repeating it.
pub const DOCS_LINE: &str = "Docs: https://winterop-com.github.io/chaps/";

/// The heading the global options are listed under, in their own block after
/// each command's own options.
const GLOBAL: &str = "Global options";

/// Deploy and manage CHAP, the Climate Health Analytics Platform.
#[derive(Debug, Parser)]
#[command(name = "chaps", version, about = ABOUT, after_help = DOCS_LINE)]
pub struct Cli {
    /// Emit machine-readable JSON instead of human output
    #[arg(long, global = true, help_heading = GLOBAL)]
    pub json: bool,

    /// Never colour the output (NO_COLOR does the same)
    #[arg(long, global = true, help_heading = GLOBAL)]
    pub no_color: bool,

    /// Show the commands and requests as they run
    #[arg(short, long, global = true, help_heading = GLOBAL)]
    pub verbose: bool,

    /// --verbose plus raw responses
    #[arg(short, long, global = true, help_heading = GLOBAL)]
    pub debug: bool,

    /// Project directory, or any directory inside one
    #[arg(
        short = 'C',
        long,
        global = true,
        value_name = "DIR",
        default_value = ".",
        help_heading = GLOBAL
    )]
    pub project_dir: PathBuf,

    /// URL of the marketplace registry index
    #[arg(long, global = true, value_name = "URL", default_value = DEFAULT_REGISTRY_URL, help_heading = GLOBAL)]
    pub registry_url: String,

    /// Never touch the network; use the cache or the snapshot
    #[arg(long, global = true, help_heading = GLOBAL)]
    pub offline: bool,

    /// Directory for the cached registry snapshot
    #[arg(long, global = true, value_name = "DIR", help_heading = GLOBAL)]
    pub cache_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Create a deployment directory: compose files, .env and .chaps/
    Init(InitArgs),

    /// Browse and manage marketplace models
    Models(ModelsArgs),

    /// Show and change what this deployment is made of
    Components(ComponentsArgs),

    /// Open the model browser
    Ui(UiArgs),

    /// Inspect and refresh the marketplace registry
    Registry(RegistryArgs),

    /// Render the compose files from .chaps/
    Sync(SyncArgs),

    /// Move the pins to what upstream publishes now, and pull
    Update(UpdateArgs),

    /// Sync, then start CHAP (docker compose up)
    Up(UpArgs),

    /// Stop CHAP (docker compose down)
    Down(DownArgs),

    /// Show container logs (docker compose logs)
    Logs(LogsArgs),

    /// Recreate the services whose image or configuration changed
    Restart(RestartArgs),

    /// Talk to Docker directly: containers, images, compose
    Docker(DockerArgs),

    /// Create or restore a backup of this deployment
    Backup(BackupArgs),

    /// Show chap-core health and which models registered
    Status(StatusArgs),

    /// Run a checklist over this machine and this deployment
    Doctor(DoctorArgs),

    /// Turn API authentication on or off, and show the token
    Auth(AuthArgs),

    /// Update chaps itself, and report what this build is
    #[command(name = "self")]
    SelfCmd(SelfArgs),

    /// Print a shell completion script for chaps
    Completions(CompletionsArgs),

    /// Print the whole command tree as Markdown
    #[command(hide = true)]
    DocsMarkdown(DocsMarkdownArgs),
}

/// Create a deployment directory: compose files, .env and .chaps/
#[derive(Debug, Clone, Args)]
pub struct InitArgs {
    /// Directory to create
    #[arg(value_name = "DIR", default_value = ".")]
    pub dir: PathBuf,

    /// Models to enable: none, default, or a list of ids
    #[arg(long, value_name = "SPEC", default_value = "default")]
    pub models: String,

    /// Pick the models in the browser instead of taking --models
    #[arg(long)]
    pub interactive: bool,

    /// chap-core image tag: latest, master, dev or vX.Y.Z
    #[arg(long, value_name = "TAG", default_value = "latest")]
    pub chap_tag: String,

    /// Overwrite an existing project in the target directory
    #[arg(long)]
    pub force: bool,

    /// Host port to publish chap-core's API on
    #[arg(long, value_name = "PORT", default_value_t = DEFAULT_API_PORT)]
    pub api_port: u16,

    /// Lowest host port a model may be published on
    #[arg(long, value_name = "PORT", default_value_t = 5001)]
    pub port_base: u16,

    /// Protect the API with a token (generated when no value is given)
    #[arg(long, value_name = "TOKEN", num_args = 0..=1, conflicts_with = "no_env")]
    pub api_token: Option<Option<String>>,

    /// Do not write a .env file
    #[arg(long)]
    pub no_env: bool,

    /// Regenerate .env, rotating the database password
    #[arg(long, conflicts_with = "no_env")]
    pub fresh_env: bool,

    /// Build chap-core from a local checkout (not supported yet)
    #[arg(long, value_name = "PATH")]
    pub source: Option<PathBuf>,

    /// Components to add: ocs, s3
    #[arg(long = "with", value_name = "LIST")]
    pub with: Option<String>,

    /// Components to leave out: chap-core, ocs, s3
    #[arg(long = "without", value_name = "LIST")]
    pub without: Option<String>,

    #[command(flatten)]
    pub ocs: OcsConfigArgs,
}

/// The values that go into the scaffolded `ocs/climate-service.yaml`.
///
/// Shared by `init --with ocs` and `chaps components enable ocs`, because both
/// write that file the first time the component is turned on.
#[derive(Debug, Clone, Default, Args)]
pub struct OcsConfigArgs {
    /// Country or region the OCS instance covers, e.g. Malawi
    #[arg(long = "ocs-name", value_name = "NAME")]
    pub ocs_name: Option<String>,

    /// ISO 3166-1 alpha-3 country code for the OCS extent
    #[arg(long = "ocs-country", value_name = "CODE")]
    pub ocs_country: Option<String>,

    /// OCS extent as xmin,ymin,xmax,ymax in degrees
    #[arg(long = "ocs-bbox", value_name = "BBOX")]
    pub ocs_bbox: Option<String>,
}

/// Show and change what this deployment is made of
#[derive(Debug, Args)]
#[command(arg_required_else_help = true)]
pub struct ComponentsArgs {
    #[command(subcommand)]
    pub command: ComponentsCmd,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ComponentsCmd {
    /// List every component and whether this deployment has it
    List(ComponentsListArgs),

    /// Turn a component on, or change the settings of one that is
    Enable(ComponentsEnableArgs),

    /// Turn a component off and remove its compose file
    Disable(ComponentsDisableArgs),
}

/// List every component and whether this deployment has it
#[derive(Debug, Clone, Args)]
pub struct ComponentsListArgs {}

/// Turn a component on, or change the settings of one that is
#[derive(Debug, Clone, Args)]
pub struct ComponentsEnableArgs {
    /// Component name: ocs, s3 or chap-core
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Host port to publish this component on
    #[arg(long, value_name = "PORT")]
    pub port: Option<u16>,

    #[command(flatten)]
    pub ocs: OcsConfigArgs,
}

/// Turn a component off and remove its compose file
#[derive(Debug, Clone, Args)]
pub struct ComponentsDisableArgs {
    /// Component name: ocs, s3 or chap-core
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Delete the component's data volume as well
    #[arg(long)]
    pub purge: bool,
}

/// Browse and manage marketplace models
#[derive(Debug, Args)]
pub struct ModelsArgs {
    #[command(subcommand)]
    pub command: ModelsCmd,
}

#[derive(Debug, Subcommand)]
pub enum ModelsCmd {
    /// List marketplace models
    List(ModelsListArgs),

    /// Search the marketplace by id, name or summary
    Search(ModelsSearchArgs),

    /// Show everything known about one model
    Info(ModelsInfoArgs),

    /// Enable a model and write its compose overlay
    Enable(ModelsEnableArgs),

    /// Disable a model and remove its compose overlay
    Disable(ModelsDisableArgs),

    /// Publish a host port for an enabled model
    Expose(ModelsExposeArgs),

    /// Take an enabled model's host port away again
    Unexpose(ModelsUnexposeArgs),
}

/// List marketplace models
#[derive(Debug, Clone, Args)]
pub struct ModelsListArgs {
    /// Include every entry, templates as well as models
    #[arg(long)]
    pub all: bool,

    /// List only templates
    #[arg(long)]
    pub templates: bool,

    /// List only the models enabled in this project
    #[arg(long)]
    pub enabled: bool,
}

/// Search the marketplace by id, name or summary
#[derive(Debug, Clone, Args)]
pub struct ModelsSearchArgs {
    /// Text to look for; matching is case-insensitive
    #[arg(value_name = "QUERY")]
    pub query: String,
}

/// Show everything known about one model
#[derive(Debug, Clone, Args)]
pub struct ModelsInfoArgs {
    /// Marketplace id or service id
    #[arg(value_name = "ID")]
    pub id: String,
}

/// Enable a model and write its compose overlay
#[derive(Debug, Clone, Args)]
pub struct ModelsEnableArgs {
    /// Marketplace id or service id
    #[arg(value_name = "ID")]
    pub id: String,

    /// Follow a release channel instead of pinning a version
    #[arg(long, value_name = "CHANNEL", conflicts_with = "version")]
    pub channel: Option<Channel>,

    /// Pin an exact version from the model's versions list
    #[arg(long, value_name = "VERSION")]
    pub version: Option<String>,

    /// Host port to publish the model on, or auto for a free one
    #[arg(long, value_name = "PORT|auto")]
    pub port: Option<PortArg>,

    /// Data directory inside the container
    #[arg(long, value_name = "PATH")]
    pub data_dir: Option<String>,

    /// User the container runs as, as user:group
    #[arg(long, value_name = "USER")]
    pub user: Option<String>,

    /// Enable a template even though it is not a model
    #[arg(long)]
    pub allow_template: bool,
}

/// Disable a model and remove its compose overlay
#[derive(Debug, Clone, Args)]
pub struct ModelsDisableArgs {
    /// Marketplace id or service id
    #[arg(value_name = "ID")]
    pub id: String,

    /// Delete the model's data volume as well
    #[arg(long)]
    pub purge: bool,
}

/// Publish a host port for an enabled model
#[derive(Debug, Clone, Args)]
pub struct ModelsExposeArgs {
    /// Marketplace id or service id
    #[arg(value_name = "ID")]
    pub id: String,

    /// Host port to publish on, or auto for the lowest free one
    #[arg(long, value_name = "PORT|auto")]
    pub port: Option<PortArg>,
}

/// Take an enabled model's host port away again
#[derive(Debug, Clone, Args)]
pub struct ModelsUnexposeArgs {
    /// Marketplace id or service id
    #[arg(value_name = "ID")]
    pub id: String,
}

/// Open the model browser
#[derive(Debug, Clone, Args)]
pub struct UiArgs {}

/// Inspect and refresh the marketplace registry
#[derive(Debug, Args)]
pub struct RegistryArgs {
    #[command(subcommand)]
    pub command: RegistryCmd,
}

#[derive(Debug, Subcommand)]
pub enum RegistryCmd {
    /// Fetch the registry from the network and refresh the cache
    Update(RegistryUpdateArgs),

    /// Show where the registry was loaded from and what it holds
    Show(RegistryShowArgs),
}

/// Fetch the registry from the network and refresh the cache
#[derive(Debug, Clone, Args)]
pub struct RegistryUpdateArgs {}

/// Show where the registry was loaded from and what it holds
#[derive(Debug, Clone, Args)]
pub struct RegistryShowArgs {}

/// Render the compose files from .chaps/
#[derive(Debug, Clone, Args)]
pub struct SyncArgs {
    /// Write nothing; exit non-zero if anything would change
    #[arg(long)]
    pub check: bool,
}

/// Move the pins to what upstream publishes now, and pull
#[derive(Debug, Clone, Args)]
pub struct UpdateArgs {
    /// Show what would change: no pull, and nothing written
    #[arg(long)]
    pub dry_run: bool,

    /// Pin a moving chap-core tag to the newest release
    #[arg(long)]
    pub pin_chap_core: bool,
}

/// Sync, then start CHAP (docker compose up)
#[derive(Debug, Clone, Args)]
pub struct UpArgs {
    /// Run in the foreground and stream all logs (Ctrl-C stops CHAP)
    #[arg(short = 'a', long, visible_alias = "foreground")]
    pub attach: bool,

    /// Pull every image first (docker compose up --pull always)
    #[arg(long)]
    pub pull: bool,

    /// Do not check the host ports first
    #[arg(long)]
    pub no_preflight: bool,

    /// Extra arguments passed through to docker compose up
    #[arg(
        value_name = "EXTRA",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub extra: Vec<String>,
}

/// Stop CHAP (docker compose down)
#[derive(Debug, Clone, Args)]
pub struct DownArgs {
    /// Extra arguments passed through to docker compose down
    #[arg(
        value_name = "EXTRA",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub extra: Vec<String>,
}

/// Show container logs (docker compose logs)
#[derive(Debug, Clone, Args)]
pub struct LogsArgs {
    /// Keep streaming new output
    #[arg(short = 'f', long)]
    pub follow: bool,

    /// Services to show logs for; all of them when omitted
    #[arg(value_name = "SERVICE")]
    pub services: Vec<String>,
}

/// Recreate the services whose image or configuration changed
#[derive(Debug, Clone, Args)]
pub struct RestartArgs {
    /// Recreate the named services even when nothing changed
    #[arg(long)]
    pub all: bool,

    /// Services to restart; the whole project when omitted
    #[arg(value_name = "SERVICE")]
    pub services: Vec<String>,
}

/// Talk to Docker directly: containers, images, compose
#[derive(Debug, Args)]
#[command(arg_required_else_help = true)]
pub struct DockerArgs {
    #[command(subcommand)]
    pub command: DockerSub,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DockerSub {
    /// List the containers this project is running
    Ps(PsArgs),

    /// Download the pinned images into the local Docker daemon
    Pull(PullArgs),

    /// Run a command inside one of the running containers
    Exec(ExecArgs),

    /// Run any docker compose command against this project
    Run(RunArgs),

    /// Print every compose file merged into one document
    Config(ConfigArgs),
}

/// List the containers this project is running
#[derive(Debug, Clone, Args)]
pub struct PsArgs {
    /// Extra arguments passed through to docker compose ps
    #[arg(
        value_name = "EXTRA",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub extra: Vec<String>,
}

/// Download the pinned images into the local Docker daemon
#[derive(Debug, Clone, Args)]
pub struct PullArgs {}

/// Run a command inside one of the running containers
#[derive(Debug, Clone, Args)]
pub struct ExecArgs {
    /// Service to run the command in
    #[arg(value_name = "SERVICE")]
    pub service: String,

    /// Command to run; a shell (sh) when omitted
    #[arg(
        value_name = "CMD",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub cmd: Vec<String>,
}

/// Run any docker compose command against this project
#[derive(Debug, Clone, Args)]
pub struct RunArgs {
    /// Arguments passed to docker compose after the -f list
    #[arg(
        value_name = "ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub args: Vec<String>,
}

/// Print every compose file merged into one document
#[derive(Debug, Clone, Args)]
pub struct ConfigArgs {
    /// Extra arguments passed through to docker compose config
    #[arg(
        value_name = "EXTRA",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub extra: Vec<String>,
}

/// Create or restore a backup of this deployment
#[derive(Debug, Args)]
#[command(arg_required_else_help = true)]
pub struct BackupArgs {
    #[command(subcommand)]
    pub command: BackupSub,
}

#[derive(Debug, Clone, Subcommand)]
pub enum BackupSub {
    /// Write a tar.gz of the database, the data and the files
    Create(BackupCreateArgs),

    /// Put a deployment back from a backup archive
    Restore(RestoreArgs),
}

/// Write a tar.gz of the database, the data and the files
#[derive(Debug, Clone, Args)]
pub struct BackupCreateArgs {
    /// Where to write the archive: a file, or a directory
    #[arg(long, value_name = "PATH")]
    pub out: Option<PathBuf>,

    /// Leave the chap-core database out of the archive
    #[arg(long)]
    pub no_db: bool,

    /// Leave the model data volumes out of the archive
    #[arg(long)]
    pub no_models: bool,

    /// Leave the component data volumes out of the archive
    #[arg(long)]
    pub no_components: bool,
}

/// Put a deployment back from a backup archive
#[derive(Debug, Clone, Args)]
pub struct RestoreArgs {
    /// The tar.gz written by chaps backup create
    #[arg(value_name = "ARCHIVE")]
    pub archive: PathBuf,

    /// Skip the confirmation
    #[arg(long)]
    pub yes: bool,

    /// Restore only the project files; no Docker needed
    #[arg(long, conflicts_with_all = ["db_only", "no_models", "no_components", "no_start"])]
    pub files_only: bool,

    /// Restore only the chap-core database
    #[arg(long, conflicts_with_all = ["no_models", "no_components"])]
    pub db_only: bool,

    /// Leave the model data volumes as they are
    #[arg(long)]
    pub no_models: bool,

    /// Leave the component data volumes as they are
    #[arg(long)]
    pub no_components: bool,

    /// Take over the compose project name from the archive
    #[arg(long, conflicts_with = "db_only")]
    pub adopt_identity: bool,

    /// Do not start the deployment at the end
    #[arg(long)]
    pub no_start: bool,
}

/// Show chap-core health and which models registered
#[derive(Debug, Clone, Args)]
pub struct StatusArgs {
    /// Base URL of the chap-core API
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,

    /// Request timeout in seconds
    #[arg(long, value_name = "SECONDS", default_value_t = 5)]
    pub timeout: u64,
}

/// Run a checklist over this machine and this deployment
#[derive(Debug, Clone, Args)]
pub struct DoctorArgs {}

/// Turn API authentication on or off, and show the token
#[derive(Debug, Args)]
#[command(arg_required_else_help = true)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub command: AuthSub,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AuthSub {
    /// Say whether the API is protected, and by which token
    Show(AuthShowArgs),

    /// Turn authentication on: write both secrets and sync
    Enable(AuthEnableArgs),

    /// Turn authentication off, keeping both values as comments
    Disable(AuthDisableArgs),

    /// Replace both secrets with freshly generated ones
    Rotate(AuthRotateArgs),
}

/// Say whether the API is protected, and by which token
#[derive(Debug, Clone, Args)]
pub struct AuthShowArgs {
    /// Print the API token in full
    #[arg(long)]
    pub reveal: bool,
}

/// Turn authentication on: write both secrets and sync
#[derive(Debug, Clone, Args)]
pub struct AuthEnableArgs {
    /// Use this API token instead of generating one
    #[arg(long, value_name = "TOKEN")]
    pub token: Option<String>,
}

/// Turn authentication off, keeping both values as comments
#[derive(Debug, Clone, Args)]
pub struct AuthDisableArgs {}

/// Replace both secrets with freshly generated ones
#[derive(Debug, Clone, Args)]
pub struct AuthRotateArgs {}

/// Update chaps itself, and report what this build is
#[derive(Debug, Args)]
#[command(arg_required_else_help = true)]
pub struct SelfArgs {
    #[command(subcommand)]
    pub command: SelfSub,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SelfSub {
    /// Replace this binary with the newest release
    Update(SelfUpdateArgs),

    /// Show what this build is: version, revision, target and path
    Version(SelfVersionArgs),
}

/// Replace this binary with the newest release
#[derive(Debug, Clone, Args)]
pub struct SelfUpdateArgs {
    /// Report what an update would do and change nothing
    #[arg(long)]
    pub check: bool,

    /// Install this release tag instead of the newest one
    #[arg(long, value_name = "TAG")]
    pub version: Option<String>,

    /// Do not ask before replacing the binary
    #[arg(short, long)]
    pub yes: bool,
}

/// Show what this build is: version, revision, target and path
#[derive(Debug, Clone, Args)]
pub struct SelfVersionArgs {}

/// Print a shell completion script for chaps
#[derive(Debug, Clone, Args)]
pub struct CompletionsArgs {
    /// Shell to generate for
    #[arg(value_name = "SHELL")]
    pub shell: clap_complete::Shell,
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
    Restart(RestartArgs),
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

    /// Asking for help is not asking for the manual: the explanations live in
    /// the book, so no command's help may grow into a page and no option's
    /// help into a paragraph. The whole tree is walked, so a command added
    /// later is held to the same limits.
    ///
    /// The count is taken from `render_help`, which is what both `-h` and
    /// `--help` print now that nothing in the tree has a `long_about` or a
    /// `long_help`. `render_long_help` is clap's expanded layout - every
    /// default value on a line of its own - and is measured only for the help
    /// texts it carries, which are the same ones.
    #[test]
    fn no_help_is_longer_than_a_screen_and_no_option_longer_than_a_clause() {
        /// One screen of `--help`, and one clause of option help.
        const MAX_LINES: usize = 40;
        const MAX_CHARS: usize = 90;

        fn check(command: &mut clap::Command, path: &str) {
            let help = command.render_help().to_string();
            let lines = help.lines().count();
            assert!(
                lines <= MAX_LINES,
                "`{path} --help` is {lines} lines, over {MAX_LINES}:\n{help}"
            );
            assert!(
                command.get_long_about().is_none(),
                "`{path}` has a long_about; the chapters carry the prose"
            );
            // The long rendering says the same thing, in clap's roomier
            // layout: nothing is written twice, at two lengths.
            let long = command.render_long_help().to_string();
            for arg in command.get_arguments() {
                let text = arg
                    .get_help()
                    .map(|help| help.to_string())
                    .unwrap_or_default();
                let chars = text.chars().count();
                assert!(
                    chars <= MAX_CHARS,
                    "`{path}` {}: {chars} characters of help, over {MAX_CHARS}: {text}",
                    arg.get_id()
                );
                assert!(
                    arg.get_long_help().is_none(),
                    "`{path}` {} has a long_help",
                    arg.get_id()
                );
                assert!(
                    text.is_empty() || arg.is_hide_set() || long.contains(&text),
                    "`{path}` {} is missing from the long help",
                    arg.get_id()
                );
            }

            let names: Vec<String> = command
                .get_subcommands()
                .map(|sub| sub.get_name().to_string())
                .collect();
            for name in names {
                let path = format!("{path} {name}");
                let sub = command
                    .find_subcommand_mut(&name)
                    .expect("just listed as a subcommand");
                check(sub, &path);
            }
        }

        let mut root = Cli::command();
        // The globals only reach the subcommands, and the usage lines only
        // exist, once clap has built the tree.
        root.build();
        check(&mut root, "chaps");
    }

    /// Both spellings of help say the same thing, rather than a short one and
    /// a long one that drift apart. Clap's two renderings differ only in
    /// layout - the long one gives every default a line of its own - so the
    /// words have to match exactly.
    #[test]
    fn the_long_help_is_the_short_help() {
        fn words(text: &str) -> Vec<String> {
            text.split_whitespace().map(str::to_string).collect()
        }

        let short = Cli::command().render_help().to_string();
        let long = Cli::command().render_long_help().to_string();
        assert!(Cli::command().get_long_about().is_none(), "{long}");
        assert!(short.starts_with(ABOUT), "{short}");
        assert!(long.starts_with(ABOUT), "{long}");
        assert_eq!(words(&short), words(&long));
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
        assert_eq!(args.api_token, None, "no flag, no authentication");

        let cli = Cli::try_parse_from(["chap", "init", "--api-port", "8123"]).unwrap();
        let Command::Init(args) = cli.command else {
            panic!("expected init");
        };
        assert_eq!(args.api_port, 8123);
    }

    /// The `InitArgs` behind `chap init <argv..>`.
    fn init_args(argv: &[&str]) -> InitArgs {
        let mut args = vec!["chap", "init"];
        args.extend_from_slice(argv);
        let cli = Cli::try_parse_from(args).unwrap();
        let Command::Init(args) = cli.command else {
            panic!("expected init");
        };
        args
    }

    #[test]
    fn the_api_token_flag_takes_an_optional_value() {
        // Bare: the command generates one.
        assert_eq!(init_args(&["--api-token"]).api_token, Some(None));
        // With a value: used verbatim.
        assert_eq!(
            init_args(&["--api-token", "sekret"]).api_token,
            Some(Some("sekret".to_string()))
        );
        assert_eq!(
            init_args(&["--api-token=sekret"]).api_token,
            Some(Some("sekret".to_string()))
        );
        // And it still parses next to the positional directory.
        assert_eq!(
            init_args(&["mychap", "--api-token"]).dir,
            PathBuf::from("mychap")
        );

        // A file that is not written cannot hold a secret.
        assert!(Cli::try_parse_from(["chap", "init", "--api-token", "--no-env"]).is_err());
        assert!(Cli::try_parse_from(["chap", "init", "--no-env"]).is_ok());
    }

    /// The `AuthSub` behind `chap auth <argv..>`.
    fn auth_sub(argv: &[&str]) -> AuthSub {
        let mut args = vec!["chap", "auth"];
        args.extend_from_slice(argv);
        let cli = Cli::try_parse_from(args).unwrap();
        let Command::Auth(a) = cli.command else {
            panic!("expected auth");
        };
        a.command
    }

    #[test]
    fn auth_has_show_enable_disable_and_rotate() {
        let AuthSub::Show(args) = auth_sub(&["show"]) else {
            panic!("expected auth show");
        };
        assert!(!args.reveal);
        let AuthSub::Show(args) = auth_sub(&["show", "--reveal"]) else {
            panic!("expected auth show");
        };
        assert!(args.reveal);

        let AuthSub::Enable(args) = auth_sub(&["enable"]) else {
            panic!("expected auth enable");
        };
        assert_eq!(args.token, None, "one is generated");
        let AuthSub::Enable(args) = auth_sub(&["enable", "--token", "sekret"]) else {
            panic!("expected auth enable");
        };
        assert_eq!(args.token.as_deref(), Some("sekret"));

        assert!(matches!(auth_sub(&["disable"]), AuthSub::Disable(_)));
        assert!(matches!(auth_sub(&["rotate"]), AuthSub::Rotate(_)));

        // `--token` on `enable` needs a value; rotating takes none at all.
        assert!(Cli::try_parse_from(["chap", "auth", "enable", "--token"]).is_err());
        assert!(Cli::try_parse_from(["chap", "auth", "rotate", "--token", "x"]).is_err());
    }

    #[test]
    fn auth_needs_a_subcommand() {
        let err = Cli::try_parse_from(["chap", "auth"]).expect_err("a subcommand is required");
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
        let help = err.to_string();
        for name in ["show", "enable", "disable", "rotate"] {
            assert!(help.contains(name), "{name} is missing from:\n{help}");
        }
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
            help.contains("Run in the foreground and stream all logs (Ctrl-C stops CHAP)"),
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
        assert!(!args.dry_run && !args.pin_chap_core);
        let cli = Cli::try_parse_from(["chap", "update", "--dry-run", "--pin-chap-core"]).unwrap();
        let Command::Update(args) = cli.command else {
            panic!("expected update");
        };
        assert!(args.dry_run && args.pin_chap_core);

        // `update` no longer touches containers, so the flag that said not to
        // is gone rather than accepted and ignored.
        assert!(Cli::try_parse_from(["chap", "update", "--no-restart"]).is_err());
    }

    /// The `RestartArgs` behind `chap restart <argv..>`.
    fn restart_args(argv: &[&str]) -> RestartArgs {
        let mut args = vec!["chap", "restart"];
        args.extend_from_slice(argv);
        let cli = Cli::try_parse_from(args).unwrap();
        let Command::Restart(args) = cli.command else {
            panic!("expected restart");
        };
        args
    }

    #[test]
    fn restart_takes_service_names_with_the_flag_on_either_side() {
        let args = restart_args(&[]);
        assert!(!args.all && args.services.is_empty());

        let args = restart_args(&["--all"]);
        assert!(args.all && args.services.is_empty());

        // The order the status hint prints: flag first, then the service.
        let args = restart_args(&["--all", "chapkit-ewars-model"]);
        assert!(args.all);
        assert_eq!(args.services, vec!["chapkit-ewars-model"]);

        // And the other way round, plus more than one name.
        let args = restart_args(&["chap", "worker", "--all"]);
        assert!(args.all);
        assert_eq!(args.services, vec!["chap", "worker"]);
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
