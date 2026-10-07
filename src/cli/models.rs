//! `varde models`, `varde ui` and `varde registry`: the marketplace and what
//! this deployment runs from it.

use super::PortArg;
use crate::registry::Channel;
use clap::{Args, Subcommand};

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

    /// Create the configured models that chap-core needs to run each model
    Configure(ModelsConfigureArgs),

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

    /// Remove the host port of an enabled model
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

    /// List only the models enabled in this deployment
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

    /// Test every model this deployment has enabled
    #[arg(long)]
    pub all: bool,

    /// Run a backtest through chap-core instead of the model's own test
    #[arg(long)]
    pub backtest: bool,

    /// Seed for the generated data, so a run can be repeated
    #[arg(long, value_name = "N")]
    pub seed: Option<i64>,

    /// Time limit for the test of one model, in seconds
    #[arg(long, value_name = "SECONDS")]
    pub timeout: Option<u64>,

    /// Keep what the test created instead of deleting it
    #[arg(long)]
    pub keep: bool,
}

/// Create the configured models that chap-core needs to run each model
#[derive(Debug, Clone, Args)]
pub struct ModelsConfigureArgs {
    /// Marketplace ids or service ids; all enabled models when none is given
    #[arg(value_name = "ID")]
    pub ids: Vec<String>,
}

/// Add a model the marketplace does not list
#[derive(Debug, Clone, Args)]
pub struct ModelsAddArgs {
    /// GitHub repository URL, ghcr image reference, or local image name:tag
    #[arg(value_name = "SOURCE")]
    pub source: String,

    /// Identifier to record the model under, or auto for a free one
    #[arg(long, value_name = "ID|auto")]
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

    /// Host address to publish the port on, e.g. 127.0.0.1 for this machine
    #[arg(long, value_name = "ADDR")]
    pub bind: Option<std::net::IpAddr>,

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

    /// Host address to publish the port on, e.g. 127.0.0.1 for this machine
    #[arg(long, value_name = "ADDR")]
    pub bind: Option<std::net::IpAddr>,

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

    /// Host address to publish the port on, e.g. 127.0.0.1 for this machine
    #[arg(long, value_name = "ADDR")]
    pub bind: Option<std::net::IpAddr>,
}

/// Remove the host port of an enabled model
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
