//! Typed errors for the parts of `chaps` where the caller needs to react to the
//! kind of failure, plus the crate-wide [`Result`] alias.
//!
//! Everything that is merely context is an [`anyhow::Error`]; only the cases
//! listed in [`ChapError`] carry meaning (exit codes, TUI messages, tests).

use std::path::PathBuf;

/// Errors that callers are expected to match on.
#[derive(thiserror::Error, Debug)]
pub enum ChapError {
    #[error(
        "{0} is not a chaps project (no .chaps/project.yaml here or in a parent directory); \
         run `chaps init` first"
    )]
    NotAProject(PathBuf),

    #[error("compose files are out of date with .chaps/; run `chaps sync`")]
    OutOfSync,

    #[error("{0} already contains a chaps project; use --force to overwrite")]
    AlreadyInitialized(PathBuf),

    #[error("unknown model `{0}`")]
    UnknownModel(String),

    #[error(
        "`{0}` is a template, not a deployable model; pass --allow-template to enable it anyway"
    )]
    IsTemplate(String),

    #[error("model `{id}` has no version `{version}`")]
    UnknownVersion { id: String, version: String },

    #[error("version {version} of `{id}` is yanked")]
    YankedVersion { id: String, version: String },

    #[error("host port {0} is already in use by another service")]
    PortInUse(u16),

    #[error("host port {0} is outside the allowed range")]
    PortOutOfRange(u16),

    #[error("model registry unavailable: {0}")]
    RegistryUnavailable(String),

    #[error("docker compose exited with status {0}")]
    DockerFailed(i32),

    #[error("HTTP {status} from {url}")]
    Http { url: String, status: u16 },
}

/// Crate-wide result type. Errors are [`anyhow::Error`]; downcast to
/// [`ChapError`] when the specific variant matters.
pub type Result<T> = anyhow::Result<T>;
