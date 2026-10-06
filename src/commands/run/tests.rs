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
