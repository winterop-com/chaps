use super::*;

fn lines(body: &str) -> Vec<String> {
    body.lines().map(str::to_string).collect()
}

/// Compose reads the file top to bottom and keeps overwriting, so the last
/// active assignment is the one in force. Reading the first is the bug
/// this module exists to make impossible.
#[test]
fn the_last_active_assignment_wins() {
    assert_eq!(
        value("CHAP_API_TOKEN=one\nCHAP_API_TOKEN=two\n", "CHAP_API_TOKEN").as_deref(),
        Some("two")
    );
    // Three of them, and a commented one that counts for nothing.
    let body = "K=a\n# K=b\nK=c\nK=d\n";
    assert_eq!(value(body, "K").as_deref(), Some("d"));
    // A later commented line does not take the value away.
    assert_eq!(value("K=a\n# K=b\n", "K").as_deref(), Some("a"));
}

#[test]
fn an_export_prefix_assigns_the_same_thing() {
    assert_eq!(value("export K=v\n", "K").as_deref(), Some("v"));
    assert_eq!(value("  export   K=v\n", "K").as_deref(), Some("v"));
    // `export` is not part of the name, and a variable merely called
    // `exportK` is a different one.
    assert_eq!(value("exportK=v\n", "K"), None);
    assert_eq!(value("export K=v\nK=w\n", "K").as_deref(), Some("w"));
}

#[test]
fn matching_quotes_are_not_part_of_the_value() {
    assert_eq!(value("K=\"v w\"\n", "K").as_deref(), Some("v w"));
    assert_eq!(value("K='v w'\n", "K").as_deref(), Some("v w"));
    assert_eq!(value("export K=\"v\"\n", "K").as_deref(), Some("v"));
    // Only a matching pair: a lone quote belongs to the value.
    assert_eq!(value("K=\"v\n", "K").as_deref(), Some("\"v"));
    assert_eq!(value("K=\"\n", "K").as_deref(), Some("\""));
    assert_eq!(value("K='v\"\n", "K").as_deref(), Some("'v\""));
    // And an empty pair is an empty value.
    assert_eq!(value("K=\"\"\n", "K").as_deref(), Some(""));
    assert_eq!(non_empty("K=\"\"\n", "K"), None);
}

/// Each case is what `docker compose config` printed for the same line,
/// on Compose v5.
#[test]
fn values_are_read_the_way_compose_reads_them() {
    let read = |line: &str| value(&format!("K={line}\n"), "K").unwrap();
    // A `#` with a space before it starts a comment; one without does not.
    assert_eq!(read("18984 # operator comment"), "18984");
    assert_eq!(read("val#ue"), "val#ue");
    assert_eq!(read("x #"), "x");
    assert_eq!(read("abc\t# tab"), "abc\t# tab");
    // Anything after a closing quote is not part of the value.
    assert_eq!(read("'a # b' # c"), "a # b");
    assert_eq!(read("\"a # b\" # c"), "a # b");
    // Single quotes are literal but for an escaped quote.
    assert_eq!(read("'$HOME'"), "$HOME");
    assert_eq!(read("'it\\'s'"), "it's");
    assert_eq!(read("'a\\nb'"), "a\\nb");
    // Double quotes take the escapes compose takes.
    assert_eq!(read("\"q \\\"x\\\" \\\\\""), "q \"x\" \\");
    assert_eq!(read("\"a\\nb\""), "a\nb");
    assert_eq!(read("  spaced value  "), "spaced value");
    // The port the finding was about.
    assert_eq!(
        non_empty("CHAP_API_PORT=18984 # operator comment\n", "CHAP_API_PORT").as_deref(),
        Some("18984")
    );
}

/// A value is written so compose reads it back unchanged, `$` included:
/// written bare, `$UNSET` became an empty API token.
#[test]
fn a_value_is_written_so_it_reads_back_as_itself() {
    assert_eq!(encode("abcDEF-_.:/+=@,~%123"), "abcDEF-_.:/+=@,~%123");
    assert_eq!(encode("$CHAPS_UNSET"), "'$CHAPS_UNSET'");
    assert_eq!(encode("a b#c"), "'a b#c'");
    assert_eq!(encode(""), "");
    for token in ["$CHAPS_UNSET", "a b # c", "${X:-y}", "semi;colon&", "plain"] {
        assert_eq!(literal_problem(token), None, "{token}");
        let mut body = lines("K=old\n");
        set(&mut body, "K", token);
        assert_eq!(value(&join(&body), "K").as_deref(), Some(token), "{token}");
    }
    for bad in ["it's", "back\\slash", "new\nline", "dq\""] {
        assert!(literal_problem(bad).is_some(), "{bad}");
    }
}

#[test]
fn a_rewritten_line_keeps_its_comment() {
    let mut body = lines("CHAP_API_PORT=8000 # the operator's\n");
    set(&mut body, "CHAP_API_PORT", "8001");
    assert_eq!(join(&body), "CHAP_API_PORT=8001 # the operator's\n");
    let mut body = lines("K='a b' # kept\n");
    set(&mut body, "K", "c");
    assert_eq!(join(&body), "K=c # kept\n");
}

#[test]
fn comments_blanks_and_other_variables_assign_nothing() {
    let body = "# K=commented\n\n   # K=indented\nOTHER=v\nK_SUFFIX=v\nPREFIX_K=v\nJUNK\n";
    assert_eq!(value(body, "K"), None);
    assert_eq!(value("", "K"), None);
    // A bare assignment is a line that sets nothing, which is not the same
    // as no line at all.
    assert_eq!(value("K=\n", "K").as_deref(), Some(""));
    assert_eq!(non_empty("K=\n", "K"), None);
    assert_eq!(non_empty("K=  \n", "K"), None);
}

/// The rotation bug, from the other side: a second active line must not
/// survive the write, or compose goes on reading the old secret.
#[test]
fn writing_rewrites_the_first_assignment_and_removes_the_duplicates() {
    let mut body = lines("K=old\n# a comment\nK=older\nOTHER=v\nK=oldest\n");
    assert_eq!(set(&mut body, "K", "new"), Wrote::Active { duplicates: 2 });
    assert_eq!(join(&body), "K=new\n# a comment\nOTHER=v\n");
    assert_eq!(value(&join(&body), "K").as_deref(), Some("new"));
    assert_eq!(
        join(&body).lines().filter(|l| l.starts_with("K=")).count(),
        1
    );

    // One line is the ordinary case, and it changes nothing else.
    let mut body = lines("K=old\n# a comment\nOTHER=v\n");
    assert_eq!(set(&mut body, "K", "new"), Wrote::Active { duplicates: 0 });
    assert_eq!(join(&body), "K=new\n# a comment\nOTHER=v\n");
}

#[test]
fn writing_keeps_an_export_prefix_and_leaves_commented_lines_alone() {
    let mut body = lines("export K=old\n");
    assert_eq!(set(&mut body, "K", "new"), Wrote::Active { duplicates: 0 });
    assert_eq!(join(&body), "export K=new\n");

    // A commented line is documentation, not an assignment: it is left as
    // it is when there is an active line to write.
    let mut body = lines("# K=documented\nK=old\n");
    assert_eq!(set(&mut body, "K", "new"), Wrote::Active { duplicates: 0 });
    assert_eq!(join(&body), "# K=documented\nK=new\n");
}

#[test]
fn writing_uncomments_a_placeholder_when_there_is_no_active_line() {
    let mut body = lines("# before\n# K=\n# after\n");
    assert_eq!(set(&mut body, "K", "new"), Wrote::Uncommented);
    assert_eq!(join(&body), "# before\nK=new\n# after\n");

    // The first placeholder is the one that gets the value.
    let mut body = lines("# K=\n# K=\n");
    assert_eq!(set(&mut body, "K", "new"), Wrote::Uncommented);
    assert_eq!(join(&body), "K=new\n# K=\n");
}

#[test]
fn a_variable_the_file_never_mentions_is_the_callers_to_append() {
    let mut body = lines("OTHER=v\n");
    assert_eq!(set(&mut body, "K", "new"), Wrote::Absent);
    assert_eq!(join(&body), "OTHER=v\n", "nothing was touched");
}

#[test]
fn commenting_out_takes_every_active_line_and_keeps_the_values() {
    let out = comment_out("K=a\n# K=b\nOTHER=v\nexport K=c\n", "K");
    assert_eq!(out, "# K=a\n# K=b\nOTHER=v\n# export K=c\n");
    assert_eq!(value(&out, "K"), None);
    // The last one is what `enable` recovers, the way compose read it.
    assert_eq!(commented_value(&out, "K").as_deref(), Some("c"));
    // An already commented line is not commented twice.
    assert!(!comment_out(&out, "K").contains("# # K"));
    // Nothing to comment out is no change at all.
    assert_eq!(comment_out("OTHER=v\n", "K"), "OTHER=v\n");
}

#[test]
fn a_commented_placeholder_carries_no_value_to_recover() {
    assert_eq!(commented_value("# K=\n", "K"), None);
    assert_eq!(commented_value("K=active\n", "K"), None);
    assert_eq!(
        commented_value("#   K=\"v w\"\n", "K").as_deref(),
        Some("v w")
    );
}

#[cfg(unix)]
#[test]
fn env_is_written_whole_and_readable_by_its_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".env");
    std::fs::write(&path, "OLD=1\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    write(&path, "POSTGRES_PASSWORD=secret\n").unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "POSTGRES_PASSWORD=secret\n"
    );
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let leftovers: Vec<_> = std::fs::read_dir(temp.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name() != ".env")
        .collect();
    assert!(leftovers.is_empty(), "no temporary file is left behind");
}

#[cfg(unix)]
#[test]
fn protect_closes_an_existing_env_to_its_owner() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(".env");
    std::fs::write(&path, "A=1\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    protect(&path);
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    protect(&temp.path().join("missing"));
}
