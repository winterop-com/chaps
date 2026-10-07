use super::*;

#[test]
fn compose_is_quoted_by_its_error_line() {
    let stderr = " nothing Pulling \n nothing Error pull access denied\nError response from daemon: error from registry: denied\ndenied\n";
    assert_eq!(
        compose_said(stderr).as_deref(),
        Some("Error response from daemon: error from registry: denied")
    );
    assert_eq!(compose_said("one\ntwo\n\n").as_deref(), Some("two"));
    assert_eq!(compose_said("\n"), None);
}

#[test]
fn commands_named_inside_a_group_reach_it_with_dash_c() {
    let dir = Path::new("/home/u/.local/share/varde/run/g");
    assert_eq!(
        with_dir("run `varde models remove x` first", dir),
        "run `varde -C /home/u/.local/share/varde/run/g models remove x` first"
    );

    let err = anyhow::anyhow!("inner").context("x was added; run `varde models remove x`");
    let out = for_group(err, true, dir);
    assert_eq!(
        out.to_string(),
        "x was added; run `varde -C /home/u/.local/share/varde/run/g models remove x`"
    );
    assert_eq!(out.chain().nth(1).unwrap().to_string(), "inner");

    // A typed error keeps its type, which the exit code is read from.
    let typed = anyhow::Error::from(ChapError::Usage("`varde x`".into()));
    let out = for_group(typed, true, dir);
    assert!(out.downcast_ref::<ChapError>().is_some());
}

#[test]
fn a_group_name_is_one_safe_path_segment() {
    for name in ["dengue", "g-1", "a_b", "2026"] {
        let dir = group_dir(name).expect(name);
        assert_eq!(dir, groups_dir().join(name));
        assert!(dir.starts_with(groups_dir()), "{name}");
    }
}

#[test]
fn a_group_name_that_could_leave_the_groups_directory_is_refused() {
    // The directory is what `stop --purge` removes, so a name must never
    // reach outside the groups directory or name a second level.
    for name in [
        "", ".", "..", "../x", "a/b", "/abs", "a\\b", "Dengue", "a b", "a.b", "ø",
    ] {
        let err = group_dir(name).expect_err(name).to_string();
        assert!(err.contains("is not a group name"), "{name}: {err}");
        assert!(err.contains("--group dengue"), "{name}: {err}");
    }
}

#[test]
fn a_path_is_quoted_for_the_shell_only_when_it_needs_it() {
    assert_eq!(shell_quote("/home/u/chap"), "/home/u/chap");
    assert_eq!(shell_quote("~/my-dir_1.x"), "~/my-dir_1.x");
    assert_eq!(shell_quote("/home/u/my chap"), "'/home/u/my chap'");
    assert_eq!(shell_quote("/tmp/it's"), r"'/tmp/it'\''s'");
    assert_eq!(shell_quote("/tmp/$HOME"), "'/tmp/$HOME'");
}

fn run_report(group: Option<&str>, waited: Option<u64>) -> RunReport {
    RunReport {
        model: ModelRef {
            id: "chapkit_ewars_model".to_string(),
            service_id: "chapkit-ewars-model".to_string(),
            port: Some(5001),
            bind: None,
            url: Some("http://localhost:5001".to_string()),
        },
        group: group.map(str::to_string),
        project_dir: PathBuf::from("/data/run/g"),
        enabled: true,
        wait: waited.map(|waited_s| Readiness {
            ready: true,
            waited_s,
            api_url: None,
            api_up: true,
            models: Vec::new(),
        }),
        was_running: false,
        chap_core: None,
        registered: None,
        warnings: Vec::new(),
    }
}

fn said(report: &RunReport, attached: bool) -> String {
    let mut lines = Report::default();
    say(report, attached, &mut lines);
    lines.text()
}

/// A run that answered is one line; how to stop it and its log are hints.
#[test]
fn a_run_is_one_line_and_the_next_commands_are_hints() {
    let text = said(&run_report(Some(DEFAULT_GROUP), Some(41)), false);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "running chapkit_ewars_model on http://localhost:5001 (answered in 41s)"
    );
    assert_eq!(lines[1], "hint: `varde stop chapkit_ewars_model` stops it");
    assert!(
        lines[2].starts_with("hint: `varde -C /data/run/g logs"),
        "{text}"
    );
    assert_eq!(lines.len(), 3, "{text}");

    // In the foreground, Ctrl-C stops it, so nothing else is named.
    assert_eq!(
        said(&run_report(None, Some(2)), true),
        "running chapkit_ewars_model on http://localhost:5001 (answered in 2s)\n"
    );
}

/// A group other than the default is in the line, and in the stop command.
#[test]
fn a_run_in_a_group_names_it() {
    let text = said(&run_report(Some("trial"), None), false);
    assert!(
        text.starts_with("started chapkit_ewars_model in group trial on http://localhost:5001\n"),
        "{text}"
    );
    assert!(
        text.contains("hint: `varde ps` shows when it answers"),
        "{text}"
    );
    assert!(
        text.contains("hint: `varde stop chapkit_ewars_model --group trial` stops it"),
        "{text}"
    );
}

/// Registration with a chap-core elsewhere is info, and its absence a
/// warning.
#[test]
fn registration_is_info_and_its_absence_a_warning() {
    let mut report = run_report(None, Some(2));
    report.chap_core = Some("http://localhost:8000".to_string());
    report.registered = Some(true);
    let text = said(&report, true);
    assert!(
        text.ends_with("\nregistered with http://localhost:8000\n"),
        "{text}"
    );
    report.registered = Some(false);
    report.warnings = vec!["a sync warning".to_string()];
    let text = said(&report, true);
    assert!(text.contains("warning: a sync warning\n"), "{text}");
    assert!(
        text.contains("warning: not registered with http://localhost:8000; "),
        "{text}"
    );
}

/// An empty `varde ps` says so in one line; the next command is a hint.
#[test]
fn an_empty_ps_says_so() {
    let report = ps::PsReport { models: Vec::new() };
    let mut lines = Report::default();
    ps::say(&report, &[], &mut lines);
    assert_eq!(
        lines.text(),
        "nothing has been started with `varde run` yet\nhint: `varde run <model>` starts one\n"
    );
    assert_eq!(ps::ps_table(&report, &crate::output::Out::default()), "");
}

/// `denied` is a registry that will not hand out the image: it does not exist
/// or it is private. The error says that once, with both ways out.
#[test]
fn a_denied_image_says_what_denied_means_and_the_way_out() {
    let line = start_failure(
        "y",
        "ghcr.io/x/y:tag",
        Some("Error response from daemon: error from registry: denied"),
        "varde run ghcr.io/x/y:tag",
    );
    assert_eq!(
        line,
        "y did not start: the registry answered `denied` for ghcr.io/x/y:tag, so the image does \
         not exist or is private; check the reference, or run `docker login ghcr.io`, then \
         `varde run ghcr.io/x/y:tag` tries again"
    );
    // The Docker socket is not the registry.
    let socket = start_failure(
        "y",
        "ghcr.io/x/y:tag",
        Some("permission denied while trying to connect to the Docker daemon socket"),
        "varde run y",
    );
    assert!(
        socket.starts_with("y did not start (permission denied"),
        "{socket}"
    );
}
