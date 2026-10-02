//! The `chaps` command tree.
//!
//! Doc comments on the types and fields are the `--help` text, and every one of
//! them is a single clause: asking for help is not asking for the manual, so the
//! explanations live in the book (`docs/`) and the reference chapter is
//! generated from these one-liners. A subcommand's own help comes from its
//! variant in the enum, never from the argument struct the variant names. This
//! module and `main.rs` are complete: the command modules only read the arg
//! structs.

mod dhis2;
mod lifecycle;
mod models;
mod operate;
mod project;
mod run;

pub use dhis2::*;
pub use lifecycle::*;
pub use models::*;
pub use operate::*;
pub use project::*;
pub use run::*;

use crate::compose::PortRequest;
use crate::registry::DEFAULT_REGISTRY_URL;
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
const ABOUT: &str = "deploy Chap, the Climate Health Analytics Platform, and its services";

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

/// Deploy Chap, the Climate Health Analytics Platform, and its services.
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

    /// Start one model and print where it answers
    Run(ModelRunArgs),

    /// List the models that run, and where each one answers
    Ps(ModelPsArgs),

    /// Stop a model and take its overlay away; its data stays
    Stop(ModelStopArgs),

    /// Watch every chaps deployment on this machine as a live tree
    Top(TopArgs),

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

    /// Sync, then start Chap (docker compose up)
    Up(UpArgs),

    /// Stop Chap (docker compose down)
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

    /// Delete what deployments whose directory is gone left in docker
    Cleanup(CleanupArgs),

    /// Turn API authentication on or off, and show the token
    Auth(AuthArgs),

    /// Let the DHIS2 Modeling App reach this deployment's Chap
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

#[cfg(test)]
mod tests;
