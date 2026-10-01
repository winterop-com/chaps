//! `chaps run`, `ps` and `stop`: one model at a time, with or without a
//! deployment directory of your own.

use super::PortArg;
use clap::Args;

/// Start one model and print where it answers
#[derive(Debug, Clone, Args)]
pub struct ModelRunArgs {
    /// Marketplace id, GitHub repository URL, ghcr image, or local image:tag
    #[arg(value_name = "MODEL")]
    pub source: String,

    /// Host port to publish the model on, or auto for a free one
    #[arg(long, value_name = "PORT|auto", default_value = "auto")]
    pub port: PortArg,

    /// Host address to publish the port on, e.g. 0.0.0.0 for the network
    #[arg(long, value_name = "ADDR")]
    pub bind: Option<std::net::IpAddr>,

    /// Identifier to record an added model under, or auto for a free one
    #[arg(long, value_name = "ID|auto")]
    pub id: Option<String>,

    /// Group to start it in, outside a deployment; ps and stop take it too
    #[arg(long, value_name = "NAME")]
    pub group: Option<String>,

    /// Return once the container started, without waiting for it to answer
    #[arg(long)]
    pub no_wait: bool,

    /// How long to wait for the model to answer, in seconds
    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 300,
        conflicts_with = "no_wait"
    )]
    pub timeout: u64,
}

/// List the models that run, and where each one answers
#[derive(Debug, Clone, Args)]
pub struct ModelPsArgs {
    /// List this group only; every group when omitted
    #[arg(long, value_name = "NAME")]
    pub group: Option<String>,
}

/// Watch every chaps deployment on this machine as a live tree
#[derive(Debug, Clone, Args)]
pub struct TopArgs {
    /// Seconds between two looks at docker
    #[arg(long, value_name = "SECONDS", default_value_t = 2, value_parser = clap::value_parser!(u64).range(1..))]
    pub interval: u64,
}

/// Stop a model and take its overlay away; its data stays
#[derive(Debug, Clone, Args)]
pub struct ModelStopArgs {
    /// Id or service id of the model
    #[arg(value_name = "ID", required_unless_present = "all")]
    pub id: Option<String>,

    /// Group the model runs in, when more than one group has it
    #[arg(long, value_name = "NAME")]
    pub group: Option<String>,

    /// Stop every model in the group, or in every group
    #[arg(long, conflicts_with = "id")]
    pub all: bool,

    /// Delete the model's data volume as well
    #[arg(long)]
    pub purge: bool,
}
