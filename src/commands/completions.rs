//! `chaps completions <shell>`: a completion script on stdout.
//!
//! Generated from the same [`Cli::command()`] tree the reference chapter is
//! generated from, so a new flag is completable as soon as it exists.
//!
//! The release archives carry four of the scripts under `completions/`, named
//! the way each shell looks for them - `chaps.bash`, `_chaps`, `chaps.fish`
//! and `_chaps.ps1` - and `install.sh` puts them in place; this command is
//! what produced them.

use crate::cli::{Cli, CompletionsArgs};
use crate::commands::Ctx;
use crate::error::Result;
use clap::CommandFactory;
use clap_complete::Shell;
use std::io::Write;

/// `chaps completions SHELL`: write the script to stdout.
///
/// Not routed through [`crate::output::Out`]: a completion script is a file
/// being piped somewhere, not a report, so it is written verbatim and `--json`
/// has nothing to wrap it in.
pub fn run(_ctx: &Ctx, args: &CompletionsArgs) -> Result<()> {
    // Rendered into memory first. `clap_complete::generate` panics on a write
    // error, and `chaps completions bash | head` closes the pipe early, which
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
    clap_complete::generate(shell, &mut command, "chaps", &mut buffer);
    String::from_utf8(buffer).expect("clap_complete writes UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five shells `chaps completions` accepts, which is every shell
    /// `clap_complete::Shell` knows.
    const SHELLS: &[Shell] = &[
        Shell::Bash,
        Shell::Zsh,
        Shell::Fish,
        Shell::PowerShell,
        Shell::Elvish,
    ];

    #[test]
    fn every_shell_gets_a_script_that_completes_chaps() {
        for shell in SHELLS {
            let script = script(*shell);
            assert!(!script.trim().is_empty(), "{shell} produced nothing");
            assert!(script.contains("chaps"), "{shell} does not name chaps");
        }
    }

    /// The scripts are generated from the live command tree, so a command
    /// added to `src/cli.rs` is completable without a second edit.
    #[test]
    fn the_scripts_know_the_commands_this_build_has() {
        for shell in SHELLS {
            let script = script(*shell);
            for command in ["init", "models", "status", "self", "completions"] {
                assert!(
                    script.contains(command),
                    "{shell} does not mention `{command}`"
                );
            }
        }
    }

    /// Each shell's script is in that shell's language, so a script filed
    /// under the wrong name would be noticed here rather than by a user whose
    /// shell refuses to load it.
    #[test]
    fn each_script_is_written_for_its_own_shell() {
        assert!(script(Shell::Bash).contains("complete -F _chaps"));
        assert!(script(Shell::Zsh).starts_with("#compdef chaps"));
        assert!(script(Shell::Fish).contains("complete -c chaps"));
        assert!(script(Shell::PowerShell).contains("Register-ArgumentCompleter"));
        assert!(script(Shell::Elvish).contains("edit:completion:arg-completer"));
    }
}
