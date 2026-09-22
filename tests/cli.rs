//! End-to-end tests for the commands that write a deployment directory.
//!
//! Every run is `--offline` with its own cache directory, so the embedded
//! marketplace snapshot is what the CLI sees and no test touches the network
//! or the developer's real cache.

use assert_cmd::Command;
use serde_json::Value as Json;
use serde_yaml_ng::Value as Yaml;
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
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// The `POSTGRES_PASSWORD=` line of a generated `.env`.
fn password_line(env: &str) -> &str {
    env.lines()
        .find(|l| l.starts_with("POSTGRES_PASSWORD="))
        .expect("a password line")
}

fn yaml(path: &Path) -> Yaml {
    serde_yaml_ng::from_str(&read(path)).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `.chaps/project.yaml` with `.chaps/models.yaml` folded in under `models`,
/// as one JSON value so assertions can index into it.
fn state(dir: &Path) -> Json {
    let chaps = dir.join(".chaps");
    let mut project: Json =
        serde_json::to_value(yaml(&chaps.join("project.yaml"))).expect("project.yaml maps to JSON");
    let models: Json =
        serde_json::to_value(yaml(&chaps.join("models.yaml"))).expect("models.yaml maps to JSON");
    project["models"] = models;
    project
}

/// `chaps <args>` run from `cwd`, without `-C`.
fn chap_in(sandbox: &Sandbox, cwd: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::cargo_bin("chaps").expect("the chaps binary is built");
    cmd.env("CHAPS_CACHE_DIR", sandbox.cache.path())
        .current_dir(cwd)
        .arg("--offline")
        .args(args);
    cmd
}

fn includes(dir: &Path) -> Vec<String> {
    match yaml(&dir.join("compose.marketplace.yml")).get("include") {
        Some(Yaml::Sequence(items)) => items
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect(),
        _ => Vec::new(),
    }
}

#[test]
fn init_writes_every_file_of_a_deployment() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    for name in [
        "compose.yml",
        ".env",
        "compose.marketplace.yml",
        "compose.chapkit-ewars-model.yml",
        ".chaps/project.yaml",
        ".chaps/models.yaml",
    ] {
        assert!(dir.join(name).is_file(), "{name} was not written");
    }
    assert!(
        !dir.join("chaps.json").exists(),
        "the JSON state file is gone"
    );
    // Both state files open with a comment saying who manages them.
    assert!(
        read(&dir.join(".chaps/project.yaml"))
            .starts_with("# .chaps/project.yaml - managed by chaps")
    );
    assert!(
        read(&dir.join(".chaps/models.yaml"))
            .starts_with("# .chaps/models.yaml - managed by chaps")
    );

    let state = state(&dir);
    assert_eq!(state["schema_version"], 1);
    assert_eq!(state["chap_image_tag"], "latest");
    assert_eq!(
        state["compose_files"],
        serde_json::json!(["compose.yml", "compose.marketplace.yml"])
    );
    assert_eq!(
        state["rendered_files"],
        serde_json::json!(["compose.chapkit-ewars-model.yml", "compose.marketplace.yml"])
    );
    assert_eq!(state["port_range"], serde_json::json!([5001, 5999]));
    let model = &state["models"]["chapkit_ewars_model"];
    // --port-base defaults to 5001, so the first model lands there.
    assert_eq!(model["host_port"], 5001);
    assert_eq!(model["service_id"], "chapkit-ewars-model");
    assert_eq!(model["image_tag"], "sha-fa880a1");
    assert_eq!(model["data_dir"], "/app/data");
    assert_eq!(model["user"], "chapkit:chapkit");
    assert_eq!(model["platform"], "linux/amd64");

    assert_eq!(includes(&dir), vec!["compose.chapkit-ewars-model.yml"]);
    let overlay = yaml(&dir.join("compose.chapkit-ewars-model.yml"));
    assert_eq!(
        overlay["services"]["chapkit-ewars-model"]["ports"][0]
            .as_str()
            .unwrap(),
        "5001:8000"
    );

    // The generated .env carries a fresh password and the model's pin.
    let env = read(&dir.join(".env"));
    let password = env
        .lines()
        .find_map(|l| l.strip_prefix("POSTGRES_PASSWORD="))
        .expect("a password line");
    assert_eq!(password.len(), 32);
    assert!(env.contains("# CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-fa880a1"));
    assert!(env.contains("# CHAP_IMAGE_TAG=latest"));
}

#[test]
fn init_with_no_models_still_writes_a_valid_umbrella() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    let umbrella = yaml(&dir.join("compose.marketplace.yml"));
    assert!(
        umbrella.get("include").is_none(),
        "an empty include list is rejected by compose"
    );
    assert_eq!(umbrella["services"], Yaml::Mapping(Default::default()));
    assert_eq!(state(&dir)["models"], serde_json::json!({}));
    assert!(read(&dir.join(".env")).contains("none yet"));
}

#[test]
fn init_can_skip_the_env_file() {
    let sandbox = Sandbox::new();
    sandbox
        .init(&["--models", "none", "--no-env"])
        .assert()
        .success();
    assert!(!sandbox.project().join(".env").exists());
}

#[test]
fn enabling_a_second_model_takes_the_next_port_and_sorts_the_umbrella() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    sandbox
        .models(&["enable", "auto_arima_chapkit"])
        .assert()
        .success()
        .stdout(predicates::str::contains("chaps up"));

    assert_eq!(
        state(&dir)["models"]["auto_arima_chapkit"]["host_port"],
        5002
    );
    assert_eq!(
        includes(&dir),
        vec![
            "compose.auto-arima-chapkit.yml",
            "compose.chapkit-ewars-model.yml",
        ],
        "includes follow the marketplace id order"
    );
    assert!(read(&dir.join(".env")).contains("# AUTO_ARIMA_CHAPKIT_IMAGE_TAG=sha-70c07a9"));
}

#[test]
fn disable_accepts_the_service_id_and_removes_the_overlay() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model,auto_arima_chapkit"])
        .assert()
        .success();
    assert_eq!(includes(&dir).len(), 2);

    sandbox
        .models(&["disable", "chapkit-ewars-model"])
        .assert()
        .success()
        .stdout(predicates::str::contains("disabled chapkit_ewars_model"));

    assert!(!dir.join("compose.chapkit-ewars-model.yml").exists());
    assert_eq!(includes(&dir), vec!["compose.auto-arima-chapkit.yml"]);
    assert!(state(&dir)["models"].get("chapkit_ewars_model").is_none());
}

#[test]
fn disabling_a_model_that_is_not_enabled_fails() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "none"]).assert().success();
    sandbox
        .models(&["disable", "auto_arima_chapkit"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("unknown model"));
}

#[test]
fn a_template_needs_allow_template() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    sandbox
        .models(&["enable", "chapkit_minimalist_example_py"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--allow-template"));
    assert!(
        !dir.join("compose.chapkit-minimalist-example-py.yml")
            .exists()
    );

    sandbox
        .models(&[
            "enable",
            "chapkit_minimalist_example_py",
            "--allow-template",
        ])
        .assert()
        .success();
    assert!(
        dir.join("compose.chapkit-minimalist-example-py.yml")
            .is_file()
    );
    // The template's image is built from /work, unlike ewars.
    let model = &state(&dir)["models"]["chapkit_minimalist_example_py"];
    assert_eq!(model["data_dir"], "/work/data");
    assert_eq!(model["user"], "chapkit:chapkit");
}

#[test]
fn enable_takes_an_explicit_port_and_rejects_a_taken_one() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    sandbox
        .models(&["enable", "auto_arima_chapkit", "--port", "5100"])
        .assert()
        .success();
    assert_eq!(
        state(&dir)["models"]["auto_arima_chapkit"]["host_port"],
        5100
    );

    sandbox
        .models(&["enable", "chapkit_simple_multistep_model", "--port", "5100"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("already in use"));
}

#[test]
fn json_output_parses_for_init_and_enable() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    let out = sandbox
        .init(&["--models", "chapkit_ewars_model", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("init --json is JSON");
    assert_eq!(value["dir"], dir.to_string_lossy().as_ref());
    let written: Vec<String> = value["written"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            Path::new(v.as_str().unwrap())
                .strip_prefix(&dir)
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    for name in [
        "compose.yml",
        ".env",
        "compose.chapkit-ewars-model.yml",
        "compose.marketplace.yml",
        ".chaps/project.yaml",
        ".chaps/models.yaml",
    ] {
        assert!(written.contains(&name.to_string()), "{name} not reported");
    }
    assert_eq!(value["report"]["enabled"][0][0], "chapkit_ewars_model");
    assert_eq!(value["report"]["enabled"][0][1]["host_port"], 5001);
    assert_eq!(value["env"], "written");

    let out = sandbox
        .models(&["enable", "auto_arima_chapkit", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("enable --json is JSON");
    assert_eq!(value["enabled"][0][1]["host_port"], 5002);
    assert!(value["disabled"].as_array().unwrap().is_empty());
}

#[test]
fn json_errors_are_reported_as_json_on_stdout() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "none"]).assert().success();
    let out = sandbox
        .models(&["enable", "nope", "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("the error is JSON");
    assert!(value["error"].as_str().unwrap().contains("unknown model"));
}

#[test]
fn a_second_init_needs_force() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    sandbox
        .init(&["--models", "none"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--force"));
    // The failed run changed nothing.
    assert!(state(&dir)["models"].get("chapkit_ewars_model").is_some());

    sandbox
        .init(&["--models", "none", "--force"])
        .assert()
        .success();
    assert_eq!(state(&dir)["models"], serde_json::json!({}));
    // --force rewrites the state, so the old overlay is dropped, not orphaned.
    assert!(includes(&dir).is_empty());
    assert!(!dir.join("compose.chapkit-ewars-model.yml").exists());

    // Its host port is free again for the next model.
    sandbox
        .models(&["enable", "auto_arima_chapkit"])
        .assert()
        .success();
    assert_eq!(
        state(&dir)["models"]["auto_arima_chapkit"]["host_port"],
        5001
    );
}

#[test]
fn force_keeps_the_env_file_it_found() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let before = read(&dir.join(".env"));

    // .env holds the database password the postgres volume was created with,
    // the API token and the registration key: --force re-renders everything
    // else, but rewriting this file locks the operator out of their own data.
    sandbox
        .init(&["--models", "none", "--force"])
        .assert()
        .success()
        .stdout(predicates::str::contains("kept .env (already present)"));

    let after = read(&dir.join(".env"));
    assert_eq!(
        password_line(&after),
        password_line(&before),
        "--force rotated POSTGRES_PASSWORD"
    );
    assert_eq!(after, before, "--force rewrote .env");

    // A kept file is reported as kept rather than as one of init's writes.
    let out = sandbox
        .init(&["--models", "none", "--force", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("init --json is JSON");
    assert_eq!(value["env"], "kept");
    let written: Vec<&str> = value["written"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        !written.iter().any(|p| p.ends_with("/.env")),
        "the kept .env is listed as written: {written:?}"
    );
    assert_eq!(read(&dir.join(".env")), before);
}

#[test]
fn fresh_env_rewrites_the_env_file_and_warns() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let before = read(&dir.join(".env"));

    let out = sandbox
        .init(&["--models", "chapkit_ewars_model", "--force", "--fresh-env"])
        .assert()
        .success()
        .stderr(predicates::str::contains("POSTGRES_PASSWORD"))
        .stderr(predicates::str::contains("down -v"))
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).expect("utf-8 stdout");
    assert!(!stdout.contains("kept .env"), "{stdout}");

    let after = read(&dir.join(".env"));
    assert_ne!(
        password_line(&after),
        password_line(&before),
        "--fresh-env kept the old password"
    );
    // The file is a complete render again, pin comment included.
    assert!(after.contains("# CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-fa880a1"));
}

#[test]
fn fresh_env_and_no_env_are_mutually_exclusive() {
    let sandbox = Sandbox::new();
    sandbox
        .init(&["--models", "none", "--fresh-env", "--no-env"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot be used with"));
    assert!(!sandbox.project().exists());
}

#[test]
fn forcing_the_same_model_keeps_its_port() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model,auto_arima_chapkit"])
        .assert()
        .success();
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        5001
    );

    sandbox
        .init(&["--models", "chapkit_ewars_model", "--force"])
        .assert()
        .success();
    // The old overlay is rewritten in place rather than blocking its own port.
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        5001
    );
    assert!(!dir.join("compose.auto-arima-chapkit.yml").exists());
    assert_eq!(includes(&dir), vec!["compose.chapkit-ewars-model.yml"]);
}

#[test]
fn a_local_chap_core_build_is_rejected() {
    let sandbox = Sandbox::new();
    sandbox
        .init(&["--source", "/tmp/chap-core"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "local chap-core build not yet supported",
        ));
    assert!(!sandbox.project().exists(), "nothing is written");
}

#[test]
fn the_chap_tag_reaches_the_env_file_and_the_state() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--chap-tag", "v1.2.3"])
        .assert()
        .success();

    assert_eq!(state(&dir)["chap_image_tag"], "v1.2.3");
    let env = read(&dir.join(".env"));
    assert!(env.contains("\nCHAP_IMAGE_TAG=v1.2.3\n"));
    assert!(!env.contains("# CHAP_IMAGE_TAG"));
    // The base file still reads the variable with `latest` as its default.
    assert!(read(&dir.join("compose.yml")).contains("${CHAP_IMAGE_TAG:-latest}"));
}

#[test]
fn the_port_base_moves_the_whole_range() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model", "--port-base", "5500"])
        .assert()
        .success();
    assert_eq!(state(&dir)["port_range"], serde_json::json!([5500, 5999]));
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        5500
    );
}

#[test]
fn enable_outside_a_project_says_so() {
    let sandbox = Sandbox::new();
    sandbox
        .models(&["enable", "chapkit_ewars_model"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not a chaps project"));
}

#[test]
fn an_exact_version_pins_without_a_channel() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();
    sandbox
        .models(&["enable", "auto_arima_chapkit", "--version", "1.0.0"])
        .assert()
        .success();

    let model = &state(&dir)["models"]["auto_arima_chapkit"];
    assert_eq!(model["version"], "1.0.0");
    assert_eq!(model["channel"], Json::Null);

    sandbox
        .models(&["enable", "auto_arima_chapkit", "--version", "9.9.9"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("no version `9.9.9`"));
}

#[test]
fn re_enabling_keeps_the_port_and_applies_the_overrides() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model,auto_arima_chapkit"])
        .assert()
        .success();
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        5001
    );

    sandbox
        .models(&[
            "enable",
            "chapkit_ewars_model",
            "--data-dir",
            "/srv/data",
            "--user",
            "1000:1000",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("updated chapkit_ewars_model"));

    let model = &state(&dir)["models"]["chapkit_ewars_model"];
    assert_eq!(model["host_port"], 5001, "the port survives a rewrite");
    assert_eq!(model["data_dir"], "/srv/data");
    assert_eq!(model["user"], "1000:1000");

    let overlay = yaml(&dir.join("compose.chapkit-ewars-model.yml"));
    let svc = &overlay["services"]["chapkit-ewars-model"];
    assert_eq!(svc["user"].as_str(), Some("1000:1000"));
    assert_eq!(svc["volumes"][1]["target"].as_str(), Some("/srv/data"));
}

#[test]
fn commands_find_the_project_from_a_subdirectory() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let sub = dir.join("ops").join("notes");
    std::fs::create_dir_all(&sub).unwrap();

    chap_in(&sandbox, &sub, &["models", "list", "--enabled"])
        .assert()
        .success()
        .stdout(predicates::str::contains("chapkit_ewars_model"));
    chap_in(&sandbox, &sub, &["sync", "--check"])
        .assert()
        .success()
        .stdout(predicates::str::contains("in sync"));

    // Above the project there is nothing to find, and the error names where
    // the search started.
    chap_in(&sandbox, sandbox.home.path(), &["sync", "--check"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not a chaps project"))
        .stderr(predicates::str::contains(".chaps/project.yaml"));
}

#[test]
fn sync_check_is_clean_after_init_and_reports_drift_after_a_deletion() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    let mut check = sandbox.chap();
    check
        .arg("-C")
        .arg(&dir)
        .args(["sync", "--check", "--json"]);
    let out = check.assert().success().get_output().stdout.clone();
    let value: Json = serde_json::from_slice(&out).expect("sync --json is JSON");
    assert_eq!(value["drift"], false);
    assert_eq!(value["check"], true);
    assert!(value["written"].as_array().unwrap().is_empty());
    assert_eq!(value["unchanged"].as_array().unwrap().len(), 2);

    let overlay = dir.join("compose.chapkit-ewars-model.yml");
    std::fs::remove_file(&overlay).unwrap();
    let mut check = sandbox.chap();
    check.arg("-C").arg(&dir).args(["sync", "--check"]);
    check
        .assert()
        .failure()
        .stdout(predicates::str::contains(
            "would write   compose.chapkit-ewars-model.yml",
        ))
        .stderr(predicates::str::contains("chaps sync"));
    assert!(!overlay.exists(), "--check writes nothing");

    // --json exits non-zero too, with the report as the only stdout document.
    let mut check = sandbox.chap();
    check
        .arg("-C")
        .arg(&dir)
        .args(["sync", "--check", "--json"]);
    let out = check.assert().failure().get_output().stdout.clone();
    let value: Json = serde_json::from_slice(&out).expect("one JSON document");
    assert_eq!(value["drift"], true);
}

#[test]
fn sync_recreates_a_deleted_overlay() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let overlay = dir.join("compose.chapkit-ewars-model.yml");
    let original = read(&overlay);
    std::fs::remove_file(&overlay).unwrap();

    let mut sync = sandbox.chap();
    sync.arg("-C").arg(&dir).arg("sync");
    sync.assert()
        .success()
        .stdout(predicates::str::contains(
            "written       compose.chapkit-ewars-model.yml",
        ))
        .stdout(predicates::str::contains(
            "1 written, 1 unchanged, 0 removed",
        ));
    assert_eq!(read(&overlay), original, "rendering is deterministic");

    let mut again = sandbox.chap();
    again.arg("-C").arg(&dir).arg("sync");
    again.assert().success().stdout(predicates::str::contains(
        "in sync: 0 written, 2 unchanged, 0 removed",
    ));
}

#[test]
fn sync_never_removes_a_hand_written_overlay() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model,auto_arima_chapkit"])
        .assert()
        .success();
    let custom = dir.join("compose.custom.yml");
    std::fs::write(&custom, "services:\n  mine:\n    image: busybox\n").unwrap();

    sandbox
        .models(&["disable", "auto_arima_chapkit"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "removed compose.auto-arima-chapkit.yml",
        ));
    assert!(!dir.join("compose.auto-arima-chapkit.yml").exists());
    assert!(custom.is_file());

    // A hand edit of models.yaml is picked up by sync the same way.
    let models = dir.join(".chaps/models.yaml");
    std::fs::write(&models, "# emptied by hand\n").unwrap();
    let mut sync = sandbox.chap();
    sync.arg("-C").arg(&dir).arg("sync");
    sync.assert().success().stdout(predicates::str::contains(
        "removed       compose.chapkit-ewars-model.yml",
    ));
    assert!(!dir.join("compose.chapkit-ewars-model.yml").exists());
    assert!(custom.is_file(), "compose.custom.yml is not ours to delete");
    assert!(includes(&dir).is_empty());
    assert_eq!(state(&dir)["models"], serde_json::json!({}));
}

#[test]
fn update_needs_the_network_even_for_a_dry_run() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let before = read(&dir.join(".chaps/models.yaml"));

    let mut update = sandbox.chap();
    update.arg("-C").arg(&dir).args(["update", "--dry-run"]);
    update
        .assert()
        .failure()
        .stderr(predicates::str::contains("--offline"))
        .stderr(predicates::str::contains("registry"));
    assert_eq!(read(&dir.join(".chaps/models.yaml")), before);
}

#[test]
fn the_help_lists_only_the_commands_that_can_work_here() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();

    // Nothing has been created yet, so the whole home directory is outside a
    // project: only the commands that need no deployment are offered.
    let outside = chap_in(&sandbox, sandbox.home.path(), &["--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let outside = String::from_utf8(outside).expect("help is text");
    assert!(outside.contains("init"), "init has to be reachable");
    assert!(outside.contains("models"));
    assert!(outside.contains("registry"));
    assert!(
        !outside.contains("\n  up "),
        "up needs a project:\n{outside}"
    );
    assert!(
        !outside.contains("\n  docker "),
        "docker needs a project:\n{outside}"
    );
    assert!(outside.contains("Inside a directory created by `chaps init`"));

    sandbox.init(&["--models", "none"]).assert().success();

    let inside = chap_in(&sandbox, &dir, &["--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let inside = String::from_utf8(inside).expect("help is text");
    for name in [
        "up", "down", "logs", "docker", "status", "sync", "update", "tui",
    ] {
        assert!(
            inside.contains(&format!("\n  {name} ")),
            "{name} should be listed inside a project:\n{inside}"
        );
    }
    assert!(!inside.contains("Inside a directory created by `chaps init`"));

    // Hiding is cosmetic: a hidden command still runs, and `docker` reaches
    // its own help from a subdirectory of the project.
    let sub = dir.join("ops");
    std::fs::create_dir_all(&sub).unwrap();
    chap_in(&sandbox, &sub, &["docker", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("exec"))
        .stdout(predicates::str::contains("pull"));

    // And outside a project it is hidden, not removed.
    chap_in(&sandbox, sandbox.home.path(), &["docker", "ps"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not a chaps project"));
}

#[test]
fn init_inside_a_project_warns_about_the_parent() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    let mut nested = sandbox.chap();
    nested
        .arg("init")
        .arg(dir.join("inner"))
        .args(["--models", "none"]);
    nested
        .assert()
        .success()
        .stderr(predicates::str::contains("is inside the chaps project at"));
    assert!(dir.join("inner/.chaps/project.yaml").is_file());
}
