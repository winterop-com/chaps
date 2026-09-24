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

    /// `chaps -C <project> models ...`.
    fn models(&self, args: &[&str]) -> Command {
        let mut cmd = self.chap();
        cmd.arg("-C").arg(self.project()).arg("models").args(args);
        cmd
    }

    /// `chaps -C <project> components ...`.
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
    read(&dir.join(".chaps").join("project.yaml"))
        .lines()
        .find_map(|line| line.strip_prefix("compose_project:"))
        .map(|value| value.trim().to_string())
        .expect("init records a compose project name")
}

/// `chaps init --force` re-renders the whole deployment, which means deleting
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
    let state_before = read(&dir.join(".chaps").join("models.yaml"));
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
        read(&dir.join(".chaps").join("models.yaml")),
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

/// A model service registers with chap-core and is reached through it, so a
/// deployment without chap-core has nowhere to put one. `init` says so about
/// its own flag; this is the same dependency seen from `models enable`.
#[test]
fn a_model_cannot_be_enabled_into_a_deployment_without_chap_core() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--without", "chap-core"]).assert().success();

    sandbox
        .models(&["enable", "chapkit_ewars_model"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "models need the chap-core component",
        ))
        .stderr(predicates::str::contains(
            "chaps components enable chap-core",
        ));

    // Nothing was written: no overlay, and the model set is still empty.
    assert!(!dir.join("compose.chapkit-ewars-model.yml").exists());
    assert!(
        !read(&dir.join(".chaps").join("models.yaml")).contains("chapkit_ewars_model"),
        "the state records a model the deployment refused to enable"
    );

    // What the message says to do is what makes it work.
    sandbox
        .components(&["enable", "chap-core"])
        .assert()
        .success();
    sandbox
        .models(&["enable", "chapkit_ewars_model"])
        .assert()
        .success();
    assert!(dir.join("compose.chapkit-ewars-model.yml").is_file());
}

/// Disabling a model keeps its data, and the only thing that still knows
/// where that data is is this line: the overlay that declared the volume has
/// just been removed, so `chaps docker run -- down -v` no longer reaches it.
#[test]
fn disabling_a_model_names_the_data_volume_it_keeps() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let volume = format!("{}_ck_chapkit_ewars_model_data", compose_project(&dir));

    // The service id is accepted, and the line is about the volume's real
    // name and the marketplace id that names it again.
    sandbox
        .models(&["disable", "chapkit-ewars-model"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "kept volume {volume}; remove it with `chaps models disable chapkit_ewars_model \
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
/// and says so rather than pretending: the volume is created by `chaps up`,
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

    sandbox
        .components(&["disable", "s3"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "kept volume {project}_s3_data; remove it with `chaps components disable s3 --purge` \
             or `docker volume rm {project}_s3_data`"
        )));

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
            "chap-core keeps no volume of its own that chaps names",
        ))
        .stderr(predicates::str::contains("down -v"));
    // Refused before anything was written: chap-core is still on.
    assert!(dir.join("compose.yml").is_file());
}
