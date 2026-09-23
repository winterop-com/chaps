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

    #[error("unknown component `{0}`; the components are chap-core, ocs and s3")]
    UnknownComponent(String),

    #[error(
        "`{0}` is a template, not a deployable model; pass --allow-template to enable it anyway"
    )]
    IsTemplate(String),

    #[error("model `{id}` has no version `{version}`")]
    UnknownVersion { id: String, version: String },

    #[error("version {version} of `{id}` is yanked")]
    YankedVersion { id: String, version: String },

    #[error("host port {port} is already in use by {holder}")]
    PortInUse { port: u16, holder: PortHolder },

    #[error("host port {0} is outside the allowed range")]
    PortOutOfRange(u16),

    #[error("model registry unavailable: {0}")]
    RegistryUnavailable(String),

    #[error("docker compose exited with status {0}")]
    DockerFailed(i32),

    #[error("HTTP {status} from {url}")]
    Http { url: String, status: u16 },
}

/// Who holds a host port that was asked for, so the error can say where to
/// look: inside the deployment, or outside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortHolder {
    /// A service in this project, or any `compose*.yml` in its directory.
    ComposeFile,
    /// Something outside this project has a listener bound to it.
    Host,
}

impl std::fmt::Display for PortHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortHolder::ComposeFile => f.write_str("another compose file"),
            PortHolder::Host => f.write_str("the host (something is listening)"),
        }
    }
}

/// Crate-wide result type. Errors are [`anyhow::Error`]; downcast to
/// [`ChapError`] when the specific variant matters.
pub type Result<T> = anyhow::Result<T>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_port_says_who_has_it() {
        assert_eq!(
            ChapError::PortInUse {
                port: 5001,
                holder: PortHolder::ComposeFile,
            }
            .to_string(),
            "host port 5001 is already in use by another compose file"
        );
        assert_eq!(
            ChapError::PortInUse {
                port: 8000,
                holder: PortHolder::Host,
            }
            .to_string(),
            "host port 8000 is already in use by the host (something is listening)"
        );
    }
}
