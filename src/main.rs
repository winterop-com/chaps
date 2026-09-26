//! `chaps` — deploy and manage CHAP, the Climate Health Analytics Platform:
//! chap-core and marketplace model services on Docker Compose.

// Stubs owned by agents A, B and C are not called yet; remove after A/B/C land.

mod api;
mod auth;
mod backup;
mod chapcore;
mod cli;
mod commands;
mod components;
mod compose;
mod diagnose;
mod docker;
mod dotenv;
mod error;
mod github;
mod jobs;
mod manual;
mod modeltest;
mod output;
mod paths;
mod ports;
mod project;
mod registry;
mod selfupdate;
mod status;
mod tui;

use clap::{CommandFactory, FromArgMatches};
use cli::{
    AuthSub, BackupSub, Cli, Command, ComponentsCmd, DockerCmd, JobsCmd, ModelsCmd, SelfSub,
};
use commands::Ctx;
use error::ChapError;
use output::Out;
use project::Project;
use std::path::{Path, PathBuf};

/// Commands that need a deployment directory, hidden from `--help` when there
/// is none. They still run when typed, and say what is missing.
const PROJECT_ONLY: &[&str] = &[
    "up",
    "down",
    "logs",
    "restart",
    "docker",
    "backup",
    "status",
    "jobs",
    "sync",
    "update",
    "ui",
    "auth",
    "components",
];

/// The same, for the subcommands of `models`: browsing the marketplace works
/// anywhere, changing a project's model set does not.
const PROJECT_ONLY_MODELS: &[&str] = &[
    "add", "remove", "enable", "disable", "expose", "unexpose", "test",
];

/// The mirror of [`PROJECT_ONLY`]: commands that answer "there is no
/// deployment here", hidden from `--help` inside one, where that is not the
/// question. They still run when typed: `chaps init --force` is how a
/// deployment's settings are rewritten, and `chaps init sub` nests a second
/// one.
const OUTSIDE_ONLY: &[&str] = &["init"];

/// The [`PROJECT_ONLY`] commands that are a group rather than a verb, so clap
/// answers the bare command with the group's help.
///
/// Outside a deployment that help lists only subcommands that cannot run, so
/// the missing project is the answer instead - the same one `chaps components
/// list` gives. `jobs` is not here: its bare form is `list`, which already
/// says so. `models` and `registry` are not project-only at all.
const PROJECT_ONLY_GROUPS: &[&str] = &["auth", "backup", "components", "docker"];

fn main() {
    // Windows cannot rename over a running image, so `chaps self update`
    // parks the outgoing binary beside the new one and the next run is the
    // first moment it can be deleted.
    #[cfg(windows)]
    if let Ok(path) = std::env::current_exe() {
        selfupdate::clean_backup(&path);
    }

    let cli = parse();
    let ctx = Ctx::from_cli(&cli);

    if let Err(err) = dispatch(&ctx, &cli) {
        fail(&ctx.out, err);
    }

    // Only after the command said what it did, and only when it worked: a
    // notice ahead of the answer would be noise, and one after a failure
    // would be advice about the wrong problem.
    if wants_update_notice(&cli.command) {
        commands::selfcmd::notify(&ctx);
    }
}

/// Report a failure the way every `chaps` failure is reported, and exit.
fn fail(out: &Out, err: anyhow::Error) -> ! {
    let rendered = out.error(&err);
    // JSON consumers read stdout, so a --json error goes there too.
    if out.json {
        println!("{rendered}");
    } else {
        eprintln!("{rendered}");
    }
    std::process::exit(exit_code(&err));
}

/// Whether this command may be followed by the once-a-day update notice.
///
/// Everything except the `self` group, which is the thing the notice would be
/// telling you to run, and `docs-markdown`, whose output is a file. The rest
/// of the conditions (a terminal, no `--json`, no `--offline`) are
/// [`commands::selfcmd::notify`]'s to check.
fn wants_update_notice(command: &Command) -> bool {
    !matches!(command, Command::SelfCmd(_) | Command::DocsMarkdown(_))
}

fn dispatch(ctx: &Ctx, cli: &Cli) -> error::Result<()> {
    match &cli.command {
        Command::Init(args) => commands::init::run(ctx, args),

        Command::Models(m) => match &m.command {
            ModelsCmd::List(args) => commands::models::list(ctx, args),
            ModelsCmd::Search(args) => commands::models::search(ctx, args),
            ModelsCmd::Info(args) => commands::models::info(ctx, args),
            ModelsCmd::Test(args) => commands::modeltest::run(ctx, args),
            ModelsCmd::Add(args) => commands::manual_models::add(ctx, args),
            ModelsCmd::Remove(args) => commands::manual_models::remove(ctx, args),
            ModelsCmd::Enable(args) => commands::enable::enable(ctx, args),
            ModelsCmd::Disable(args) => commands::enable::disable(ctx, args),
            ModelsCmd::Expose(args) => commands::enable::expose(ctx, args),
            ModelsCmd::Unexpose(args) => commands::enable::unexpose(ctx, args),
        },
        Command::Components(c) => match &c.command {
            ComponentsCmd::List(args) => commands::components::list(ctx, args),
            ComponentsCmd::Enable(args) => commands::components::enable(ctx, args),
            ComponentsCmd::Disable(args) => commands::components::disable(ctx, args),
        },

        Command::Ui(args) => run_ui(ctx, args),

        Command::Registry(r) => commands::registry::run(ctx, &r.command),

        Command::Sync(args) => commands::sync::run(ctx, args),
        Command::Update(args) => commands::update::run(ctx, args),

        Command::Up(args) => commands::docker::run(ctx, &DockerCmd::Up(args.clone())),
        Command::Down(args) => commands::docker::run(ctx, &DockerCmd::Down(args.clone())),
        Command::Logs(args) => commands::docker::run(ctx, &DockerCmd::Logs(args.clone())),
        Command::Restart(args) => commands::docker::run(ctx, &DockerCmd::Restart(args.clone())),
        Command::Docker(d) => commands::docker::run(ctx, &DockerCmd::from(&d.command)),

        Command::Backup(b) => match &b.command {
            BackupSub::Create(args) => commands::backup::run(ctx, args),
            BackupSub::Restore(args) => commands::restore::run(ctx, args),
        },

        Command::Status(args) => commands::status::run(ctx, args),

        // No subcommand is `list`: "what has this deployment been doing" is
        // the question `chaps jobs` is typed to answer.
        Command::Jobs(j) => match &j.command {
            None => commands::jobs::list(ctx, &j.list),
            Some(JobsCmd::List(args)) => commands::jobs::list(ctx, args),
            Some(JobsCmd::Show(args)) => commands::jobs::show(ctx, args),
            Some(JobsCmd::Logs(args)) => commands::jobs::logs(ctx, args),
            Some(JobsCmd::Cancel(args)) => commands::jobs::cancel(ctx, args),
            Some(JobsCmd::Delete(args)) => commands::jobs::delete(ctx, args),
        },

        Command::Api(args) => commands::api::run(ctx, args),

        Command::Doctor(args) => commands::doctor::run(ctx, args),

        Command::Auth(a) => match &a.command {
            AuthSub::Show(args) => commands::auth::show(ctx, args),
            AuthSub::Token(args) => commands::auth::token(ctx, args),
            AuthSub::Enable(args) => commands::auth::enable(ctx, args),
            AuthSub::Disable(args) => commands::auth::disable(ctx, args),
            AuthSub::Rotate(args) => commands::auth::rotate(ctx, args),
        },

        Command::SelfCmd(s) => match &s.command {
            SelfSub::Update(args) => commands::selfcmd::update(ctx, args),
            SelfSub::Version(args) => commands::selfcmd::version(ctx, args),
        },

        Command::Completions(args) => commands::completions::run(ctx, args),

        Command::DocsMarkdown(args) => commands::docs::run(ctx, args),
    }
}

/// Parse the command line against a help listing shaped by where we are.
///
/// Outside a deployment directory most of the tree cannot do anything, so it is
/// hidden rather than offered; the commands still parse and run, and fail with
/// the error that names the missing `.chaps/project.yaml`. Inside one the
/// hiding runs the other way, over the commands that are about not having a
/// deployment yet.
fn parse() -> Cli {
    let argv: Vec<String> = std::env::args().collect();
    let project_dir = project_dir_of(&argv[1..]);
    let inside = Project::find_root(&project_dir).is_some();
    if !inside && let Some(matches) = bare_project_group(&argv) {
        no_project(&matches, &project_dir);
    }
    let command = if inside {
        hide_outside_commands(Cli::command())
    } else {
        hide_project_commands(Cli::command())
    };
    let matches = match command.try_get_matches_from(&argv) {
        Ok(matches) => matches,
        Err(err) => print_and_exit(err),
    };
    match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(err) => print_and_exit(err),
    }
}

/// Print what clap has to say - help, the version, or a usage error - and exit
/// with its code, leaving a blank line after the help that ends on the book.
///
/// The blank line cannot come from `after_help`: clap renders the help,
/// `trim_end`s it and appends exactly one newline, so a newline written into
/// [`cli::DOCS_LINE`] or into the `after_help` value never reaches the
/// terminal. It is added here instead, where the help is printed, which is
/// also what it is: a blank line between the book's address and the prompt,
/// not part of the address. Only the root help gets it - it is the one that
/// ends on the docs line, however it was reached (`chaps --help`, `chaps -h`,
/// `chaps help`, or `chaps` with nothing after it) - so a subcommand's help
/// still ends on its last line.
fn print_and_exit(err: clap::Error) -> ! {
    use clap::error::ErrorKind;
    use std::io::Write;

    let ends_on_the_docs_line = matches!(
        err.kind(),
        ErrorKind::DisplayHelp | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
    ) && err
        .render()
        .to_string()
        .trim_end()
        .ends_with(cli::DOCS_LINE);
    // Broken pipes are swallowed, as clap's own `exit` does.
    let _ = err.print();
    if ends_on_the_docs_line {
        if err.use_stderr() {
            let mut stream = std::io::stderr();
            let _ = stream.write_all(b"\n");
            let _ = stream.flush();
        } else {
            let mut stream = std::io::stdout();
            let _ = stream.write_all(b"\n");
            let _ = stream.flush();
        }
    }
    std::process::exit(err.exit_code());
}

/// Hide the commands that need a project: outside one they cannot work.
fn hide_project_commands(mut command: clap::Command) -> clap::Command {
    for name in PROJECT_ONLY {
        command = command.mut_subcommand(name, |c| c.hide(true));
    }
    command.mut_subcommand("models", |models| {
        PROJECT_ONLY_MODELS.iter().fold(models, |models, name| {
            models.mut_subcommand(name, |c| c.hide(true))
        })
    })
}

/// Hide the commands that are about not having a project yet: inside one they
/// are not what to run next.
fn hide_outside_commands(mut command: clap::Command) -> clap::Command {
    for name in OUTSIDE_ONLY {
        command = command.mut_subcommand(name, |c| c.hide(true));
    }
    command
}

/// The matches for a [`PROJECT_ONLY_GROUPS`] command typed with nothing after
/// it, and `None` for every other command line.
///
/// Clap's own answer to a bare group is `arg_required_else_help`, and outside
/// a deployment that help lists only subcommands that cannot run either. The
/// classification is a parse of its own, against a throwaway tree that lets
/// the bare form through, so that the tree the command actually runs against
/// keeps clap's required-subcommand usage line and its own help. Anything
/// else - a subcommand, an unknown word, `--help` - stays clap's to answer.
fn bare_project_group(argv: &[String]) -> Option<clap::ArgMatches> {
    let mut command = Cli::command();
    for name in PROJECT_ONLY_GROUPS {
        command = command.mut_subcommand(name, |c| {
            c.subcommand_required(false).arg_required_else_help(false)
        });
    }
    let matches = command.try_get_matches_from(argv).ok()?;
    let (name, group) = matches.subcommand()?;
    (PROJECT_ONLY_GROUPS.contains(&name) && group.subcommand().is_none()).then_some(matches)
}

/// The missing-project error, reported before there is a [`Cli`] to carry the
/// flags its rendering reads: only `--json` and `--no-color` matter, and clap
/// has both already.
fn no_project(matches: &clap::ArgMatches, project_dir: &Path) -> ! {
    let out = Out::detect(matches.get_flag("json"), matches.get_flag("no_color"));
    let shown = std::path::absolute(project_dir).unwrap_or_else(|_| project_dir.to_path_buf());
    fail(&out, ChapError::NotAProject(shown).into())
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
        // The code clap exits with for a usage error, because that is what
        // this is: a usage error clap could not catch.
        Some(ChapError::Usage(_)) => 2,
        // "CHAP is not up" is a different answer from "CHAP said no", and a
        // script driving `chaps api` has to be able to tell them apart.
        Some(ChapError::Unreachable { .. }) => 2,
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

    /// A usage error clap could not catch exits the way clap's own do, so a
    /// script can tell "you typed it wrong" from "it did not work".
    #[test]
    fn a_usage_error_exits_two() {
        let err = anyhow::Error::new(ChapError::Usage("`-v` is --verbose".to_string()));
        assert_eq!(exit_code(&err), 2);
        assert_eq!(err.to_string(), "`-v` is --verbose");
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
        for shown in ["init", "models", "registry", "doctor"] {
            assert!(help.contains(shown), "`{shown}` works anywhere");
        }
        // Hiding is the whole of it: the help still ends on the docs link.
        assert!(help.trim_end().ends_with(cli::DOCS_LINE));

        // Hidden is not gone: the command still parses and runs.
        assert!(matches!(
            Cli::try_parse_from(["chap", "up"]).unwrap().command,
            Command::Up(_)
        ));
    }

    #[test]
    fn inside_a_project_the_help_hides_only_what_belongs_outside() {
        let mut command = hide_outside_commands(Cli::command());
        let help = command.render_long_help().to_string();
        for name in PROJECT_ONLY {
            assert!(
                help.contains(&format!("\n  {name} ")),
                "`{name}` should be listed in a project"
            );
        }
        for hidden in OUTSIDE_ONLY {
            assert!(
                !help.contains(&format!("\n  {hidden} ")) && !help.ends_with(hidden),
                "`{hidden}` should not be listed inside a project"
            );
        }
        assert!(help.trim_end().ends_with(cli::DOCS_LINE));

        // Hidden is not gone: `chaps init --force` over a deployment is the
        // documented way to change what it was created with.
        assert!(matches!(
            Cli::try_parse_from(["chap", "init", "--force"])
                .unwrap()
                .command,
            Command::Init(_)
        ));
    }

    /// The two lists are opposites, so nothing may sit in both: a command in
    /// both would be hidden wherever it was typed.
    #[test]
    fn the_two_hidden_lists_do_not_overlap() {
        for name in OUTSIDE_ONLY {
            assert!(!PROJECT_ONLY.contains(name), "`{name}` is in both lists");
        }
        for name in PROJECT_ONLY_GROUPS {
            assert!(
                PROJECT_ONLY.contains(name),
                "`{name}` is a group that needs a project but is not hidden outside one"
            );
        }
    }

    /// Outside a project a bare group command is a missing project, not a
    /// help listing of subcommands that cannot run either.
    #[test]
    fn outside_a_project_a_bare_group_command_asks_for_a_project() {
        for name in PROJECT_ONLY_GROUPS {
            assert!(
                bare_project_group(&argv(&["chap", name])).is_some(),
                "`chaps {name}` should ask for a project"
            );
        }

        // A named subcommand is the subcommand's business, the groups that
        // work anywhere keep their own help, and so does `--help` itself.
        for args in [
            vec!["chap", "components", "list"],
            vec!["chap", "auth", "show"],
            vec!["chap", "components", "--help"],
            vec!["chap", "components", "bogus"],
            vec!["chap", "models"],
            vec!["chap", "registry"],
            vec!["chap", "jobs"],
            vec!["chap", "doctor"],
            vec!["chap", "--help"],
        ] {
            assert!(bare_project_group(&argv(&args)).is_none(), "{args:?}");
        }

        // The globals are read off the same matches the rendering needs, in
        // either position.
        for args in [
            vec!["chap", "--json", "backup"],
            vec!["chap", "backup", "--json"],
        ] {
            let matches = bare_project_group(&argv(&args)).expect("a bare group");
            assert!(matches.get_flag("json"), "{args:?}");
            assert!(!matches.get_flag("no_color"), "{args:?}");
        }
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
