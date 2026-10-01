//! `chaps logs`: what it hands compose, and what reaches a reader that is not
//! a terminal. Unix only: the stand-in docker is a shell script.

#![cfg(unix)]

use crate::common::*;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A `docker` on PATH with one running `chap` container whose log is coloured.
///
/// Every invocation is recorded, so a test can check the arguments compose
/// was given.
fn coloured_docker() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let log = temp.path().join("calls.log");
    let script = format!(
        "#!/bin/sh\n\
         echo \"$*\" >> {log}\n\
         case \"$*\" in\n\
         *' ps '*) printf '{{\"Service\":\"chap\",\"State\":\"running\"}}\\n'; exit 0;;\n\
         *'--services'*) printf 'chap\\n'; exit 0;;\n\
         *' logs '*) printf 'chap-1  | \\033[32mINFO\\033[0m ready\\n'; exit 0;;\n\
         esac\n\
         exit 0\n",
        log = log.display(),
    );
    let docker = bin.join("docker");
    std::fs::write(&docker, script).expect("the fake docker");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake docker");
    }
    (temp, log)
}

fn project(sandbox: &Sandbox) -> PathBuf {
    sandbox
        .init(&["--models", "none", "--api-port", &free_port().to_string()])
        .assert()
        .success();
    sandbox.project()
}

fn logs(sandbox: &Sandbox, dir: &Path, bin: &Path, args: &[&str]) -> String {
    let out = chap_with_docker(sandbox, dir, bin, args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(out).expect("the log is text")
}

#[test]
fn logs_tail_reaches_compose_and_a_pipe_gets_no_colour_codes() {
    let sandbox = Sandbox::new();
    let dir = project(&sandbox);
    let (fake, calls) = coloured_docker();
    let bin = fake.path().join("bin");

    let text = logs(&sandbox, &dir, &bin, &["logs", "--tail", "60", "chap"]);
    assert_eq!(text, "chap-1  | INFO ready\n");

    let calls = read(&calls);
    let call = calls
        .lines()
        .find(|line| line.contains(" logs "))
        .expect("compose logs was run");
    assert!(call.ends_with("logs --tail 60 chap"), "{call}");
}
