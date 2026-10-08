//! `varde completions <shell>`: a completion script on stdout.
//!
//! Generated from the same [`crate::cli::command()`] tree the reference chapter is
//! generated from, so a new flag is completable as soon as it exists.
//!
//! The release archives carry four of the scripts under `completions/`, named
//! the way each shell looks for them - `varde.bash`, `_varde`, `varde.fish`
//! and `_varde.ps1` - and `install.sh` puts them in place; this command is
//! what produced them.
//!
//! Every script also registers `vg`, the short form `install.sh` and `make
//! install` link next to the binary, so `vg <TAB>` completes like `varde <TAB>`.

use crate::cli::CompletionsArgs;
use crate::commands::Ctx;
use crate::error::Result;
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

/// The short form of `varde` that the install script links next to it.
pub const SHORT_NAME: &str = "vg";

/// The completion script for `shell`.
pub fn script(shell: Shell) -> String {
    let mut command = crate::cli::command();
    let mut buffer = Vec::new();
    clap_complete::generate(shell, &mut command, "varde", &mut buffer);
    let script = String::from_utf8(buffer).expect("clap_complete writes UTF-8");
    with_short_form(shell, script)
}

/// `script` with `vg` registered next to `varde`.
///
/// clap_complete knows one name per script. Each generated completer reads
/// the subcommands from the words after the first, so the same function
/// serves the link; only the line that registers it has to name both.
fn with_short_form(shell: Shell, script: String) -> String {
    match shell {
        Shell::Bash => script.replace(
            " -o default varde\n",
            &format!(" -o default varde {SHORT_NAME}\n"),
        ),
        Shell::Zsh => script
            .replacen(
                "#compdef varde\n",
                &format!("#compdef varde {SHORT_NAME}\n"),
                1,
            )
            .replace(
                "compdef _varde varde\n",
                &format!("compdef _varde varde {SHORT_NAME}\n"),
            ),
        Shell::Fish => format!("{script}complete -c {SHORT_NAME} -w varde\n"),
        Shell::PowerShell => script.replacen(
            "-CommandName 'varde'",
            &format!("-CommandName 'varde', '{SHORT_NAME}'"),
            1,
        ),
        Shell::Elvish => format!(
            "{script}\nset edit:completion:arg-completer[{SHORT_NAME}] = \
             $edit:completion:arg-completer[varde]\n"
        ),
        _ => script,
    }
}

#[cfg(test)]
mod tests;
