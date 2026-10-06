use super::*;

#[test]
fn a_bounded_run_reports_a_binary_that_is_not_there() {
    let outcome = run_bounded("varde-no-such-binary-exists", &["--help"], DOCKER_TIMEOUT);
    assert_eq!(outcome, Outcome::Missing);
    assert!(!outcome.succeeded());
    assert_eq!(outcome.stderr(), "the binary is not on PATH");
}

#[test]
fn a_bounded_run_captures_what_the_command_said() {
    // `cargo` is what is running this test, so it is on PATH by
    // construction, and `--version` is the same two words everywhere.
    let outcome = run_bounded("cargo", &["--version"], DOCKER_TIMEOUT);
    let Outcome::Done { ok, stdout, .. } = &outcome else {
        panic!("expected a finished command, got {outcome:?}");
    };
    assert!(ok);
    assert!(stdout.starts_with("cargo "), "{stdout}");
    assert!(outcome.succeeded());
}

#[test]
fn long_output_is_cut_to_one_readable_line() {
    assert_eq!(first_line("\n\n  hello  \nworld\n"), "hello");
    assert_eq!(first_line(""), "");
    let long = "x".repeat(400);
    let cut = first_line(&long);
    assert_eq!(cut.chars().count(), 120);
    assert!(cut.ends_with("..."));
}
