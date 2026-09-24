//! End-to-end tests for the `.env` rules: which line the CLI reads, and which
//! line it writes.
//!
//! `.env` is the one file in a deployment that an operator edits by hand and
//! that docker compose acts on, so `chaps` and compose have to agree about it
//! completely. Two disagreements have cost a deployment already:
//!
//! - `chaps auth rotate` rewrote the *first* `CHAP_API_TOKEN=` line while
//!   compose went on reading a later duplicate, so a rotation that reported
//!   success left the old token in force;
//! - `chaps status` derived the API URL from `.chaps/project.yaml` alone, so a
//!   deployment moved with `CHAP_API_PORT=` in `.env` was probed on the port
//!   it had left behind.
//!
//! Both are checked here through the binary, because the rule is only worth
//! anything if it holds for the file on disk.
//!
//! Every run is `--offline` with a cache directory of its own, so the embedded
//! marketplace snapshot is what the CLI sees and no test touches the network.

use assert_cmd::Command;
use serde_json::Value as Json;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A cache directory plus the project directory the tests write into.
struct Sandbox {
    cache: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Sandbox {
        Sandbox {
            cache: tempfile::tempdir().unwrap(),
            home: tempfile::tempdir().unwrap(),
        }
    }

    /// The directory `chaps init DIR` is pointed at.
    fn project(&self) -> PathBuf {
        self.home.path().join("chapx")
    }

    fn chap(&self) -> Command {
        let mut cmd = Command::cargo_bin("chaps").expect("the chaps binary is built");
        cmd.env("CHAPS_CACHE_DIR", self.cache.path())
            .env("CHAPS_NO_UPDATE_CHECK", "1")
            .current_dir(self.home.path())
            .arg("--offline");
        cmd
    }

    /// `chaps init <project> ...`, with the arguments appended.
    fn init(&self, args: &[&str]) -> Command {
        let mut cmd = self.chap();
        cmd.arg("init").arg(self.project()).args(args);
        cmd
    }

    /// `chaps -C <project> <args>...`.
    fn run(&self, args: &[&str]) -> Command {
        let mut cmd = self.chap();
        cmd.arg("-C").arg(self.project()).args(args);
        cmd
    }

    fn env_path(&self) -> PathBuf {
        self.project().join(".env")
    }

    /// The project's `.env`, as text.
    fn env(&self) -> String {
        read(&self.env_path())
    }

    fn write_env(&self, body: &str) {
        std::fs::write(self.env_path(), body).expect("writing .env");
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// A port the kernel handed out at or above `from`, and that nothing is
/// listening on.
///
/// `status` probes the API port for real, so a fixed number would be a
/// question about whatever the developer happens to be running. The search
/// starts high and walks upwards, so the port in the assertion is usually the
/// 18000 the review that prompted these tests used, without the test depending
/// on it being free.
fn free_port_from(from: u16) -> u16 {
    (from..u16::MAX)
        // Both the addresses the CLI's own probe tries first, which is what it
        // takes for the CLI to call the port free. Each listener is bound only
        // long enough to answer the question and is dropped at the end of the
        // expression, so a probe aimed at the port finds nothing there.
        .find(|port| {
            [Ipv4Addr::UNSPECIFIED, Ipv4Addr::LOCALHOST]
                .into_iter()
                .all(|addr| std::net::TcpListener::bind((addr, *port)).is_ok())
        })
        .expect("a free port above the ephemeral range")
}

/// Every active (uncommented) assignment of `var` in a `.env`, by compose's
/// rules: `export ` allowed, surrounding quotes not part of the value.
fn active_values<'a>(env: &'a str, var: &str) -> Vec<&'a str> {
    env.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
            let (name, value) = line.split_once('=')?;
            (name.trim() == var).then(|| value.trim())
        })
        .filter(|value| !value.is_empty())
        .collect()
}

/// A generated secret: 64 lowercase hex characters.
fn is_generated_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
}

/// `chaps status --json`, whose exit code is non-zero whenever the API is not
/// answering - which it never is here, since nothing was started.
fn status_json(sandbox: &Sandbox) -> Json {
    let out = sandbox
        .run(&["status", "--json", "--timeout", "1"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("status --json is one JSON document")
}

/// The rotation bug, end to end: a `.env` carrying two active
/// `CHAP_API_TOKEN=` lines is what compose reads the *second* of, so a rotate
/// that rewrites only the first reports a rotation it has not made.
#[test]
fn auth_rotate_leaves_one_active_token_line_on_an_env_with_a_duplicate() {
    let sandbox = Sandbox::new();
    sandbox
        .init(&["--models", "none", "--api-token"])
        .assert()
        .success();

    let before = sandbox.env();
    let first = active_values(&before, "CHAP_API_TOKEN")
        .first()
        .expect("`init --api-token` wrote a token")
        .to_string();
    assert!(is_generated_secret(&first));

    // The shape the review found: a second active assignment further down,
    // which is the one compose uses. A hand edit, a merge or a `cat` of two
    // files all produce it.
    let duplicate = "b".repeat(64);
    sandbox.write_env(&format!(
        "{before}\n# a stray duplicate, further down\nexport CHAP_API_TOKEN={duplicate}\n"
    ));
    assert_eq!(
        active_values(&sandbox.env(), "CHAP_API_TOKEN"),
        vec![first.as_str(), duplicate.as_str()],
        "the file has to start out with two active assignments"
    );

    sandbox
        .run(&["auth", "rotate"])
        .assert()
        .success()
        .stdout(predicates::str::contains("API authentication rotated"));

    let after = sandbox.env();
    let active = active_values(&after, "CHAP_API_TOKEN");
    assert_eq!(
        active.len(),
        1,
        "exactly one assignment may survive a rotate: {after}"
    );
    let token = active[0];
    assert!(is_generated_secret(token), "{token}");
    assert_ne!(token, first);
    assert_ne!(token, duplicate);
    // Neither old value is anywhere in the file, commented or not: a rotation
    // that leaves the old token behind is the bug.
    assert!(!after.contains(&first), "the first token survived: {after}");
    assert!(
        !after.contains(&duplicate),
        "the duplicate survived: {after}"
    );

    // And the CLI reports the token the file now sets, rather than one of the
    // two it replaced.
    let shown = sandbox
        .run(&["--json", "auth", "show", "--reveal"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let shown: Json = serde_json::from_slice(&shown).expect("auth show --json is JSON");
    assert_eq!(shown["token"], token);
    assert_eq!(shown["api_token"], true);
}

/// Turning authentication off has to comment out *every* active assignment:
/// one line left behind still protects the API, and the operator was told it
/// no longer does. Turning it back on then leaves exactly one.
#[test]
fn auth_disable_and_enable_normalise_duplicates_of_both_secrets() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "none"]).assert().success();

    let secret = "c".repeat(64);
    let base = sandbox.env();
    sandbox.write_env(&format!(
        "{base}\nCHAP_API_TOKEN={secret}\nSERVICEKIT_REGISTRATION_KEY={secret}\n\
         export CHAP_API_TOKEN={secret}\nSERVICEKIT_REGISTRATION_KEY={secret}\n"
    ));
    let vars = ["CHAP_API_TOKEN", "SERVICEKIT_REGISTRATION_KEY"];
    for var in vars {
        assert_eq!(active_values(&sandbox.env(), var).len(), 2, "{var}");
    }

    sandbox.run(&["auth", "disable"]).assert().success();
    let after = sandbox.env();
    for var in vars {
        assert!(
            active_values(&after, var).is_empty(),
            "{var} is still set after disable: {after}"
        );
    }
    // The values are kept behind the `#`, which is what makes an accidental
    // disable a round trip.
    assert!(
        after.contains(&format!("# CHAP_API_TOKEN={secret}")),
        "{after}"
    );

    sandbox.run(&["auth", "enable"]).assert().success();
    let after = sandbox.env();
    for var in vars {
        assert_eq!(
            active_values(&after, var),
            vec![secret.as_str()],
            "{var} came back as something other than one line of the old value: {after}"
        );
    }
}

/// The API port bug, end to end: `.env` is what compose publishes from, so it
/// is what `status` has to ask.
#[test]
fn status_follows_the_api_port_env_override() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    let recorded = free_port_from(17000);
    let moved = free_port_from(18000);
    assert_ne!(recorded, moved);

    sandbox
        .init(&["--models", "none", "--api-port", &recorded.to_string()])
        .assert()
        .success();
    assert_eq!(
        status_json(&sandbox)["api_url"],
        format!("http://localhost:{recorded}"),
        "with no override the recorded port is the answer"
    );

    // Move the port the way an operator does: one line of `.env`, nothing in
    // `.chaps/`.
    let env = sandbox.env().replace(
        &format!("CHAP_API_PORT={recorded}"),
        &format!("CHAP_API_PORT={moved}"),
    );
    assert!(env.contains(&format!("CHAP_API_PORT={moved}")), "{env}");
    sandbox.write_env(&env);

    let report = status_json(&sandbox);
    assert_eq!(
        report["api_url"],
        format!("http://localhost:{moved}"),
        "status queried the port .chaps/project.yaml records, not the one .env sets"
    );
    assert_eq!(report["api_port"], moved);
    assert_eq!(report["api_port_source"], "env");
    // `.chaps/project.yaml` is untouched: the override is the operator's, not
    // a state change.
    let state = read(&dir.join(".chaps").join("project.yaml"));
    assert!(state.contains(&format!("api_port: {recorded}")), "{state}");

    // A commented-out line is not an override, and the recorded port comes
    // back.
    sandbox.write_env(&env.replace(
        &format!("CHAP_API_PORT={moved}"),
        &format!("# CHAP_API_PORT={moved}"),
    ));
    let report = status_json(&sandbox);
    assert_eq!(report["api_url"], format!("http://localhost:{recorded}"));
    assert_eq!(report["api_port"], recorded);
    assert_eq!(report["api_port_source"], "project");
}

/// The checklist names the file that moved the port, so `18000 is free` next
/// to a recorded `api_port: 17000` reads as a decision rather than a bug.
#[test]
fn doctor_says_which_file_the_api_port_came_from() {
    let sandbox = Sandbox::new();
    let recorded = free_port_from(17100);
    let moved = free_port_from(18100);

    sandbox
        .init(&["--models", "none", "--api-port", &recorded.to_string()])
        .assert()
        .success();
    sandbox.write_env(&sandbox.env().replace(
        &format!("CHAP_API_PORT={recorded}"),
        &format!("CHAP_API_PORT={moved}"),
    ));

    let out = sandbox
        .run(&["--json", "doctor"])
        .output()
        .expect("doctor runs");
    let report: Json = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("doctor --json did not parse: {e}"));
    let check = report["checks"]
        .as_array()
        .expect("an array of checks")
        .iter()
        .find(|check| check["id"] == "api-port")
        .unwrap_or_else(|| panic!("no api-port check: {report}"));

    let detail = check["detail"].as_str().expect("a detail line");
    assert!(detail.starts_with(&format!("{moved} ")), "{detail}");
    assert!(detail.contains("from .env"), "{detail}");
    assert!(
        detail.contains(&format!(
            "over the {recorded} recorded in .chaps/project.yaml"
        )),
        "{detail}"
    );
}
