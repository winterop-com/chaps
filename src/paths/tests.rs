use super::*;

#[test]
fn cache_dir_is_always_named() {
    // The env-dependent branches are exercised by the CLI tests, which
    // control the whole environment; here we only pin the invariant that
    // every branch yields a named directory.
    assert!(cache_dir().file_name().is_some());
}
