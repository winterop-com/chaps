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

    // Hidden is not gone: `varde init --force` over a deployment is the
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
            "`varde {name}` should ask for a project"
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
