//! `chaps chap`: chap-core's own command line, run in a container.

use clap::{Args, ValueEnum};

/// Run the chap CLI in a container, with no Python or uv installed
#[derive(Debug, Clone, Args)]
pub struct ChapArgs {
    /// chap-core image tag; the deployment's tag or the newest release if omitted
    #[arg(long, value_name = "TAG")]
    pub tag: Option<String>,

    /// Image to run in; chosen from --model-name when omitted
    #[arg(long, value_name = "IMAGE")]
    pub image: Option<ChapImage>,

    /// `chaps run` group whose network the container joins
    #[arg(long, value_name = "NAME")]
    pub group: Option<String>,

    /// Give the container the docker socket, for docker_env models
    #[arg(long)]
    pub docker: bool,

    /// Stop the model this run started, once chap is done
    #[arg(long)]
    pub stop: bool,

    /// How long a model this run starts may take to answer, in seconds
    #[arg(long, value_name = "SECONDS", default_value_t = 300)]
    pub timeout: u64,

    /// Arguments for chap, such as `eval --model-name URL ...`
    #[arg(
        value_name = "CHAP_ARGS",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub args: Vec<String>,
}

/// The two chap-core images the CLI can run in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ChapImage {
    // ghcr.io/dhis2-chap/chap-core: chapkit services and the commands without
    // a model. The values carry no help text: clap shows that only in a long
    // help, and `docs/chap-cli.md` explains the choice.
    Core,
    // ghcr.io/dhis2-chap/chap-worker: also uv, R and docker models.
    Worker,
}
