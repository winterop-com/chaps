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
        serde_json::json!([
            "compose.yml",
            "compose.chapkit-ewars-model.yml",
            "compose.marketplace.yml"
        ]),
        "compose.yml is rendered by sync like the overlays"
    );
    assert_eq!(
        state["chap_compose_source"],
        serde_json::json!({"kind": "embedded"}),
        "--offline cannot fetch chap-core's own compose.ghcr.yml"
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
        .success()
        // Offline, the tag is taken as given but the compose file that tag
        // publishes cannot be downloaded; the summary and a warning say so.
        .stdout(predicates::str::contains(
            "chap-core: v1.2.3 (the compose.ghcr.yml built into this binary)",
        ))
        .stderr(predicates::str::contains("--offline"))
        .stderr(predicates::str::contains("compose.ghcr.yml"));

    assert_eq!(state(&dir)["chap_image_tag"], "v1.2.3");
    assert_eq!(
        state(&dir)["chap_compose_source"],
        serde_json::json!({"kind": "embedded"})
    );
    let env = read(&dir.join(".env"));
    assert!(env.contains("\nCHAP_IMAGE_TAG=v1.2.3\n"));
    assert!(!env.contains("# CHAP_IMAGE_TAG"));
    // The base file still reads the variable with `latest` as its default.
    assert!(read(&dir.join("compose.yml")).contains("${CHAP_IMAGE_TAG:-latest}"));
    // Nothing was cached, so nothing but the two state files is in .chaps/.
    let mut names: Vec<String> = std::fs::read_dir(dir.join(".chaps"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["models.yaml", "project.yaml"]);
}

#[test]
fn offline_leaves_the_default_tag_moving_and_says_so() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    // The default is `latest`, which init resolves to the newest release when
    // it can reach GitHub. It cannot here, so the literal tag survives and the
    // warning spells out what that means.
    sandbox
        .init(&["--models", "none"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "chap-core: latest (the compose.ghcr.yml built into this binary)",
        ))
        .stderr(predicates::str::contains(
            "the chap-core tag stays `latest`",
        ))
        .stderr(predicates::str::contains("--chap-tag vX.Y.Z"));

    assert_eq!(state(&dir)["chap_image_tag"], "latest");
    assert_eq!(
        state(&dir)["chap_compose_source"],
        serde_json::json!({"kind": "embedded"})
    );
    // A moving tag is the compose default, so the .env line stays commented.
    let env = read(&dir.join(".env"));
    assert!(env.contains("\n# CHAP_IMAGE_TAG=latest\n"), "{env}");
}

#[test]
fn init_reuses_a_cached_compose_file_and_sync_renders_from_it() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    // Seed the cache the way an online `init --chap-tag v9.9.9` would have,
    // then re-init at that tag: offline, the copy on disk is used.
    let cached = dir.join(".chaps/compose.chap-core.v9.9.9.yml");
    let body = "services:\n  chap:\n    image: ghcr.io/x/chap-core:${CHAP_IMAGE_TAG:-latest}\n";
    std::fs::write(&cached, body).unwrap();
    sandbox
        .init(&["--models", "none", "--force", "--chap-tag", "v9.9.9"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "chap-core: v9.9.9 (chap-core compose.ghcr.yml at v9.9.9)",
        ));

    let state = state(&dir);
    assert_eq!(state["chap_image_tag"], "v9.9.9");
    assert_eq!(state["chap_compose_source"]["kind"], "fetched");
    assert_eq!(state["chap_compose_source"]["tag"], "v9.9.9");
    assert!(
        state["chap_compose_source"]["url"]
            .as_str()
            .unwrap()
            .ends_with("/dhis2-chap/chap-core/v9.9.9/compose.ghcr.yml")
    );
    assert_eq!(
        state["chap_compose_source"]["sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );

    // compose.yml is the cached body behind two header lines.
    let base = read(&dir.join("compose.yml"));
    assert!(base.starts_with("# Generated by chaps init "));
    assert!(base.contains("from chap-core compose.ghcr.yml at v9.9.9.\n"));
    assert!(base.ends_with(body));
    assert_eq!(read(&cached), body, "the raw copy is left as fetched");

    // And a later sync renders the same file from the same copy.
    let mut sync = sandbox.chap();
    sync.arg("-C").arg(&dir).args(["sync", "--check"]);
    sync.assert().success().stdout(predicates::str::contains(
        "in sync: 0 to write, 2 unchanged, 0 to remove",
    ));
}

#[test]
fn sync_restores_a_hand_edited_compose_yml() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let base = dir.join("compose.yml");
    let original = read(&base);

    std::fs::write(&base, "services: {}\n").unwrap();
    let mut check = sandbox.chap();
    check.arg("-C").arg(&dir).args(["sync", "--check"]);
    check
        .assert()
        .failure()
        .stdout(predicates::str::contains("would write   compose.yml"))
        .stderr(predicates::str::contains("chaps sync"));
    assert_eq!(read(&base), "services: {}\n", "--check writes nothing");

    let mut sync = sandbox.chap();
    sync.arg("-C").arg(&dir).arg("sync");
    sync.assert()
        .success()
        .stdout(predicates::str::contains("written       compose.yml"));
    assert_eq!(
        read(&base),
        original,
        "the base stack is rendered, not held"
    );
}

#[test]
fn a_cached_compose_file_that_goes_missing_leaves_compose_yml_alone() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();
    let cached = dir.join(".chaps/compose.chap-core.v9.9.9.yml");
    std::fs::write(
        &cached,
        "services:\n  chap:\n    image: x:${CHAP_IMAGE_TAG:-latest}\n",
    )
    .unwrap();
    sandbox
        .init(&["--models", "none", "--force", "--chap-tag", "v9.9.9"])
        .assert()
        .success();
    let before = read(&dir.join("compose.yml"));

    std::fs::remove_file(&cached).unwrap();
    let mut sync = sandbox.chap();
    sync.arg("-C").arg(&dir).arg("sync");
    let out = sync
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "warning: .chaps/compose.chap-core.v9.9.9.yml is missing",
        ))
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).expect("utf-8 stdout");
    assert!(
        !stdout
            .lines()
            .any(|l| !l.starts_with("warning:") && l.contains("compose.yml")),
        "a base file we cannot reproduce is not reported as written: {stdout}"
    );
    assert_eq!(read(&dir.join("compose.yml")), before);
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
    assert_eq!(value["unchanged"].as_array().unwrap().len(), 3);

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
            "1 written, 2 unchanged, 0 removed",
        ));
    assert_eq!(read(&overlay), original, "rendering is deterministic");

    let mut again = sandbox.chap();
    again.arg("-C").arg(&dir).arg("sync");
    again.assert().success().stdout(predicates::str::contains(
        "in sync: 0 written, 3 unchanged, 0 removed",
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
    assert!(
        !outside.contains("\n  backup "),
        "backup needs a project:\n{outside}"
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
        "up", "down", "logs", "docker", "backup", "status", "sync", "update", "ui",
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

/// The member names inside a `tar.gz`, via the same `tar` the CLI uses.
fn members(archive: &Path) -> Vec<String> {
    let out = std::process::Command::new("tar")
        .arg("-tzf")
        .arg(archive)
        .output()
        .expect("tar lists an archive");
    assert!(
        out.status.success(),
        "tar -tzf failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim_end_matches('/').to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// The one file in `dir` whose name looks like a backup archive.
fn only_archive(dir: &Path) -> PathBuf {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".tar.gz"))
        .collect();
    found.sort();
    assert_eq!(found.len(), 1, "expected one archive in {}", dir.display());
    found.pop().unwrap()
}

#[test]
fn backup_create_packs_the_files_without_touching_docker() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    let out = sandbox.home.path().join("archives");
    std::fs::create_dir_all(&out).unwrap();
    let text = String::from_utf8(
        chap_in(
            &sandbox,
            &dir,
            &[
                "backup",
                "create",
                "--no-db",
                "--no-models",
                "--out",
                out.to_str().unwrap(),
            ],
        )
        .assert()
        .success()
        .get_output()
        .stdout
        .clone(),
    )
    .expect("the report is text");

    let archive = only_archive(&out);
    let name = archive.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("chapx-") || name.starts_with("chaps-backup-chapx-"),
        "the archive is named after the project: {name}"
    );
    assert!(text.contains(&archive.display().to_string()));
    assert!(text.contains("database  not included (--no-db)"));

    let members = members(&archive);
    assert!(members.contains(&"manifest.yaml".to_string()));
    for rel in [
        "files/.env",
        "files/.chaps/project.yaml",
        "files/.chaps/models.yaml",
        "files/compose.yml",
        "files/compose.marketplace.yml",
        "files/compose.chapkit-ewars-model.yml",
    ] {
        assert!(members.contains(&rel.to_string()), "{rel} is missing");
    }
    assert!(
        !members.iter().any(|m| m.starts_with("db/")),
        "--no-db leaves the dump out"
    );
    assert!(
        !members.iter().any(|m| m.starts_with("models/")),
        "--no-models leaves the volumes out"
    );
    // The staging directory is scratch space and is cleaned up after itself.
    assert!(!dir.join(".chaps/tmp").exists());

    // The manifest says what the archive is, without unpacking it.
    let manifest = std::process::Command::new("tar")
        .args(["-xzf", archive.to_str().unwrap(), "-O", "manifest.yaml"])
        .output()
        .unwrap();
    let manifest: Json =
        serde_json::to_value(serde_yaml_ng::from_slice::<Yaml>(&manifest.stdout).unwrap()).unwrap();
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["project"], "chapx");
    assert!(manifest["created_at"].as_str().unwrap().ends_with('Z'));
    assert!(manifest["database"].is_null());
    assert_eq!(manifest["models"][0]["service_id"], "chapkit-ewars-model");
    assert_eq!(manifest["models"][0]["skipped"], "--no-models");
}

#[test]
fn backup_create_json_reports_the_path_the_size_and_the_manifest() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    let out = chap_in(
        &sandbox,
        &dir,
        &["--json", "backup", "create", "--no-db", "--no-models"],
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let report: Json = serde_json::from_slice(&out).expect("--json is one JSON document");

    let path = PathBuf::from(report["path"].as_str().unwrap());
    assert!(path.is_file(), "the archive lands where the report says");
    assert_eq!(
        std::fs::canonicalize(path.parent().unwrap()).unwrap(),
        std::fs::canonicalize(&dir).unwrap(),
        "no --out means the working directory"
    );
    assert!(report["size_bytes"].as_u64().unwrap() > 0);
    assert_eq!(report["manifest"]["project"], "chapx");
    assert!(
        report["manifest"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == ".env")
    );
}

#[test]
fn backup_restore_files_only_rebuilds_a_second_deployment() {
    let sandbox = Sandbox::new();
    let source = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    let out = sandbox.home.path().join("archives");
    std::fs::create_dir_all(&out).unwrap();
    chap_in(
        &sandbox,
        &source,
        &[
            "backup",
            "create",
            "--no-db",
            "--no-models",
            "--out",
            out.to_str().unwrap(),
        ],
    )
    .assert()
    .success();
    let archive = only_archive(&out);

    // A second, empty deployment with its own generated password.
    let target = sandbox.home.path().join("chapy");
    let mut init = sandbox.chap();
    init.arg("init").arg(&target).args(["--models", "none"]);
    init.assert().success();
    let before = read(&target.join(".env"));
    assert_ne!(
        password_line(&before),
        password_line(&read(&source.join(".env"))),
        "the two deployments start with different secrets"
    );

    let text = String::from_utf8(
        chap_in(
            &sandbox,
            &target,
            &[
                "backup",
                "restore",
                archive.to_str().unwrap(),
                "--files-only",
                "--yes",
            ],
        )
        .assert()
        .success()
        .get_output()
        .stdout
        .clone(),
    )
    .expect("the report is text");

    // The plan is printed before anything happens, then what was done.
    assert!(text.contains("this overwrites"));
    // The directory is printed as the CLI resolved it, which on macOS means
    // the symlink-free form of the temporary directory.
    assert!(text.contains("into   "));
    assert!(text.contains("chapy"), "the plan names the target:\n{text}");
    assert!(text.contains("from   project chapx"));
    assert!(text.contains("files     "));
    assert!(text.contains(".env.before-restore"));
    assert!(text.contains("the stack was left as it is"));

    assert_eq!(read(&target.join(".env")), read(&source.join(".env")));
    assert_eq!(read(&target.join(".env.before-restore")), before);
    let models = read(&target.join(".chaps/models.yaml"));
    assert!(models.contains("chapkit_ewars_model"));
    assert_eq!(
        state(&target)["models"]["chapkit_ewars_model"]["service_id"],
        "chapkit-ewars-model"
    );
    // The restore ends in a sync, so the overlay is on disk and included.
    assert!(target.join("compose.chapkit-ewars-model.yml").is_file());
    assert_eq!(
        includes(&target),
        vec!["compose.chapkit-ewars-model.yml".to_string()]
    );
    assert!(!target.join(".chaps/tmp").exists());
}

#[test]
fn restore_without_yes_refuses_when_there_is_no_terminal_to_ask_at() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    let out = sandbox.home.path().join("archives");
    std::fs::create_dir_all(&out).unwrap();
    chap_in(
        &sandbox,
        &dir,
        &[
            "backup",
            "create",
            "--no-db",
            "--no-models",
            "--out",
            out.to_str().unwrap(),
        ],
    )
    .assert()
    .success();
    let archive = only_archive(&out);

    chap_in(
        &sandbox,
        &dir,
        &[
            "backup",
            "restore",
            archive.to_str().unwrap(),
            "--files-only",
        ],
    )
    .assert()
    .failure()
    .stdout(predicates::str::contains("this overwrites"))
    .stderr(predicates::str::contains("pass --yes"));
}

#[test]
fn restore_rejects_something_that_is_not_a_backup() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    let bogus = sandbox.home.path().join("notes.tar.gz");
    std::fs::write(&bogus, b"this is not a tar.gz at all").unwrap();
    chap_in(
        &sandbox,
        &dir,
        &["backup", "restore", bogus.to_str().unwrap(), "--yes"],
    )
    .assert()
    .failure()
    .stderr(predicates::str::contains("is it a chaps backup?"));

    chap_in(
        &sandbox,
        &dir,
        &["backup", "restore", "/nowhere/missing.tar.gz", "--yes"],
    )
    .assert()
    .failure()
    .stderr(predicates::str::contains("is not a file"));
}

#[test]
fn backup_outside_a_project_says_so() {
    let sandbox = Sandbox::new();
    chap_in(
        &sandbox,
        sandbox.home.path(),
        &["backup", "create", "--no-db", "--no-models"],
    )
    .assert()
    .failure()
    .stderr(predicates::str::contains("not a chaps project"));

    chap_in(
        &sandbox,
        sandbox.home.path(),
        &["backup", "restore", "x.tar.gz", "--yes"],
    )
    .assert()
    .failure()
    .stderr(predicates::str::contains("not a chaps project"));
}
