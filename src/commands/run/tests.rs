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
    let dir = Path::new("/home/u/.local/share/chaps/run/g");
    assert_eq!(
        with_dir("run `chaps models remove x` first", dir),
        "run `chaps -C /home/u/.local/share/chaps/run/g models remove x` first"
    );

    let err = anyhow::anyhow!("inner").context("x was added; run `chaps models remove x`");
    let out = for_group(err, true, dir);
    assert_eq!(
        out.to_string(),
        "x was added; run `chaps -C /home/u/.local/share/chaps/run/g models remove x`"
    );
    assert_eq!(out.chain().nth(1).unwrap().to_string(), "inner");

    // A typed error keeps its type, which the exit code is read from.
    let typed = anyhow::Error::from(ChapError::Usage("`chaps x`".into()));
    let out = for_group(typed, true, dir);
    assert!(out.downcast_ref::<ChapError>().is_some());
}
