//! End-to-end tests for the shared apply path: what it refuses, and what a
//! refusal leaves behind.
//!
//! Every run is `--offline` with its own cache directory, so the embedded
//! marketplace snapshot is what the CLI sees and no test touches the network
//! or the developer's real cache.

use assert_cmd::Command;
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
