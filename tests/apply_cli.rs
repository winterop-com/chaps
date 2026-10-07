//! End-to-end tests for the shared apply path: what it refuses, and what a
//! refusal leaves behind.
//!
//! Every run is `--offline` with its own cache directory, so the embedded
//! marketplace snapshot is what the CLI sees and no test touches the network
//! or the developer's real cache.

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A named docker volume a test created, removed again however the test ends.
///
/// `None` when docker would not create one, which is a machine without a
/// daemon: the half of a test that needs a real volume is then skipped.
struct TestVolume(String);

impl TestVolume {
    fn create(name: &str) -> Option<TestVolume> {
        let made = std::process::Command::new("docker")
            .args(["volume", "create", name])
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|out| out.status.success());
        if !made {
            eprintln!("skipping the half that needs a docker volume: none could be created");
        }
        made.then(|| TestVolume(name.to_string()))
    }
}

impl Drop for TestVolume {
    fn drop(&mut self) {
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &self.0])
            .stdin(std::process::Stdio::null())
            .output();
    }
}

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

    /// The directory `varde init DIR` is pointed at.
    fn project(&self) -> PathBuf {
        self.home.path().join("chapx")
    }

    fn chap(&self) -> Command {
        let mut cmd = Command::cargo_bin("varde").expect("the varde binary is built");
        cmd.env("VARDE_CACHE_DIR", self.cache.path())
            // The data directory too: `init` records the deployment there, and
            // a test must not write the developer's own record.
            .env("VARDE_DATA_DIR", self.cache.path().join("data"))
            .env("VARDE_NO_DOCKER_PROBE", "1")
            .env_remove("GITHUB_TOKEN")
            .env_remove("GH_TOKEN")
            .current_dir(self.home.path())
            .arg("--offline");
        cmd
    }

    /// `varde init <project> ...`, with the arguments appended.
    fn init(&self, args: &[&str]) -> Command {
        let mut cmd = self.chap();
        cmd.arg("init").arg(self.project()).args(args);
        cmd
    }

    /// `varde -C <project> models ...`.
    fn models(&self, args: &[&str]) -> Command {
        let mut cmd = self.chap();
        cmd.arg("-C").arg(self.project()).arg("models").args(args);
        cmd
    }

    /// `varde -C <project> components ...`.
    fn components(&self, args: &[&str]) -> Command {
        let mut cmd = self.chap();
        cmd.arg("-C")
            .arg(self.project())
            .arg("components")
            .args(args);
        cmd
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// The compose project name `init` recorded, which is what docker prefixes
/// every named volume of the deployment with.
fn compose_project(dir: &Path) -> String {
    read(&dir.join(".varde").join("project.yaml"))
        .lines()
        .find_map(|line| line.strip_prefix("compose_project:"))
        .map(|value| value.trim().to_string())
        .expect("init records a compose project name")
}

/// `varde init --force` re-renders the whole deployment, which means deleting
/// the overlays the previous one owned. A selection that was never going to
/// apply must not take them with it: the directory is a running deployment,
/// and a typo in `--models` is not a reason to take its model offline.
#[test]
fn a_forced_init_that_cannot_apply_leaves_the_deployment_it_found_alone() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    let overlay = dir.join("compose.chapkit-ewars-model.yml");
    let before = read(&overlay);
    let state_before = read(&dir.join(".varde").join("models.yaml"));
    let umbrella_before = read(&dir.join("compose.marketplace.yml"));

    // A template is scaffolding to copy, not something to deploy.
    sandbox
        .init(&["--force", "--models", "chapkit_minimalist_example_py"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("template"));

    assert!(
        overlay.is_file(),
        "the previous deployment's overlay was deleted by a run that then failed"
    );
    assert_eq!(read(&overlay), before, "the overlay was rewritten");
    assert_eq!(
        read(&dir.join(".varde").join("models.yaml")),
        state_before,
        "the model set changed"
    );
    assert_eq!(read(&dir.join("compose.marketplace.yml")), umbrella_before);

    // A model the marketplace does not list is refused the same way.
    sandbox
        .init(&["--force", "--models", "no_such_model"])
        .assert()
        .failure();
    assert!(overlay.is_file());
    assert_eq!(read(&overlay), before);

    // And a `--force` that does apply still starts the state over.
    sandbox
        .init(&["--force", "--models", "auto_arima_chapkit"])
        .assert()
        .success();
    assert!(
        !overlay.exists(),
        "--force re-renders, so the overlay of a model it no longer enables goes"
    );
    assert!(dir.join("compose.auto-arima-chapkit.yml").is_file());
}

/// A model enabled into a deployment without chap-core runs on its own: it
/// is published on a host port, registers nowhere and waits for no `chap`.
/// Turning chap-core on afterwards puts it back behind chap-core.
#[test]
fn a_model_runs_on_its_own_in_a_deployment_without_chap_core() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    // A port base of its own, clear of whatever this machine already
    // publishes in the default 5001 range.
    sandbox
        .init(&["--only", "none", "--port-base", "18110"])
        .assert()
        .success();

    sandbox
        .models(&["enable", "chapkit_ewars_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains("http://localhost:18110"));
    let overlay = read(&dir.join("compose.chapkit-ewars-model.yml"));
    assert!(overlay.contains("\"18110:8000\""), "{overlay}");
    assert!(
        !overlay.contains("SERVICEKIT_ORCHESTRATOR_URL"),
        "{overlay}"
    );
    assert!(!overlay.contains("      chap:\n"), "{overlay}");
    assert!(!dir.join("compose.yml").exists());

    // chap-core added later: the overlay registers with it again, and keeps
    // the port it was given.
    sandbox
        .components(&["enable", "chap-core"])
        .assert()
        .success();
    let overlay = read(&dir.join("compose.chapkit-ewars-model.yml"));
    assert!(overlay.contains("SERVICEKIT_ORCHESTRATOR_URL"), "{overlay}");
    assert!(overlay.contains("      chap:\n"), "{overlay}");
    assert!(overlay.contains("\"18110:8000\""), "{overlay}");

    // And off again with the model still on: allowed, the model stays.
    sandbox
        .components(&["disable", "chap-core"])
        .assert()
        .success();
    let overlay = read(&dir.join("compose.chapkit-ewars-model.yml"));
    assert!(
        !overlay.contains("SERVICEKIT_ORCHESTRATOR_URL"),
        "{overlay}"
    );
}

/// Model services that register with a chap-core elsewhere: over the host
/// gateway, under this machine's name and their published port. The
/// chap-core commands talk to that chap-core instead of refusing.
#[test]
fn models_register_with_a_chap_core_elsewhere() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&[
            "--chap-core-url",
            "http://localhost:18999",
            "--models",
            "chapkit_ewars_model",
            "--port-base",
            "18120",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "components: chap-core (elsewhere)",
        ))
        .stdout(predicates::str::contains(
            "chap-core: http://localhost:18999 (elsewhere)",
        ));
    assert!(!dir.join("compose.yml").exists(), "no chap-core of its own");
    let overlay = read(&dir.join("compose.chapkit-ewars-model.yml"));
    assert!(
        overlay.contains(
            "SERVICEKIT_ORCHESTRATOR_URL: http://host.docker.internal:18999/v2/services/$$register"
        ),
        "{overlay}"
    );
    assert!(overlay.contains("SERVICEKIT_HOST: localhost"), "{overlay}");
    assert!(overlay.contains("SERVICEKIT_PORT: \"18120\""), "{overlay}");
    assert!(
        overlay.contains("host.docker.internal:host-gateway"),
        "{overlay}"
    );

    sandbox
        .components(&["list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("external"))
        .stdout(predicates::str::contains("http://localhost:18999"));

    // Disabling chap-core forgets it; the models then register nowhere.
    sandbox
        .components(&["disable", "chap-core"])
        .assert()
        .success();
    let overlay = read(&dir.join("compose.chapkit-ewars-model.yml"));
    assert!(
        !overlay.contains("SERVICEKIT_ORCHESTRATOR_URL"),
        "{overlay}"
    );

    // And back, through `components enable --url`.
    sandbox
        .components(&["enable", "chap-core", "--url", "http://localhost:18998"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "register with the chap-core at http://localhost:18998",
        ));
    let overlay = read(&dir.join("compose.chapkit-ewars-model.yml"));
    assert!(overlay.contains("host.docker.internal:18998"), "{overlay}");

    // A chap-core that is itself a container calls the models back through
    // the gateway rather than at its own localhost.
    sandbox
        .components(&[
            "enable",
            "chap-core",
            "--url",
            "http://localhost:18998",
            "--models-host",
            "host.docker.internal",
        ])
        .assert()
        .success();
    let overlay = read(&dir.join("compose.chapkit-ewars-model.yml"));
    assert!(
        overlay.contains("SERVICEKIT_HOST: host.docker.internal"),
        "{overlay}"
    );

    // EWARS hard-codes its port, so enabling it here says it will not register.
    sandbox
        .models(&["enable", "chapkit_ewars_model"])
        .assert()
        .success()
        .stderr(predicates::str::contains(
            "cannot register with the chap-core elsewhere",
        ));

    // --url is refused for anything but chap-core, and while chap-core runs here.
    sandbox
        .components(&["enable", "ocs", "--url", "http://x"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--url is a chap-core setting"));
    let own = Sandbox::new();
    own.init(&[]).assert().success();
    own.components(&["enable", "chap-core", "--url", "http://localhost:18998"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "`varde components disable chap-core`",
        ));
}

/// A DHIS2 in this deployment next to a chap-core elsewhere: its container
/// maps the host gateway and its `dhis.conf` allows any http target, so the
/// route `varde dhis2 connect` writes can reach it.
#[test]
fn a_dhis2_here_can_route_to_a_chap_core_elsewhere() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&[
            "--chap-core-url",
            "http://localhost:18999",
            "--with",
            "dhis2",
            "--dhis2-seed",
            "none",
            "--models",
            "none",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("no models are enabled").not());
    let compose = read(&dir.join("compose.dhis2.yml"));
    assert!(
        compose.contains("host.docker.internal:host-gateway"),
        "{compose}"
    );
    let conf = read(&dir.join("dhis2/dhis.conf"));
    assert!(
        conf.contains("route.remote_servers_allowed = http://*,https://*"),
        "{conf}"
    );

    // Without the chap-core elsewhere, neither.
    let plain = Sandbox::new();
    plain
        .init(&[
            "--with",
            "dhis2",
            "--dhis2-seed",
            "none",
            "--models",
            "none",
        ])
        .assert()
        .success();
    let compose = read(&plain.project().join("compose.dhis2.yml"));
    assert!(!compose.contains("host-gateway"), "{compose}");
}

/// `components disable chap-core` under a model that has no host port
/// publishes one, and says where.
#[test]
fn disabling_chap_core_publishes_the_models_left_behind() {
    let sandbox = Sandbox::new();
    sandbox
        .init(&["--models", "chapkit_ewars_model", "--port-base", "18130"])
        .assert()
        .success();
    sandbox
        .components(&["disable", "chap-core"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "chapkit_ewars_model now registers nowhere and is published on http://localhost:18130",
        ));
}

/// Disabling a model keeps its data, and the only thing that still knows
/// where that data is is this line: the overlay that declared the volume has
/// just been removed, so `varde down --volumes` no longer reaches it.
#[test]
fn disabling_a_model_names_the_data_volume_it_keeps() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let volume = format!("{}_ck_chapkit_ewars_model_data", compose_project(&dir));

    // Never started, so there is no volume, and none is said to be kept.
    sandbox
        .models(&["disable", "chapkit-ewars-model"])
        .assert()
        .success()
        .stdout(predicates::str::contains("kept volume").not());

    // With the volume a `varde up` would have created, the service id is
    // accepted, and the line is about the volume's real name and the
    // marketplace id that names it again.
    let Some(_created) = TestVolume::create(&volume) else {
        return;
    };
    sandbox
        .models(&["enable", "chapkit_ewars_model"])
        .assert()
        .success();
    sandbox
        .models(&["disable", "chapkit-ewars-model"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "kept volume {volume}; remove it with `varde models disable chapkit_ewars_model \
             --purge` or `docker volume rm {volume}`"
        )));

    // The same run as a document: nothing was removed, one volume was kept.
    sandbox
        .models(&["enable", "chapkit_ewars_model"])
        .assert()
        .success();
    let out = sandbox
        .chap()
        .arg("--json")
        .arg("-C")
        .arg(&dir)
        .args(["models", "disable", "chapkit_ewars_model"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let report: serde_json::Value =
        serde_json::from_slice(&out).expect("models disable --json is one document");
    assert_eq!(report["kept_volumes"], serde_json::json!([volume]));
    assert_eq!(report["purged"], serde_json::json!([]));
}

/// `--purge` on a deployment that was never started has nothing to remove,
/// and says so rather than pretending: the volume is created by `varde up`,
/// not by enabling the model.
///
/// It also has to work on a model that is no longer enabled, because that is
/// the state the kept-volume line above leaves the reader in.
#[test]
fn purging_a_model_with_no_volume_yet_says_it_found_none() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let volume = format!("{}_ck_chapkit_ewars_model_data", compose_project(&dir));

    sandbox
        .models(&["disable", "chapkit_ewars_model", "--purge"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "volume {volume} not found"
        )))
        .stdout(predicates::str::contains("kept volume").not());
    assert!(!dir.join("compose.chapkit-ewars-model.yml").exists());

    // And again, with the model already gone: only the volume is looked for,
    // and the run still succeeds.
    sandbox
        .models(&["disable", "chapkit-ewars-model", "--purge"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "chapkit_ewars_model is not enabled here, so only its data volume was looked for",
        ))
        .stdout(predicates::str::contains(format!(
            "volume {volume} not found"
        )));

    // Without `--purge` it is still the typo it always was.
    sandbox
        .models(&["disable", "chapkit_ewars_model"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("chapkit_ewars_model"));

    // And `--purge` does not turn a name nobody has ever heard of into a
    // successful no-op: nothing lists it and no volume answers to it.
    sandbox
        .models(&["disable", "no_such_model", "--purge"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("no_such_model"));
}

/// The component half of the same thing, and the one component whose volumes
/// `--purge` cannot mean.
#[test]
fn disabling_a_component_names_its_volume_and_chap_core_has_none_to_purge() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--with", "ocs,s3"])
        .assert()
        .success();
    let project = compose_project(&dir);

    // Never started: no volume, and no line claiming one was kept.
    sandbox
        .components(&["disable", "s3"])
        .assert()
        .success()
        .stdout(predicates::str::contains("kept volume").not());

    // With the volume there, it is named with both ways to remove it.
    if let Some(_created) = TestVolume::create(&format!("{project}_s3_data")) {
        sandbox.components(&["enable", "s3"]).assert().success();
        sandbox
            .components(&["disable", "s3"])
            .assert()
            .success()
            .stdout(predicates::str::contains(format!(
                "kept volume {project}_s3_data; remove it with \
                 `varde components disable s3 --purge` or `docker volume rm {project}_s3_data`"
            )));
    }

    // `--purge` is accepted on a component that is already off: the volume
    // outlives the component, which is the whole point of the flag.
    sandbox
        .components(&["disable", "s3", "--purge"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "volume {project}_s3_data not found"
        )));

    // OCS keeps a directory of its own as well, and it is never touched.
    sandbox
        .components(&["disable", "ocs", "--purge"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "volume {project}_ocs_data not found"
        )))
        .stdout(predicates::str::contains(
            "the ocs/ directory is left alone",
        ));
    assert!(dir.join("ocs").join("climate-service.yaml").is_file());

    // chap-core's volumes are upstream's, declared in a compose file this CLI
    // does not author, so there is no one volume `--purge` could mean.
    sandbox
        .components(&["disable", "chap-core", "--purge"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "chap-core keeps no volume of its own that varde names",
        ))
        .stderr(predicates::str::contains("varde down --volumes"));
    // Refused before anything was written: chap-core is still on.
    assert!(dir.join("compose.yml").is_file());
}
