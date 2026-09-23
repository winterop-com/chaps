//! End-to-end tests for what a build says about its channel.
//!
//! `chaps self version` is the one command that reports which release series
//! a binary follows, and `chaps self update` is the one that acts on it. Both
//! are exercised here without touching the network: the channel of a test
//! build is decided at compile time by `build.rs`, and every command that
//! would reach the release feed is run under `--offline`, which is refused
//! before any request is made.

use assert_cmd::Command;
use serde_json::Value as Json;
use tempfile::TempDir;

/// A `chaps` with a cache directory of its own and no update check, run from
/// a directory that is not a project.
fn bare() -> (TempDir, Command) {
    let cache = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("chaps").expect("the chaps binary is built");
    cmd.env("CHAPS_CACHE_DIR", cache.path())
        .env("CHAPS_NO_UPDATE_CHECK", "1")
        .current_dir(cache.path());
    (cache, cmd)
}

fn stdout(cmd: &mut Command) -> String {
    let out = cmd.assert().success().get_output().stdout.clone();
    String::from_utf8(out).expect("utf-8")
}

/// A build that was not told otherwise is a stable one, and it says so on a
/// line of its own rather than by leaving one out: a field that only appears
/// on a dev build would make its absence the thing to notice.
#[test]
fn self_version_names_the_channel_of_this_build() {
    let (_cache, mut cmd) = bare();
    let text = stdout(cmd.args(["self", "version"]));
    assert!(text.contains("channel"), "{text}");
    assert!(text.contains("stable"), "{text}");
    assert!(!text.contains("channel       dev"), "{text}");
}

#[test]
fn self_version_json_carries_the_channel() {
    let (_cache, mut cmd) = bare();
    let text = stdout(cmd.args(["--json", "self", "version"]));
    let value: Json = serde_json::from_str(&text).expect("--json is JSON");
    assert_eq!(value["channel"], "stable");
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
}

/// `--version dev` is how the rolling build is asked for by name. There is no
/// separate flag for it, and the tag reaches the release lookup like any
/// other: `releases/tags/dev`. Under `--offline` the refusal comes before the
/// request, which is what keeps this test off the network while still proving
/// the argument is accepted rather than rejected by the parser.
#[test]
fn self_update_takes_dev_as_a_version_and_still_refuses_to_run_offline() {
    let (_cache, mut cmd) = bare();
    cmd.args(["--offline", "self", "update", "--check", "--version", "dev"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--offline"));
}

#[test]
fn self_update_help_says_a_tag_is_what_version_takes() {
    let (_cache, mut cmd) = bare();
    let help = stdout(cmd.args(["self", "update", "--help"]));
    assert!(help.contains("--version"), "{help}");
    assert!(help.contains("TAG"), "{help}");
}
