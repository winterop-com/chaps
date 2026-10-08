//! The port check of `varde up`: a busy port that another deployment
//! publishes names that deployment only when it runs. Unix only: the
//! stand-in docker is a shell script.

#![cfg(unix)]

use crate::common::*;
use std::path::PathBuf;
use tempfile::TempDir;

/// A `docker` whose compose `ps` reports chap running only in the
/// deployment directory named `up`, and nothing in any other.
fn docker_up_in(up: &str) -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let script = format!(
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *' ps '*) case \"$(basename \"$(pwd -P)\")\" in\n\
           {up}) printf '{{\"Service\":\"chap\",\"State\":\"running\"}}\\n';;\n\
           esac; exit 0;;\n\
         esac\n\
         exit 0\n"
    );
    let docker = bin.join("docker");
    std::fs::write(&docker, script).expect("the fake docker");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake docker");
    }
    (temp, bin)
}

#[test]
fn up_names_only_a_running_deployment_as_the_holder_of_a_port() {
    // A listener on a port the kernel picked stands in for the container.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
    let port = listener.local_addr().unwrap().port().to_string();
    let sandbox = Sandbox::new();
    for name in ["a", "c"] {
        sandbox
            .chap()
            .arg("init")
            .arg(name)
            .args(["--models", "none", "--api-port", &port])
            .assert()
            .success();
    }
    let c = sandbox.home.path().join("c");

    // `a` is stopped: the port is held by something else, and `--replace`
    // has nothing of varde's to stop.
    let (_fake, bin) = docker_up_in("none");
    let out = chap_with_docker(&sandbox, &c, &bin, &["up"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8(out).expect("text");
    assert!(
        text.contains(&format!("port {port} is already in use on this machine")),
        "{text}"
    );
    assert!(!text.contains("publishes it too"), "{text}");
    assert!(!text.contains("--replace"), "{text}");

    // `a` runs: it is the holder, and `--replace` stops it.
    let (_fake, bin) = docker_up_in("a");
    let first = resolved(&sandbox.home.path().join("a"));
    chap_with_docker(&sandbox, &c, &bin, &["up"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(format!(
            "and a ({first}) publishes it too; stop it with"
        )))
        .stderr(predicates::str::contains(
            "or run `varde up --replace` to stop a first",
        ));
    drop(listener);
}

/// A `docker` whose compose `ps` reports chap on `api` and dhis2 on
/// `dhis2`, both running, with the host ports they publish now.
fn docker_publishing(api: u16, dhis2: u16) -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let row = |service: &str, port: u16| {
        format!(
            "{{\"Service\":\"{service}\",\"State\":\"running\",\
             \"Publishers\":[{{\"PublishedPort\":{port}}}]}}\\n"
        )
    };
    let script = format!(
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *' ps '*) printf '{}{}'; exit 0;;\n\
         esac\n\
         exit 0\n",
        row("chap", api),
        row("dhis2", dhis2),
    );
    let docker = bin.join("docker");
    std::fs::write(&docker, script).expect("the fake docker");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake docker");
    }
    (temp, bin)
}

/// A restore of `.varde/` files can move a running DHIS2 to another port. The
/// recreate publishes the new port, so the check asks about that port, and
/// the container on the old one does not vouch for it.
#[test]
fn up_checks_the_port_a_running_service_is_moved_to() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
    let taken = listener.local_addr().unwrap().port();
    let api = free_port();
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--api-port", &api.to_string()])
        .assert()
        .success();
    sandbox
        .components(&["enable", "dhis2", "--port", &taken.to_string()])
        .assert()
        .success();

    let (_fake, bin) = docker_publishing(api, free_port());
    let out = chap_with_docker(&sandbox, &dir, &bin, &["up"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let text = String::from_utf8(out).expect("text");
    assert!(
        text.contains(&format!("port {taken} is already in use on this machine")),
        "{text}"
    );
    assert!(!text.contains(&format!("port {api} ")), "{text}");
    drop(listener);
}
