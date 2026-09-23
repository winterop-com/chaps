//! `chaps` — deploy and manage CHAP, the DHIS2 Climate Health Analytics
//! Platform: chap-core and marketplace model services on Docker Compose.

// Stubs owned by agents A, B and C are not called yet; remove after A/B/C land.

mod backup;
mod chapcore;
mod cli;
mod commands;
mod compose;
mod docker;
mod error;
mod output;
mod paths;
mod ports;
mod project;
mod registry;
mod status;
mod tui;

use clap::{CommandFactory, FromArgMatches};
use cli::{BackupSub, Cli, Command, DockerCmd, ModelsCmd};
use commands::Ctx;
use error::ChapError;
use project::Project;
use std::path::PathBuf;

/// Commands that need a deployment directory, hidden from `--help` when there
/// is none. They still run when typed, and say what is missing.
const PROJECT_ONLY: &[&str] = &[
    "up", "down", "logs", "docker", "backup", "status", "sync", "update", "ui",
];

/// The same, for the subcommands of `models`: browsing the marketplace works
/// anywhere, changing a project's model set does not.
const PROJECT_ONLY_MODELS: &[&str] = &["enable", "disable", "expose", "unexpose"];

/// The line appended to `--help` outside a project, so the hidden half of the
/// tree is not a surprise.
const OUTSIDE_PROJECT_HINT: &str = "Inside a directory created by `chaps init`, \
     more commands appear: up, down, logs, status, sync, update, ui, docker, backup.";

fn main() {
    let cli = parse();
    let ctx = Ctx::from_cli(&cli);

    if let Err(err) = dispatch(&ctx, &cli) {
        let rendered = ctx.out.error(&err);
        // JSON consumers read stdout, so a --json error goes there too.
        if ctx.out.json {
            println!("{rendered}");
        } else {
            eprintln!("{rendered}");
        }
        std::process::exit(exit_code(&err));
    }
}

fn dispatch(ctx: &Ctx, cli: &Cli) -> error::Result<()> {
    match &cli.command {
        Command::Init(args) => commands::init::run(ctx, args),

        Command::Models(m) => match &m.command {
            ModelsCmd::List(args) => commands::models::list(ctx, args),
            ModelsCmd::Search(args) => commands::models::search(ctx, args),
            ModelsCmd::Info(args) => commands::models::info(ctx, args),
            ModelsCmd::Enable(args) => commands::enable::enable(ctx, args),
            ModelsCmd::Disable(args) => commands::enable::disable(ctx, args),
            ModelsCmd::Expose(args) => commands::enable::expose(ctx, args),
            ModelsCmd::Unexpose(args) => commands::enable::unexpose(ctx, args),
        },
        Command::Ui(args) => run_ui(ctx, args),

        Command::Registry(r) => commands::registry::run(ctx, &r.command),

        Command::Sync(args) => commands::sync::run(ctx, args),
        Command::Update(args) => commands::update::run(ctx, args),

        Command::Up(args) => commands::docker::run(ctx, &DockerCmd::Up(args.clone())),
        Command::Down(args) => commands::docker::run(ctx, &DockerCmd::Down(args.clone())),
        Command::Logs(args) => commands::docker::run(ctx, &DockerCmd::Logs(args.clone())),
        Command::Docker(d) => commands::docker::run(ctx, &DockerCmd::from(&d.command)),

        Command::Backup(b) => match &b.command {
            BackupSub::Create(args) => commands::backup::run(ctx, args),
            BackupSub::Restore(args) => commands::restore::run(ctx, args),
        },

        Command::Status(args) => commands::status::run(ctx, args),

        Command::DocsMarkdown(args) => commands::docs::run(ctx, args),
    }
}

/// Parse the command line against a help listing shaped by where we are.
///
/// Outside a deployment directory most of the tree cannot do anything, so it is
/// hidden rather than offered; the commands still parse and run, and fail with
/// the error that names the missing `.chaps/project.yaml`.
fn parse() -> Cli {
    let argv: Vec<String> = std::env::args().collect();
    let mut command = Cli::command();
    if Project::find_root(&project_dir_of(&argv[1..])).is_none() {
        command = hide_project_commands(command);
    }
    let matches = command.get_matches_from(argv);
    match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(err) => err.exit(),
    }
}

/// Hide the commands that need a project, and say so at the end of `--help`.
fn hide_project_commands(command: clap::Command) -> clap::Command {
    let mut command = command.after_help(OUTSIDE_PROJECT_HINT);
    for name in PROJECT_ONLY {
        command = command.mut_subcommand(name, |c| c.hide(true));
    }
    command.mut_subcommand("models", |models| {
        PROJECT_ONLY_MODELS.iter().fold(models, |models, name| {
            models.mut_subcommand(name, |c| c.hide(true))
        })
    })
}

/// The `-C` / `--project-dir` value, read straight out of the raw arguments.
///
/// The help text depends on it, and help is built before clap parses, so this
/// runs first. It accepts the spellings clap does (`-C dir`, `-Cdir`, `-C=dir`,
/// `--project-dir dir`, `--project-dir=dir`) and otherwise falls back to `.`,
/// the flag's own default. Getting it wrong only changes which commands are
/// listed, never which ones run.
fn project_dir_of(args: &[String]) -> PathBuf {
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let value = if let Some(v) = arg.strip_prefix("--project-dir=") {
            Some(v)
        } else if arg == "--project-dir" || arg == "-C" {
            rest.next().map(String::as_str)
        } else if let Some(v) = arg.strip_prefix("-C").filter(|v| !v.is_empty()) {
            Some(v.strip_prefix('=').unwrap_or(v))
        } else {
            None
        };
        if let Some(value) = value {
            return PathBuf::from(value);
        }
    }
    PathBuf::from(".")
}

/// The browser owns the terminal, so there is nothing to serialise: reject
/// `--json` rather than print something a caller cannot parse.
fn run_ui(ctx: &Ctx, args: &cli::UiArgs) -> error::Result<()> {
    if ctx.out.json {
        return Err(anyhow::anyhow!(
            "--json cannot be combined with the model browser"
        ));
    }
    commands::tui::run(ctx, args)
}

/// Mirror docker compose's exit code when it is the thing that failed, so
/// `chaps up` is a drop-in for `docker compose up` in scripts.
fn exit_code(err: &anyhow::Error) -> i32 {
    match err.downcast_ref::<ChapError>() {
        Some(ChapError::DockerFailed(code)) if *code != 0 => *code,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn docker_failures_keep_their_exit_code() {
        let err = anyhow::Error::new(ChapError::DockerFailed(137));
        assert_eq!(exit_code(&err), 137);
    }

    #[test]
    fn everything_else_exits_one() {
        assert_eq!(exit_code(&anyhow::anyhow!("boom")), 1);
        assert_eq!(
            exit_code(&anyhow::Error::new(ChapError::DockerFailed(0))),
            1
        );
        assert_eq!(
            exit_code(&anyhow::Error::new(ChapError::UnknownModel("x".into()))),
            1
        );
    }

    #[test]
    fn the_project_dir_flag_is_read_in_every_spelling() {
        for args in [
            vec!["-C", "/tmp/chapx", "status"],
            vec!["-C/tmp/chapx", "status"],
            vec!["-C=/tmp/chapx", "status"],
            vec!["--project-dir", "/tmp/chapx", "status"],
            vec!["--project-dir=/tmp/chapx", "status"],
            vec!["--offline", "--json", "-C", "/tmp/chapx", "status"],
            vec!["status", "-C", "/tmp/chapx"],
        ] {
            assert_eq!(
                project_dir_of(&argv(&args)),
                PathBuf::from("/tmp/chapx"),
                "{args:?}"
            );
        }
    }

    #[test]
    fn the_project_dir_falls_back_to_the_flags_own_default() {
        assert_eq!(project_dir_of(&argv(&["status"])), PathBuf::from("."));
        assert_eq!(project_dir_of(&[]), PathBuf::from("."));
        // A trailing flag with no value is clap's error to report, not ours.
        assert_eq!(project_dir_of(&argv(&["-C"])), PathBuf::from("."));
        // --cache-dir starts with neither spelling and must not be mistaken
        // for one.
        assert_eq!(
            project_dir_of(&argv(&["--cache-dir", "/tmp/c", "status"])),
            PathBuf::from(".")
        );
    }

    #[test]
    fn outside_a_project_the_help_offers_only_what_works() {
        let mut command = hide_project_commands(Cli::command());
        let help = command.render_long_help().to_string();
        for hidden in PROJECT_ONLY {
            assert!(
                !help.contains(&format!("\n  {hidden} ")) && !help.ends_with(hidden),
                "`{hidden}` should not be listed outside a project"
            );
        }
        for shown in ["init", "models", "registry"] {
            assert!(help.contains(shown), "`{shown}` works anywhere");
        }
        assert!(help.contains("Inside a directory created by `chaps init`"));

        // Hidden is not gone: the command still parses and runs.
        assert!(matches!(
            Cli::try_parse_from(["chap", "up"]).unwrap().command,
            Command::Up(_)
        ));
    }

    #[test]
    fn inside_a_project_nothing_is_hidden() {
        let help = Cli::command().render_long_help().to_string();
        for name in PROJECT_ONLY {
            assert!(
                help.contains(name),
                "`{name}` should be listed in a project"
            );
        }
        assert!(!help.contains("Inside a directory created by `chaps init`"));
    }

    #[test]
    fn json_rejects_the_browser() {
        let cli = Cli::try_parse_from(["chap", "--json", "ui"]).unwrap();
        let ctx = Ctx::from_cli(&cli);
        let Command::Ui(args) = &cli.command else {
            panic!("expected ui");
        };
        let err = run_ui(&ctx, args).expect_err("--json with the browser is rejected");
        assert!(err.to_string().contains("--json"));
    }
}
