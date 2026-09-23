//! End-to-end tests for the commands that write a deployment directory.
//!
//! Every run is `--offline` with its own cache directory, so the embedded
//! marketplace snapshot is what the CLI sees and no test touches the network
//! or the developer's real cache.

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
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

    /// `chaps -C <project> auth ...`.
    fn auth(&self, args: &[&str]) -> Command {
        let mut cmd = self.chap();
        cmd.arg("-C").arg(self.project()).arg("auth").args(args);
        cmd
    }

    /// The project's `.env`, as text.
    fn env(&self) -> String {
        read(&self.project().join(".env"))
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Rewrite CRLF line endings as LF, for text that came out of the checkout
/// rather than out of the CLI: the CLI writes `\n` on every platform, and
/// git may not have.
fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// The `POSTGRES_PASSWORD=` line of a generated `.env`.
fn password_line(env: &str) -> &str {
    env.lines()
        .find(|l| l.starts_with("POSTGRES_PASSWORD="))
        .expect("a password line")
}

/// The value of an active (uncommented) `VAR=` line of a `.env`.
fn env_value<'a>(env: &'a str, var: &str) -> Option<&'a str> {
    env.lines()
        .find_map(|line| line.strip_prefix(&format!("{var}=")))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Every line of a `.env` that assigns none of `vars`, commented or not.
///
/// What `chaps auth` must leave byte for byte as it found it.
fn env_without<'a>(env: &'a str, vars: &[&str]) -> Vec<&'a str> {
    env.lines()
        .filter(|line| {
            let bare = line.trim_start().trim_start_matches('#').trim_start();
            !vars.iter().any(|var| bare.starts_with(&format!("{var}=")))
        })
        .collect()
}

/// A generated secret: 64 lowercase hex characters.
fn is_generated_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
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
        "compose.chaps.yml",
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
        serde_json::json!([
            "compose.yml",
            "compose.chaps.yml",
            "compose.marketplace.yml"
        ]),
        "the chaps overrides need their own -f entry, after the base file"
    );
    assert_eq!(
        state["rendered_files"],
        serde_json::json!([
            "compose.yml",
            "compose.chaps.yml",
            "compose.chapkit-ewars-model.yml",
            "compose.marketplace.yml"
        ]),
        "compose.yml is rendered by sync like the overlays"
    );
    assert_eq!(state["api_port"], 8000);
    assert_eq!(
        state["chap_compose_source"],
        serde_json::json!({"kind": "embedded"}),
        "--offline cannot fetch chap-core's own compose.ghcr.yml"
    );
    assert_eq!(state["port_range"], serde_json::json!([5001, 5999]));
    let model = &state["models"]["chapkit_ewars_model"];
    // A model publishes no host port until someone asks for one.
    assert_eq!(model["host_port"], Json::Null);
    assert_eq!(model["service_id"], "chapkit-ewars-model");
    assert_eq!(model["image_tag"], "sha-fa880a1");
    assert_eq!(model["data_dir"], "/app/data");
    assert_eq!(model["user"], "chapkit:chapkit");
    assert_eq!(model["platform"], "linux/amd64");

    assert_eq!(includes(&dir), vec!["compose.chapkit-ewars-model.yml"]);
    let overlay = yaml(&dir.join("compose.chapkit-ewars-model.yml"));
    let svc = &overlay["services"]["chapkit-ewars-model"];
    assert_eq!(svc["expose"][0].as_str().unwrap(), "8000");
    assert!(
        svc.get("ports").is_none(),
        "a model service publishes nothing by default"
    );

    // chap's port is the only published one, and it comes from a chaps-owned
    // override rather than an edit of the upstream base file.
    let chaps_overlay = read(&dir.join("compose.chaps.yml"));
    assert!(chaps_overlay.contains("ports: !override"));
    assert!(chaps_overlay.contains("\"${CHAP_API_PORT:-8000}:8000\""));

    // The generated .env carries a fresh password and the model's pin.
    let env = read(&dir.join(".env"));
    let password = env
        .lines()
        .find_map(|l| l.strip_prefix("POSTGRES_PASSWORD="))
        .expect("a password line");
    assert_eq!(password.len(), 32);
    assert!(env.contains("# CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-fa880a1"));
    assert!(env.contains("# CHAP_IMAGE_TAG=latest"));
    // Active even at the default, so the port is where you would look for it.
    assert!(env.contains("\nCHAP_API_PORT=8000\n"), "{env}");
}

/// Whether `docker compose` can be run at all, for the tests that ask it to
/// validate a rendered stack. Missing Docker skips them rather than failing.
fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["compose", "version", "--short"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn docker_accepts_the_stack_with_the_chaps_override() {
    if !docker_available() {
        eprintln!("skipping: docker compose is not available");
        return;
    }
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    // `!override` needs Compose 2.24+; this is what says the rendered stack is
    // a document the installed Docker actually accepts.
    let out = std::process::Command::new("docker")
        .args(["compose", "-f", "compose.yml", "-f", "compose.chaps.yml"])
        .args(["-f", "compose.marketplace.yml", "config"])
        .current_dir(&dir)
        .output()
        .expect("docker compose config runs");
    assert!(
        out.status.success(),
        "docker compose config failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // chap is published exactly once, on the API port; the model not at all.
    let merged: Yaml = serde_yaml_ng::from_slice(&out.stdout).expect("config is YAML");
    let ports = merged["services"]["chap"]["ports"]
        .as_sequence()
        .expect("chap publishes a port");
    assert_eq!(ports.len(), 1, "!override replaced the mapping: {ports:?}");
    assert_eq!(ports[0]["published"].as_str(), Some("8000"));
    assert_eq!(ports[0]["target"].as_u64(), Some(8000));
    assert!(
        merged["services"]["chapkit-ewars-model"]
            .get("ports")
            .is_none(),
        "the model is internal"
    );
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
fn enabling_a_second_model_publishes_nothing_and_sorts_the_umbrella() {
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
        .stdout(predicates::str::contains("chaps up"))
        // The proxy URL is how a model with no host port is reached.
        .stdout(predicates::str::contains(
            "http://localhost:8000/v2/services/auto-arima-chapkit/run/",
        ));

    assert_eq!(
        state(&dir)["models"]["auto_arima_chapkit"]["host_port"],
        Json::Null
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
        .success()
        .stdout(predicates::str::contains("http://localhost:5100"));
    assert_eq!(
        state(&dir)["models"]["auto_arima_chapkit"]["host_port"],
        5100
    );
    let svc = &yaml(&dir.join("compose.auto-arima-chapkit.yml"))["services"]["auto-arima-chapkit"];
    assert_eq!(svc["ports"][0].as_str(), Some("5100:8000"));
    assert_eq!(
        svc["expose"][0].as_str(),
        Some("8000"),
        "expose documents the container port either way"
    );

    sandbox
        .models(&["enable", "chapkit_simple_multistep_model", "--port", "5100"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "already in use by another compose file",
        ));

    // `--port auto` takes the lowest free one instead.
    sandbox
        .models(&["enable", "chapkit_simple_multistep_model", "--port", "auto"])
        .assert()
        .success();
    assert_eq!(
        state(&dir)["models"]["chapkit_simple_multistep_model"]["host_port"],
        5001
    );

    // Anything that is neither a number nor `auto` is a parse error.
    sandbox
        .models(&["enable", "auto_arima_chapkit", "--port", "nope"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("`auto`"));
}

#[test]
fn expose_and_unexpose_move_a_models_host_port_without_moving_its_pin() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let pinned = read(&dir.join(".chaps/models.yaml"));

    sandbox
        .models(&["expose", "chapkit_ewars_model", "--port", "auto"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "exposed chapkit-ewars-model on http://localhost:5001",
        ))
        .stdout(predicates::str::contains(
            "written  compose.chapkit-ewars-model.yml",
        ));
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        5001
    );
    let svc =
        &yaml(&dir.join("compose.chapkit-ewars-model.yml"))["services"]["chapkit-ewars-model"];
    assert_eq!(svc["ports"][0].as_str(), Some("5001:8000"));

    // Only the port moved: the version and the image pin are untouched.
    let after = read(&dir.join(".chaps/models.yaml"));
    assert_eq!(
        after.replace("host_port: 5001", "host_port: null"),
        pinned,
        "expose re-resolved the version"
    );

    // The service id works too, and unexpose names the way back in.
    sandbox
        .models(&["unexpose", "chapkit-ewars-model"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "unexposed chapkit-ewars-model; it stays registered with chap-core and reachable \
             at http://localhost:8000/v2/services/chapkit-ewars-model/run/",
        ));
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        Json::Null
    );
    let svc =
        &yaml(&dir.join("compose.chapkit-ewars-model.yml"))["services"]["chapkit-ewars-model"];
    assert!(svc.get("ports").is_none());
    assert_eq!(read(&dir.join(".chaps/models.yaml")), pinned);

    // A model this project never enabled cannot be exposed.
    sandbox
        .models(&["expose", "auto_arima_chapkit"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("unknown model"));
}

#[test]
fn models_list_and_info_say_internal_until_a_port_is_published() {
    let sandbox = Sandbox::new();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    sandbox
        .models(&["list", "--enabled"])
        .assert()
        .success()
        .stdout(predicates::str::contains("internal"));
    sandbox
        .models(&["info", "chapkit_ewars_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "reach    internal (proxy: http://localhost:8000/v2/services/chapkit-ewars-model/run/)",
        ));

    sandbox
        .models(&["expose", "chapkit_ewars_model"])
        .assert()
        .success();
    let listed = String::from_utf8(
        sandbox
            .models(&["list", "--enabled"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .expect("the table is text");
    assert!(listed.contains("5001"), "{listed}");
    assert!(!listed.contains("internal"), "{listed}");
    sandbox
        .models(&["info", "chapkit_ewars_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains("reach    http://localhost:5001"));
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
    let written: Vec<PathBuf> = value["written"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            Path::new(v.as_str().unwrap())
                .strip_prefix(&dir)
                .unwrap()
                .to_path_buf()
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
        // Compared as paths: the separator the CLI prints is the platform's.
        assert!(
            written.contains(&PathBuf::from(name)),
            "{name} not reported"
        );
    }
    assert_eq!(value["report"]["enabled"][0][0], "chapkit_ewars_model");
    assert_eq!(value["report"]["enabled"][0][1]["host_port"], Json::Null);
    assert_eq!(value["env"], "written");
    assert_eq!(value["api_port"], 8000);
    assert_eq!(value["api_url"], "http://localhost:8000");

    let out = sandbox
        .models(&["enable", "auto_arima_chapkit", "--port", "auto", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("enable --json is JSON");
    assert_eq!(value["enabled"][0][1]["host_port"], 5001);
    assert!(value["disabled"].as_array().unwrap().is_empty());

    // expose/unexpose have a JSON shape of their own.
    let out = sandbox
        .models(&["unexpose", "auto_arima_chapkit", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("unexpose --json is JSON");
    assert_eq!(value["id"], "auto_arima_chapkit");
    assert_eq!(value["service_id"], "auto-arima-chapkit");
    assert_eq!(value["host_port"], Json::Null);
    assert_eq!(value["previous"], 5001);
    assert_eq!(
        value["url"],
        "http://localhost:8000/v2/services/auto-arima-chapkit/run/"
    );
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

    // A host port the old overlay held is free again for the next model.
    sandbox
        .models(&["enable", "auto_arima_chapkit", "--port", "auto"])
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
        !written
            .iter()
            .any(|p| Path::new(p).file_name() == Some(".env".as_ref())),
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
fn forcing_the_same_model_starts_its_state_over() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model,auto_arima_chapkit"])
        .assert()
        .success();
    sandbox
        .models(&["expose", "chapkit_ewars_model", "--port", "5001"])
        .assert()
        .success();

    sandbox
        .init(&["--models", "chapkit_ewars_model", "--force"])
        .assert()
        .success();
    // `--force` re-renders the whole state, and `init` publishes no model
    // ports, so a port someone exposed by hand goes with it. Re-expose it.
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        Json::Null
    );
    assert!(!dir.join("compose.auto-arima-chapkit.yml").exists());
    assert_eq!(includes(&dir), vec!["compose.chapkit-ewars-model.yml"]);

    // And the port it used to hold is free, not orphaned in a stale overlay.
    sandbox
        .models(&["expose", "chapkit_ewars_model", "--port", "5001"])
        .assert()
        .success();
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
        "in sync: 0 to write, 3 unchanged, 0 to remove",
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

    // The range only matters once a port is asked for.
    sandbox
        .models(&["expose", "chapkit_ewars_model"])
        .assert()
        .success();
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        5500
    );
}

#[test]
fn the_api_port_reaches_the_env_file_the_state_and_status() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--api-port", "8123"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "API:       http://localhost:8123",
        ));

    assert_eq!(state(&dir)["api_port"], 8123);
    let env = read(&dir.join(".env"));
    assert!(env.contains("\nCHAP_API_PORT=8123\n"), "{env}");
    assert!(read(&dir.join("compose.chaps.yml")).contains("${CHAP_API_PORT:-8123}:8000"));

    // `status --url` defaults to the port the project records. The API is not
    // running, so the report is a down one - at the right URL.
    let mut status = sandbox.chap();
    status.arg("-C").arg(&dir).args(["status", "--json"]);
    let out = status.assert().failure().get_output().stdout.clone();
    let value: Json = serde_json::from_slice(&out).expect("status --json is JSON");
    assert_eq!(value["api_url"], "http://localhost:8123");
    assert_eq!(value["api"]["state"], "down");
}

#[test]
fn a_kept_env_file_that_pins_another_api_port_is_reported() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--api-port", "8010"])
        .assert()
        .success();

    // `--force` keeps .env, and compose reads .env after the compose files, so
    // the line in it wins over the new --api-port. Say so.
    sandbox
        .init(&["--models", "none", "--force", "--api-port", "8020"])
        .assert()
        .success()
        .stdout(predicates::str::contains("kept .env (already present)"))
        .stderr(predicates::str::contains(
            "warning: .env already sets CHAP_API_PORT=8010",
        ))
        .stderr(predicates::str::contains("the API stays on 8010"));
    // The recorded intent did move, and so did the rendered override.
    assert_eq!(state(&dir)["api_port"], 8020);
    assert!(read(&dir.join("compose.chaps.yml")).contains("${CHAP_API_PORT:-8020}"));
    assert!(read(&dir.join(".env")).contains("\nCHAP_API_PORT=8010\n"));

    // Re-initialising at the port .env already holds says nothing.
    let out = sandbox
        .init(&["--models", "none", "--force", "--api-port", "8010"])
        .assert()
        .success()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(out).expect("utf-8 stderr");
    assert!(!stderr.contains("CHAP_API_PORT"), "{stderr}");
}

#[test]
fn init_warns_when_something_is_already_listening_on_the_api_port() {
    // A listener on a port the kernel picked, so the test never fights another
    // process over a fixed number.
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a free port");
    let port = listener.local_addr().unwrap().port();

    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--api-port", &port.to_string()])
        .assert()
        // A warning, not a refusal: the process holding the port may be a
        // stack this deployment is meant to replace.
        .success()
        .stderr(predicates::str::contains(format!(
            "port {port} is already in use on this machine (needed by chap)"
        )))
        .stderr(predicates::str::contains("--api-port"))
        .stderr(predicates::str::contains("set CHAP_API_PORT="));

    // The directory is written all the same, at the port that was asked for.
    assert_eq!(state(&dir)["api_port"], port);
    let out = sandbox
        .init(&[
            "--models",
            "none",
            "--force",
            "--api-port",
            &port.to_string(),
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("init --json is JSON");
    assert_eq!(value["api_port_busy"], true);
    drop(listener);
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
    sandbox
        .models(&["expose", "chapkit_ewars_model", "--port", "auto"])
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
    assert_eq!(value["unchanged"].as_array().unwrap().len(), 4);

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
            "1 written, 3 unchanged, 0 removed",
        ));
    assert_eq!(read(&overlay), original, "rendering is deterministic");

    let mut again = sandbox.chap();
    again.arg("-C").arg(&dir).arg("sync");
    again.assert().success().stdout(predicates::str::contains(
        "in sync: 0 written, 4 unchanged, 0 removed",
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
    assert!(
        !outside.contains("\n  auth "),
        "auth needs a project:\n{outside}"
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
        "up", "down", "logs", "docker", "backup", "status", "sync", "update", "ui", "auth",
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
fn the_help_says_what_chap_is() {
    let sandbox = Sandbox::new();

    // The header has to name the platform, not just chap-core: someone typing
    // `chaps --help` for the first time may not know what chap-core is.
    chap_in(&sandbox, sandbox.home.path(), &["--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "Climate Health Analytics Platform",
        ));

    // The short help is the one-liner, and says the same thing.
    chap_in(&sandbox, sandbox.home.path(), &["-h"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "deploy and manage CHAP, the Climate Health Analytics Platform",
        ));
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
    assert!(text.contains("CHAP was left as it is"));

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

/// A server that answers every request with an HTML 200, on a port of its own.
///
/// Stands in for whatever else may hold chap-core's port: a dev server, a
/// proxy, a static site. The thread lives as long as the test process.
fn html_server() -> u16 {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let port = listener.local_addr().expect("a local address").port();
    std::thread::spawn(move || {
        const BODY: &str = "<!doctype html><html><body>a dev server</body></html>";
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            // The request has to be read before the answer, or the client
            // sees a reset instead of the response.
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{BODY}",
                BODY.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    port
}

#[test]
fn status_does_not_call_something_that_is_not_chap_core_up() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    let url = format!("http://127.0.0.1:{}", html_server());
    let mut status = sandbox.chap();
    status
        .arg("-C")
        .arg(&dir)
        .args(["status", "--url"])
        .arg(&url);
    let assert = status.assert().failure();
    let out = assert.get_output();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // A 200 is not enough: the report says down, and the one error line says
    // what answered instead.
    assert!(
        stdout.contains(&format!("chap-core   down   {url}")),
        "{stdout}"
    );
    assert!(stderr.contains("not chap-core"), "{stderr}");
    assert!(stderr.contains("text/html"), "{stderr}");
    assert!(!stdout.contains("   up   "), "{stdout}");

    // And --json stays one document, with the same verdict in it.
    let mut status = sandbox.chap();
    status
        .arg("-C")
        .arg(&dir)
        .args(["--json", "status", "--url"])
        .arg(&url);
    let out = status.assert().failure().get_output().stdout.clone();
    let report: Json = serde_json::from_slice(&out).expect("status --json is one document");
    assert_eq!(report["api"]["state"], "down");
    assert!(
        report["api"]["error"]
            .as_str()
            .unwrap()
            .contains("not chap-core")
    );
    assert_eq!(report["models"][0]["state"], "not-running");
    assert_eq!(report["models"][0]["reach"], "internal");
    assert_eq!(report["version"]["pinned"], true);
}

#[test]
fn the_wrappers_speak_up_for_a_project_that_was_never_started() {
    if !docker_available() {
        eprintln!("skipping: docker compose is not available");
        return;
    }
    let sandbox = Sandbox::new();
    // A directory of its own: compose names the project after it, and these
    // wrappers ask docker about that name.
    let dir = sandbox.home.path().join("chaps-never-started");
    let mut init = sandbox.chap();
    init.arg("init")
        .arg(&dir)
        .args(["--models", "chapkit_ewars_model"]);
    init.assert().success();

    // `docker compose logs` on a project with no containers prints nothing at
    // all and exits 0; the wrapper says what is going on, and fails.
    chap_in(&sandbox, &dir, &["logs"])
        .assert()
        .failure()
        .stdout(predicates::str::contains(
            "nothing is running for this project; start CHAP with `chaps up`",
        ));

    // The same line for `docker ps`, which is a question, not a failure.
    chap_in(&sandbox, &dir, &["docker", "ps"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "nothing is running for this project",
        ));

    // Under --json the answer is still one document: an empty list.
    let out = chap_in(&sandbox, &dir, &["--json", "docker", "ps"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("docker ps --json is one document");
    assert_eq!(value, serde_json::json!([]));

    // `down` says what it stopped, even when that was nothing.
    chap_in(&sandbox, &dir, &["down"])
        .assert()
        .success()
        .stdout(predicates::str::contains("nothing was running"));

    // `status` on a deployment that was never started is one line about the
    // stack, not a table of models that cannot have registered.
    chap_in(&sandbox, &dir, &["status"])
        .assert()
        .failure()
        .stdout(predicates::str::contains(
            "CHAP is not running; start it with `chaps up`",
        ));
}

#[test]
fn up_offers_both_names_for_running_in_the_foreground() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    chap_in(&sandbox, &dir, &["up", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("-a, --attach"))
        .stdout(predicates::str::contains("[alias: --foreground]"))
        .stdout(predicates::str::contains(
            "Run in the foreground and stream all logs (Ctrl-C stops CHAP)",
        ));
}

#[test]
fn the_command_reference_chapter_matches_the_help_texts() {
    // `docs-markdown` needs neither a project nor the network, so it runs
    // straight out of the repository.
    let out = Command::cargo_bin("chaps")
        .expect("the chaps binary is built")
        .arg("docs-markdown")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let generated = String::from_utf8(out).expect("the reference is UTF-8");

    // Read through whatever the checkout did to it: a Windows clone with
    // git's default `core.autocrlf` has CRLF on disk, and `--help` is
    // rendered with `\n` everywhere.
    let committed = normalize_newlines(&read(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/reference.md"),
    ));

    assert_eq!(
        generated, committed,
        "docs/reference.md no longer matches the `--help` texts; \
         run `make docs-reference` and commit the result"
    );
}

/// The active `SERVICEKIT_REGISTRATION_KEY:` line of a model overlay, which is
/// there only when the deployment has a registration key.
fn overlay_sends_the_key(dir: &Path) -> bool {
    read(&dir.join("compose.chapkit-ewars-model.yml"))
        .contains("\n      SERVICEKIT_REGISTRATION_KEY: ${SERVICEKIT_REGISTRATION_KEY:-}\n")
}

#[test]
fn init_without_the_flag_leaves_both_secrets_commented() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains("API token:").not());

    let env = sandbox.env();
    assert!(env.contains("\n# CHAP_API_TOKEN=\n"), "{env}");
    assert!(env.contains("\n# SERVICEKIT_REGISTRATION_KEY=\n"), "{env}");
    assert_eq!(env_value(&env, "CHAP_API_TOKEN"), None);
    assert_eq!(env_value(&env, "SERVICEKIT_REGISTRATION_KEY"), None);

    // Nothing is protected, and the overlay ships the line commented out, the
    // way chap-core's own example does.
    let state = state(&dir);
    assert_eq!(state["auth"]["api_token"], false);
    assert_eq!(state["auth"]["registration_key"], false);
    assert!(!overlay_sends_the_key(&dir));
    assert!(
        read(&dir.join("compose.chapkit-ewars-model.yml"))
            .contains("      # SERVICEKIT_REGISTRATION_KEY:")
    );
}

#[test]
fn init_with_an_api_token_writes_both_secrets_and_the_overlay_line() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    let out = sandbox
        .init(&["--models", "chapkit_ewars_model", "--api-token"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).expect("utf-8 stdout");

    let env = sandbox.env();
    let token = env_value(&env, "CHAP_API_TOKEN").expect("an active token line");
    let key = env_value(&env, "SERVICEKIT_REGISTRATION_KEY").expect("an active key line");
    assert!(is_generated_secret(token), "{token}");
    assert!(is_generated_secret(key), "{key}");
    assert_ne!(token, key, "two independent secrets");
    // The placeholders are gone, not duplicated below the active lines.
    assert!(!env.contains("# CHAP_API_TOKEN="), "{env}");
    assert!(!env.contains("# SERVICEKIT_REGISTRATION_KEY="), "{env}");

    // The summary shows the token masked and says where to get it in full.
    assert!(
        stdout.contains(&format!(
            "API token: {}... (chaps auth show --reveal prints it)",
            &token[..6]
        )),
        "{stdout}"
    );
    assert!(
        !stdout.contains(token),
        "the summary leaked the token:\n{stdout}"
    );

    // `.chaps/` records booleans and no secret at all.
    let state = state(&dir);
    assert_eq!(state["auth"]["api_token"], true);
    assert_eq!(state["auth"]["registration_key"], true);
    let recorded = read(&dir.join(".chaps/project.yaml"));
    assert!(!recorded.contains(token), "the state file holds a secret");
    assert!(!recorded.contains(key), "the state file holds a secret");

    // Every model overlay now hands the key to its service.
    assert!(overlay_sends_the_key(&dir));
    assert!(
        !read(&dir.join("compose.chapkit-ewars-model.yml")).contains(key),
        "the overlay must substitute the value from .env, not inline it"
    );

    // And a sync right after init has nothing left to do.
    chap_in(&sandbox, &dir, &["sync", "--check"])
        .assert()
        .success();
}

#[test]
fn an_explicit_api_token_is_used_verbatim_and_a_short_one_warns() {
    let sandbox = Sandbox::new();
    let given = "mysecrettoken-that-is-long-enough-32ch";
    sandbox
        .init(&["--models", "none", "--api-token", given])
        .assert()
        .success()
        .stderr(predicates::str::contains("chap-core warns below").not());

    let env = sandbox.env();
    assert_eq!(env_value(&env, "CHAP_API_TOKEN"), Some(given));
    // The registration key is generated either way: nobody types two secrets.
    let key = env_value(&env, "SERVICEKIT_REGISTRATION_KEY").expect("a key");
    assert!(is_generated_secret(key), "{key}");

    // A token chap-core would flag as weak is accepted with a warning.
    let short = Sandbox::new();
    short
        .init(&["--models", "none", "--api-token", "sekret"])
        .assert()
        .success()
        .stderr(predicates::str::contains("the API token is 6 characters"))
        .stderr(predicates::str::contains("below 32"));
    assert_eq!(
        env_value(&short.env(), "CHAP_API_TOKEN"),
        Some("sekret"),
        "the value is still used verbatim"
    );
}

#[test]
fn an_api_token_needs_an_env_file_to_live_in() {
    let sandbox = Sandbox::new();
    // clap rejects the pair outright: there would be nowhere to put it.
    sandbox
        .init(&["--models", "none", "--api-token", "--no-env"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot be used with"));

    // And a .env this run keeps is the operator's, so the flag says so rather
    // than silently doing nothing.
    sandbox.init(&["--models", "none"]).assert().success();
    let before = sandbox.env();
    sandbox
        .init(&["--models", "none", "--force", "--api-token"])
        .assert()
        .success()
        .stderr(predicates::str::contains("chaps auth enable"));
    assert_eq!(sandbox.env(), before, "a kept .env is never rewritten");
    assert_eq!(state(&sandbox.project())["auth"]["api_token"], false);
}

#[test]
fn auth_show_masks_the_token_until_reveal_asks_for_it() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "none"]).assert().success();

    // Off to begin with.
    sandbox
        .auth(&["show"])
        .assert()
        .success()
        .stdout(predicates::str::contains("API authentication  off"))
        .stdout(predicates::str::contains("nothing protects this API"));

    sandbox.auth(&["enable"]).assert().success();
    let token = env_value(&sandbox.env(), "CHAP_API_TOKEN")
        .expect("a token")
        .to_string();

    let masked = sandbox
        .auth(&["show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let masked = String::from_utf8(masked).expect("utf-8");
    assert!(masked.contains("API authentication  on"), "{masked}");
    assert!(masked.contains(&format!("{}...", &token[..6])), "{masked}");
    assert!(!masked.contains(&token), "the token leaked:\n{masked}");

    sandbox
        .auth(&["show", "--reveal"])
        .assert()
        .success()
        .stdout(predicates::str::contains(token.clone()))
        .stdout(predicates::str::contains(
            "paste this token in the Modeling App's CHAP settings",
        ));

    // `--json` follows the same rule: the secret only appears with --reveal.
    let out = sandbox
        .auth(&["show", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("auth show --json is JSON");
    assert_eq!(value["api_token"], true);
    assert_eq!(value["registration_key"], true);
    assert_eq!(value["token"], Json::Null);
    assert_eq!(value["token_masked"], format!("{}...", &token[..6]));
    assert!(!String::from_utf8_lossy(&out).contains(&token));

    let out = sandbox
        .auth(&["show", "--reveal", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("auth show --json is JSON");
    assert_eq!(value["token"], token);
}

#[test]
fn auth_enable_protects_a_project_that_was_created_without_a_token() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let before = sandbox.env();
    assert!(!overlay_sends_the_key(&dir));

    sandbox
        .auth(&["enable"])
        .assert()
        .success()
        .stdout(predicates::str::contains("API authentication is on"))
        .stdout(predicates::str::contains(
            "run `chaps up` to restart chap-core and the models with authentication",
        ))
        .stdout(predicates::str::contains(
            "paste this token in the Modeling App's CHAP settings",
        ))
        .stdout(predicates::str::contains(
            "written  compose.chapkit-ewars-model.yml",
        ));

    let after = sandbox.env();
    let token = env_value(&after, "CHAP_API_TOKEN").expect("a token");
    let key = env_value(&after, "SERVICEKIT_REGISTRATION_KEY").expect("a key");
    assert!(is_generated_secret(token) && is_generated_secret(key));
    // The placeholders were uncommented in place: nothing else moved.
    assert_eq!(
        env_without(&after, &["CHAP_API_TOKEN", "SERVICEKIT_REGISTRATION_KEY"]),
        env_without(&before, &["CHAP_API_TOKEN", "SERVICEKIT_REGISTRATION_KEY"]),
    );

    assert_eq!(state(&dir)["auth"]["api_token"], true);
    assert!(overlay_sends_the_key(&dir));
    chap_in(&sandbox, &dir, &["sync", "--check"])
        .assert()
        .success();

    // Enabling again says so rather than replacing a working secret.
    sandbox
        .auth(&["enable"])
        .assert()
        .success()
        .stdout(predicates::str::contains("already on"))
        .stdout(predicates::str::contains("chaps auth rotate"));
    assert_eq!(env_value(&sandbox.env(), "CHAP_API_TOKEN"), Some(token));

    // An explicit token is taken at the point authentication is turned on.
    let explicit = Sandbox::new();
    explicit.init(&["--models", "none"]).assert().success();
    explicit
        .auth(&["enable", "--token", "an-explicit-token-of-ample-length"])
        .assert()
        .success();
    assert_eq!(
        env_value(&explicit.env(), "CHAP_API_TOKEN"),
        Some("an-explicit-token-of-ample-length")
    );
}

#[test]
fn auth_disable_keeps_the_values_so_enable_recovers_them() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model", "--api-token"])
        .assert()
        .success();
    let protected = sandbox.env();
    let token = env_value(&protected, "CHAP_API_TOKEN").unwrap().to_string();
    let key = env_value(&protected, "SERVICEKIT_REGISTRATION_KEY")
        .unwrap()
        .to_string();

    sandbox
        .auth(&["disable"])
        .assert()
        .success()
        .stdout(predicates::str::contains("API authentication is off"))
        .stdout(predicates::str::contains("kept as comments"))
        .stdout(predicates::str::contains("run `chaps up`"));

    let off = sandbox.env();
    assert_eq!(env_value(&off, "CHAP_API_TOKEN"), None);
    assert_eq!(env_value(&off, "SERVICEKIT_REGISTRATION_KEY"), None);
    // The values are still there, behind a `#`.
    assert!(off.contains(&format!("# CHAP_API_TOKEN={token}")), "{off}");
    assert!(
        off.contains(&format!("# SERVICEKIT_REGISTRATION_KEY={key}")),
        "{off}"
    );
    assert_eq!(state(&dir)["auth"]["api_token"], false);
    assert!(!overlay_sends_the_key(&dir));

    // Disabling twice changes nothing more.
    sandbox
        .auth(&["disable"])
        .assert()
        .success()
        .stdout(predicates::str::contains("already off"));
    assert_eq!(sandbox.env(), off);

    // And enabling again picks the old secrets back up, so clients that were
    // never reconfigured keep working.
    sandbox.auth(&["enable"]).assert().success();
    let again = sandbox.env();
    assert_eq!(env_value(&again, "CHAP_API_TOKEN"), Some(token.as_str()));
    assert_eq!(
        env_value(&again, "SERVICEKIT_REGISTRATION_KEY"),
        Some(key.as_str())
    );
    assert_eq!(again, protected, "the round trip is byte for byte");
    assert!(overlay_sends_the_key(&dir));
    assert_eq!(state(&dir)["auth"]["registration_key"], true);
}

#[test]
fn auth_rotate_replaces_both_secrets_and_leaves_the_rest_of_env_alone() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model", "--api-token"])
        .assert()
        .success();
    let before = sandbox.env();
    let old_token = env_value(&before, "CHAP_API_TOKEN").unwrap().to_string();
    let old_key = env_value(&before, "SERVICEKIT_REGISTRATION_KEY")
        .unwrap()
        .to_string();
    let password = password_line(&before).to_string();

    sandbox
        .auth(&["rotate"])
        .assert()
        .success()
        .stdout(predicates::str::contains("API authentication rotated"))
        .stdout(predicates::str::contains(
            "run `chaps up` to restart chap-core and the models with authentication",
        ))
        .stdout(predicates::str::contains(
            "every client keeps sending the old token until it is updated",
        ));

    let after = sandbox.env();
    let token = env_value(&after, "CHAP_API_TOKEN").unwrap();
    let key = env_value(&after, "SERVICEKIT_REGISTRATION_KEY").unwrap();
    assert_ne!(token, old_token, "the token did not move");
    assert_ne!(key, old_key, "the registration key did not move");
    assert!(is_generated_secret(token) && is_generated_secret(key));

    // Only those two lines changed: the database password, the pins, the
    // comments and the blank lines are all exactly as they were.
    assert_eq!(
        env_without(&after, &["CHAP_API_TOKEN", "SERVICEKIT_REGISTRATION_KEY"]),
        env_without(&before, &["CHAP_API_TOKEN", "SERVICEKIT_REGISTRATION_KEY"]),
    );
    assert_eq!(password_line(&after), password);
    assert_eq!(after.lines().count(), before.lines().count());
    assert!(
        !after.contains(&old_token),
        "the old token is still in the file"
    );

    // Rotating an unprotected project turns authentication on.
    let fresh = Sandbox::new();
    fresh.init(&["--models", "none"]).assert().success();
    fresh.auth(&["rotate"]).assert().success();
    assert_eq!(state(&fresh.project())["auth"]["api_token"], true);

    // The compose files stay in sync with the new state.
    chap_in(&sandbox, &dir, &["sync", "--check"])
        .assert()
        .success();
}

#[test]
fn auth_outside_a_project_says_so() {
    let sandbox = Sandbox::new();
    for argv in [
        ["auth", "show"].as_slice(),
        ["auth", "enable"].as_slice(),
        ["auth", "disable"].as_slice(),
        ["auth", "rotate"].as_slice(),
    ] {
        chap_in(&sandbox, sandbox.home.path(), argv)
            .assert()
            .failure()
            .stderr(predicates::str::contains("not a chaps project"));
    }

    // A project written with --no-env has no file to keep a secret in, and
    // says which flag made it that way.
    sandbox
        .init(&["--models", "none", "--no-env"])
        .assert()
        .success();
    sandbox
        .auth(&["enable"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--no-env"));
}

#[test]
fn no_color_is_accepted_everywhere_and_changes_nothing_off_a_terminal() {
    let sandbox = Sandbox::new();
    let plain = sandbox
        .chap()
        .args(["models", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let asked = sandbox
        .chap()
        .args(["--no-color", "models", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(plain).expect("utf-8"),
        String::from_utf8(asked).expect("utf-8"),
        "piped output is plain either way"
    );

    // It is global, so it also attaches after the subcommand.
    sandbox
        .chap()
        .args(["models", "list", "--no-color"])
        .assert()
        .success();
}

#[test]
fn a_chap_core_command_typed_at_chaps_names_the_other_cli() {
    let sandbox = Sandbox::new();
    sandbox
        .chap()
        .arg("serve")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("is not a chaps command"))
        .stderr(predicates::str::contains("chap-core developer CLI"))
        .stderr(predicates::str::contains("`chap`"))
        .stderr(predicates::str::contains("chaps --help"));

    // The same for the other commands chap-core publishes.
    for name in ["evaluate", "forecast", "predict", "harmonize"] {
        sandbox
            .chap()
            .arg(name)
            .assert()
            .code(2)
            .stderr(predicates::str::contains("chap-core developer CLI"));
    }

    // Anything that is simply not a command stays clap's business.
    sandbox
        .chap()
        .arg("frobnicate")
        .assert()
        .failure()
        .stderr(predicates::str::contains("chap-core developer CLI").not());
}

#[test]
fn verbose_narrates_on_stderr_and_leaves_stdout_alone() {
    let sandbox = Sandbox::new();
    let quiet = sandbox
        .chap()
        .args(["models", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let loud = sandbox
        .chap()
        .args(["-v", "models", "list"])
        .assert()
        .success()
        .get_output()
        .clone();

    assert_eq!(
        String::from_utf8(quiet).expect("utf-8"),
        String::from_utf8(loud.stdout).expect("utf-8"),
        "-v must never reach stdout"
    );
    let stderr = String::from_utf8(loud.stderr).expect("utf-8 stderr");
    assert!(
        stderr.contains("registry:"),
        "-v says which catalogue was used: {stderr}"
    );
    assert!(
        stderr.contains("embedded") || stderr.contains("cache"),
        "-v names the source it chose: {stderr}"
    );
}

#[test]
fn debug_implies_verbose_and_adds_the_resolved_project() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "none"]).assert().success();

    let out = sandbox
        .chap()
        .arg("-d")
        .arg("-C")
        .arg(sandbox.project())
        .args(["sync", "--check"])
        .assert()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8(out).expect("utf-8 stderr");
    // -v's half: the files the sync compared.
    assert!(stderr.contains("compared"), "{stderr}");
    // -d's own half: where the state it read actually lives.
    assert!(stderr.contains("project:"), "{stderr}");
    let state_file = Path::new(".chaps").join("project.yaml");
    assert!(
        stderr.contains(&state_file.display().to_string()),
        "{stderr}"
    );

    // The long spellings do the same.
    sandbox
        .chap()
        .arg("--debug")
        .arg("-C")
        .arg(sandbox.project())
        .args(["sync", "--check"])
        .assert()
        .success()
        .stderr(predicates::str::contains("compared"));
}

// ---------------------------------------------------------------------------
// `chaps self` and `chaps completions`
//
// The two commands that are about the CLI rather than about a deployment, so
// they work outside a project and are listed everywhere.
// ---------------------------------------------------------------------------

/// A `chaps` run that needs no project: outside one, with its own cache and no
/// update check, so nothing here can reach the network.
fn bare() -> (TempDir, Command) {
    let cache = tempfile::tempdir().unwrap();
    let mut cmd = Command::cargo_bin("chaps").expect("the chaps binary is built");
    cmd.env("CHAPS_CACHE_DIR", cache.path())
        .env("CHAPS_NO_UPDATE_CHECK", "1")
        .current_dir(cache.path());
    (cache, cmd)
}

#[test]
fn self_version_says_what_this_build_is() {
    let (_cache, mut cmd) = bare();
    let out = cmd
        .args(["self", "version"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).expect("utf-8");

    assert!(
        text.contains(env!("CARGO_PKG_VERSION")),
        "the version is in there: {text}"
    );
    assert!(text.contains("target"), "{text}");
    assert!(text.contains("path"), "{text}");
    assert!(text.contains("installed by"), "{text}");
}

#[test]
fn self_version_json_carries_the_fields_a_bug_report_needs() {
    let (_cache, mut cmd) = bare();
    let out = cmd
        .args(["--json", "self", "version"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("--json is JSON");

    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        !value["target"]
            .as_str()
            .expect("a target triple")
            .is_empty(),
        "{value}"
    );
    assert!(value["path"].as_str().is_some(), "{value}");
    assert!(
        ["release archive", "cargo install"]
            .contains(&value["install_method"].as_str().expect("a method")),
        "{value}"
    );
}

/// The whole point of `--offline` is that nothing touches the network, and an
/// update is a download, so the two cannot be combined. This is also what
/// keeps the test suite off the network: nothing else in it runs `self
/// update`.
#[test]
fn self_update_refuses_to_run_offline() {
    let (_cache, mut cmd) = bare();
    cmd.args(["--offline", "self", "update", "--check"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--offline"));
}

#[test]
fn self_works_outside_a_project_and_says_what_it_offers() {
    let (_cache, mut cmd) = bare();
    let out = cmd
        .args(["self", "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let help = String::from_utf8(out).expect("utf-8");
    assert!(help.contains("update"), "{help}");
    assert!(help.contains("version"), "{help}");

    // And `chaps self` on its own is a usage error, not a no-op.
    let (_cache, mut cmd) = bare();
    cmd.arg("self").assert().failure();
}

#[test]
fn completions_are_printed_for_every_shell() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let (_cache, mut cmd) = bare();
        let out = cmd
            .args(["completions", shell])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let script = String::from_utf8(out).unwrap_or_else(|e| panic!("{shell}: {e}"));

        assert!(!script.trim().is_empty(), "{shell} printed nothing");
        assert!(script.contains("chaps"), "{shell} does not name chaps");
        // Generated from the live tree, so the newest commands are in there.
        assert!(script.contains("self"), "{shell} misses `self`");
        assert!(script.contains("init"), "{shell} misses `init`");
    }
}

#[test]
fn completions_rejects_a_shell_it_cannot_write_for() {
    let (_cache, mut cmd) = bare();
    cmd.args(["completions", "tcsh"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("possible values"));
}

/// Neither command needs a deployment directory, and both are listed in the
/// help you get outside one.
#[test]
fn the_help_outside_a_project_offers_self_and_completions() {
    let (_cache, mut cmd) = bare();
    let out = cmd
        .arg("--help")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let help = String::from_utf8(out).expect("utf-8");
    assert!(help.contains("self"), "{help}");
    assert!(help.contains("completions"), "{help}");
}

/// The update notice is a courtesy on a terminal. The test suite is not one,
/// and `assert_cmd` captures stdout, so no command in it may reach the
/// release feed or write the check file.
#[test]
fn a_captured_run_never_checks_for_an_update() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "none"]).assert().success();
    sandbox.chap().args(["models", "list"]).assert().success();

    let check = sandbox.cache.path().join("self-update-check.json");
    assert!(
        !check.exists(),
        "a piped run wrote {}; the notice must be terminal-only",
        check.display()
    );
}

/// The one check of a `doctor --json` report with this id.
fn doctor_check<'a>(report: &'a Json, id: &str) -> &'a Json {
    report["checks"]
        .as_array()
        .unwrap_or_else(|| panic!("`checks` is not an array in {report}"))
        .iter()
        .find(|check| check["id"] == id)
        .unwrap_or_else(|| panic!("no check `{id}` in {report}"))
}

/// The status word of that check.
fn doctor_status<'a>(report: &'a Json, id: &str) -> &'a str {
    doctor_check(report, id)["status"]
        .as_str()
        .unwrap_or_else(|| panic!("`{id}` has no status"))
}

/// `doctor` is one of the commands that works anywhere, so outside a project
/// it runs the machine half and says where the rest would be.
///
/// The exit code depends on whether this machine has Docker, which a test
/// cannot assume either way: only the structure is asserted, and that the
/// command stayed inside the two codes it is allowed to use.
#[test]
fn doctor_outside_a_project_checks_the_machine_and_says_so() {
    let (_cache, mut cmd) = bare();
    let out = cmd
        .args(["--offline", "doctor"])
        .output()
        .expect("doctor runs");
    assert!(
        matches!(out.status.code(), Some(0) | Some(1)),
        "doctor exited with {:?}",
        out.status.code()
    );
    let text = String::from_utf8(out.stdout).expect("utf-8");

    for name in [
        "docker cli",
        "docker daemon",
        "docker compose",
        "os and arch",
        "disk space",
        "network ghcr.io",
        "chaps",
    ] {
        assert!(text.contains(name), "`{name}` is missing from:\n{text}");
    }
    assert!(
        text.contains("project: none here (run chaps doctor inside a deployment directory"),
        "{text}"
    );
    // Nothing that needs a deployment ran.
    for name in ["project files", "api port", "chap-core pin"] {
        assert!(!text.contains(name), "`{name}` needs a project:\n{text}");
    }
    let summary = regex::Regex::new(r"(?m)^\d+ checks: \d+ ok, \d+ warn, \d+ fail").unwrap();
    assert!(summary.is_match(&text), "no summary line in:\n{text}");
}

/// `doctor --json` is the same checklist as one document.
#[test]
fn doctor_json_is_a_list_of_checks_and_a_summary() {
    let (_cache, mut cmd) = bare();
    let out = cmd
        .args(["--offline", "--json", "doctor"])
        .output()
        .expect("doctor runs");
    let report: Json = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("doctor --json did not parse: {e}"));

    let checks = report["checks"].as_array().expect("an array of checks");
    assert!(!checks.is_empty());
    for check in checks {
        for field in ["id", "name", "status", "detail", "fix"] {
            assert!(check.get(field).is_some(), "{check} has no `{field}`");
        }
        assert!(
            ["ok", "warn", "fail", "skip"].contains(&check["status"].as_str().unwrap()),
            "{check} has an unknown status"
        );
    }
    let summary = &report["summary"];
    let count = |key: &str| summary[key].as_u64().unwrap_or_else(|| panic!("no {key}"));
    assert_eq!(
        (count("ok") + count("warn") + count("fail") + count("skip")) as usize,
        checks.len(),
        "the summary has to add up to the checks: {report}"
    );
}

/// Inside a freshly written deployment the two checks that are about the
/// files alone must both pass, and `--offline` must keep every probe that
/// would touch the network out of the run.
#[test]
fn doctor_in_a_fresh_project_finds_the_files_in_order_and_skips_the_network() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "default"]).assert().success();

    let out = sandbox
        .chap()
        .arg("-C")
        .arg(sandbox.project())
        .args(["--json", "doctor"])
        .output()
        .expect("doctor runs");
    let report: Json = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("doctor --json did not parse: {e}"));

    assert_eq!(doctor_status(&report, "project-files"), "ok", "{report}");
    assert_eq!(doctor_status(&report, "sync"), "ok", "{report}");
    // `init` generates a password, writes the pin comments and leaves
    // authentication off, which is a complete `.env` and not a warning.
    assert_eq!(doctor_status(&report, "env"), "ok", "{report}");
    // Nothing has ever been started here.
    assert_eq!(doctor_status(&report, "stack"), "skip", "{report}");
    assert!(
        doctor_check(&report, "stack")["fix"]
            .as_str()
            .unwrap()
            .contains("chaps up"),
        "{report}"
    );

    // Every network probe, and the image check that would follow one.
    for id in ["net-ghcr", "net-marketplace", "net-releases"] {
        assert_eq!(doctor_status(&report, id), "skip", "{report}");
        assert!(
            doctor_check(&report, id)["detail"]
                .as_str()
                .unwrap()
                .contains("offline"),
            "{id} does not say why it was skipped: {report}"
        );
    }
    let image = doctor_check(&report, "image-chapkit-ewars-model");
    assert_eq!(image["status"], "skip", "{report}");
    assert!(image["detail"].as_str().unwrap().contains("offline"));

    // A moving chap-core tag is nothing to compare against a release list.
    assert_eq!(doctor_status(&report, "chap-core-pin"), "ok", "{report}");
}
