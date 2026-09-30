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

/// The value of `components enable --port`: a number, or `none` for a component
/// that is only reached inside the compose network.
///
/// `none` rather than a second subcommand, because `enable` is already how a
/// component's settings change and a port that can be moved should be
/// removable the same way. `Option<ComponentPortArg>` then has three states -
/// flag absent, `--port none`, `--port 9010` - which is exactly the three
/// things an operator can mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentPortArg(pub Option<u16>);

impl std::str::FromStr for ComponentPortArg {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<ComponentPortArg, String> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("none") {
            return Ok(ComponentPortArg(None));
        }
        match text.parse::<u16>() {
            Ok(0) => Err("0 is not a host port; pass a number or `none`".to_string()),
            Ok(port) => Ok(ComponentPortArg(Some(port))),
            Err(_) => Err(format!("`{text}` is neither a port number nor `none`")),
        }
    }
}

/// The one-liner both `chaps -h` and `chaps --help` open with. There is no
/// `long_about`: the two spellings of help say the same thing.
const ABOUT: &str = "deploy CHAP, the Climate Health Analytics Platform, and its services";

/// The last line of `chaps --help`. Anything longer than a clause belongs in
/// the book, so the help points at it instead of repeating it.
///
/// A line, and nothing more: the blank line the help ends on is written where
/// the help is printed (`print_and_exit` in `main`), because clap `trim_end`s
/// the rendered help and appends one newline of its own, so a newline added
/// here or to `after_help` would be thrown away.
pub const DOCS_LINE: &str = "Docs: https://winterop-com.github.io/chaps/";

/// The book itself, for the places that open it rather than print it. A test
/// keeps it and [`DOCS_LINE`] pointing at the same place.
pub const DOCS_URL: &str = "https://winterop-com.github.io/chaps/";

/// The heading the global options are listed under, in their own block after
/// each command's own options.
const GLOBAL: &str = "Global options";

/// Deploy CHAP, the Climate Health Analytics Platform, and its services.
#[derive(Debug, Parser)]
// `bin_name` as well as `name`: without it clap takes the usage line from
// argv[0], so the same help reads `chaps.exe` on Windows and `chaps`
// everywhere else - and `docs/reference.md` is generated from that help.
#[command(name = "chaps", bin_name = "chaps", version, about = ABOUT, after_help = DOCS_LINE)]
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
    Init(Box<InitArgs>),

    /// Browse and manage marketplace models
    Models(ModelsArgs),

    /// Show and change what this deployment is made of
    Components(ComponentsArgs),

    /// Open the browser: marketplace models and components
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

    /// Open a component's web interface in a browser
    Open(OpenArgs),

    /// List the backtests and predictions chap-core has run
    Jobs(JobsArgs),

    /// Send one authenticated request to chap-core's API
    Api(ApiArgs),

    /// Run a checklist over this machine and this deployment
    Doctor(DoctorArgs),

    /// Turn API authentication on or off, and show the token
    Auth(AuthArgs),

    /// Let the DHIS2 Modeling App reach this deployment's CHAP
    Dhis2(Dhis2Args),

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

    /// Models to enable: none, default, all, or a list of ids
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

    /// Build chap-core from the chap-core checkout at PATH
    #[arg(long, value_name = "PATH", conflicts_with = "chap_core_url")]
    pub source: Option<PathBuf>,

    /// Components to add: ocs, s3, dhis2
    #[arg(long = "with", value_name = "LIST")]
    pub with: Option<String>,

    /// Components to leave out: chap-core, ocs, s3, dhis2
    #[arg(long = "without", value_name = "LIST")]
    pub without: Option<String>,

    /// Exactly these components and nothing else; `none` for models alone
    #[arg(long = "only", value_name = "LIST", conflicts_with_all = ["with", "without"])]
    pub only: Option<String>,

    /// Register the models with the chap-core at URL instead of running one
    #[arg(long = "chap-core-url", value_name = "URL")]
    pub chap_core_url: Option<String>,

    /// Public URL OCS is reached at, for a proxied instance
    #[arg(long = "ocs-base-url", value_name = "URL")]
    pub ocs_base_url: Option<String>,

    /// Host port to publish OCS on, or none to keep it internal
    #[arg(long = "ocs-port", value_name = "PORT|none")]
    pub ocs_port: Option<ComponentPortArg>,

    /// Host port to publish the object store on, or none
    #[arg(long = "s3-port", value_name = "PORT|none")]
    pub s3_port: Option<ComponentPortArg>,

    /// Host port to publish DHIS2 on, or none to keep it internal
    #[arg(long = "dhis2-port", value_name = "PORT|none")]
    pub dhis2_port: Option<ComponentPortArg>,

    /// Dump to seed the DHIS2 database from: default, none, URL or path
    #[arg(long = "dhis2-seed", value_name = "SPEC")]
    pub dhis2_seed: Option<String>,

    /// DHIS2 version to run: an image tag such as 2.41, 2.42 or 2.43.1
    #[arg(long = "dhis2-tag", value_name = "TAG")]
    pub dhis2_tag: Option<String>,

    /// DHIS2 image repository, e.g. dhis2/core-dev for unreleased versions
    #[arg(long = "dhis2-image", value_name = "REPO")]
    pub dhis2_image: Option<String>,

    /// Refuse ingestion over HTTP on the new OCS instance
    #[arg(long = "ocs-read-only")]
    pub ocs_read_only: bool,

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

    // `allow_hyphen_values`: a western or southern extent starts with a minus
    // sign, and without it clap reads `--ocs-bbox -13.5,6.9,-10.1,10.0` as a
    // flag it does not know rather than as this flag's value. Only the
    // `--ocs-bbox=...` spelling would work then, which is not the spelling
    // anyone types first. `allow_negative_numbers` is the narrower version of
    // the same setting and does not help: the value is a list, not a number.
    /// OCS extent as xmin,ymin,xmax,ymax in degrees
    #[arg(long = "ocs-bbox", value_name = "BBOX", allow_hyphen_values = true)]
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
    /// Component name: ocs, s3, dhis2 or chap-core
    #[arg(value_name = "NAME")]
    pub name: String,

    /// Host port to publish on, or none to keep it internal
    #[arg(long, value_name = "PORT|none")]
    pub port: Option<ComponentPortArg>,

    /// Public URL OCS is reached at, for a proxied instance
    #[arg(long = "base-url", value_name = "URL")]
    pub base_url: Option<String>,

    /// Refuse ingestion over HTTP on this OCS instance
    #[arg(long = "read-only", conflicts_with = "read_write")]
    pub read_only: bool,

    /// Allow ingestion over HTTP again
    #[arg(long = "read-write")]
    pub read_write: bool,

    /// chap-core only: use the chap-core at URL instead of running one
    #[arg(long = "url", value_name = "URL")]
    pub url: Option<String>,

    /// dhis2 only: the DHIS2 version to run, an image tag such as 2.43
    #[arg(long = "tag", value_name = "TAG")]
    pub tag: Option<String>,

    /// dhis2 only: the image repository, e.g. dhis2/core-dev
    #[arg(long = "image", value_name = "REPO")]
    pub image: Option<String>,

    /// With --url: the host that chap-core calls the models back at
    #[arg(long = "models-host", value_name = "HOST", requires = "url")]
    pub models_host: Option<String>,

    #[command(flatten)]
    pub ocs: OcsConfigArgs,
}

/// Turn a component off and remove its compose file
#[derive(Debug, Clone, Args)]
pub struct ComponentsDisableArgs {
    /// Component name: ocs, s3, dhis2 or chap-core
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

    /// Make a model train and predict, and say whether it could
    Test(ModelsTestArgs),

    /// Add a model the marketplace does not list
    Add(ModelsAddArgs),

    /// Remove a model that was added with models add
    Remove(ModelsRemoveArgs),

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

/// Make a model train and predict, and say whether it could
#[derive(Debug, Clone, Args)]
pub struct ModelsTestArgs {
    /// Marketplace ids or service ids of the models to test
    #[arg(value_name = "ID", conflicts_with = "all")]
    pub ids: Vec<String>,

    /// Test every model this project has enabled
    #[arg(long)]
    pub all: bool,

    /// Also run each model through chap-core as a backtest
    #[arg(long)]
    pub backtest: bool,

    /// Seed for the generated data, so a run can be repeated
    #[arg(long, value_name = "N")]
    pub seed: Option<i64>,

    /// Give up on one model after this many seconds
    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<u64>,

    /// Keep what the test created instead of deleting it
    #[arg(long)]
    pub keep: bool,
}

/// Add a model the marketplace does not list
#[derive(Debug, Clone, Args)]
pub struct ModelsAddArgs {
    /// GitHub repository URL, ghcr image reference, or local image name:tag
    #[arg(value_name = "SOURCE")]
    pub source: String,

    /// Identifier to record the model under
    #[arg(long, value_name = "ID")]
    pub id: Option<String>,

    /// Compose service name, which must match the service's own id
    #[arg(long, value_name = "ID")]
    pub service_id: Option<String>,

    /// Name to show in the listings
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    /// Host port to publish the model on, or auto for a free one
    #[arg(long, value_name = "PORT|auto")]
    pub port: Option<PortArg>,

    /// Data directory inside the container
    #[arg(long, value_name = "PATH")]
    pub data_dir: Option<String>,

    /// User the container runs as, as user:group
    #[arg(long, value_name = "USER")]
    pub user: Option<String>,

    /// Record the image as published for amd64 only
    #[arg(long)]
    pub runtime_amd64: bool,
}

/// Remove a model that was added with models add
#[derive(Debug, Clone, Args)]
pub struct ModelsRemoveArgs {
    /// Id of a model added with models add
    #[arg(value_name = "ID")]
    pub id: String,

    /// Delete the model's data volume as well
    #[arg(long)]
    pub purge: bool,
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

/// Open the browser: marketplace models and components
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

    /// Move chap-core to this tag: vX.Y.Z, latest, master or dev
    #[arg(long, value_name = "TAG", conflicts_with = "pin_chap_core")]
    pub chap_tag: Option<String>,

    /// List the chap-core tags you can move to, newest first
    #[arg(long, conflicts_with_all = ["chap_tag", "pin_chap_core"])]
    pub list_tags: bool,

    /// Answer yes to the confirmation a backwards move asks for
    #[arg(long)]
    pub yes: bool,
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

    /// Stop the other chaps deployments holding these ports first
    #[arg(long, conflicts_with = "no_preflight")]
    pub replace: bool,

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
    // Long spelling only: `-v` is the global `--verbose` flag, and a `down`
    // that kept or dropped the data depending on which of the two clap
    // resolved it to would be the worst of both.
    /// Also remove this deployment's volumes, destroying its data
    #[arg(long)]
    pub volumes: bool,

    /// Skip the confirmation
    #[arg(short = 'y', long)]
    pub yes: bool,

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

/// Open a component's web interface in a browser
#[derive(Debug, Clone, Args)]
pub struct OpenArgs {
    // Optional, and `arg_required_else_help` is not what this wants: that is
    // the convention for a group with no default subcommand, and `open` is a
    // verb. With nothing named it lists what this deployment has and what each
    // one opens, which answers the question a bare `chaps open` is asking.
    /// Component to open: chap-core, ocs or dhis2
    pub name: Option<String>,
}

/// List the backtests and predictions chap-core has run
///
/// `args_conflicts_with_subcommands`: the filters below are `list`'s, repeated
/// here so `chaps jobs --status FAILURE` works without the verb. Only one side
/// may carry them, so `chaps jobs --limit 5 show ID` is a usage error rather
/// than a flag that silently does nothing.
#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct JobsArgs {
    /// Absent is `list`: the bare command is the one that answers "what has
    /// this deployment been doing", and that is the question people type.
    #[command(subcommand)]
    pub command: Option<JobsCmd>,

    #[command(flatten)]
    pub list: JobsListArgs,
}

#[derive(Debug, Clone, Subcommand)]
pub enum JobsCmd {
    /// List the jobs chap-core knows about, newest first
    List(JobsListArgs),

    /// Show everything chap-core records about one job
    Show(JobsShowArgs),

    /// Print one job's log, which is where a failure says why
    Logs(JobsLogsArgs),

    /// Ask chap-core to stop a job that is still running
    Cancel(JobsCancelArgs),

    /// Remove a finished job from chap-core's list
    Delete(JobsDeleteArgs),
}

/// List the jobs chap-core knows about, newest first
#[derive(Debug, Clone, Default, Args)]
pub struct JobsListArgs {
    /// Only jobs in this status; repeat for more than one
    #[arg(long, value_name = "STATUS")]
    pub status: Vec<String>,

    /// Only jobs of this type, such as create_backtest
    #[arg(long = "type", value_name = "TYPE")]
    pub kind: Option<String>,

    /// Show at most this many jobs
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
}

/// Show everything chap-core records about one job
#[derive(Debug, Clone, Args)]
pub struct JobsShowArgs {
    /// Job id, or enough of its start to name one job
    #[arg(value_name = "ID")]
    pub id: String,
}

/// Print one job's log, which is where a failure says why
#[derive(Debug, Clone, Args)]
pub struct JobsLogsArgs {
    /// Job id, or enough of its start to name one job
    #[arg(value_name = "ID")]
    pub id: String,

    /// Print only the last N lines of the log
    #[arg(long, value_name = "N")]
    pub tail: Option<usize>,
}

/// Ask chap-core to stop a job that is still running
#[derive(Debug, Clone, Args)]
pub struct JobsCancelArgs {
    /// Job id, or enough of its start to name one job
    #[arg(value_name = "ID")]
    pub id: String,
}

/// Remove a finished job from chap-core's list
#[derive(Debug, Clone, Args)]
pub struct JobsDeleteArgs {
    /// Job id, or enough of its start to name one job
    #[arg(value_name = "ID")]
    pub id: String,
}

/// Send one authenticated request to chap-core's API
#[derive(Debug, Clone, Args)]
pub struct ApiArgs {
    /// GET, POST, PUT, PATCH or DELETE; case does not matter
    #[arg(value_name = "METHOD")]
    pub method: String,

    /// Request path starting with /, query string and all
    #[arg(value_name = "PATH")]
    pub path: String,

    /// JSON body: inline, @file, or - to read stdin
    #[arg(long, value_name = "JSON|@FILE|-", allow_hyphen_values = true)]
    pub data: Option<String>,

    /// Base URL of the chap-core API
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,

    /// Print the body exactly as it arrived
    #[arg(long)]
    pub raw: bool,

    /// Request timeout in seconds
    #[arg(long, value_name = "SECONDS", default_value_t = 30)]
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

    /// Print the API token alone, for a script to capture
    Token(AuthTokenArgs),

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
    /// Print the API token
    #[arg(long)]
    pub reveal: bool,
}

/// Print the API token alone, for a script to capture
#[derive(Debug, Clone, Args)]
pub struct AuthTokenArgs {}

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

/// Let the DHIS2 Modeling App reach this deployment's CHAP
#[derive(Debug, Args)]
#[command(arg_required_else_help = true)]
pub struct Dhis2Args {
    #[command(subcommand)]
    pub command: Dhis2Sub,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Dhis2Sub {
    /// Say what DHIS2 has: the chap route, analytics and the apps
    Show(Dhis2ShowArgs),

    /// Point DHIS2's chap route at this deployment's chap-core
    Route(Dhis2RouteArgs),

    /// Generate DHIS2's analytics tables and wait for them
    Analytics(Dhis2AnalyticsArgs),

    /// Install the Modeling and Climate apps from the App Hub
    Apps(Dhis2AppsArgs),

    /// The route, the apps, then analytics; the route only on an external DHIS2
    Connect(Dhis2ConnectArgs),

    /// Use a DHIS2 that runs elsewhere, or show which DHIS2 is used
    Use(Dhis2UseArgs),
}

/// The two things every verb in the group needs.
///
/// Flattened rather than repeated, so the user and the wait are spelled and
/// documented once. There is deliberately no `--password` or `--token`: either
/// would sit in the shell history and in `ps`, and `.env` or the `CHAPS_DHIS2_*`
/// variables are where they belong.
#[derive(Debug, Clone, Args)]
pub struct Dhis2CommonArgs {
    /// DHIS2 user to authenticate as, with a password rather than a token
    #[arg(long, value_name = "NAME")]
    pub user: Option<String>,

    /// Seconds to wait for DHIS2's API to start answering
    #[arg(long, value_name = "SECONDS", default_value_t = crate::dhis2::DEFAULT_API_WAIT)]
    pub wait: u64,
}

/// Say what DHIS2 has: the chap route, analytics and the apps
#[derive(Debug, Clone, Args)]
pub struct Dhis2ShowArgs {
    #[command(flatten)]
    pub common: Dhis2CommonArgs,
}

/// Point DHIS2's chap route at this deployment's chap-core
#[derive(Debug, Clone, Args)]
pub struct Dhis2RouteArgs {
    #[command(flatten)]
    pub common: Dhis2CommonArgs,
}

/// Generate DHIS2's analytics tables and wait for them
#[derive(Debug, Clone, Args)]
pub struct Dhis2AnalyticsArgs {
    #[command(flatten)]
    pub common: Dhis2CommonArgs,

    /// Seconds to wait for the analytics run to finish
    #[arg(long, value_name = "SECONDS", default_value_t = crate::dhis2::DEFAULT_ANALYTICS_TIMEOUT)]
    pub timeout: u64,

    /// Start the run and leave it going instead of waiting
    #[arg(long)]
    pub no_wait: bool,
}

/// Install the Modeling and Climate apps from the App Hub
#[derive(Debug, Clone, Args)]
pub struct Dhis2AppsArgs {
    #[command(flatten)]
    pub common: Dhis2CommonArgs,
}

/// The route, the apps, then analytics; the route only on an external DHIS2
#[derive(Debug, Clone, Args)]
pub struct Dhis2ConnectArgs {
    #[command(flatten)]
    pub common: Dhis2CommonArgs,

    /// Seconds to wait for the analytics run to finish
    #[arg(long, value_name = "SECONDS", default_value_t = crate::dhis2::DEFAULT_ANALYTICS_TIMEOUT)]
    pub timeout: u64,

    /// Start the run and leave it going instead of waiting
    #[arg(long)]
    pub no_wait: bool,
}

/// Use a DHIS2 that runs elsewhere, or show which DHIS2 is used
#[derive(Debug, Clone, Args)]
pub struct Dhis2UseArgs {
    /// DHIS2's URL, as this machine reaches it
    #[arg(value_name = "URL", conflicts_with = "clear")]
    pub url: Option<String>,

    /// chap-core's URL, as that DHIS2 reaches it; the route points here
    #[arg(long, value_name = "URL", conflicts_with = "clear")]
    pub chap_url: Option<String>,

    /// Forget the external DHIS2 and use the dhis2 component again
    #[arg(long)]
    pub clear: bool,
}

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
mod tests;
