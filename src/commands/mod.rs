//! One module per command, plus the [`Ctx`] every command receives.

pub mod auth;
pub mod backup;
pub mod completions;
pub mod components;
pub mod docker;
pub mod docs;
pub mod doctor;
pub mod enable;
pub mod init;
pub mod models;
pub mod registry;
pub mod restore;
pub mod selfcmd;
pub mod status;
pub mod sync;
pub mod tui;
pub mod update;

use crate::cli::Cli;
use crate::output::Out;
use crate::registry::{DEFAULT_TIMEOUT, RegistryOptions};
use std::path::PathBuf;

/// Everything a command needs that came from the global flags.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub out: Out,
    pub project_dir: PathBuf,
    pub registry: RegistryOptions,
    pub cli_version: &'static str,
}

impl Ctx {
    /// Build a context from the parsed global flags.
    pub fn from_cli(cli: &Cli) -> Ctx {
        // The stderr side of the CLI has no `Ctx` to consult, so the flags
        // are recorded once here for `output::warn` and the trace helpers.
        crate::output::set_no_color(cli.no_color);
        let verbosity = crate::output::set_verbosity(cli.verbose, cli.debug);
        Ctx {
            out: Out {
                verbosity,
                ..Out::detect(cli.json, cli.no_color)
            },
            project_dir: cli.project_dir.clone(),
            registry: RegistryOptions {
                url: cli.registry_url.clone(),
                offline: cli.offline,
                cache_dir: cli
                    .cache_dir
                    .clone()
                    .unwrap_or_else(crate::paths::cache_dir),
                timeout: DEFAULT_TIMEOUT,
            },
            cli_version: env!("CARGO_PKG_VERSION"),
        }
    }

    /// Load the project that contains [`Ctx::project_dir`], walking up parent
    /// directories the way git finds `.git`.
    pub fn project(&self) -> crate::error::Result<crate::project::Project> {
        let project = crate::project::Project::find(&self.project_dir)?;
        // Which deployment a command actually landed on is the first thing to
        // check when it is not the one you meant.
        self.out.debug(&format!(
            "project: {} (state in {})",
            project.dir.display(),
            project
                .dir
                .join(crate::project::CHAPS_DIR)
                .join(crate::project::PROJECT_FILE)
                .display()
        ));
        Ok(project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn from_cli_carries_the_global_flags() {
        let cli = Cli::try_parse_from([
            "chap",
            "--json",
            "-C",
            "/tmp/chapx",
            "--offline",
            "--cache-dir",
            "/tmp/chapcache",
            "--registry-url",
            "https://example.test/registry.yaml",
            "status",
        ])
        .unwrap();
        let ctx = Ctx::from_cli(&cli);
        assert!(ctx.out.json);
        assert_eq!(ctx.project_dir, PathBuf::from("/tmp/chapx"));
        assert!(ctx.registry.offline);
        assert_eq!(ctx.registry.cache_dir, PathBuf::from("/tmp/chapcache"));
        assert_eq!(ctx.registry.url, "https://example.test/registry.yaml");
        assert_eq!(ctx.cli_version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn cache_dir_falls_back_when_the_flag_is_absent() {
        let cli = Cli::try_parse_from(["chap", "status"]).unwrap();
        let ctx = Ctx::from_cli(&cli);
        assert_eq!(ctx.registry.cache_dir, crate::paths::cache_dir());
    }
}
