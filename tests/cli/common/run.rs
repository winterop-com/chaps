//! Fakes and helpers for the `chaps run`, `ps` and `stop` tests: stand-in
//! `docker` scripts, so Unix only.

use super::*;
use serde_json::Value as Json;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A `docker` whose compose commands succeed and whose `ps` reports the
/// listed services running in every project.
pub(crate) fn docker_running_services(services: &[&str]) -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let rows: String = services
        .iter()
        .map(|s| format!("{{\"Service\":\"{s}\",\"State\":\"running\"}}\\n"))
        .collect();
    let script = format!(
        "#!/bin/sh\n\
         case \"$*\" in\n\
         *' ps '*) printf '{rows}'; exit 0;;\n\
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

/// A `docker` that refuses `compose up` the way a pull of a missing image
/// does, and answers everything else with success.
pub(crate) fn docker_failing_up() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let script = "#!/bin/sh\n\
         case \"$*\" in\n\
         *' up '*) echo ' m Pulling' >&2; \
         echo 'Error response from daemon: pull access denied' >&2; \
         echo 'denied' >&2; exit 1;;\n\
         esac\n\
         exit 0\n";
    let docker = bin.join("docker");
    std::fs::write(&docker, script).expect("the fake docker");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake docker");
    }
    (temp, bin)
}

/// A `docker` whose local image store holds every image asked about, as one
/// that runs as user 1000 in `/app`, and whose compose commands succeed.
pub(crate) fn docker_with_local_images() -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let script = "#!/bin/sh\n\
         case \"$*\" in\n\
         'image inspect'*) printf '1000\\t/app\\tnull\\t[\"serve\"]\\n'; exit 0;;\n\
         esac\n\
         exit 0\n";
    let docker = bin.join("docker");
    std::fs::write(&docker, script).expect("the fake docker");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake docker");
    }
    (temp, bin)
}

pub(crate) fn data(sandbox: &Sandbox) -> PathBuf {
    sandbox.cache.path().join("data")
}

pub(crate) fn run_json(sandbox: &Sandbox, cwd: &Path, bin: &Path, args: &[&str]) -> Json {
    let mut argv = vec!["--json"];
    argv.extend_from_slice(args);
    let out = chap_with_docker(sandbox, cwd, bin, &argv)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("one JSON document")
}

/// A `docker` that holds one leftover volume for every compose project, the
/// one a model stopped earlier left, and writes each call to `calls.log`.
/// `volume rm` succeeds when `rm_ok`, and is refused the way a volume still in
/// use is otherwise.
pub(crate) fn docker_with_leftover_volume(rm_ok: bool) -> (TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let log = temp.path().join("calls.log");
    let rm = match rm_ok {
        true => "exit 0",
        false => "echo 'Error response from daemon: remove x: volume is in use' >&2; exit 1",
    };
    let script = format!(
        "#!/bin/sh\n\
         echo \"$*\" >> '{log}'\n\
         case \"$*\" in\n\
         'volume ls --filter label=com.docker.compose.project='*) \
         echo \"${{4#label=com.docker.compose.project=}}_ck_old_model_data\"; exit 0;;\n\
         'volume rm '*) {rm};;\n\
         esac\n\
         exit 0\n",
        log = log.display()
    );
    let docker = bin.join("docker");
    std::fs::write(&docker, script).expect("the fake docker");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake docker");
    }
    (temp, bin, log)
}
