//! `varde completions <shell>`: a completion script on stdout.
//!
//! Generated from the same [`Cli::command()`] tree the reference chapter is
//! generated from, so a new flag is completable as soon as it exists.
//!
//! The release archives carry four of the scripts under `completions/`, named
//! the way each shell looks for them - `varde.bash`, `_varde`, `varde.fish`
//! and `_varde.ps1` - and `install.sh` puts them in place; this command is
//! what produced them.

use crate::cli::{Cli, CompletionsArgs};
use crate::commands::Ctx;
use crate::error::Result;
use clap::CommandFactory;
use clap_complete::Shell;
use std::io::Write;

/// `varde completions SHELL`: write the script to stdout.
///
/// Not routed through [`crate::output::Out`]: a completion script is a file
/// being piped somewhere, not a report, so it is written verbatim and `--json`
/// has nothing to wrap it in.
pub fn run(_ctx: &Ctx, args: &CompletionsArgs) -> Result<()> {
    // Rendered into memory first. `clap_complete::generate` panics on a write
    // error, and `varde completions bash | head` closes the pipe early, which
    // is a broken pipe and not a failure.
    let script = script(args.shell);
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(script.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// The completion script for `shell`.
pub fn script(shell: Shell) -> String {
    let mut command = Cli::command();
    let mut buffer = Vec::new();
    clap_complete::generate(shell, &mut command, "varde", &mut buffer);
    String::from_utf8(buffer).expect("clap_complete writes UTF-8")
}

#[cfg(test)]
mod tests;
