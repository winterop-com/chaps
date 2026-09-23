//! `chaps docs-markdown` — the command tree as Markdown.
//!
//! The help texts in [`crate::cli`] are the only description of the flags that
//! is guaranteed to be current, so the reference chapter of the book is
//! generated from them rather than written twice. `make docs-reference` writes
//! this output to `docs/reference.md` and a test compares the two, so a flag
//! cannot change without the chapter changing with it.
//!
//! Rendering walks the static [`Cli::command()`] tree, not the one `main.rs`
//! parsed: outside a deployment directory `main.rs` hides the commands that
//! need a project, and the reference documents all of them.

use crate::cli::{Cli, DocsMarkdownArgs};
use crate::commands::Ctx;
use crate::error::Result;
use clap::{Arg, ArgAction, Command as ClapCommand, CommandFactory};
use std::fmt::Write as _;

/// The chapter's own front matter, so the generated file stands alone in the
/// book and says where it came from.
const HEADER: &str = "\
# Command reference

Generated from the `--help` texts by `make docs-reference`; edit
`src/cli.rs` and run that target rather than editing this file.

Every command accepts the global options listed under [chaps](#chaps),
`--json` included. Commands that need a deployment directory are hidden from
`chaps --help` outside one, but they are all listed here.
";

/// `chaps docs-markdown`: print the command tree as Markdown.
pub fn run(ctx: &Ctx, _args: &DocsMarkdownArgs) -> Result<()> {
    let markdown = reference();
    ctx.out
        .emit(&serde_json::json!({ "markdown": &markdown }), || {
            markdown.clone()
        })
}

/// The whole reference, exactly as `docs/reference.md` holds it.
///
/// One trailing newline and no other trailing whitespace, so a shell
/// redirection and a `assert_eq!` against the file agree byte for byte.
fn reference() -> String {
    let mut root = Cli::command();
    // `render_usage` and the propagated global options only exist once clap has
    // built the tree.
    root.build();

    // Each block below ends in a blank line, so the header needs one too.
    let mut out = format!("{HEADER}\n");
    render(&root, "chaps", true, &mut out);
    format!("{}\n", out.trim_end())
}

/// Append one command and then its visible subcommands, depth first.
fn render(command: &ClapCommand, path: &str, is_root: bool, out: &mut String) {
    let _ = writeln!(out, "## {path}\n");

    if let Some(about) = command.get_long_about().or_else(|| command.get_about()) {
        let _ = writeln!(out, "{}\n", sentence(&about.to_string()));
    }

    let _ = writeln!(out, "```text\n{}\n```\n", usage_of(command));

    let args: Vec<&Arg> = command
        .get_arguments()
        .filter(|arg| documented(arg, is_root))
        .collect();
    if !args.is_empty() {
        let heading = if is_root { "Global option" } else { "Argument" };
        let _ = writeln!(out, "| {heading} | Description |");
        let _ = writeln!(out, "| --- | --- |");
        for arg in args {
            let _ = writeln!(out, "| {} | {} |", spelling(arg), description(arg));
        }
        out.push('\n');
    }

    let subs: Vec<&ClapCommand> = command.get_subcommands().filter(documented_sub).collect();
    if !subs.is_empty() {
        let _ = writeln!(
            out,
            "Subcommands: {}\n",
            subs.iter()
                .map(|sub| format!("[`{path} {0}`](#{1})", sub.get_name(), anchor(path, sub)))
                .collect::<Vec<_>>()
                .join(", ")
        );
        for sub in subs {
            render(sub, &format!("{path} {}", sub.get_name()), false, out);
        }
    }
}

/// `Usage: chaps models enable [OPTIONS] <ID>`, as clap renders it.
///
/// Clap renders the usage from `&mut self`, so this works on a copy: the tree
/// is walked by reference and must not be mutated under the walk.
fn usage_of(command: &ClapCommand) -> String {
    command
        .clone()
        .render_usage()
        .to_string()
        .trim()
        .to_string()
}

/// Clap strips the full stop off a one-line doc comment; put it back, so the
/// rendered prose does not end mid-sentence. A multi-line `long_about` is left
/// exactly as it was written.
fn sentence(text: &str) -> String {
    let text = text.trim_end();
    if text.contains('\n') || text.is_empty() || ends_closed(text) {
        return text.to_string();
    }
    format!("{text}.")
}

/// Does this text already end in punctuation that closes it?
fn ends_closed(text: &str) -> bool {
    text.ends_with(['.', '!', '?', ':'])
}

/// The GitHub-style anchor mdbook gives `## chaps models enable`.
fn anchor(path: &str, sub: &ClapCommand) -> String {
    format!("{path} {}", sub.get_name()).replace(' ', "-")
}

/// Is this argument worth a row?
///
/// `-h` and `-V` are clap's own and say nothing about `chaps`; they are
/// recognised by their action, not their name, because `chaps models enable`
/// has a `--version` of its own that must stay. The global options are
/// documented once, under the root command, even though clap copies them into
/// every subcommand.
fn documented(arg: &Arg, is_root: bool) -> bool {
    if arg.is_hide_set() {
        return false;
    }
    if matches!(
        arg.get_action(),
        ArgAction::Help | ArgAction::HelpShort | ArgAction::HelpLong | ArgAction::Version
    ) {
        return false;
    }
    is_root || !arg.is_global_set()
}

/// Is this subcommand worth a section? `help` is clap's own, and a hidden
/// command (`docs-markdown` itself) is not part of the interface.
fn documented_sub(command: &&ClapCommand) -> bool {
    command.get_name() != "help" && !command.is_hide_set()
}

/// How the argument is typed: `<ID>`, `--port <PORT|auto>`, `-f, --follow`.
fn spelling(arg: &Arg) -> String {
    let values = if takes_a_value(arg) {
        value_names(arg)
    } else {
        // A `--flag` takes no value, even though clap gives it a value name.
        String::new()
    };
    let many = arg.get_num_args().is_some_and(|n| n.max_values() > 1);
    let repeat = if many && !values.is_empty() {
        "..."
    } else {
        ""
    };

    let mut spelling = String::new();
    if arg.is_positional() {
        spelling.push_str(&format!("{values}{repeat}"));
    } else {
        let mut names = Vec::new();
        if let Some(short) = arg.get_short() {
            names.push(format!("-{short}"));
        }
        if let Some(long) = arg.get_long() {
            names.push(format!("--{long}"));
        }
        for alias in arg.get_visible_aliases().unwrap_or_default() {
            names.push(format!("--{alias}"));
        }
        spelling.push_str(&names.join(", "));
        if !values.is_empty() {
            spelling.push_str(&format!(" {values}{repeat}"));
        }
    }
    format!("`{}`", escape(&spelling))
}

/// `<ID>`, or `<SERVICE> <CMD>` for an argument that names several.
fn value_names(arg: &Arg) -> String {
    arg.get_value_names()
        .map(|names| {
            names
                .iter()
                .map(|name| format!("<{name}>"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

/// A flag is `--json`; an option is `--api-port <PORT>`.
fn takes_a_value(arg: &Arg) -> bool {
    arg.get_num_args().is_none_or(|n| n.max_values() > 0)
}

/// The help text, plus the values and the default when clap knows them.
fn description(arg: &Arg) -> String {
    let mut text = arg
        .get_long_help()
        .or_else(|| arg.get_help())
        .map(|help| collapse(&help.to_string()))
        .unwrap_or_default();

    let possible: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|value| !value.is_hide_set())
        .map(|value| format!("`{}`", value.get_name()))
        .collect();
    if !possible.is_empty() {
        push_sentence(&mut text, &format!("Values: {}.", possible.join(", ")));
    }

    // A flag's default is `false`, which says nothing a reader needs.
    let defaults: Vec<String> = arg
        .get_default_values()
        .iter()
        .map(|value| format!("`{}`", value.to_string_lossy()))
        .filter(|_| takes_a_value(arg))
        .collect();
    if !defaults.is_empty() {
        push_sentence(&mut text, &format!("Default: {}.", defaults.join(", ")));
    }

    if text.is_empty() {
        "-".to_string()
    } else {
        escape(&sentence(&text))
    }
}

/// Append a sentence to a cell that may already end in one.
fn push_sentence(text: &mut String, sentence: &str) {
    if !text.is_empty() {
        if !text.ends_with('.') {
            text.push('.');
        }
        text.push(' ');
    }
    text.push_str(sentence);
}

/// A help text is a paragraph; a table cell is one line.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `|` ends a table cell, and both `PORT|auto` and a help text can hold one.
fn escape(text: &str) -> String {
    text.replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_path_gets_a_section() {
        let out = reference();
        for path in [
            "## chaps",
            "## chaps init",
            "## chaps models",
            "## chaps models enable",
            "## chaps models unexpose",
            "## chaps registry show",
            "## chaps docker run",
            "## chaps backup restore",
            "## chaps status",
            "## chaps update",
        ] {
            assert!(out.contains(path), "{path} is missing from the reference");
        }
    }

    #[test]
    fn the_project_only_commands_are_documented_even_though_help_hides_them() {
        let out = reference();
        for path in [
            "## chaps up",
            "## chaps down",
            "## chaps logs",
            "## chaps ui",
        ] {
            assert!(out.contains(path), "{path} is missing from the reference");
        }
    }

    #[test]
    fn clap_s_own_commands_and_the_generator_itself_stay_out() {
        let out = reference();
        assert!(!out.contains("## chaps help"));
        assert!(!out.contains("## chaps docs-markdown"));
        assert!(!out.contains("| `-h, --help` |"));
        // `chaps models enable --version` is not clap's `-V` and must stay.
        assert!(out.contains("| `--version <VERSION>` |"), "{out}");
    }

    #[test]
    fn the_globals_are_listed_once_under_the_root() {
        let out = reference();
        assert_eq!(out.matches("| `-C, --project-dir <DIR>` |").count(), 1);
        assert_eq!(out.matches("| `--json` |").count(), 1);
        assert_eq!(out.matches("| Global option | Description |").count(), 1);
    }

    #[test]
    fn usage_names_the_full_command_path() {
        let out = reference();
        assert!(
            out.contains("Usage: chaps models enable [OPTIONS] <ID>"),
            "{out}"
        );
    }

    #[test]
    fn a_table_cell_never_ends_early() {
        let out = reference();
        for line in out.lines().filter(|line| line.starts_with("| `")) {
            assert_eq!(
                line.replace("\\|", "").matches('|').count(),
                3,
                "an unescaped pipe splits this row: {line}"
            );
        }
        // `--port PORT|auto` is the row that needs the escaping.
        assert!(out.contains("--port <PORT\\|auto>"), "{out}");
    }

    #[test]
    fn help_texts_and_defaults_reach_the_table() {
        let out = reference();
        assert!(out.contains("Default: `default`"), "init --models default");
        assert!(out.contains("Default: `8000`"), "init --api-port 8000");
        assert!(
            out.contains("Never touch the network"),
            "the global --offline help"
        );
    }

    #[test]
    fn rendering_is_deterministic() {
        assert_eq!(reference(), reference());
    }
}
