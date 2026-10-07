//! Typed errors for the parts of `varde` where the caller needs to react to the
//! kind of failure, plus the crate-wide [`Result`] alias.
//!
//! Everything that is merely context is an [`anyhow::Error`]; only the cases
//! listed in [`ChapError`] carry meaning (exit codes, TUI messages, tests).

use std::path::PathBuf;

/// Errors that callers are expected to match on.
#[derive(thiserror::Error, Debug)]
pub enum ChapError {
    #[error(
        "{0} is not a varde deployment (no .varde/project.yaml here or in a parent directory); \
         run `varde init` first"
    )]
    NotAProject(PathBuf),

    #[error("compose files are out of date with .varde/; run `varde sync`")]
    OutOfSync,

    #[error("{0} already contains a varde deployment; use --force to overwrite")]
    AlreadyInitialized(PathBuf),

    #[error("unknown model `{0}`")]
    UnknownModel(String),

    #[error("unknown component `{0}`; the components are chap-core, ocs, s3 and dhis2")]
    UnknownComponent(String),

    #[error(
        "`{0}` is a template, not a deployable model; pass `--allow-template` to enable it anyway"
    )]
    IsTemplate(String),

    #[error("model `{id}` has no version `{version}`")]
    UnknownVersion { id: String, version: String },

    #[error("version {version} of `{id}` is yanked")]
    YankedVersion { id: String, version: String },

    #[error(
        "host port {port} is already in use by {holder}; pick another with `--port <n>`, or `--port auto`"
    )]
    PortInUse { port: u16, holder: PortHolder },

    #[error(
        "host port {port} is outside this deployment's model port range {lo}-{hi}; \
         pick one inside it or `--port auto`, or widen `port_range` in `.varde/project.yaml`"
    )]
    PortOutOfRange { port: u16, lo: u16, hi: u16 },

    #[error("model registry unavailable: {0}")]
    RegistryUnavailable(String),

    #[error("docker compose exited with status {0}")]
    DockerFailed(i32),

    /// Ctrl-C stopped the command, after it took back out what it started.
    #[error("stopped by Ctrl-C; {0}")]
    Interrupted(String),

    /// A program varde ran in a one-shot container exited non-zero. Its own
    /// message is already on the screen, so this names the program, the
    /// status and the way out, and keeps the status as the exit code.
    #[error("{command} exited with status {code}; {next}")]
    Exited {
        command: &'static str,
        code: i32,
        next: String,
    },

    /// A mistake about `varde`'s own command line that clap cannot catch,
    /// such as a `-v` meant for compose reaching `varde down`'s passthrough.
    #[error("{0}")]
    Usage(String),

    #[error("HTTP {status} from {url}")]
    Http { url: String, status: u16 },

    /// chap-core could not be reached at all, as opposed to answering with
    /// something the caller did not want. Its own exit code, so a script can
    /// tell "Chap is not up" from "Chap said no".
    #[error("chap-core at {url} is not responding: {reason}; run `varde status`")]
    Unreachable { url: String, reason: String },

    /// The same for DHIS2, in its own words: a DHIS2 that is not answering is
    /// a different problem from a chap-core that is not, and the sentence has
    /// to send the reader to the right container. Its own exit code for the
    /// same reason [`ChapError::Unreachable`] has one.
    ///
    /// `next` is the way out: for the deployment's own DHIS2 that is its
    /// container, and for an external one the URL `varde dhis2 use` recorded.
    #[error("DHIS2 at {url} is not responding: {reason}; {next}")]
    Dhis2Unreachable {
        url: String,
        reason: String,
        next: &'static str,
    },
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
mod tests;
