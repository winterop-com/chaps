use super::*;

#[test]
fn every_command_path_gets_a_section() {
    let out = reference();
    for path in [
        "## varde",
        "## varde init",
        "## varde models",
        "## varde models enable",
        "## varde models unexpose",
        "## varde registry show",
        "## varde docker run",
        "## varde backup restore",
        "## varde status",
        "## varde update",
    ] {
        assert!(out.contains(path), "{path} is missing from the reference");
    }
}

#[test]
fn the_project_only_commands_are_documented_even_though_help_hides_them() {
    let out = reference();
    for path in [
        "## varde up",
        "## varde down",
        "## varde logs",
        "## varde ui",
    ] {
        assert!(out.contains(path), "{path} is missing from the reference");
    }
}

#[test]
fn clap_s_own_commands_and_the_generator_itself_stay_out() {
    let out = reference();
    assert!(!out.contains("## varde help"));
    assert!(!out.contains("## varde docs-markdown"));
    assert!(!out.contains("| `-h, --help` |"));
    // `varde models enable --version` is not clap's `-V` and must stay.
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
        out.contains("Usage: varde models enable [OPTIONS] <ID>"),
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
    assert!(out.contains("Default: `8700`"), "init --api-port 8700");
    assert!(
        out.contains("Never touch the network"),
        "the global --offline help"
    );
}

#[test]
fn rendering_is_deterministic() {
    assert_eq!(reference(), reference());
}
