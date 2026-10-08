use super::*;

/// The five shells `varde completions` accepts, which is every shell
/// `clap_complete::Shell` knows.
const SHELLS: &[Shell] = &[
    Shell::Bash,
    Shell::Zsh,
    Shell::Fish,
    Shell::PowerShell,
    Shell::Elvish,
];

#[test]
fn every_shell_gets_a_script_that_completes_varde() {
    for shell in SHELLS {
        let script = script(*shell);
        assert!(!script.trim().is_empty(), "{shell} produced nothing");
        assert!(script.contains("varde"), "{shell} does not name varde");
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
    assert!(script(Shell::Bash).contains("complete -F _varde"));
    assert!(script(Shell::Zsh).starts_with("#compdef varde"));
    assert!(script(Shell::Fish).contains("complete -c varde"));
    assert!(script(Shell::PowerShell).contains("Register-ArgumentCompleter"));
    assert!(script(Shell::Elvish).contains("edit:completion:arg-completer"));
}

/// `vg` is the link `install.sh` and `make install` add, so every script
/// registers it next to `varde`.
#[test]
fn every_script_also_completes_the_short_form() {
    let bash = script(Shell::Bash);
    assert!(
        bash.contains("-o nosort -o bashdefault -o default varde vg\n"),
        "{bash}"
    );
    assert!(bash.contains("complete -F _varde -o bashdefault -o default varde vg\n"));
    let zsh = script(Shell::Zsh);
    assert!(zsh.starts_with("#compdef varde vg\n"));
    assert!(zsh.contains("compdef _varde varde vg\n"));
    assert!(script(Shell::Fish).ends_with("complete -c vg -w varde\n"));
    assert!(script(Shell::PowerShell).contains("-CommandName 'varde', 'vg'"));
    assert!(
        script(Shell::Elvish).contains("arg-completer[vg] = $edit:completion:arg-completer[varde]")
    );
}
