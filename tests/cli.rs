//! End-to-end tests for the commands that write a deployment directory.
//!
//! Every run is `--offline` with its own cache directory, so the embedded
//! marketplace snapshot is what the CLI sees and no test touches the network
//! or the developer's real cache.

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use serde_json::Value as Json;
use serde_yaml_ng::Value as Yaml;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU16, Ordering};
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
            // An `--offline` enable reads the user off a locally pulled image
            // before it falls back to the table, and what this machine has
            // pulled is not something a test may depend on.
            .env("CHAPS_NO_DOCKER_PROBE", "1")
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

    /// `chaps -C <project> ...` with the network pointed at a local
    /// stand-in rather than switched off.
    ///
    /// `chaps models add` resolves a repository over HTTP, so it is the one
    /// command these tests cannot run with `--offline`. Everything it would
    /// reach - GitHub, ghcr and the marketplace index - is answered by
    /// [`Hub`] on `port`, and the docker probe is turned off because a `pull`
    /// aimed at the real registry is exactly what must not happen here.
    fn online(&self, port: u16) -> Command {
        let base = format!("http://127.0.0.1:{port}");
        let mut cmd = Command::cargo_bin("chaps").expect("the chaps binary is built");
        cmd.env("CHAPS_CACHE_DIR", self.cache.path())
            .env("CHAPS_NO_UPDATE_CHECK", "1")
            .env("CHAPS_GITHUB_API", &base)
            .env("CHAPS_GITHUB_RAW", &base)
            .env("CHAPS_GHCR_URL", &base)
            .env("CHAPS_NO_DOCKER_PROBE", "1")
            .current_dir(self.home.path())
            .arg("--registry-url")
            .arg(format!("{base}/registry.yaml"))
            .arg("-C")
            .arg(self.project());
        cmd
    }

    /// `chaps init <project> ...` with the same network, which is how a
    /// deployment gets a chap-core pin and the compose file that goes with it
    /// without touching the real GitHub.
    fn online_init(&self, port: u16, args: &[&str]) -> Command {
        let base = format!("http://127.0.0.1:{port}");
        let mut cmd = Command::cargo_bin("chaps").expect("the chaps binary is built");
        cmd.env("CHAPS_CACHE_DIR", self.cache.path())
            .env("CHAPS_NO_UPDATE_CHECK", "1")
            .env("CHAPS_GITHUB_API", &base)
            .env("CHAPS_GITHUB_RAW", &base)
            .env("CHAPS_GHCR_URL", &base)
            .env("CHAPS_NO_DOCKER_PROBE", "1")
            .current_dir(self.home.path())
            .arg("--registry-url")
            .arg(format!("{base}/registry.yaml"))
            .arg("init")
            .arg(self.project())
            .args(args);
        cmd
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

/// `.chaps/models-manual.yaml` as JSON, or `Json::Null` when the deployment
/// has added no model of its own.
fn manual_models(dir: &Path) -> Json {
    let path = dir.join(".chaps").join("models-manual.yaml");
    if !path.is_file() {
        return Json::Null;
    }
    serde_json::to_value(yaml(&path)).expect("models-manual.yaml maps to JSON")
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

/// The `--json` document one command printed, whatever it exited with:
/// `status` on a stack that is not running and `doctor` with a failing check
/// both print their report and exit non-zero.
fn json_of(cmd: &mut Command) -> Json {
    let out = cmd.output().expect("the command runs");
    serde_json::from_slice(&out.stdout).expect("the --json output is one document")
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

/// A `--port-base` no other test in this binary hands out, and nothing on this
/// machine is listening on.
///
/// `--port auto` walks upwards from the base and takes the first port nothing
/// holds, and the probe behind that is a real bind. Two tests walking from the
/// same base can therefore catch each other mid-probe and be told the port is
/// taken, so a test that asserts a port it did not choose takes a base of its
/// own. `--port N` probes too, so a test naming a fixed port picks it inside
/// its own slot.
fn port_base() -> u16 {
    /// The first base handed out. Inside the default range, so the top of it
    /// stays 5999, and clear of the ports a developer machine tends to run
    /// (5432 for postgres, 5900 for screen sharing).
    const FIRST: u16 = 5200;
    /// Room for a test to name a fixed port or two above its base.
    const STRIDE: u16 = 10;

    static NEXT: AtomicU16 = AtomicU16::new(FIRST);
    loop {
        let base = NEXT.fetch_add(STRIDE, Ordering::Relaxed);
        assert!(base < 5990, "this test binary ran out of port bases");
        if is_free(base) {
            return base;
        }
    }
}

/// Whether `port` can be bound on both the addresses the CLI's own probe
/// tries first, which is what it takes for the CLI to call the port free.
fn is_free(port: u16) -> bool {
    [Ipv4Addr::UNSPECIFIED, Ipv4Addr::LOCALHOST]
        .into_iter()
        .all(|addr| std::net::TcpListener::bind((addr, port)).is_ok())
}

/// A port the kernel handed out, and that nothing is listening on.
///
/// A test that names a fixed port - 8000 for the API, 9000 for OCS - is asking
/// about whatever the developer happens to be running, not about the
/// deployment it wrote. The listener here is bound only long enough to learn
/// which port is free and is dropped before the number is handed out, so a
/// probe aimed at it finds nothing there. Ports already handed out are
/// remembered, so two calls in one test never name the same one.
fn free_port() -> u16 {
    static TAKEN: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());
    loop {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a free port");
        let port = listener.local_addr().expect("a local address").port();
        drop(listener);
        let mut taken = TAKEN.lock().expect("the port list outlives its panics");
        if !taken.contains(&port) {
            taken.push(port);
            return port;
        }
    }
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

/// The version and image tag the vendored snapshot's `stable` channel pins
/// for one marketplace id.
///
/// Read off `vendor/marketplace/` rather than written out here, so refreshing
/// the snapshot moves these assertions with it instead of failing them. The
/// tests run `--offline`, which is exactly what the CLI resolves against.
fn stable_pin(id: &str) -> (String, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("vendor")
        .join("marketplace")
        .join("models")
        .join(format!("{id}.yaml"));
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let doc: Yaml = serde_yaml_ng::from_str(&body).expect("the vendored model file parses");
    let version = doc["channels"]["stable"]
        .as_str()
        .unwrap_or_else(|| panic!("{id} has no stable channel"))
        .to_string();
    let tag = doc["versions"]
        .as_sequence()
        .unwrap_or_else(|| panic!("{id} lists no versions"))
        .iter()
        .find(|v| v["version"].as_str() == Some(version.as_str()))
        .unwrap_or_else(|| panic!("{id} has no version {version}"))["image_tag"]
        .as_str()
        .unwrap_or_else(|| panic!("{id} {version} has no image tag"))
        .to_string();
    (version, tag)
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
    // Both state files open with the one line that says who manages them.
    const MANAGED: &str = "# Managed by chaps; change it with the chaps commands, not by hand.\n";
    assert!(read(&dir.join(".chaps/project.yaml")).starts_with(MANAGED));
    assert!(read(&dir.join(".chaps/models.yaml")).starts_with(MANAGED));

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
    assert_eq!(model["image_tag"], stable_pin("chapkit_ewars_model").1);
    assert_eq!(model["data_dir"], "/app/data");
    // `--offline` with no local image: the built-in table, whose `chapkit` is
    // recorded as the numbers the overlay renders.
    assert_eq!(model["user"], "1000:1000");
    assert_eq!(model["user_from"], "table");
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
    assert!(env.contains(&format!(
        "# CHAPKIT_EWARS_MODEL_IMAGE_TAG={}",
        stable_pin("chapkit_ewars_model").1
    )));
    assert!(env.contains("# CHAP_IMAGE_TAG=latest"));
    // Active even at the default, so the port is where you would look for it.
    assert!(env.contains("\nCHAP_API_PORT=8000\n"), "{env}");
}

/// The shape of a generated compose project name: a slug of the directory
/// name, then six hex characters of its own.
///
/// Compose's own rule as well: lowercase, `[a-z0-9-]`, and it starts with a
/// letter or a digit.
const PROJECT_NAME: &str = "^[a-z0-9][a-z0-9-]*-[0-9a-f]{6}$";

/// The `name:` of a rendered compose file, which is the compose project name.
fn compose_name(path: &Path) -> String {
    yaml(path)["name"]
        .as_str()
        .unwrap_or_else(|| panic!("{} has no name: key", path.display()))
        .to_string()
}

#[test]
fn init_gives_the_deployment_a_compose_project_name_of_its_own() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    // `status` below really calls the API port, so this deployment gets one
    // nothing on this machine answers on rather than the default 8000.
    let api_port = free_port();
    sandbox
        .init(&["--models", "none", "--api-port", &api_port.to_string()])
        .assert()
        .success();

    // Without it compose would name the project after the directory, and two
    // deployments in directories both called `chapx` would share every named
    // volume - including the database.
    let name = state(&dir)["compose_project"]
        .as_str()
        .expect("init records a compose project name")
        .to_string();
    let shape = regex::Regex::new(PROJECT_NAME).unwrap();
    assert!(shape.is_match(&name), "unexpected project name {name:?}");
    assert!(name.starts_with("chapx-"), "{name}");

    // Both chaps-owned files carry it: the umbrella is the one always in the
    // `-f` list, and compose takes the `name:` of the last file that sets one.
    assert_eq!(compose_name(&dir.join("compose.chaps.yml")), name);
    assert_eq!(compose_name(&dir.join("compose.marketplace.yml")), name);

    // A second deployment of the same directory name gets a different one.
    let other = Sandbox::new();
    other.init(&["--models", "none"]).assert().success();
    let second = state(&other.project())["compose_project"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(shape.is_match(&second), "{second}");
    assert_ne!(name, second, "two `chapx` directories, two project names");

    // `chaps status --json` and `chaps doctor` both report it.
    let out = sandbox
        .chap()
        .arg("-C")
        .arg(&dir)
        .args(["status", "--json", "--timeout", "1"])
        .assert()
        .get_output()
        .stdout
        .clone();
    let report: Json = serde_json::from_slice(&out).expect("status --json is one document");
    assert_eq!(report["project"], Json::String(name.clone()));

    let out = sandbox
        .chap()
        .arg("-C")
        .arg(&dir)
        .args(["--json", "doctor"])
        .output()
        .expect("doctor runs");
    let report: Json = serde_json::from_slice(&out.stdout).expect("doctor --json parses");
    assert_eq!(doctor_status(&report, "compose-project"), "ok", "{report}");
    assert!(
        doctor_check(&report, "compose-project")["detail"]
            .as_str()
            .unwrap()
            .starts_with(&name),
        "{report}"
    );
}

#[test]
fn force_keeps_the_compose_project_name_it_found() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();
    let name = state(&dir)["compose_project"].as_str().unwrap().to_string();

    // A new name would leave the running deployment's containers and volumes
    // behind under the old one, which is the trap this whole thing avoids.
    sandbox
        .init(&["--models", "none", "--force"])
        .assert()
        .success();
    assert_eq!(state(&dir)["compose_project"].as_str(), Some(name.as_str()));
    assert_eq!(compose_name(&dir.join("compose.chaps.yml")), name);
}

#[test]
fn a_project_written_before_the_name_existed_records_the_directory_name() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    // What an older chaps wrote: no compose_project at all. Compose was
    // naming that deployment after its directory, so that is the name its
    // containers and volumes already carry.
    let project_yaml = dir.join(".chaps").join("project.yaml");
    let body = read(&project_yaml);
    let without: String = body
        .lines()
        .filter(|line| !line.starts_with("compose_project:"))
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&project_yaml, &without).unwrap();
    assert!(!read(&project_yaml).contains("compose_project:"));

    sandbox
        .chap()
        .arg("-C")
        .arg(&dir)
        .arg("sync")
        .assert()
        .success();

    // The directory name, with no suffix: nothing is renamed, and it is now
    // written down rather than derived.
    assert_eq!(state(&dir)["compose_project"].as_str(), Some("chapx"));
    assert_eq!(compose_name(&dir.join("compose.chaps.yml")), "chapx");
    assert_eq!(compose_name(&dir.join("compose.marketplace.yml")), "chapx");

    // And a second sync has nothing left to do.
    sandbox
        .chap()
        .arg("-C")
        .arg(&dir)
        .args(["sync", "--check"])
        .assert()
        .success();
}

/// The smallest stack the probe below can ask about: one service that is
/// never started. `ps` only reads, so the image is never pulled.
const PROBE_STACK: &str = "services:\n  probe:\n    image: alpine:3\n";

/// Whether the docker-backed tests can run here, saying why when they cannot.
///
/// `docker compose version` is not the question these tests need answered. A
/// runner can have the CLI while its daemon is unusable - not running, or in
/// Windows-containers mode - and then the call every wrapper makes,
/// `docker compose ps -a --format json`, exits 1 while `version` still
/// answers happily. That is a flaky failure, not a finding, so the probe is
/// that exact call against a stack of its own, and the tests that depend on
/// docker answering skip when it does not.
///
/// The probe runs once per test binary: the cached verdict is what every
/// caller after the first reads.
fn docker_ready() -> bool {
    static PROBE: OnceLock<Result<(), String>> = OnceLock::new();
    match PROBE.get_or_init(probe_docker) {
        Ok(()) => true,
        Err(reason) => {
            eprintln!("skipping: {reason}");
            false
        }
    }
}

/// Ask docker the question the tests depend on, in a directory of its own.
fn probe_docker() -> Result<(), String> {
    let temp = tempfile::tempdir().map_err(|e| format!("no directory to probe docker in: {e}"))?;
    let dir = temp.path().join("chaps-docker-probe");
    let write = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join("compose.yml"), PROBE_STACK));
    write.map_err(|e| format!("the docker probe stack could not be written: {e}"))?;

    let out = std::process::Command::new("docker")
        .args([
            "compose",
            "-f",
            "compose.yml",
            "ps",
            "-a",
            "--format",
            "json",
        ])
        .current_dir(&dir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("docker could not be run: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let first = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("it said nothing about why");
    Err(format!(
        "`docker compose ps -a --format json` exited with status {}: {first}",
        out.status.code().unwrap_or(-1)
    ))
}

#[test]
fn docker_accepts_the_stack_with_the_chaps_override() {
    if !docker_ready() {
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
    // The project name compose resolves to is this deployment's own, from the
    // `name:` in the second `-f` file rather than from the directory.
    let name = state(&dir)["compose_project"].as_str().unwrap().to_string();
    assert_eq!(
        merged["name"].as_str(),
        Some(name.as_str()),
        "compose takes the name: of a file that is not the first -f"
    );
    // And chap-core is handed the registration key: upstream's compose.ghcr.yml
    // passes only CHAP_API_TOKEN, so without this the API answers every model
    // registration with 401.
    assert_eq!(
        merged["services"]["chap"]["environment"]["SERVICEKIT_REGISTRATION_KEY"].as_str(),
        Some(""),
        "the variable reaches the container, empty while .env sets none"
    );
    assert!(
        merged["services"]["chap"]["environment"]
            .get("CHAP_API_TOKEN")
            .is_some(),
        "adding to environment must not replace upstream's own variables"
    );
    // And gunicorn's control socket lands on the tmpfs rather than on the
    // read-only root, which it otherwise logs an error about on every start.
    assert_eq!(
        merged["services"]["chap"]["environment"]["XDG_RUNTIME_DIR"].as_str(),
        Some("/tmp")
    );
    assert!(
        merged["services"]["worker"]["environment"]
            .get("XDG_RUNTIME_DIR")
            .is_none(),
        "the worker runs celery, not gunicorn"
    );
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
    assert!(read(&dir.join(".env")).contains(&format!(
        "# AUTO_ARIMA_CHAPKIT_IMAGE_TAG={}",
        stable_pin("auto_arima_chapkit").1
    )));
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
    // The template's image is built from /work, unlike ewars, and it runs as
    // root - so its overlay hands it no user, and its init container chowns
    // the volume to root rather than to an account the image does not use.
    let model = &state(&dir)["models"]["chapkit_minimalist_example_py"];
    assert_eq!(model["data_dir"], "/work/data");
    assert_eq!(model["user"], "root");
    let overlay = yaml(&dir.join("compose.chapkit-minimalist-example-py.yml"));
    assert!(
        overlay["services"]["chapkit-minimalist-example-py"]
            .get("user")
            .is_none()
    );
    assert_eq!(
        overlay["services"]["chapkit-minimalist-example-py-init"]["command"][2].as_str(),
        Some("chown -R 0:0 /work/data")
    );
}

#[test]
fn enable_takes_an_explicit_port_and_rejects_a_taken_one() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    let base = port_base();
    // A port of this test's own, above the base so `--port auto` below still
    // has the base itself to hand out.
    let fixed = base + 5;
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model",
            "--port-base",
            &base.to_string(),
        ])
        .assert()
        .success();

    sandbox
        .models(&["enable", "auto_arima_chapkit", "--port", &fixed.to_string()])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "http://localhost:{fixed}"
        )));
    assert_eq!(
        state(&dir)["models"]["auto_arima_chapkit"]["host_port"],
        fixed
    );
    let svc = &yaml(&dir.join("compose.auto-arima-chapkit.yml"))["services"]["auto-arima-chapkit"];
    let published = format!("{fixed}:8000");
    assert_eq!(svc["ports"][0].as_str(), Some(published.as_str()));
    assert_eq!(
        svc["expose"][0].as_str(),
        Some("8000"),
        "expose documents the container port either way"
    );

    sandbox
        .models(&[
            "enable",
            "chapkit_simple_multistep_model",
            "--port",
            &fixed.to_string(),
        ])
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
        base
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
    let base = port_base();
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model",
            "--port-base",
            &base.to_string(),
        ])
        .assert()
        .success();
    let pinned = read(&dir.join(".chaps/models.yaml"));

    sandbox
        .models(&["expose", "chapkit_ewars_model", "--port", "auto"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "exposed chapkit-ewars-model on http://localhost:{base}",
        )))
        .stdout(predicates::str::contains(
            "written  compose.chapkit-ewars-model.yml",
        ));
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        base
    );
    let svc =
        &yaml(&dir.join("compose.chapkit-ewars-model.yml"))["services"]["chapkit-ewars-model"];
    let published = format!("{base}:8000");
    assert_eq!(svc["ports"][0].as_str(), Some(published.as_str()));

    // Only the port moved: the version and the image pin are untouched.
    let after = read(&dir.join(".chaps/models.yaml"));
    assert_eq!(
        after.replace(&format!("host_port: {base}"), "host_port: null"),
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
fn models_list_and_info_name_the_proxy_until_a_port_is_published() {
    let sandbox = Sandbox::new();
    let base = port_base();
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model",
            "--port-base",
            &base.to_string(),
        ])
        .assert()
        .success();

    sandbox
        .models(&["list", "--enabled"])
        .assert()
        .success()
        .stdout(predicates::str::contains("via chap-core"));
    sandbox
        .models(&["info", "chapkit_ewars_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "reach     internal (proxy: http://localhost:8000/v2/services/chapkit-ewars-model/run/)",
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
    assert!(listed.contains(&base.to_string()), "{listed}");
    assert!(!listed.contains("via chap-core"), "{listed}");
    sandbox
        .models(&["info", "chapkit_ewars_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "reach     http://localhost:{base}"
        )));
}

#[test]
fn json_output_parses_for_init_and_enable() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    let base = port_base();
    let out = sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model",
            "--json",
            "--port-base",
            &base.to_string(),
        ])
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
    assert_eq!(value["enabled"][0][1]["host_port"], base);
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
    assert_eq!(value["previous"], base);
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
    let base = port_base();
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model",
            "--port-base",
            &base.to_string(),
        ])
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
        .init(&[
            "--models",
            "none",
            "--force",
            "--port-base",
            &base.to_string(),
        ])
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
        base
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
        .stderr(predicates::str::contains("chaps down --volumes"))
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
    assert!(after.contains(&format!(
        "# CHAPKIT_EWARS_MODEL_IMAGE_TAG={}",
        stable_pin("chapkit_ewars_model").1
    )));
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
    let base = port_base();
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model,auto_arima_chapkit",
            "--port-base",
            &base.to_string(),
        ])
        .assert()
        .success();
    sandbox
        .models(&["expose", "chapkit_ewars_model", "--port", &base.to_string()])
        .assert()
        .success();

    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model",
            "--force",
            "--port-base",
            &base.to_string(),
        ])
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
        .models(&["expose", "chapkit_ewars_model", "--port", &base.to_string()])
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
    // Nothing was cached, so nothing but the three state files is in .chaps/.
    let mut names: Vec<String> = std::fs::read_dir(dir.join(".chaps"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["components.yaml", "models.yaml", "project.yaml"]
    );
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
    assert!(
        base.starts_with("# Generated by chaps from .chaps/; edit there and run `chaps sync`.\n"),
        "{base}"
    );
    assert!(base.contains("# chap-core compose.ghcr.yml at v9.9.9\n"));
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
    let base = port_base();
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model",
            "--port-base",
            &base.to_string(),
        ])
        .assert()
        .success();
    assert_eq!(state(&dir)["port_range"], serde_json::json!([base, 5999]));

    // The range only matters once a port is asked for.
    sandbox
        .models(&["expose", "chapkit_ewars_model"])
        .assert()
        .success();
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        base
    );
}

#[test]
fn the_api_port_reaches_the_env_file_the_state_and_status() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    // A port of this test's own: `status` below probes it for real, so a
    // fixed number would be a question about the developer's machine.
    let port = free_port();
    sandbox
        .init(&["--models", "none", "--api-port", &port.to_string()])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "API:       http://localhost:{port}"
        )));

    assert_eq!(state(&dir)["api_port"], port);
    let env = read(&dir.join(".env"));
    assert!(env.contains(&format!("\nCHAP_API_PORT={port}\n")), "{env}");
    assert!(
        read(&dir.join("compose.chaps.yml")).contains(&format!("${{CHAP_API_PORT:-{port}}}:8000"))
    );

    // `status --url` defaults to the port the project records. The API is not
    // running, so the report is a down one - at the right URL.
    let mut status = sandbox.chap();
    status.arg("-C").arg(&dir).args(["status", "--json"]);
    let out = status.assert().failure().get_output().stdout.clone();
    let value: Json = serde_json::from_slice(&out).expect("status --json is JSON");
    assert_eq!(value["api_url"], format!("http://localhost:{port}"));
    assert_eq!(value["api"]["state"], "down");
}

#[test]
fn a_kept_env_file_that_pins_another_api_port_is_reported() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    // Two ports of this test's own: the last assertion is that nothing was
    // said about CHAP_API_PORT, and `init` warns about a port something else
    // on this machine is holding in those very words.
    let pinned = free_port();
    let moved = free_port();
    sandbox
        .init(&["--models", "none", "--api-port", &pinned.to_string()])
        .assert()
        .success();

    // `--force` keeps .env, and compose reads .env after the compose files, so
    // the line in it wins over the new --api-port. Say so.
    sandbox
        .init(&[
            "--models",
            "none",
            "--force",
            "--api-port",
            &moved.to_string(),
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("kept .env (already present)"))
        .stderr(predicates::str::contains(format!(
            "warning: .env already sets CHAP_API_PORT={pinned}"
        )))
        .stderr(predicates::str::contains(format!(
            "the API stays on {pinned}"
        )));
    // The recorded intent did move, and so did the rendered override.
    assert_eq!(state(&dir)["api_port"], moved);
    assert!(read(&dir.join("compose.chaps.yml")).contains(&format!("${{CHAP_API_PORT:-{moved}}}")));
    assert!(read(&dir.join(".env")).contains(&format!("\nCHAP_API_PORT={pinned}\n")));

    // Re-initialising at the port .env already holds says nothing.
    let out = sandbox
        .init(&[
            "--models",
            "none",
            "--force",
            "--api-port",
            &pinned.to_string(),
        ])
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

/// The way `chaps` spells a deployment directory in a warning: resolved, and
/// without the `\\?\` prefix Windows' `canonicalize` puts on one.
/// `src/ports.rs` takes it off the same way, and a binary crate has nothing an
/// integration test can import, so this mirrors it. String work, so it hands
/// back the canonical path unchanged on Unix.
fn resolved(path: &Path) -> String {
    let real = std::fs::canonicalize(path).expect("the directory exists");
    let text = real.display().to_string();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

/// The gap the live probe cannot see: `chaps init a && chaps init b` with
/// nothing running gives two deployments on one port, and nothing says so
/// until the second `chaps up`.
#[test]
fn init_warns_when_a_deployment_beside_it_already_uses_the_port() {
    let sandbox = Sandbox::new();
    // A port high enough that the test says nothing about what this machine
    // has on 8000, and nothing is listening on it either way.
    let port = "18400";
    sandbox
        .chap()
        .arg("init")
        .arg("a")
        .args(["--models", "none", "--api-port", port])
        .assert()
        .success();
    // The CLI names the deployment by the resolved directory, whatever
    // spelling the shell it was started from happens to use.
    let first = resolved(&sandbox.home.path().join("a"));

    sandbox
        .chap()
        .arg("init")
        .arg("b")
        .args(["--models", "none", "--api-port", port])
        .assert()
        // A warning, not a refusal: two deployments on one port is a fine way
        // to take turns, and the directory is worth writing either way.
        .success()
        .stderr(predicates::str::contains(format!(
            "port 18400 is also used by a ({first}), which is not running; both cannot be up at \
             once. Keep it, or run `chaps init --api-port 18401 --force` here / set \
             CHAP_API_PORT=18401 in .env"
        )));

    // Written all the same, at the port that was asked for.
    assert_eq!(state(&sandbox.home.path().join("b"))["api_port"], 18400);
}

#[test]
fn init_on_a_port_no_other_deployment_uses_warns_about_nothing() {
    let sandbox = Sandbox::new();
    sandbox
        .chap()
        .arg("init")
        .arg("a")
        .args(["--models", "none", "--api-port", "18400"])
        .assert()
        .success();
    sandbox
        .chap()
        .arg("init")
        .arg("b")
        .args(["--models", "none", "--api-port", "18401"])
        .assert()
        .success()
        .stderr(predicates::str::contains("is also used by").not());
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
    let pinned = stable_pin("auto_arima_chapkit").0;
    sandbox.init(&["--models", "none"]).assert().success();
    sandbox
        .models(&["enable", "auto_arima_chapkit", "--version", &pinned])
        .assert()
        .success();

    let model = &state(&dir)["models"]["auto_arima_chapkit"];
    assert_eq!(model["version"], pinned);
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
    let base = port_base();
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model,auto_arima_chapkit",
            "--port-base",
            &base.to_string(),
        ])
        .assert()
        .success();
    sandbox
        .models(&["expose", "chapkit_ewars_model", "--port", "auto"])
        .assert()
        .success();
    assert_eq!(
        state(&dir)["models"]["chapkit_ewars_model"]["host_port"],
        base
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
    assert_eq!(model["host_port"], base, "the port survives a rewrite");
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
fn restart_outside_a_project_says_so() {
    let sandbox = Sandbox::new();
    chap_in(&sandbox, sandbox.home.path(), &["restart"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not a chaps project"));
}

#[test]
fn restart_says_what_it_is_for_and_what_it_leaves_alone() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    let help = chap_in(&sandbox, &dir, &["restart", "--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let help = String::from_utf8(help).expect("help is text");
    assert!(help.contains("--all"), "{help}");
    assert!(help.contains("[SERVICE]"), "{help}");
    // One clause, and the clause is what the command is for. What it leaves
    // alone is the book's to explain, which the last line points at.
    assert!(
        help.contains("Recreate the named services even when nothing changed"),
        "{help}"
    );
    assert!(
        !help.contains("\n\n  It "),
        "no paragraphs in help:\n{help}"
    );
}

#[test]
fn restart_on_a_deployment_that_was_never_started_says_to_start_it() {
    if !docker_ready() {
        return;
    }
    let sandbox = Sandbox::new();
    // Its own directory: compose names the project after it, and this asks
    // docker about that name.
    let dir = sandbox.home.path().join("chaps-never-restarted");
    let mut init = sandbox.chap();
    init.arg("init").arg(&dir).args(["--models", "none"]);
    init.assert().success();

    chap_in(&sandbox, &dir, &["restart"])
        .assert()
        .failure()
        .stdout(predicates::str::contains(
            "CHAP is not running; start it with `chaps up`",
        ));

    // And the same before it has any opinion about the service names it was
    // given: there is nothing to restart either way.
    chap_in(&sandbox, &dir, &["restart", "--all", "chap"])
        .assert()
        .failure()
        .stdout(predicates::str::contains("CHAP is not running"));
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
    // Hiding is all that changes: the help still ends on the docs link.
    assert!(
        outside
            .trim_end()
            .ends_with("Docs: https://winterop-com.github.io/chaps/"),
        "{outside}"
    );

    sandbox.init(&["--models", "none"]).assert().success();

    let inside = chap_in(&sandbox, &dir, &["--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let inside = String::from_utf8(inside).expect("help is text");
    for name in [
        "up", "down", "logs", "restart", "docker", "backup", "status", "sync", "update", "ui",
        "auth",
    ] {
        assert!(
            inside.contains(&format!("\n  {name} ")),
            "{name} should be listed inside a project:\n{inside}"
        );
    }
    // And the mirror of it: here there is a deployment, so `init` is not the
    // command to run next and is not offered.
    assert!(
        !inside.contains("\n  init "),
        "init should not be listed inside a project:\n{inside}"
    );
    assert!(
        inside
            .trim_end()
            .ends_with("Docs: https://winterop-com.github.io/chaps/"),
        "{inside}"
    );

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

/// `init` is hidden inside a deployment, and every way of reaching it there
/// still works: its help, the refusal that names `--force`, `--force` itself,
/// and a nested deployment in a subdirectory.
#[test]
fn init_still_runs_inside_a_project_where_the_help_hides_it() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    chap_in(&sandbox, &dir, &["init", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "Create a deployment directory: compose files, .env and .chaps/",
        ))
        .stdout(predicates::str::contains("--force"));

    // Bare, it still refuses rather than overwriting what is there.
    chap_in(&sandbox, &dir, &["init"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "already contains a chaps project; use --force to overwrite",
        ));

    // `chaps init --api-port N --force` is the documented way to move the
    // API port of a deployment that already exists.
    let port = port_base();
    let mut force = sandbox.chap();
    force
        .current_dir(&dir)
        .args(["init", "--force", "--models", "none", "--api-port"])
        .arg(port.to_string());
    force.assert().success();
    assert!(
        read(&dir.join(".chaps/project.yaml")).contains(&format!("api_port: {port}")),
        "--force rewrote the project"
    );

    // And a nested deployment in a subdirectory still works, warning about
    // the one it is inside.
    let mut nested = sandbox.chap();
    nested
        .current_dir(&dir)
        .args(["init", "inner", "--models", "none"]);
    nested
        .assert()
        .success()
        .stderr(predicates::str::contains("is inside the chaps project at"));
    assert!(dir.join("inner/.chaps/project.yaml").is_file());
}

/// A group command with no subcommand is not a question its own help answers
/// outside a deployment: every subcommand it would list needs the project
/// that is missing, so that is what the reader is told instead.
#[test]
fn a_bare_group_command_outside_a_project_asks_for_a_project() {
    let sandbox = Sandbox::new();
    let missing = "is not a chaps project (no .chaps/project.yaml here or in a parent \
                   directory); run `chaps init` first";

    for group in ["components", "auth", "backup", "docker"] {
        chap_in(&sandbox, sandbox.home.path(), &[group])
            .assert()
            .code(1)
            .stderr(predicates::str::contains(missing));
    }

    // Word for word, and exit code for exit code, what the subcommands say.
    for args in [
        vec!["components", "list"],
        vec!["auth", "show"],
        vec!["jobs"],
    ] {
        chap_in(&sandbox, sandbox.home.path(), &args)
            .assert()
            .code(1)
            .stderr(predicates::str::contains(missing));
    }

    // `models` and `registry` read the catalogue, which works anywhere, so
    // their bare form is still their own help.
    for group in ["models", "registry"] {
        chap_in(&sandbox, sandbox.home.path(), &[group])
            .assert()
            .code(2)
            .stderr(predicates::str::contains("Usage: chaps"));
    }
    chap_in(&sandbox, sandbox.home.path(), &["models", "list"])
        .assert()
        .success();

    // Inside a deployment the group's help is the right answer again.
    sandbox.init(&["--models", "none"]).assert().success();
    chap_in(&sandbox, &sandbox.project(), &["components"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("Usage: chaps components"))
        .stderr(predicates::str::contains("enable"));
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
    let short = chap_in(&sandbox, sandbox.home.path(), &["-h"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "deploy and manage CHAP, the Climate Health Analytics Platform",
        ))
        .get_output()
        .stdout
        .clone();

    // The same thing, exactly: there is no longer version to ask for.
    let long = chap_in(&sandbox, sandbox.home.path(), &["--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(short).expect("help is text"),
        String::from_utf8(long).expect("help is text"),
        "`chaps -h` and `chaps --help` print the same thing"
    );

    // And the whole of it fits on a screen, with the book one line away.
    let help = chap_in(&sandbox, sandbox.home.path(), &["--help"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let help = String::from_utf8(help).expect("help is text");
    assert!(help.lines().count() <= 40, "{help}");
    assert!(
        help.trim_end()
            .ends_with("Docs: https://winterop-com.github.io/chaps/"),
        "{help}"
    );
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

/// A server that answers every request with the same 200, on a port of its
/// own. The thread lives as long as the test process.
fn server(content_type: &'static str, body: &'static str) -> u16 {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a free port");
    let port = listener.local_addr().expect("a local address").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            // The request has to be read before the answer, or the client
            // sees a reset instead of the response.
            let mut buffer = [0u8; 1024];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    port
}

/// A server that answers every request with an HTML 200, on a port of its own.
///
/// Stands in for whatever else may hold chap-core's port: a dev server, a
/// proxy, a static site.
fn html_server() -> u16 {
    server(
        "text/html; charset=utf-8",
        "<!doctype html><html><body>a dev server</body></html>",
    )
}

/// A server that answers a component's `/health` the way a running one does,
/// on a port of its own.
///
/// Stands in for OCS, which `chaps status` calls up when `/health` answers at
/// all. Nothing here needs docker, so the verdict is the same on every
/// machine.
fn health_server() -> u16 {
    server("application/json", r#"{"status":"healthy"}"#)
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
    if !docker_ready() {
        return;
    }
    let sandbox = Sandbox::new();
    // A directory of its own: compose names the project after it, and these
    // wrappers ask docker about that name.
    let dir = sandbox.home.path().join("chaps-never-started");
    // An API port of this test's own: `status` below has to find nothing
    // answering for "CHAP is not running" to be the truth about it, and the
    // default 8000 is a port a developer may well be serving something on.
    let api_port = free_port();
    let mut init = sandbox.chap();
    init.arg("init")
        .arg(&dir)
        .args(["--models", "chapkit_ewars_model", "--api-port"])
        .arg(api_port.to_string());
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

/// A command that is not one is clap's business: it names what was typed,
/// suggests what was probably meant, and exits 2. `chaps` adds nothing of its
/// own, not even for the commands chap-core's developer CLI publishes.
#[test]
fn an_unknown_subcommand_is_claps_own_error() {
    let sandbox = Sandbox::new();
    sandbox
        .chap()
        .arg("serve")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("unrecognized subcommand 'serve'"))
        .stderr(predicates::str::contains("--help"));

    // And a near miss of a real command gets clap's suggestion.
    sandbox
        .chap()
        .arg("stat")
        .assert()
        .code(2)
        .stderr(predicates::str::contains("similar subcommands"))
        .stderr(predicates::str::contains("status"));
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
    assert_eq!(doctor_status(&report, "health"), "skip", "{report}");
    assert!(
        doctor_check(&report, "health")["fix"]
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

/// The `registry pin <id>` line: what the marketplace pins for an enabled
/// model, against what that model's own repository has published since.
///
/// The hub's catalogue pins `sha-fa880a1`, committed on 1 September, while
/// `main` has published `sha-1eb8cf1` on the 8th. That lag is what the check
/// exists for: it is the marketplace that is behind, not the deployment, so
/// the line warns and points at the marketplace rather than at `chaps`.
#[test]
fn doctor_warns_when_the_marketplace_pin_lags_the_model_repository() {
    let (sandbox, _dir, port) = added_sandbox(Hub::new().publishing(MARKETPLACE_TAG));
    sandbox
        .online(port)
        .args(["models", "enable", "chapkit_ewars_model"])
        .assert()
        .success();

    let report = json_of(sandbox.online(port).args(["--json", "doctor"]));
    let check = doctor_check(&report, "registry-pin-chapkit_ewars_model");
    assert_eq!(check["status"], "warn", "{report}");
    assert_eq!(
        check["name"], "registry pin chapkit_ewars_model",
        "{report}"
    );
    assert_eq!(
        check["detail"].as_str().unwrap(),
        "the registry pins sha-fa880a1 (2026-09-01) but main's newest build is \
         sha-1eb8cf1 (2026-09-08)",
        "{report}"
    );
    assert!(
        check["fix"]
            .as_str()
            .unwrap()
            .starts_with("ask the marketplace maintainers for a new pin"),
        "{report}"
    );
    // A marketplace that lags is nothing a deployment has to act on: the pin
    // lines warn, never fail. (The rest of the report depends on the machine,
    // a runner without docker fails its docker checks, so only these lines
    // are judged.)
    let pin_failures: Vec<&serde_json::Value> = report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["id"].as_str().unwrap_or("").starts_with("registry-pin-"))
        .filter(|c| c["status"] == "fail")
        .collect();
    assert!(pin_failures.is_empty(), "{report}");

    // The same deployment with the network switched off asks nothing, and
    // every one of these lines says that is why.
    let offline = json_of(
        sandbox
            .chap()
            .arg("-C")
            .arg(sandbox.project())
            .args(["--json", "doctor"]),
    );
    let check = doctor_check(&offline, "registry-pin-chapkit_ewars_model");
    assert_eq!(check["status"], "skip", "{offline}");
    assert!(
        check["detail"].as_str().unwrap().contains("--offline"),
        "{offline}"
    );
}

/// The same line when the catalogue is up to date: the commit it pins is the
/// newest one the branch has a published build for.
#[test]
fn doctor_confirms_a_marketplace_pin_that_is_the_newest_build() {
    let (sandbox, _dir, port) = added_sandbox(Hub::new().pinning(OLD_SHA));
    sandbox
        .online(port)
        .args(["models", "enable", "chapkit_ewars_model"])
        .assert()
        .success();

    let report = json_of(sandbox.online(port).args(["--json", "doctor"]));
    let check = doctor_check(&report, "registry-pin-chapkit_ewars_model");
    assert_eq!(check["status"], "ok", "{report}");
    assert_eq!(
        check["detail"].as_str().unwrap(),
        format!("{OLD_TAG} is the newest build on main"),
        "{report}"
    );
    assert_eq!(check["fix"], Json::Null, "nothing to do: {report}");
}

// ---------------------------------------------------------------------------
// Components: chap-core, ocs and the object store
// ---------------------------------------------------------------------------

#[test]
fn init_with_ocs_writes_the_component_and_its_scaffold() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--with", "ocs"])
        .assert()
        .success()
        .stdout(predicates::str::contains("components: chap-core, ocs"))
        .stdout(predicates::str::contains(
            "OCS:       http://localhost:9000",
        ))
        // The heads-up about the object store OCS will need, once.
        .stdout(predicates::str::contains("chaps components enable s3"));

    for name in [
        "compose.ocs.yml",
        "ocs/climate-service.yaml",
        ".chaps/components.yaml",
    ] {
        assert!(dir.join(name).is_file(), "{name} was not written");
    }
    assert!(
        !dir.join("compose.s3.yml").exists(),
        "the object store was not asked for"
    );

    // The component file sits between the chaps override and the umbrella.
    let state = state(&dir);
    assert_eq!(
        state["compose_files"],
        serde_json::json!([
            "compose.yml",
            "compose.chaps.yml",
            "compose.ocs.yml",
            "compose.marketplace.yml"
        ])
    );
    assert!(
        state["rendered_files"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("compose.ocs.yml"))
    );

    let components = yaml(&dir.join(".chaps/components.yaml"));
    assert_eq!(components["chap-core"]["enabled"], Yaml::Bool(true));
    assert_eq!(components["ocs"]["enabled"], Yaml::Bool(true));
    assert_eq!(components["ocs"]["port"].as_u64(), Some(9000));
    assert_eq!(components["ocs"]["image_tag"].as_str(), Some("main"));
    assert_eq!(components["s3"]["enabled"], Yaml::Bool(false));

    // The service itself: published, no S3 variables yet, no platform pin.
    let ocs = yaml(&dir.join("compose.ocs.yml"));
    let svc = &ocs["services"]["ocs"];
    assert_eq!(
        svc["image"].as_str(),
        Some("ghcr.io/dhis2/open-climate-service:${OCS_IMAGE_TAG:-main}")
    );
    assert_eq!(svc["expose"][0].as_str(), Some("9000"));
    assert_eq!(svc["ports"][0].as_str(), Some("9000:9000"));
    assert!(svc.get("platform").is_none());
    assert!(svc["environment"].get("S3_ENDPOINT").is_none());
    // The five dataset credentials are always passed, empty until .env has
    // them, and so is the base URL: OCS reads an empty value as absent.
    let env_block = &svc["environment"];
    for var in [
        "ECMWF_DATASTORES_URL",
        "ECMWF_DATASTORES_KEY",
        "EDH_API_KEY",
        "CDSE_S3_ACCESS_KEY",
        "CDSE_S3_SECRET_KEY",
        "CLIMATE_SERVICE_BASE_URL",
    ] {
        assert_eq!(
            env_block[var].as_str(),
            Some(format!("${{{var}:-}}").as_str()),
            "{var}"
        );
    }
    // Nothing to mount: the plugin directory is opt-in.
    assert_eq!(svc["volumes"].as_sequence().unwrap().len(), 2);

    // The scaffold is OCS's own example, and says so where doctor can see it.
    let config = read(&dir.join("ocs/climate-service.yaml"));
    assert!(config.contains("Sierra Leone example values"), "{config}");
    let parsed = yaml(&dir.join("ocs/climate-service.yaml"));
    assert_eq!(parsed["extent"]["country_code"].as_str(), Some("SLE"));
    assert_eq!(parsed["data_dir"].as_str(), Some("/app/data"));

    // The image pin reaches .env as a commented line, like a model's, and so
    // does the data source section: placeholders, for the operator to fill in.
    let env = sandbox.env();
    assert!(env.contains("\n# OCS_IMAGE_TAG=main\n"), "{env}");
    assert!(
        env.contains(
            "\n# OCS data sources (optional): ERA5-Land needs one of these; \
             WorldPop and CHIRPS3 need none.\n"
        ),
        "{env}"
    );
    assert!(
        env.contains("\n# ECMWF_DATASTORES_URL=https://cds.climate.copernicus.eu/api\n"),
        "{env}"
    );
    for var in [
        "ECMWF_DATASTORES_KEY",
        "EDH_API_KEY",
        "CDSE_S3_ACCESS_KEY",
        "CDSE_S3_SECRET_KEY",
    ] {
        assert!(env.contains(&format!("\n# {var}=\n")), "{var}: {env}");
    }
    assert!(
        !env.contains("\nS3_ACCESS_KEY="),
        "no store, no credentials"
    );
}

#[test]
fn the_ocs_flags_fill_the_scaffold_instead_of_the_example() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&[
            "--models",
            "none",
            "--with",
            "ocs",
            "--ocs-name",
            "Malawi",
            "--ocs-country",
            "mwi",
            "--ocs-bbox",
            "32.6,-17.2,35.9,-9.3",
        ])
        .assert()
        .success();

    let path = dir.join("ocs/climate-service.yaml");
    let config = yaml(&path);
    assert_eq!(config["id"].as_str(), Some("malawi-climate-service"));
    assert_eq!(config["name"].as_str(), Some("Malawi Climate Service"));
    assert_eq!(config["extent"]["name"].as_str(), Some("Malawi"));
    assert_eq!(config["extent"]["country_code"].as_str(), Some("MWI"));
    assert_eq!(config["extent"]["bbox"][0].as_f64(), Some(32.6));
    assert!(
        !read(&path).contains("Sierra Leone example values"),
        "these are the operator's own values"
    );
}

#[test]
fn enabling_the_object_store_adds_its_file_its_secrets_and_the_ocs_variables() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--with", "ocs"])
        .assert()
        .success();

    sandbox
        .components(&["enable", "s3"])
        .assert()
        .success()
        .stdout(predicates::str::contains("enabled s3"))
        .stdout(predicates::str::contains("compose.s3.yml"));

    assert!(dir.join("compose.s3.yml").is_file());
    let s3 = yaml(&dir.join("compose.s3.yml"));
    let svc = &s3["services"]["s3"];
    assert_eq!(
        svc["environment"]["RUSTFS_ACCESS_KEY"].as_str(),
        Some("${S3_ACCESS_KEY:-}")
    );
    assert_eq!(svc["expose"][0].as_str(), Some("9000"));
    assert!(svc.get("ports").is_none(), "internal by default");
    assert!(s3["services"]["s3-init"].is_mapping());

    // OCS is re-rendered with the (forward-looking) S3 variables.
    let ocs = yaml(&dir.join("compose.ocs.yml"));
    let env = &ocs["services"]["ocs"]["environment"];
    assert_eq!(env["S3_ENDPOINT"].as_str(), Some("http://s3:9000"));
    assert_eq!(env["S3_BUCKET"].as_str(), Some("ocs"));

    // The credentials are generated once and land in .env, nowhere else.
    let body = sandbox.env();
    let access = env_value(&body, "S3_ACCESS_KEY").expect("an access key");
    let secret = env_value(&body, "S3_SECRET_KEY").expect("a secret key");
    assert_eq!(access.len(), 32);
    assert_eq!(secret.len(), 32);
    assert_ne!(access, secret);
    assert!(body.contains("\n# S3_IMAGE_TAG=latest\n"), "{body}");
    assert!(
        !read(&dir.join("compose.s3.yml")).contains(access),
        "the compose file substitutes them, it does not hold them"
    );

    // A second enable changes nothing, credentials least of all.
    sandbox.components(&["enable", "s3"]).assert().success();
    assert_eq!(env_value(&sandbox.env(), "S3_ACCESS_KEY"), Some(access));

    assert_eq!(
        state(&dir)["compose_files"],
        serde_json::json!([
            "compose.yml",
            "compose.chaps.yml",
            "compose.ocs.yml",
            "compose.s3.yml",
            "compose.marketplace.yml"
        ])
    );
}

#[test]
fn a_component_port_is_recorded_and_published() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--with", "ocs,s3"])
        .assert()
        .success();

    sandbox
        .components(&["enable", "ocs", "--port", "9010"])
        .assert()
        .success()
        .stdout(predicates::str::contains("http://localhost:9010"));
    assert_eq!(
        yaml(&dir.join("compose.ocs.yml"))["services"]["ocs"]["ports"][0].as_str(),
        Some("9010:9000")
    );

    sandbox
        .components(&["enable", "s3", "--port", "9002"])
        .assert()
        .success();
    assert_eq!(
        yaml(&dir.join("compose.s3.yml"))["services"]["s3"]["ports"][0].as_str(),
        Some("9002:9000")
    );

    let listed = sandbox.components(&["list"]).assert().success();
    let text = String::from_utf8_lossy(&listed.get_output().stdout).into_owned();
    assert!(text.contains("http://localhost:9010"), "{text}");
    assert!(text.contains("http://localhost:9002"), "{text}");
}

/// A western extent starts with a minus sign, and the spelling without `=` is
/// the one anyone types first.
#[test]
fn init_takes_a_negative_bbox_without_the_equals_sign() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&[
            "--models",
            "none",
            "--with",
            "ocs",
            "--ocs-name",
            "Sierra Leone",
            "--ocs-country",
            "SLE",
            "--ocs-bbox",
            "-13.5,6.9,-10.1,10.0",
        ])
        .assert()
        .success();

    let config = yaml(&dir.join("ocs/climate-service.yaml"));
    assert_eq!(config["extent"]["bbox"][0].as_f64(), Some(-13.5));
    assert_eq!(config["extent"]["bbox"][2].as_f64(), Some(-10.1));
    assert!(
        !read(&dir.join("ocs/climate-service.yaml")).contains("Sierra Leone example values"),
        "these are the operator's own values, even where they match the example"
    );
}

/// The reverse-proxy shape, set at init and then changed: no host port, a base
/// URL in its place, and ingestion over HTTP refused.
#[test]
fn ocs_can_be_put_behind_a_proxy_and_made_read_only() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&[
            "--models",
            "none",
            "--with",
            "ocs",
            "--ocs-base-url",
            "https://ocs.example.org",
        ])
        .assert()
        .success();
    assert_eq!(
        yaml(&dir.join("compose.ocs.yml"))["services"]["ocs"]["environment"]
            ["CLIMATE_SERVICE_BASE_URL"]
            .as_str(),
        Some("${CLIMATE_SERVICE_BASE_URL:-https://ocs.example.org}")
    );

    sandbox
        .components(&[
            "enable",
            "ocs",
            "--port",
            "none",
            "--base-url",
            "https://climate.example.org/",
            "--read-only",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "reached through the proxy at https://climate.example.org",
        ))
        .stdout(predicates::str::contains("read_only: true"))
        // Not a plain `chaps restart`: the config is a bind mount, so compose
        // compares nothing that changed and recreates nothing.
        .stdout(predicates::str::contains(
            "`chaps restart --all ocs` applies it",
        ));

    // The overlay: exposed on the compose network, published nowhere.
    let svc = yaml(&dir.join("compose.ocs.yml"))["services"]["ocs"].clone();
    assert_eq!(svc["expose"][0].as_str(), Some("9000"));
    assert!(svc.get("ports").is_none(), "nothing is bound on the host");
    assert_eq!(
        svc["environment"]["CLIMATE_SERVICE_BASE_URL"].as_str(),
        Some("${CLIMATE_SERVICE_BASE_URL:-https://climate.example.org}"),
        "the trailing slash goes: OCS appends a path to this"
    );

    // The instance config: one key added, the rest of the file untouched.
    let config = read(&dir.join("ocs/climate-service.yaml"));
    assert!(config.ends_with("read_only: true\n"), "{config}");
    assert!(config.contains("sierra-leone-climate-service"), "{config}");

    // And both facts in the record and in `status --json`.
    let components = yaml(&dir.join(".chaps/components.yaml"));
    assert_eq!(components["ocs"]["port"], Yaml::Null);
    assert_eq!(
        components["ocs"]["base_url"].as_str(),
        Some("https://climate.example.org")
    );
    assert_eq!(components["ocs"]["read_only"], Yaml::Bool(true));

    let status = json_of(
        sandbox
            .chap()
            .arg("-C")
            .arg(&dir)
            .args(["status", "--json"]),
    );
    let ocs = &status["components"][0];
    assert_eq!(ocs["name"].as_str(), Some("ocs"));
    assert_eq!(
        ocs["reach"].as_str(),
        Some("internal (proxy: https://climate.example.org)")
    );
    assert_eq!(ocs["read_only"], serde_json::json!(true));
    assert_eq!(ocs["health_url"], serde_json::Value::Null);

    // --read-write puts it back, editing the one key in place.
    sandbox
        .components(&["enable", "ocs", "--read-write"])
        .assert()
        .success()
        .stdout(predicates::str::contains("read_only: false"));
    assert_eq!(
        read(&dir.join("ocs/climate-service.yaml")),
        config.replace("read_only: true", "read_only: false")
    );
    assert_eq!(
        yaml(&dir.join(".chaps/components.yaml"))["ocs"]["read_only"],
        Yaml::Bool(false)
    );

    // A port again, and the address is the address: it is where this machine
    // reaches it, whatever the proxy is called.
    sandbox
        .components(&["enable", "ocs", "--port", "9010"])
        .assert()
        .success()
        .stdout(predicates::str::contains("http://localhost:9010"));
    assert_eq!(
        yaml(&dir.join("compose.ocs.yml"))["services"]["ocs"]["ports"][0].as_str(),
        Some("9010:9000")
    );
}

/// The directory is the whole declaration: a project that has one gets the
/// mount and the `plugins_dir` key, and one that does not gets neither.
#[test]
fn an_ocs_plugin_directory_is_mounted_and_configured_by_sync() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&[
            "--models",
            "none",
            "--with",
            "ocs",
            "--ocs-name",
            "Malawi",
            "--ocs-country",
            "MWI",
        ])
        .assert()
        .success();
    assert!(!read(&dir.join("compose.ocs.yml")).contains("/app/plugins"));
    assert!(!read(&dir.join("ocs/climate-service.yaml")).contains("plugins_dir"));

    std::fs::create_dir_all(dir.join("ocs/plugins/datasets")).unwrap();
    std::fs::write(dir.join("ocs/plugins/datasets/clms_gpp.py"), "# a plugin\n").unwrap();
    sandbox
        .chap()
        .arg("-C")
        .arg(&dir)
        .arg("sync")
        .assert()
        .success();

    assert_eq!(
        yaml(&dir.join("compose.ocs.yml"))["services"]["ocs"]["volumes"][1].as_str(),
        Some("./ocs/plugins:/app/plugins:ro")
    );
    let config = read(&dir.join("ocs/climate-service.yaml"));
    assert!(config.ends_with("plugins_dir: /app/plugins\n"), "{config}");
    assert!(config.contains("malawi-climate-service"), "{config}");

    // `doctor` reports the count, and it is not a problem.
    let checks = json_of(
        sandbox
            .chap()
            .arg("-C")
            .arg(&dir)
            .args(["doctor", "--json"]),
    );
    let line = checks["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "components")
        .expect("a components line")
        .clone();
    assert_eq!(line["status"].as_str(), Some("ok"));
    assert!(
        line["detail"]
            .as_str()
            .unwrap()
            .ends_with("plugins/: 1 file"),
        "{line}"
    );
    assert!(
        line["detail"]
            .as_str()
            .unwrap()
            .contains("ERA5-Land: credentials unset (WorldPop and CHIRPS3 work without them)"),
        "{line}"
    );

    // A second sync changes nothing.
    sandbox
        .chap()
        .arg("-C")
        .arg(&dir)
        .args(["sync", "--check"])
        .assert()
        .success();
}

/// The block is about third-party accounts rather than this deployment's own
/// secret, so it is masked whatever `--reveal` asked for.
#[test]
fn auth_show_lists_the_ocs_data_sources_as_set_or_unset() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--with", "ocs"])
        .assert()
        .success();

    let shown = sandbox.auth(&["show"]).assert().success();
    let text = String::from_utf8_lossy(&shown.get_output().stdout).into_owned();
    assert!(text.contains("OCS data sources"), "{text}");
    assert!(text.contains("ECMWF_DATASTORES_KEY"), "{text}");
    assert!(text.contains("unset"), "{text}");

    // Fill two in, exactly as an operator would.
    let env = dir.join(".env");
    let body = read(&env)
        .replace("# ECMWF_DATASTORES_URL=", "ECMWF_DATASTORES_URL=")
        .replace(
            "# ECMWF_DATASTORES_KEY=",
            "ECMWF_DATASTORES_KEY=0123456789abcdef",
        );
    std::fs::write(&env, body).unwrap();

    for args in [vec!["show"], vec!["show", "--reveal"]] {
        let shown = sandbox.auth(&args).assert().success();
        let text = String::from_utf8_lossy(&shown.get_output().stdout).into_owned();
        assert!(
            text.contains("ECMWF_DATASTORES_KEY  set 012345..."),
            "{args:?}: {text}"
        );
        assert!(
            !text.contains("0123456789abcdef"),
            "a third-party credential leaked with {args:?}: {text}"
        );
        assert!(text.contains("EDH_API_KEY           unset"), "{text}");
    }

    let value = json_of(&mut sandbox.auth(&["show", "--json"]));
    let sources = value["ocs_data_sources"].as_array().unwrap();
    assert_eq!(sources.len(), 5);
    assert_eq!(
        sources[1]["variable"].as_str(),
        Some("ECMWF_DATASTORES_KEY")
    );
    assert_eq!(sources[1]["set"], serde_json::json!(true));
    assert_eq!(sources[1]["masked"].as_str(), Some("012345..."));
    assert_eq!(sources[2]["set"], serde_json::json!(false));
    assert_eq!(sources[2]["masked"], serde_json::Value::Null);

    // A deployment without the component has no block at all.
    sandbox.components(&["disable", "ocs"]).assert().success();
    assert!(
        json_of(&mut sandbox.auth(&["show", "--json"]))["ocs_data_sources"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn disabling_a_component_removes_its_file_and_its_place_in_the_f_list() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--with", "ocs,s3"])
        .assert()
        .success();

    sandbox
        .components(&["disable", "ocs"])
        .assert()
        .success()
        .stdout(predicates::str::contains("disabled ocs"))
        .stdout(predicates::str::contains("compose.ocs.yml"));

    assert!(!dir.join("compose.ocs.yml").exists());
    assert!(
        dir.join("ocs/climate-service.yaml").is_file(),
        "the operator's own config is never deleted"
    );
    assert_eq!(
        state(&dir)["compose_files"],
        serde_json::json!([
            "compose.yml",
            "compose.chaps.yml",
            "compose.s3.yml",
            "compose.marketplace.yml"
        ])
    );
    assert_eq!(
        yaml(&dir.join(".chaps/components.yaml"))["ocs"]["enabled"],
        Yaml::Bool(false)
    );

    // Enabling it again restores the file and keeps the config that is there.
    std::fs::write(dir.join("ocs/climate-service.yaml"), "id: mine\n").unwrap();
    sandbox.components(&["enable", "ocs"]).assert().success();
    assert!(dir.join("compose.ocs.yml").is_file());
    assert_eq!(read(&dir.join("ocs/climate-service.yaml")), "id: mine\n");
}

#[test]
fn chap_core_cannot_be_disabled_while_a_model_is_enabled() {
    let sandbox = Sandbox::new();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    sandbox
        .components(&["disable", "chap-core"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("chapkit_ewars_model"))
        .stderr(predicates::str::contains("chaps models disable"));
    assert!(sandbox.project().join("compose.yml").is_file());

    // With the model gone it is allowed, and the base stack goes with it.
    sandbox
        .models(&["disable", "chapkit_ewars_model"])
        .assert()
        .success();
    sandbox
        .components(&["disable", "chap-core"])
        .assert()
        .success();
    assert!(!sandbox.project().join("compose.yml").exists());
    assert!(!sandbox.project().join("compose.chaps.yml").exists());
}

#[test]
fn a_standalone_ocs_deployment_leaves_chap_core_out() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--with", "ocs", "--without", "chap-core"])
        .assert()
        .success()
        .stdout(predicates::str::contains("components: ocs"))
        .stdout(predicates::str::contains("OCS:"))
        .stdout(predicates::str::contains("API:").not());

    assert!(!dir.join("compose.yml").exists());
    assert!(!dir.join("compose.chaps.yml").exists());
    assert!(dir.join("compose.ocs.yml").is_file());
    assert_eq!(
        state(&dir)["compose_files"],
        serde_json::json!(["compose.ocs.yml", "compose.marketplace.yml"])
    );

    // Asking for a model at the same time is a contradiction, not a surprise.
    let other = Sandbox::new();
    other
        .init(&["--without", "chap-core", "--models", "chapkit_ewars_model"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--without chap-core"));
}

#[test]
fn an_unknown_component_name_is_reported_before_anything_is_written() {
    let sandbox = Sandbox::new();
    sandbox
        .init(&["--with", "nope"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("unknown component `nope`"));
    assert!(!sandbox.project().exists());
}

#[test]
fn status_reports_every_enabled_component() {
    let sandbox = Sandbox::new();
    // Ports of this test's own, never the defaults: `status` really calls
    // `/health`, and OCS's default 9000 is a port a developer running an OCS
    // of their own would answer on.
    let api_port = free_port().to_string();
    let ocs_port = free_port();
    sandbox
        .init(&["--models", "none", "--api-port", &api_port])
        .assert()
        .success();
    // Enabled here rather than with `init --with`, because this is where the
    // host port can be named, and init would probe 9000 on the way.
    sandbox
        .components(&["enable", "ocs", "--port", &ocs_port.to_string()])
        .assert()
        .success();
    sandbox.components(&["enable", "s3"]).assert().success();

    // Nothing is running, so this exits non-zero; the document is the point.
    let out = sandbox
        .chap()
        .arg("-C")
        .arg(sandbox.project())
        .args(["status", "--json", "--timeout", "1"])
        .assert()
        .get_output()
        .stdout
        .clone();
    let report: Json = serde_json::from_slice(&out).expect("status --json is one document");
    let components = report["components"].as_array().expect("a component list");
    assert_eq!(components.len(), 2);
    assert_eq!(components[0]["name"], "ocs");
    assert_eq!(
        components[0]["reach"],
        format!("http://localhost:{ocs_port}")
    );
    assert_eq!(
        components[0]["health_url"],
        format!("http://localhost:{ocs_port}/health")
    );
    // The port was free when this test took it and nothing bound it since.
    assert_eq!(components[0]["state"], "not-running");
    assert_eq!(components[1]["name"], "s3");
    assert_eq!(components[1]["reach"], "internal");
    assert_eq!(components[1]["health_url"], Json::Null);
}

/// The other half of the same report: a component whose `/health` answers is
/// `up`, without a container anywhere in it.
#[test]
fn status_calls_a_component_up_when_its_health_endpoint_answers() {
    let sandbox = Sandbox::new();
    let api_port = free_port().to_string();
    // A stand-in OCS on a port of its own, answering for as long as this test
    // runs.
    let ocs_port = health_server();
    sandbox
        .init(&["--models", "none", "--api-port", &api_port])
        .assert()
        .success();
    sandbox
        .components(&["enable", "ocs", "--port", &ocs_port.to_string()])
        .assert()
        .success();

    let out = sandbox
        .chap()
        .arg("-C")
        .arg(sandbox.project())
        .args(["status", "--json", "--timeout", "5"])
        .assert()
        .get_output()
        .stdout
        .clone();
    let report: Json = serde_json::from_slice(&out).expect("status --json is one document");
    let components = report["components"].as_array().expect("a component list");
    assert_eq!(components.len(), 1, "{report}");
    assert_eq!(components[0]["name"], "ocs");
    assert_eq!(
        components[0]["health_url"],
        format!("http://localhost:{ocs_port}/health")
    );
    assert_eq!(components[0]["state"], "up", "{report}");
}

#[test]
fn doctor_checks_the_components_and_the_files_they_add() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--with", "ocs"])
        .assert()
        .success();

    let out = sandbox
        .chap()
        .arg("-C")
        .arg(&dir)
        .arg("doctor")
        .assert()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8_lossy(&out).into_owned();
    // The scaffold is still the example, which is a warning with a fix.
    assert!(
        text.contains("components") && text.contains("still holds OCS's example values"),
        "{text}"
    );
    assert!(text.contains("port ocs"), "{text}");
    assert!(text.contains("project files"), "{text}");

    // Editing the config and deleting the note turns the line green.
    std::fs::write(
        dir.join("ocs/climate-service.yaml"),
        "id: mine\nname: Mine\nextent:\n  name: Mine\n  bbox: [0, 0, 1, 1]\n  \
         country_code: MWI\ndata_dir: /app/data\n",
    )
    .unwrap();
    let out = sandbox
        .chap()
        .arg("-C")
        .arg(&dir)
        .arg("doctor")
        .assert()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8_lossy(&out).into_owned();
    assert!(
        text.contains("chap-core, ocs; ocs/climate-service.yaml present"),
        "{text}"
    );
}

#[test]
fn docker_accepts_a_deployment_with_both_components() {
    if !docker_ready() {
        return;
    }
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--with", "ocs,s3", "--api-port", "8123"])
        .assert()
        .success();

    let out = std::process::Command::new("docker")
        .args(["compose", "-f", "compose.yml", "-f", "compose.chaps.yml"])
        .args(["-f", "compose.ocs.yml", "-f", "compose.s3.yml"])
        .args(["-f", "compose.marketplace.yml", "config"])
        .current_dir(&dir)
        .output()
        .expect("docker compose config runs");
    assert!(
        out.status.success(),
        "docker compose config failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let merged: Yaml = serde_yaml_ng::from_slice(&out.stdout).expect("config is YAML");
    let ocs = &merged["services"]["ocs"];
    assert_eq!(ocs["ports"][0]["published"].as_str(), Some("9000"));
    assert_eq!(ocs["ports"][0]["target"].as_u64(), Some(9000));
    // The .env values reached the merged document rather than an empty string.
    let key = ocs["environment"]["S3_ACCESS_KEY"].as_str().unwrap();
    assert_eq!(key.len(), 32, "compose substituted S3_ACCESS_KEY");
    assert_eq!(
        merged["services"]["s3"]["environment"]["RUSTFS_ACCESS_KEY"].as_str(),
        Some(key)
    );
    assert!(
        merged["services"]["s3"].get("ports").is_none(),
        "the store is internal"
    );
    assert_eq!(
        merged["services"]["s3-init"]["restart"].as_str(),
        Some("no")
    );
}

// ------------------------------------------------- purging data volumes ---

/// The named volumes docker holds whose names start with `prefix`.
///
/// `docker volume ls --filter name=` matches substrings, so the prefix is
/// checked again here: one deployment's name must not answer for another's.
fn docker_volumes(prefix: &str) -> Vec<String> {
    let out = std::process::Command::new("docker")
        .args(["volume", "ls", "--filter"])
        .arg(format!("name={prefix}"))
        .args(["--format", "{{.Name}}"])
        .stdin(std::process::Stdio::null())
        .output()
        .expect("docker volume ls runs");
    assert!(
        out.status.success(),
        "docker volume ls failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| name.starts_with(prefix))
        .map(str::to_string)
        .collect()
}

/// A named volume a test created, removed again however the test ends.
struct Volume(String);

impl Volume {
    /// Create it, as compose would when it first starts the service.
    fn create(name: &str) -> Volume {
        let out = std::process::Command::new("docker")
            .args(["volume", "create", name])
            .stdin(std::process::Stdio::null())
            .output()
            .expect("docker volume create runs");
        assert!(
            out.status.success(),
            "docker volume create failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Volume(name.to_string())
    }
}

impl Drop for Volume {
    fn drop(&mut self) {
        let _ = std::process::Command::new("docker")
            .args(["volume", "rm", "-f", &self.0])
            .stdin(std::process::Stdio::null())
            .output();
    }
}

/// `chaps models disable --purge` removes the volume docker really holds.
///
/// Which is the whole reason the flag exists: the overlay that declared the
/// volume is gone by then, so `down --volumes` cannot reach it and only its
/// name can. The volume is created here rather than by `chaps up`, which would
/// pull the entire stack for one `docker volume rm`; compose creates it under
/// exactly this name, and `live_up_then_purge_removes_the_model_volume`
/// below is the same thing through a real deployment.
#[test]
fn models_disable_purge_removes_the_volume_docker_holds() {
    if !docker_ready() {
        return;
    }
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    // A directory of its own: this asks docker about the compose project
    // name, which is derived from the directory the deployment lives in.
    let dir = dir.with_file_name("chaps-purged");
    let mut init = sandbox.chap();
    init.arg("init")
        .arg(&dir)
        .args(["--models", "chapkit_ewars_model", "--api-port"])
        .arg(free_port().to_string());
    init.assert().success();

    let project = state(&dir)["compose_project"]
        .as_str()
        .expect("init records a compose project name")
        .to_string();
    let volume = format!("{project}_ck_chapkit_ewars_model_data");
    let _created = Volume::create(&volume);
    assert_eq!(
        docker_volumes(&format!("{project}_")),
        std::slice::from_ref(&volume)
    );

    chap_in(
        &sandbox,
        &dir,
        &["models", "disable", "chapkit_ewars_model", "--purge"],
    )
    .assert()
    .success()
    .stdout(predicates::str::contains(format!(
        "removed volume {volume}"
    )))
    .stdout(predicates::str::contains("kept volume").not());
    assert!(
        docker_volumes(&format!("{project}_")).is_empty(),
        "the model's data volume is still there"
    );

    // And `doctor` no longer has a leftover to warn about, because there is
    // nothing under this deployment's name at all.
    chap_in(&sandbox, &dir, &["doctor"])
        .assert()
        .stdout(predicates::str::contains("leftover volumes").not());
}

/// `chaps doctor` warns about the volume of a model this deployment no longer
/// enables, and names the command that removes it.
#[test]
fn doctor_warns_about_the_volume_of_a_disabled_model() {
    if !docker_ready() {
        return;
    }
    let sandbox = Sandbox::new();
    let dir = sandbox.project().with_file_name("chaps-leftover");
    let mut init = sandbox.chap();
    init.arg("init")
        .arg(&dir)
        .args(["--models", "chapkit_ewars_model", "--api-port"])
        .arg(free_port().to_string());
    init.assert().success();

    let project = state(&dir)["compose_project"]
        .as_str()
        .expect("init records a compose project name")
        .to_string();
    let volume = format!("{project}_ck_chapkit_ewars_model_data");
    let _created = Volume::create(&volume);

    // While the model is enabled the volume is its data, not a leftover.
    chap_in(&sandbox, &dir, &["doctor"])
        .assert()
        .stdout(predicates::str::contains("1 volume named"))
        .stdout(predicates::str::contains("leftover volumes").not());

    chap_in(
        &sandbox,
        &dir,
        &["models", "disable", "chapkit_ewars_model"],
    )
    .assert()
    .success();
    chap_in(&sandbox, &dir, &["doctor"])
        .assert()
        .stdout(predicates::str::contains(format!(
            "leftover volumes from disabled models or components: {volume}"
        )))
        .stdout(predicates::str::contains(
            "chaps models disable <id> --purge",
        ));
}

/// The live version: `chaps up` creates the model's volume, and
/// `models disable --purge` takes it away again.
///
/// Ignored by default because `up` pulls chap-core, PostgreSQL, Valkey and
/// the model image - gigabytes on a cold cache, and minutes either way, which
/// is not what `cargo test` is for. Run it by hand on a machine with Docker:
///
/// ```sh
/// cargo test --test cli -- --ignored live_up_then_purge
/// ```
#[test]
#[ignore = "pulls the whole stack; run with `cargo test -- --ignored`"]
fn live_up_then_purge_removes_the_model_volume() {
    if !docker_ready() {
        return;
    }
    let sandbox = Sandbox::new();
    let dir = sandbox.project().with_file_name("chaps-live-purge");
    let mut init = sandbox.chap();
    init.arg("init")
        .arg(&dir)
        .args(["--models", "chapkit_ewars_model", "--api-port"])
        .arg(free_port().to_string());
    init.assert().success();

    let project = state(&dir)["compose_project"]
        .as_str()
        .expect("init records a compose project name")
        .to_string();
    let volume = format!("{project}_ck_chapkit_ewars_model_data");

    /// Whatever this test does next, the stack it started goes away with it,
    /// volumes and all.
    ///
    /// `--yes` because nothing here is watching a prompt: this is the drop
    /// that has to happen whether or not the assertions above it held.
    struct Started {
        dir: PathBuf,
        cache: PathBuf,
    }
    impl Drop for Started {
        fn drop(&mut self) {
            let mut down = Command::cargo_bin("chaps").expect("the chaps binary is built");
            let _ = down
                .env("CHAPS_CACHE_DIR", &self.cache)
                .current_dir(&self.dir)
                .args(["--offline", "down", "--volumes", "--yes"])
                .output();
        }
    }
    let _started = Started {
        dir: dir.clone(),
        cache: sandbox.cache.path().to_path_buf(),
    };

    // Ten minutes: the images are several gigabytes on a cold cache.
    chap_in(&sandbox, &dir, &["up"])
        .timeout(std::time::Duration::from_secs(600))
        .assert()
        .success();
    assert!(
        docker_volumes(&format!("{project}_")).contains(&volume),
        "compose did not create {volume}"
    );

    chap_in(
        &sandbox,
        &dir,
        &["models", "disable", "chapkit_ewars_model", "--purge"],
    )
    .assert()
    .success()
    .stdout(predicates::str::contains(format!(
        "removed volume {volume}"
    )))
    .stdout(predicates::str::contains("stopped and removed"));

    assert!(
        !docker_volumes(&format!("{project}_")).contains(&volume),
        "docker volume ls still lists {volume}"
    );
}

// ---------------------------------------------------- down --volumes ---

/// `-v` after `down` is refused, in whichever way it was typed.
///
/// It is the global `--verbose` flag, so compose never sees it: passing it
/// through would have been a lie either way round, and a `down` that quietly
/// kept the data of someone who asked for it to go is exactly the failure
/// `--volumes` exists to prevent. Refused before the project is loaded and
/// before docker is asked anything, so the answer is the same on a machine
/// with no daemon.
#[test]
fn a_volume_flag_after_down_says_which_flag_to_use() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    for argv in [
        vec!["down", "-v"],
        vec!["down", "--", "-v"],
        vec!["down", "--", "--volumes"],
    ] {
        chap_in(&sandbox, &dir, &argv)
            .assert()
            // Clap's own code for a usage error, because that is what it is.
            .code(2)
            .stderr(predicates::str::contains("chaps's own --verbose flag"))
            .stderr(predicates::str::contains("`chaps down --volumes`"))
            .stderr(predicates::str::contains("`chaps -v down`"));
    }

    // And the verbose stop it could have meant is still a verbose stop: the
    // flag before the subcommand is unambiguous, so it runs (and fails on
    // whatever docker says, if anything).
    chap_in(&sandbox, &dir, &["-v", "down"])
        .assert()
        .stderr(predicates::str::contains("--verbose flag").not());
}

/// `down --volumes` destroys data, so a run nobody can answer refuses.
///
/// No terminal on stdin here, which is every script and every CI job: the
/// prompt would be a hang, and going ahead unasked would take the database of
/// whatever deployment the script was standing in.
#[test]
fn down_volumes_refuses_to_destroy_data_nobody_confirmed() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();

    chap_in(&sandbox, &dir, &["down", "--volumes"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("pass --yes"));

    // `--json` is the same kind of run: nobody is watching it either.
    chap_in(&sandbox, &dir, &["--json", "down", "--volumes"])
        .assert()
        .failure()
        .stdout(predicates::str::contains("pass --yes"));
}

/// `chaps down --volumes --yes` removes the volumes docker holds, and names
/// them.
///
/// The volume is created here rather than by `chaps up`, which would pull the
/// whole stack for one `docker volume rm`; compose declares the database
/// under exactly this name, and `chap-db` is the one whose loss is the point
/// of the flag.
#[test]
fn down_volumes_removes_the_database_volume_docker_holds() {
    if !docker_ready() {
        return;
    }
    let sandbox = Sandbox::new();
    // A directory of its own: the volumes carry the compose project name,
    // which is derived from the directory the deployment lives in.
    let dir = sandbox.project().with_file_name("chaps-down-volumes");
    let mut init = sandbox.chap();
    init.arg("init")
        .arg(&dir)
        .args(["--models", "none", "--api-port"])
        .arg(free_port().to_string());
    init.assert().success();

    let project = state(&dir)["compose_project"]
        .as_str()
        .expect("init records a compose project name")
        .to_string();
    let volume = format!("{project}_chap-db");
    let _created = Volume::create(&volume);
    assert_eq!(
        docker_volumes(&format!("{project}_")),
        std::slice::from_ref(&volume)
    );

    chap_in(&sandbox, &dir, &["down", "--volumes", "--yes"])
        .assert()
        .success()
        // Named before it goes, and again once it is gone.
        .stdout(predicates::str::contains(format!(
            "docker holds 1 volume under {project}_*, data and all: {volume}"
        )))
        .stdout(predicates::str::contains(format!(
            "removed 1 volume ({volume})"
        )))
        .stdout(predicates::str::contains("volumes kept").not());

    assert!(
        docker_volumes(&format!("{project}_")).is_empty(),
        "docker volume ls still lists {volume}"
    );
}

// ------------------------------------------------- models add and remove ---

/// The newest commit on the example repository's default branch.
const NEW_SHA: &str = "b1d6c31f4b2f0d8a0f4c6e2a5e9d3c7b8a1f0e2d";
/// The commit before it, and the only one this hub publishes by default.
const OLD_SHA: &str = "1eb8cf1a2b3c4d5e6f708192a3b4c5d6e7f80910";
/// The tags those two commits are built as.
const NEW_TAG: &str = "sha-b1d6c31";
const OLD_TAG: &str = "sha-1eb8cf1";
/// The commit the served marketplace entry pins, older than either of them.
const MARKETPLACE_SHA: &str = "fa880a1";
/// The day every commit this hub knows about was made. The `registry pin`
/// checks read these, and place a pin the branch listing does not carry by
/// asking for its commit by name.
const COMMIT_DAYS: &[(&str, &str)] = &[
    (NEW_SHA, "2026-09-23T08:15:00Z"),
    (OLD_SHA, "2026-09-08T11:00:00Z"),
    (MARKETPLACE_SHA, "2026-09-01T09:00:00Z"),
];

/// The committer date this hub gives one commit, matched on the short SHA so
/// a full one and the seven characters a pin carries find the same day.
fn commit_date(sha: &str) -> Option<&'static str> {
    let short = |sha: &str| sha.chars().take(7).collect::<String>();
    COMMIT_DAYS
        .iter()
        .find(|(known, _)| short(known) == short(sha))
        .map(|(_, date)| *date)
}
/// The repository `models add` is pointed at.
const REPO_URL: &str = "https://github.com/example/chapkit_example_manual_model";
/// The image that repository publishes to.
const IMAGE: &str = "ghcr.io/example/chapkit_example_manual_model";

/// A local stand-in for GitHub, ghcr and the marketplace, routed by path.
///
/// One listener answers every request one `chaps models add` makes: the
/// repository's default branch, its commits, a ghcr pull token, the OCI index
/// of a tag, the amd64 manifest inside it, the config blob that manifest
/// points at, and the registry index `--registry-url` names. Nothing in these
/// tests touches the real network.
#[derive(Clone)]
struct Hub {
    /// Commits on the default branch, newest first.
    commits: Vec<String>,
    /// The `sha-` tags the registry has an image for.
    published: Vec<String>,
    /// What the image's config says it runs as.
    user: String,
    /// Its `WorkingDir`, which is what the data directory is derived from.
    working_dir: String,
    /// chap-core's releases, newest first, as `(tag, published_at)`. They
    /// answer `releases/latest`, `releases`, `releases/tags/<tag>` and the
    /// `compose.ghcr.yml` every one of them publishes.
    releases: Vec<(String, String)>,
    /// The commit the served marketplace entry pins, which is what the
    /// `registry pin` checks compare against the branch.
    pinned: String,
}

/// The chap-core releases the hub publishes by default: two of them, so a
/// switch can go forwards and backwards between releases as well as to a
/// moving tag.
const CHAP_RELEASES: &[(&str, &str)] = &[
    ("v2.3.1", "2026-09-21T10:00:25Z"),
    ("v2.3.0", "2026-09-11T09:20:41Z"),
];

/// The day the hub's `dev` branch was last committed to.
const DEV_COMMITTED: &str = "2026-09-24T08:15:00Z";

/// chap-core's `compose.ghcr.yml` as the hub publishes it at one ref.
///
/// The ref is written into the document, so a test can tell which one
/// `compose.yml` was rendered from; everything else is the little the CLI
/// requires of it, the `${CHAP_IMAGE_TAG}` pin included.
fn chap_compose(reference: &str) -> String {
    format!(
        "services:\n  \
         chap:\n    \
         image: ghcr.io/dhis2-chap/chap-core:${{CHAP_IMAGE_TAG:-latest}}\n    \
         environment:\n      \
         CHAPS_TEST_REF: {reference}\n  \
         worker:\n    \
         image: ghcr.io/dhis2-chap/chap-core:${{CHAP_IMAGE_TAG:-latest}}\n"
    )
}

/// The digest of the amd64 manifest inside the index.
const AMD64_DIGEST: &str =
    "sha256:1111111111111111111111111111111111111111111111111111111111111111";
/// The digest of the config blob that manifest points at.
const CONFIG_DIGEST: &str =
    "sha256:3333333333333333333333333333333333333333333333333333333333333333";

impl Hub {
    /// The example repository as it stands: two commits on `main`, only the
    /// older of them published, and an image that runs as 10001:10001 out of
    /// `/work`.
    ///
    /// Publishing only the older commit is the case that matters: the pin has
    /// to land on the newest build there *is*, not on the newest commit.
    fn new() -> Hub {
        Hub {
            commits: vec![NEW_SHA.to_string(), OLD_SHA.to_string()],
            published: vec![OLD_TAG.to_string()],
            user: "10001:10001".to_string(),
            working_dir: "/work".to_string(),
            releases: CHAP_RELEASES
                .iter()
                .map(|(tag, at)| (tag.to_string(), at.to_string()))
                .collect(),
            pinned: MARKETPLACE_SHA.to_string(),
        }
    }

    /// The same hub once the newer commit has been published too, which is
    /// what `chaps update` is meant to notice.
    fn advanced(self) -> Hub {
        Hub {
            published: vec![NEW_TAG.to_string(), OLD_TAG.to_string()],
            ..self
        }
    }

    /// The same, for an image that runs as an account name nothing here can
    /// turn into numbers.
    fn running_as(self, user: &str) -> Hub {
        Hub {
            user: user.to_string(),
            ..self
        }
    }

    /// The same hub, also publishing the tag the served marketplace entry
    /// pins - which is what `models enable` reads the user from.
    fn publishing(self, tag: &str) -> Hub {
        let mut published = self.published.clone();
        published.push(tag.to_string());
        Hub { published, ..self }
    }

    /// The same, with the marketplace entry pinning `commit` rather than the
    /// one it carries by default.
    fn pinning(self, commit: &str) -> Hub {
        Hub {
            pinned: commit.to_string(),
            ..self
        }
    }

    /// The marketplace entry as this hub serves it: the template, repinned.
    fn marketplace_model(&self) -> String {
        let short: String = self.pinned.chars().take(7).collect();
        MARKETPLACE_MODEL
            .replace(
                &format!("commit: {MARKETPLACE_SHA}"),
                &format!("commit: {}", self.pinned),
            )
            .replace(
                &format!("image_tag: sha-{MARKETPLACE_SHA}"),
                &format!("image_tag: sha-{short}"),
            )
    }

    /// `WorkingDir` of the image, which the data directory follows.
    fn working_in(self, working_dir: &str) -> Hub {
        Hub {
            working_dir: working_dir.to_string(),
            ..self
        }
    }

    /// Serve this hub on a port of its own; the thread lives as long as the
    /// test process.
    fn start(self) -> u16 {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("a free port");
        let port = listener.local_addr().expect("a local address").port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                // The request has to be read before the answer, or the client
                // sees a reset instead of the response.
                let mut buffer = [0u8; 2048];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/")
                    .to_string();
                let (status, content_type, body) = self.respond(&path);
                let reason = if status == 200 { "OK" } else { "Not Found" };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        port
    }

    /// The answer one request path gets: `(status, content type, body)`.
    fn respond(&self, path: &str) -> (u16, &'static str, String) {
        let json = "application/json";
        if path.starts_with("/registry.yaml") {
            return (200, "text/plain", REGISTRY_INDEX.to_string());
        }
        // chap-core's own releases and the compose file each ref publishes.
        // Before the model repository's routes, which are the wider match.
        if let Some(answer) = self.chap_core(path) {
            return answer;
        }
        if path.starts_with("/models/chapkit_ewars_model.yaml") {
            return (200, "text/plain", self.marketplace_model());
        }
        if path.starts_with("/token") {
            return (200, json, r#"{"token":"anonymous"}"#.to_string());
        }
        // One commit by name, which is how a pin the branch listing does not
        // carry is placed. Before the listing, which is the wider match.
        if let Some((_, sha)) = path.split_once("/commits/") {
            let sha = sha.split('?').next().unwrap_or(sha);
            return match commit_date(sha) {
                Some(date) => (200, json, dated_commit(sha, date)),
                None => (404, json, r#"{"message":"Not Found"}"#.to_string()),
            };
        }
        if path.starts_with("/repos/") && path.contains("/commits") {
            let entries: Vec<String> = self
                .commits
                .iter()
                .map(|sha| dated_commit(sha, commit_date(sha).unwrap_or_default()))
                .collect();
            return (200, json, format!("[{}]", entries.join(",")));
        }
        if path.starts_with("/repos/") {
            return (
                200,
                json,
                r#"{"name":"chapkit_example_manual_model","default_branch":"main"}"#.to_string(),
            );
        }
        if let Some((_, reference)) = path.split_once("/manifests/") {
            if reference == AMD64_DIGEST {
                return (200, json, manifest());
            }
            // A digest pin asks for the index by digest; a tag pin asks for
            // it by tag, and an unpublished tag is a 404.
            if reference.starts_with("sha256:") || self.published.iter().any(|t| t == reference) {
                return (200, json, index());
            }
            return (
                404,
                json,
                r#"{"errors":[{"code":"MANIFEST_UNKNOWN"}]}"#.to_string(),
            );
        }
        if path.contains("/blobs/") {
            return (
                200,
                json,
                format!(
                    r#"{{"architecture":"amd64","os":"linux","config":{{"User":"{}","WorkingDir":"{}"}}}}"#,
                    self.user, self.working_dir
                ),
            );
        }
        (404, json, "{}".to_string())
    }

    /// The chap-core half of the hub: the release feed and the raw
    /// `compose.ghcr.yml` of every ref that publishes one.
    ///
    /// `None` when the path is not one of chap-core's, which is what lets the
    /// model repository keep the routes it had.
    fn chap_core(&self, path: &str) -> Option<(u16, &'static str, String)> {
        const REPO: &str = "dhis2-chap/chap-core";
        let json = "application/json";
        let release = |(tag, at): &(String, String)| {
            format!(
                r#"{{"tag_name":"{tag}","published_at":"{at}","draft":false,"prerelease":false}}"#
            )
        };
        let releases = format!("/repos/{REPO}/releases");

        if path.starts_with(&format!("{releases}/latest")) {
            return Some(match self.releases.first() {
                Some(newest) => (200, json, release(newest)),
                None => (404, json, r#"{"message":"Not Found"}"#.to_string()),
            });
        }
        if let Some(tag) = path.strip_prefix(&format!("{releases}/tags/")) {
            let tag = tag.split('?').next().unwrap_or(tag);
            return Some(match self.releases.iter().find(|(name, _)| name == tag) {
                Some(found) => (200, json, release(found)),
                None => (404, json, r#"{"message":"Not Found"}"#.to_string()),
            });
        }
        if path.starts_with(&releases) {
            let entries: Vec<String> = self.releases.iter().map(release).collect();
            return Some((200, json, format!("[{}]", entries.join(","))));
        }
        if path.starts_with(&format!("/repos/{REPO}/commits")) {
            return Some((
                200,
                json,
                format!(
                    r#"[{{"sha":"{NEW_SHA}","commit":{{"committer":{{"date":"{DEV_COMMITTED}"}}}}}}]"#
                ),
            ));
        }
        // The raw host: `/<repo>/<ref>/compose.ghcr.yml`. Every release and
        // the two branches publish one; anything else is a 404, which is what
        // a tag that was never built looks like.
        let reference = path
            .strip_prefix(&format!("/{REPO}/"))?
            .strip_suffix("/compose.ghcr.yml")?;
        let known = reference == "dev"
            || reference == "master"
            || self.releases.iter().any(|(tag, _)| tag == reference);
        Some(match known {
            true => (200, "text/plain", chap_compose(reference)),
            false => (404, "text/plain", "404: Not Found".to_string()),
        })
    }
}

/// One entry of a commit listing, as GitHub writes it.
fn dated_commit(sha: &str, date: &str) -> String {
    format!(r#"{{"sha":"{sha}","commit":{{"message":"x","committer":{{"date":"{date}"}}}}}}"#)
}

/// An OCI index with the attestation entry ghcr adds to every one of them.
fn index() -> String {
    format!(
        r#"{{"schemaVersion":2,"mediaType":"application/vnd.oci.image.index.v1+json","manifests":[
          {{"digest":"{AMD64_DIGEST}","platform":{{"architecture":"amd64","os":"linux"}}}},
          {{"digest":"sha256:2222222222222222222222222222222222222222222222222222222222222222",
            "platform":{{"architecture":"unknown","os":"unknown"}}}}]}}"#
    )
}

/// The amd64 manifest inside that index.
fn manifest() -> String {
    format!(r#"{{"schemaVersion":2,"config":{{"digest":"{CONFIG_DIGEST}","size":1}},"layers":[]}}"#)
}

/// A one-model marketplace, so a collision with the catalogue can be told
/// from a collision with something the deployment added itself.
const REGISTRY_INDEX: &str = "\
schema_version: 2
marketplace:
  name: test marketplace
  description: served by the CLI tests
  repository: https://example.test/marketplace
review_policy:
  required_approvals: 3
models:
  - models/chapkit_ewars_model.yaml
";

const MARKETPLACE_MODEL: &str = "\
schema_version: 2
id: chapkit_ewars_model
service_id: chapkit-ewars-model
display_name: CHAP-EWARS
kind: model
assessed_status: orange
summary: stands in for the marketplace entry
source:
  repository: https://github.com/chap-models/chapkit_ewars_model
  image: ghcr.io/chap-models/chapkit_ewars_model
  runtime_image: ghcr.io/dhis2-chap/chapkit-r-inla
attribution:
  author: nobody
compatibility:
  period_types:
    - month
  min_prediction_periods: 1
  max_prediction_periods: 6
covariates: {}
channels:
  stable: 1.0.0
  latest: 1.0.0
versions:
  - version: 1.0.0
    commit: fa880a1
    image_tag: sha-fa880a1
    chapkit: '>=2,<3'
    status: verified
configurations: {}
";

/// A deployment with nothing enabled, plus a hub to add against.
fn added_sandbox(hub: Hub) -> (Sandbox, PathBuf, u16) {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--api-port", &free_port().to_string()])
        .assert()
        .success();
    (sandbox, dir, hub.start())
}

/// The tag the served marketplace entry pins, which is the image
/// `models enable` asks about.
const MARKETPLACE_TAG: &str = "sha-fa880a1";

/// `models enable` reads the user off the image's own config, not off the
/// table compiled into the binary.
#[test]
fn enable_reads_the_user_from_the_image_config() {
    let (sandbox, dir, port) = added_sandbox(
        Hub::new()
            .publishing(MARKETPLACE_TAG)
            .running_as("root")
            .working_in("/work"),
    );
    sandbox
        .online(port)
        .args(["models", "enable", "chapkit_ewars_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains("enabled chapkit_ewars_model"))
        // Nothing was guessed at, so nothing is reported as guessed at.
        .stdout(predicates::str::contains("built-in table").not());

    let model = &state(&dir)["models"]["chapkit_ewars_model"];
    assert_eq!(model["user"], "root", "the image config says root");
    assert_eq!(model["user_from"], "image-config");
    assert_eq!(model["data_dir"], "/work/data", "from its WorkingDir");

    // The overlay overrides nothing on a root image, and hands its volume to
    // root: docker creates a fresh one root-owned already, but a volume
    // carried over from a deployment that ran this model as an account is
    // owned by that account, and root cannot write to it with every
    // capability dropped.
    let overlay = yaml(&dir.join("compose.chapkit-ewars-model.yml"));
    assert!(
        overlay["services"]["chapkit-ewars-model"]
            .get("user")
            .is_none(),
        "the model service overrides nothing"
    );
    assert_eq!(
        overlay["services"]["chapkit-ewars-model-init"]["command"][2].as_str(),
        Some("chown -R 0:0 /work/data")
    );

    // `models info` says where the answer came from.
    sandbox
        .models(&["info", "chapkit_ewars_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains("user      root  (image config)"));

    // An image that runs as an account gets the numeric pair in both places.
    let (sandbox, dir, port) = added_sandbox(
        Hub::new()
            .publishing(MARKETPLACE_TAG)
            .running_as("chapkit")
            .working_in("/app"),
    );
    sandbox
        .online(port)
        .args(["models", "enable", "chapkit_ewars_model"])
        .assert()
        .success();
    let model = &state(&dir)["models"]["chapkit_ewars_model"];
    assert_eq!(model["user"], "1000:1000");
    assert_eq!(model["user_from"], "image-config");
    assert_eq!(model["data_dir"], "/app/data");
    let overlay = read(&dir.join("compose.chapkit-ewars-model.yml"));
    assert!(overlay.contains("    user: 1000:1000\n"), "{overlay}");
    assert!(
        overlay.contains("chown -R 1000:1000 /app/data"),
        "{overlay}"
    );
}

/// An `--offline` enable has no image to read, and says which answer it fell
/// back to rather than passing the table off as the image's word.
#[test]
fn an_offline_enable_falls_back_to_the_table_and_says_so() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--api-port", &free_port().to_string()])
        .assert()
        .success();

    sandbox
        .models(&["enable", "chapkit_rwanda_malaria_bym_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "warning: nothing could say what \
             ghcr.io/chap-models/chapkit_rwanda_malaria_bym_model:{} runs as",
            stable_pin("chapkit_rwanda_malaria_bym_model").1
        )))
        .stdout(predicates::str::contains("the built-in table"))
        .stdout(predicates::str::contains("chaps models enable"));

    // The table's own answer for that image: root, which is the whole point.
    let model = &state(&dir)["models"]["chapkit_rwanda_malaria_bym_model"];
    assert_eq!(model["user"], "root");
    assert_eq!(model["user_from"], "table");
    let overlay = yaml(&dir.join("compose.chapkit-rwanda-malaria-bym-model.yml"));
    let service = &overlay["services"]["chapkit-rwanda-malaria-bym-model"];
    assert!(
        service.get("user").is_none(),
        "the model service overrides nothing"
    );
    assert_eq!(
        overlay["services"]["chapkit-rwanda-malaria-bym-model-init"]["command"][2].as_str(),
        Some("chown -R 0:0 /work/data")
    );
}

#[test]
fn models_add_from_a_repository_pins_the_newest_published_build() {
    let (sandbox, dir, port) = added_sandbox(Hub::new());
    sandbox
        .online(port)
        .args(["models", "add", REPO_URL])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "added chapkit_example_manual_model (chapkit-example-manual-model)",
        ))
        // The newest commit has no published build, so the pin lands on the
        // newest build there is.
        .stdout(predicates::str::contains(format!("pin       {OLD_TAG}")))
        .stdout(predicates::str::contains("follows   main"))
        .stdout(predicates::str::contains(
            "data dir  /work/data  (from the image config)",
        ))
        .stdout(predicates::str::contains(
            "user      10001:10001  (from the image config)",
        ))
        .stdout(predicates::str::contains("must register with chap-core as"))
        .stdout(predicates::str::contains("run `chaps up` to apply"));

    // The definition, in a file of its own.
    let manual = manual_models(&dir);
    let entry = &manual["chapkit_example_manual_model"];
    assert_eq!(entry["service_id"], "chapkit-example-manual-model");
    assert_eq!(entry["image"], IMAGE);
    assert_eq!(entry["tag"], OLD_TAG);
    assert_eq!(entry["commit"], OLD_SHA);
    assert_eq!(entry["follow"], "main");
    assert_eq!(entry["repository"], REPO_URL);
    assert_eq!(entry["runtime_amd64"], true);
    assert_eq!(entry["added"].as_str().map(str::len), Some(10));

    // And the enablement, in the file every other model uses.
    let enabled = &state(&dir)["models"]["chapkit_example_manual_model"];
    assert_eq!(enabled["image_tag"], OLD_TAG);
    assert_eq!(enabled["version"], OLD_TAG);
    assert_eq!(enabled["channel"], "latest");
    assert_eq!(enabled["host_port"], Json::Null);
    assert_eq!(enabled["data_dir"], "/work/data");
    assert_eq!(enabled["user"], "10001:10001");
    assert_eq!(
        enabled["compose_file"],
        "compose.chapkit-example-manual-model.yml"
    );

    // The overlay is rendered like any other model's, chown and all.
    let overlay = read(&dir.join("compose.chapkit-example-manual-model.yml"));
    assert!(
        overlay.contains(&format!(
            "image: {IMAGE}:${{CHAPKIT_EXAMPLE_MANUAL_MODEL_IMAGE_TAG:-{OLD_TAG}}}"
        )),
        "{overlay}"
    );
    assert!(
        overlay.contains("chown -R 10001:10001 /work/data"),
        "{overlay}"
    );
    assert!(overlay.contains("user: 10001:10001"), "{overlay}");
    assert_eq!(
        includes(&dir),
        vec!["compose.chapkit-example-manual-model.yml"]
    );
    assert!(
        sandbox.env().contains(&format!(
            "# CHAPKIT_EXAMPLE_MANUAL_MODEL_IMAGE_TAG={OLD_TAG}"
        )),
        "{}",
        sandbox.env()
    );
}

#[test]
fn a_manually_added_model_is_marked_in_the_list_and_on_its_page() {
    let (sandbox, dir, port) = added_sandbox(Hub::new());
    sandbox
        .online(port)
        .args(["models", "add", REPO_URL])
        .assert()
        .success();

    // The kind column appears because there is something to tell apart, and
    // the marketplace row keeps its own word for what it is.
    sandbox
        .online(port)
        .args(["models", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("KIND"))
        .stdout(predicates::str::contains("manual"))
        .stdout(predicates::str::contains("chapkit_ewars_model"));

    let out = sandbox
        .online(port)
        .args(["--json", "models", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let rows: Json = serde_json::from_slice(&out).expect("models list --json is one document");
    let manual = rows
        .as_array()
        .expect("an array")
        .iter()
        .find(|row| row["id"] == "chapkit_example_manual_model")
        .expect("the added model is listed");
    assert_eq!(manual["manual"], true);
    assert_eq!(manual["enabled"], true);

    // The page says where it came from and what it follows.
    let date = manual_models(&dir)["chapkit_example_manual_model"]["added"]
        .as_str()
        .expect("a date")
        .to_string();
    sandbox
        .online(port)
        .args(["models", "info", "chapkit_example_manual_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains("kind        manual"))
        .stdout(predicates::str::contains(format!(
            "source      manual (`chaps models add`, {date})"
        )))
        .stdout(predicates::str::contains("follows     main"))
        .stdout(predicates::str::contains(format!(
            "image       {IMAGE}:{OLD_TAG}"
        )));
}

#[test]
fn update_moves_a_following_manual_model_when_a_newer_build_appears() {
    let (sandbox, _dir, port) = added_sandbox(Hub::new());
    sandbox
        .online(port)
        .args(["models", "add", REPO_URL])
        .assert()
        .success();

    // The same hub: the branch has published nothing new.
    sandbox
        .online(port)
        .args(["update", "--dry-run"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "chapkit_example_manual_model  {OLD_TAG}  unchanged"
        )))
        .stdout(predicates::str::contains("already up to date"));

    // And once the newer commit is published, the pin would move to it.
    let advanced = Hub::new().advanced().start();
    sandbox
        .online(advanced)
        .args(["update", "--dry-run"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "chapkit_example_manual_model  {OLD_TAG} -> {NEW_TAG}"
        )))
        .stdout(predicates::str::contains("would update 1 model pin"));

    // A repository that will not answer leaves the pin alone and says that
    // nothing was established, rather than that nothing moved. The registry
    // itself still answers: `update` has no fallback for that one.
    sandbox
        .online(port)
        .env(
            "CHAPS_GITHUB_API",
            format!("http://127.0.0.1:{}", free_port()),
        )
        .args(["update", "--dry-run"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "chapkit_example_manual_model  {OLD_TAG}  unchanged (could not check)"
        )))
        .stderr(predicates::str::contains("could not check"));
}

#[test]
fn models_add_from_an_image_reference_is_pinned() {
    let (sandbox, dir, port) = added_sandbox(Hub::new());
    sandbox
        .online(port)
        .args(["models", "add", &format!("{IMAGE}:{NEW_TAG}")])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!("pin       {NEW_TAG}")))
        .stdout(predicates::str::contains("follows   nothing (pinned)"));

    let entry = &manual_models(&dir)["chapkit_example_manual_model"];
    assert_eq!(entry["tag"], NEW_TAG);
    assert_eq!(entry["follow"], Json::Null);
    assert_eq!(entry["repository"], Json::Null);
    assert_eq!(entry["commit"], Json::Null);
    // An exact pin follows no channel, and `update` says so.
    assert_eq!(
        state(&dir)["models"]["chapkit_example_manual_model"]["channel"],
        Json::Null
    );
    sandbox
        .online(Hub::new().advanced().start())
        .args(["update", "--dry-run"])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!(
            "chapkit_example_manual_model  {NEW_TAG}  pinned, skipped"
        )))
        .stdout(predicates::str::contains("already up to date"));
}

#[test]
fn models_add_accepts_a_digest_and_renders_a_digest_reference() {
    let (sandbox, dir, port) = added_sandbox(Hub::new());
    let digest = format!("sha256:{}", "a".repeat(64));
    sandbox
        .online(port)
        .args(["models", "add", &format!("{IMAGE}@{digest}")])
        .assert()
        .success()
        .stdout(predicates::str::contains(format!("pin       @{digest}")));

    assert_eq!(
        manual_models(&dir)["chapkit_example_manual_model"]["tag"],
        format!("@{digest}")
    );
    // A digest carries its own separator, so the rendered reference has no
    // colon in front of it.
    let overlay = read(&dir.join("compose.chapkit-example-manual-model.yml"));
    assert!(
        overlay.contains(&format!(
            "image: {IMAGE}${{CHAPKIT_EXAMPLE_MANUAL_MODEL_IMAGE_TAG:-@{digest}}}"
        )),
        "{overlay}"
    );
    assert!(!overlay.contains(&format!("{IMAGE}:@")), "{overlay}");
}

#[test]
fn models_add_keeps_a_user_it_cannot_resolve_and_says_what_it_will_chown() {
    let (sandbox, dir, port) = added_sandbox(Hub::new().running_as("app"));
    sandbox
        .online(port)
        .args(["models", "add", REPO_URL])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "user      app  (from the image config)",
        ))
        // The note says what the init container will do instead, and how to
        // decide it properly.
        .stdout(predicates::str::contains("`app` has no uid this CLI knows"))
        .stdout(predicates::str::contains("--user <uid>:<gid>"))
        .stdout(predicates::str::contains("chown 1000:1000").not());

    let overlay = read(&dir.join("compose.chapkit-example-manual-model.yml"));
    assert!(overlay.contains("user: app"), "{overlay}");
    assert!(
        overlay.contains("chown -R 1000:1000 /work/data"),
        "{overlay}"
    );

    // And `--user` is what settles it.
    sandbox
        .online(port)
        .args([
            "models",
            "add",
            REPO_URL,
            "--id",
            "second_manual_model",
            "--service-id",
            "second-manual-model",
            "--user",
            "10001:10001",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "user      10001:10001  (given on the command line)",
        ));
    assert!(
        read(&dir.join("compose.second-manual-model.yml"))
            .contains("chown -R 10001:10001 /work/data"),
        "the flag reaches the init container"
    );
}

#[test]
fn models_add_from_a_repository_refuses_to_run_offline() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "none"]).assert().success();
    sandbox
        .models(&["add", REPO_URL])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--offline"))
        .stderr(predicates::str::contains(IMAGE));
    assert_eq!(manual_models(&sandbox.project()), Json::Null);
}

#[test]
fn models_add_refuses_a_name_the_marketplace_or_this_project_holds() {
    let (sandbox, dir, port) = added_sandbox(Hub::new());
    // An id the catalogue lists is a model to enable, not one to add.
    sandbox
        .online(port)
        .args(["models", "add", REPO_URL, "--id", "chapkit_ewars_model"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "the marketplace already lists chapkit_ewars_model",
        ))
        .stderr(predicates::str::contains("chaps models enable"));
    // And a compose service it already uses would be two overlays fighting.
    sandbox
        .online(port)
        .args([
            "models",
            "add",
            REPO_URL,
            "--service-id",
            "chapkit-ewars-model",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--service-id"));
    assert_eq!(manual_models(&dir), Json::Null, "nothing was written");

    // Adding the same source twice needs a name of its own.
    sandbox
        .online(port)
        .args(["models", "add", REPO_URL])
        .assert()
        .success();
    sandbox
        .online(port)
        .args(["models", "add", REPO_URL])
        .assert()
        .failure()
        .stderr(predicates::str::contains("was already added"))
        .stderr(predicates::str::contains("chaps models remove"));

    // A bare image name is neither form.
    sandbox
        .online(port)
        .args(["models", "add", "chapkit_example_manual_model"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("neither a repository URL"));
}

#[test]
fn models_remove_takes_the_definition_and_the_overlay_with_it() {
    let (sandbox, dir, port) = added_sandbox(Hub::new());
    sandbox
        .online(port)
        .args(["models", "add", REPO_URL])
        .assert()
        .success();
    assert!(
        dir.join("compose.chapkit-example-manual-model.yml")
            .is_file()
    );

    // Offline: the definition is the deployment's own, so removing it needs
    // nothing from the network.
    sandbox
        .models(&["remove", "chapkit-example-manual-model"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "removed chapkit_example_manual_model (models-manual.yaml)",
        ))
        .stdout(predicates::str::contains(
            "disabled chapkit_example_manual_model",
        ))
        .stdout(predicates::str::contains(
            "removed compose.chapkit-example-manual-model.yml",
        ));

    assert!(
        !dir.join("compose.chapkit-example-manual-model.yml")
            .exists()
    );
    assert!(includes(&dir).is_empty());
    let manual = read(&dir.join(".chaps").join("models-manual.yaml"));
    assert!(!manual.contains("chapkit_example_manual_model"), "{manual}");
    assert_eq!(state(&dir)["models"], serde_json::json!({}));

    // A marketplace model has no local definition to remove.
    sandbox
        .online(port)
        .args(["models", "remove", "chapkit_ewars_model"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("is a marketplace model"))
        .stderr(predicates::str::contains("chaps models disable"));
    // And a name nothing answers to is the typo it looks like.
    sandbox
        .models(&["remove", "nope"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("unknown model `nope`"));
}

#[test]
fn a_manual_model_can_be_disabled_and_enabled_again_at_the_recorded_tag() {
    let (sandbox, dir, port) = added_sandbox(Hub::new());
    sandbox
        .online(port)
        .args(["models", "add", REPO_URL])
        .assert()
        .success();

    // Disabling keeps the definition, which is what makes it reversible.
    sandbox
        .models(&["disable", "chapkit_example_manual_model"])
        .assert()
        .success();
    assert_eq!(state(&dir)["models"], serde_json::json!({}));
    assert_eq!(
        manual_models(&dir)["chapkit_example_manual_model"]["tag"],
        OLD_TAG,
        "the definition outlives the enablement"
    );

    // And enabling it again needs no network at all: the deployment is where
    // its definition lives.
    sandbox
        .models(&["enable", "chapkit_example_manual_model"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "enabled chapkit_example_manual_model",
        ));
    let enabled = &state(&dir)["models"]["chapkit_example_manual_model"];
    assert_eq!(enabled["image_tag"], OLD_TAG);
    assert_eq!(enabled["data_dir"], "/work/data");
    assert_eq!(enabled["user"], "10001:10001");
    assert!(
        dir.join("compose.chapkit-example-manual-model.yml")
            .is_file()
    );

    // A forced re-init carries the definition over rather than dropping it.
    sandbox
        .init(&["--models", "none", "--force"])
        .assert()
        .success();
    assert_eq!(
        manual_models(&dir)["chapkit_example_manual_model"]["tag"],
        OLD_TAG
    );
}

#[test]
fn models_add_outside_a_project_says_so() {
    let sandbox = Sandbox::new();
    let port = Hub::new().start();
    sandbox
        .online(port)
        .args(["models", "add", REPO_URL])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not a chaps project"));
}

// --------------------------------------------------------------- jobs/api ---

/// The three jobs the stand-in chap-core reports, in the order it sends them:
/// chap-core answers newest first already, and `chaps jobs` is what decides
/// the order the table is in.
const DONE_ID: &str = "11111111-aaaa-4aaa-8aaa-000000000001";
const FAILED_ID: &str = "22222222-bbbb-4bbb-8bbb-000000000002";
const RUNNING_ID: &str = "33333333-cccc-4ccc-8ccc-000000000003";

/// A job log shaped like the real one: chap-core's own lines, then the model's
/// output, then the `--- stderr ---` section where the reason is.
const JOB_LOG: &str = "\
2026-09-24 16:28:19,440 [INFO] chap_status: Starting backtest for model '15'
Fitting INLA model...

--- stderr ---
Loading required package: Matrix
Error in predict_chap() : the inla program crashed
In addition: Warning messages:
1: In poly2nb(polygons, queen = FALSE) :
  some observations have no neighbours
Execution halted
";

/// The `/v1/jobs` payload, in chap-core's snake_case with its unquoted
/// timestamps.
fn job_list() -> String {
    format!(
        r#"[
  {{"id":"{RUNNING_ID}","type":"create_prediction","name":"eval-live",
    "status":"STARTED","start_time":"2026-09-24T16:34:12.911182",
    "end_time":null,"result":null,"prediction_setup_id":1}},
  {{"id":"{FAILED_ID}","type":"create_backtest","name":"eval-bym",
    "status":"FAILURE","start_time":"2026-09-24T16:28:19.437116",
    "end_time":"2026-09-24T16:28:29.686103","result":null,
    "prediction_setup_id":null}},
  {{"id":"{DONE_ID}","type":"create_backtest","name":"eval-ewars",
    "status":"SUCCESS","start_time":"2026-09-24T16:25:17.007503",
    "end_time":"2026-09-24T16:25:58.351174","result":"3",
    "prediction_setup_id":null}}
]"#
    )
}

/// A stand-in for chap-core's job and CRUD endpoints, on `port`.
///
/// Answers the paths `chaps jobs` and `chaps api` reach, and nothing else:
/// the point is the shapes - a bare JSON string for a status and for a log, a
/// `detail` object for a 404, a `message` for a cancel - because those are
/// what the commands render. The thread lives as long as the test process.
fn chap_core_server(port: u16) {
    use std::io::Write;

    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).expect("a free port");
    std::thread::spawn(move || {
        // What this server was asked to do, for the tests that check the
        // cleanup: one server per port, so the record is this thread's own.
        let mut recorded = Recorded::default();
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            // The request has to be read before the answer, or the client
            // sees a reset instead of the response.
            let request = read_request(&mut stream);
            let mut head = request.lines().next().unwrap_or("").split_whitespace();
            let method = head.next().unwrap_or("GET").to_string();
            let path = head.next().unwrap_or("/").to_string();
            let authed = request.to_lowercase().contains("authorization: bearer ");
            let body = request
                .split_once("\r\n\r\n")
                .map(|(_, rest)| rest.to_string())
                .unwrap_or_default();

            let (status, content_type, payload) =
                chap_core_route(&method, &path, authed, &body, &mut recorded);
            let reason = match status {
                200 => "OK",
                400 => "Bad Request",
                401 => "Unauthorized",
                _ => "Not Found",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
}

/// The three models the `models test` stand-in knows, and what each one is
/// there to cover: a model that backtests cleanly, one whose prediction
/// crashes, and one whose chapkit predates `$generate-sample-data`.
const PASSING_MODEL: &str = "chapkit-ewars-model";
const FAILING_MODEL: &str = "chapkit-rwanda-malaria-bym-model";
const OLD_CHAPKIT_MODEL: &str = "auto-arima-chapkit";

/// The dataset and backtest rows the stand-in says the jobs wrote, which are
/// the two rows the cleanup has to delete and nothing else.
const TEST_DATASET: i64 = 41;
const TEST_BACKTEST: i64 = 7;

/// The config and the artifact `chapkit test` leaves in a model service's own
/// database, which the model level has to find and delete.
const TEST_CONFIG: &str = "01M3A4TSB2YHRPFBEFCT4TMX4A";
const TEST_ARTIFACT: &str = "01M3A4TSB9WZ8V5XN0A4F4Y1M6";

/// `GET /v1/crud/backtests/7`, cut to the field `models test` reads.
const BACKTEST_ROW: &str = r#"{"id":7,"datasetId":41,"modelId":"chapkit-ewars-model",
  "aggregateMetrics":{"ratio_above_truth":0.45,"crps":20.04,"mae":28.96,"rmse":34.88,
  "mape":12.98,"coverage_10_90":0.6}}"#;

/// The configured model a backtest of the passing model has to name.
const PASSING_CONFIGURED: i64 = 20;

/// What chap-core has configured, as `(id, name, archived)`.
///
/// The passing model is in the state that makes the id necessary: it has
/// re-registered with a new version, so the row it first got - the one named
/// plainly after the service - is archived, and what is live are the configs
/// chap-core synced out of the service itself, named `<service>:<config>`.
/// One of those is what a previous `chapkit test` left behind, and picking
/// that one would be backtesting a test's leftovers.
fn configured_models() -> Vec<(i64, String, bool)> {
    vec![
        (14, PASSING_MODEL.to_string(), true),
        (
            19,
            format!("{PASSING_MODEL}:test_config_{TEST_CONFIG}"),
            false,
        ),
        (
            PASSING_CONFIGURED,
            format!("{PASSING_MODEL}:{PASSING_MODEL}_1790267117082761"),
            false,
        ),
        // The other two are as a service looks the day it registers.
        (15, FAILING_MODEL.to_string(), false),
        (16, OLD_CHAPKIT_MODEL.to_string(), false),
    ]
}

/// `GET /v1/crud/configured-models`, with the fields chap-core carries.
fn configured_models_payload() -> String {
    let rows: Vec<Json> = configured_models()
        .into_iter()
        .map(|(id, name, archived)| {
            serde_json::json!({
                "id": id,
                "name": name,
                "version": "1.0.1",
                "archived": archived,
                "usesChapkit": true,
                "sourceDigest": "f9a1c0d",
            })
        })
        .collect();
    serde_json::to_string(&rows).expect("JSON")
}

/// The service a `create-backtest` body is about, from either spelling of
/// `modelId`: the integer key of a configured model, or a service id.
fn backtest_service(model_id: &Json) -> String {
    if let Some(name) = model_id.as_str() {
        return name.to_string();
    }
    let wanted = model_id.as_i64().unwrap_or_default();
    configured_models()
        .into_iter()
        .find(|(id, ..)| *id == wanted)
        .map(|(_, name, _)| match name.split_once(':') {
            Some((service, _)) => service.to_string(),
            None => name,
        })
        .unwrap_or_default()
}

/// `GET /v2/services/...` and everything under its proxy.
fn services_route(
    method: &str,
    route: &str,
    recorded: &mut Recorded,
) -> Option<(u16, &'static str, String)> {
    let json = "application/json";
    let missing = || Some((404, json, r#"{"detail":"Not Found"}"#.to_string()));
    let rest = route.strip_prefix("/v2/services/")?;
    if method != "GET" {
        return missing();
    }
    let (service, tail) = match rest.split_once('/') {
        Some(parts) => parts,
        None => (rest, ""),
    };
    if ![PASSING_MODEL, FAILING_MODEL, OLD_CHAPKIT_MODEL].contains(&service) {
        return Some((
            404,
            json,
            format!(r#"{{"detail":"Service '{service}' not found"}}"#),
        ));
    }
    if tail.is_empty() {
        // The `info` block a chapkit service registers with, cut to the three
        // fields `models test` reads off it.
        let chapkit = if service == OLD_CHAPKIT_MODEL {
            "1.0.0"
        } else {
            "2.0.0"
        };
        let geo = service == FAILING_MODEL;
        return Some((
            200,
            json,
            format!(
                r#"{{"id":"{service}","url":"http://{service}:8000","info":{{"id":"{service}",
                   "period_type":"monthly","required_covariates":["population"],
                   "requires_geo":{geo},"chapkit_version":"{chapkit}"}}}}"#
            ),
        ));
    }
    if tail.contains("generate-sample-data") {
        recorded.sampled.push(service.to_string());
        if service == OLD_CHAPKIT_MODEL {
            return missing();
        }
        // The passing model stands in for a chapkit that answered without a
        // `geo` at all, which is what the null-geometry fallback is for.
        return Some((200, json, sample_payload(service == FAILING_MODEL)));
    }
    // The model level lists these before the run and again after it, and
    // deletes the difference; the second listing is therefore the one that
    // has to carry what `chapkit test` left behind.
    if tail.ends_with("api/v1/configs") {
        recorded.config_lists += 1;
        let body = if recorded.config_lists == 2 {
            format!(
                r#"[{{"id":"{TEST_CONFIG}","created_at":"2026-09-24T16:43:57",
                   "name":"test_config_{TEST_CONFIG}","data":{{}}}}]"#
            )
        } else {
            "[]".to_string()
        };
        return Some((200, json, body));
    }
    if tail.ends_with("api/v1/artifacts") {
        recorded.artifact_lists += 1;
        // Gone again on the third listing: chapkit cascades a config delete
        // to the artifact trees linked to it.
        let body = if recorded.artifact_lists == 2 {
            format!(r#"[{{"id":"{TEST_ARTIFACT}","data":{{"type":"ml_training_workspace"}}}}]"#)
        } else {
            "[]".to_string()
        };
        return Some((200, json, body));
    }
    missing()
}

/// What `$generate-sample-data?kind=train` answers with.
///
/// The shape rather than the size: a column-oriented frame, and - for a model
/// that needs geometry - a `geo` whose features carry their location id in
/// `properties` and nothing at the top level, which is exactly the thing the
/// CLI has to fix before chap-core will match an org unit to one.
fn sample_payload(with_geo: bool) -> String {
    let mut columns = vec![
        "time_period",
        "location",
        "disease_cases",
        "population",
        "rainfall",
        "mean_temperature",
    ];
    if with_geo {
        columns.push("relative_humidity");
    }
    let mut rows: Vec<Json> = Vec::new();
    for period in ["2020-01", "2020-02", "2020-03"] {
        for (at, location) in ["location_0", "location_1"].iter().enumerate() {
            let mut row = vec![Json::from(period), Json::from(*location)];
            for n in 0..columns.len() - 2 {
                row.push(Json::from((at + n + 1) as f64 * 1.5));
            }
            rows.push(Json::Array(row));
        }
    }
    let mut payload = serde_json::json!({"data": {"columns": columns, "data": rows}});
    if with_geo {
        payload["geo"] = serde_json::json!({
            "type": "FeatureCollection",
            "features": [
                {"type": "Feature", "geometry": {"type": "Point", "coordinates": [0.0, 0.0]},
                 "properties": {"id": "location_0"}},
                {"type": "Feature", "geometry": {"type": "Point", "coordinates": [1.0, 1.0]},
                 "properties": {"id": "location_1"}},
            ],
        });
    }
    payload.to_string()
}

/// `make-dataset`, `create-backtest`, the CRUD rows and the deletes.
fn analytics_route(
    method: &str,
    route: &str,
    body: &str,
    recorded: &mut Recorded,
) -> Option<(u16, &'static str, String)> {
    let json = "application/json";
    match (method, route) {
        ("POST", "/v1/analytics/make-dataset") => {
            let sent: Json = serde_json::from_str(body).unwrap_or(Json::Null);
            recorded.observations = sent["providedData"].as_array().map(Vec::len).unwrap_or(0);
            let name = sent["name"].as_str().unwrap_or_default().to_string();
            // chap-core drops every org unit whose feature has no top-level
            // `id`, so a geojson without them imports nothing at all - which
            // is what makes this the check that the CLI sets them.
            let imported = sent["geojson"]["features"]
                .as_array()
                .map(|features| features.iter().filter(|f| f["id"].is_string()).count())
                .unwrap_or(0);
            let service = dataset_service(&name);
            recorded.datasets.push(name);
            Some((
                200,
                json,
                format!(r#"{{"id":"ds-{service}","importedCount":{imported},"rejected":[]}}"#),
            ))
        }
        ("POST", "/v1/analytics/create-backtest") => {
            let sent: Json = serde_json::from_str(body).unwrap_or(Json::Null);
            recorded.backtests.push(sent.clone());
            let service = backtest_service(&sent["modelId"]);
            Some((200, json, format!(r#"{{"id":"bt-{service}"}}"#)))
        }
        // Not chap-core's: how a test asks this server what it was asked.
        ("GET", "/v1/recorded") => Some((
            200,
            json,
            serde_json::json!({
                "deleted": recorded.deleted,
                "observations": recorded.observations,
                "datasets": recorded.datasets,
                "sampled": recorded.sampled,
                "backtests": recorded.backtests,
            })
            .to_string(),
        )),
        ("DELETE", _) if route.starts_with("/v1/crud/") => {
            recorded.deleted.push(route.to_string());
            Some((200, json, r#"{"message":"deleted"}"#.to_string()))
        }
        ("GET", "/v1/crud/configured-models") => Some((200, json, configured_models_payload())),
        ("GET", _) if route == format!("/v1/crud/backtests/{TEST_BACKTEST}") => {
            Some((200, json, BACKTEST_ROW.to_string()))
        }
        _ => None,
    }
}

/// The service a `chaps-test-<service>-<stamp>` dataset name is about.
fn dataset_service(name: &str) -> String {
    let rest = name.strip_prefix("chaps-test-").unwrap_or(name);
    // The stamp is the last two hyphen-separated parts: `20260924-181500`.
    let parts: Vec<&str> = rest.rsplitn(3, '-').collect();
    parts.get(2).copied().unwrap_or(rest).to_string()
}

/// One whole HTTP request, headers and body.
///
/// A single `read` is enough for the requests `chaps jobs` makes and nowhere
/// near enough for `make-dataset`, which is a few thousand observations, so
/// the headers are read first and then exactly as much body as
/// `Content-Length` promises.
fn read_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read;
    let mut raw: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        if let Some(at) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&raw[..at]).to_lowercase();
            let want: usize = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0);
            if raw.len() >= at + 4 + want {
                break;
            }
        }
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => raw.extend_from_slice(&buffer[..read]),
        }
    }
    String::from_utf8_lossy(&raw).into_owned()
}

/// What one stand-in chap-core was asked to do, so a test can check that the
/// cleanup happened and that what went out was the right shape.
#[derive(Debug, Default)]
struct Recorded {
    /// Every path a `DELETE` reached, in order.
    deleted: Vec<String>,
    /// Observations in the last `make-dataset` body.
    observations: usize,
    /// The `name` of every dataset that was asked for.
    datasets: Vec<String>,
    /// The services whose sample data was fetched.
    sampled: Vec<String>,
    /// Every `create-backtest` body, whole.
    backtests: Vec<Json>,
    /// How many times the configs and the artifacts have been listed, so the
    /// second listing can be the one with the test's leftovers in it.
    config_lists: usize,
    artifact_lists: usize,
}

/// The answer one request gets: `(status, content type, body)`.
fn chap_core_route(
    method: &str,
    path: &str,
    authed: bool,
    body: &str,
    recorded: &mut Recorded,
) -> (u16, &'static str, String) {
    let json = "application/json";
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let status_of = |id: &str| match id {
        DONE_ID => Some("SUCCESS"),
        FAILED_ID => Some("FAILURE"),
        RUNNING_ID => Some("STARTED"),
        // The backtest of the model whose prediction crashes is the one job
        // `chaps models test --backtest` has to report a reason for.
        id if id == format!("bt-{FAILING_MODEL}") => Some("FAILURE"),
        id if id.starts_with("ds-") || id.starts_with("bt-") => Some("SUCCESS"),
        _ => None,
    };
    if let Some(answer) = services_route(method, route, recorded) {
        return answer;
    }
    if let Some(answer) = analytics_route(method, route, body, recorded) {
        return answer;
    }

    if route == "/v1/jobs" && method == "GET" {
        // The `status` query parameter is the one filter the CLI sends.
        let wanted: Vec<&str> = query
            .split('&')
            .filter_map(|pair| pair.strip_prefix("status="))
            .collect();
        if wanted.is_empty() {
            return (200, json, job_list());
        }
        let list: Json = serde_json::from_str(&job_list()).expect("the list is JSON");
        let kept: Vec<Json> = list
            .as_array()
            .expect("a list")
            .iter()
            .filter(|job| {
                let status = job["status"].as_str().unwrap_or_default();
                wanted.iter().any(|w| w.eq_ignore_ascii_case(status))
            })
            .cloned()
            .collect();
        return (200, json, serde_json::to_string(&kept).expect("JSON"));
    }
    if let Some(rest) = route.strip_prefix("/v1/jobs/") {
        let (id, tail) = match rest.split_once('/') {
            Some((id, tail)) => (id, tail),
            None => (rest, ""),
        };
        let Some(status) = status_of(id) else {
            return (404, json, format!(r#"{{"detail":"Job '{id}' not found"}}"#));
        };
        return match (method, tail) {
            ("GET", "") => (200, json, format!(r#""{status}""#)),
            // The log is one JSON string, which is what makes reading it a
            // question of rendering rather than of parsing.
            ("GET", "logs") => (
                200,
                json,
                serde_json::to_string(JOB_LOG).expect("a JSON string"),
            ),
            ("GET", "database_result") if status == "SUCCESS" => {
                let row = if id.starts_with("ds-") {
                    TEST_DATASET
                } else if id.starts_with("bt-") {
                    TEST_BACKTEST
                } else {
                    4
                };
                (200, json, format!(r#"{{"id":{row}}}"#))
            }
            ("GET", "database_result") => {
                (400, json, r#"{"detail":"Job is not finished"}"#.to_string())
            }
            ("POST", "cancel") => (200, json, r#"{"message":"Job cancelled"}"#.to_string()),
            ("DELETE", "") if status == "STARTED" => (
                400,
                json,
                r#"{"detail":"Cannot delete a running job"}"#.to_string(),
            ),
            ("DELETE", "") => (200, json, r#"{"message":"Job deleted"}"#.to_string()),
            _ => (404, json, r#"{"detail":"Not Found"}"#.to_string()),
        };
    }
    // A handful of paths for `chaps api` itself: an echo for a body, a
    // non-JSON body, and one that reports whether a token arrived.
    match (method, route) {
        ("POST", "/v1/echo") | ("PUT", "/v1/echo") | ("PATCH", "/v1/echo") => {
            (200, json, format!(r#"{{"received":{body}}}"#))
        }
        ("GET", "/v1/text") => (200, "text/plain", "plain text, not JSON\n".to_string()),
        ("GET", "/v1/whoami") => (200, json, format!(r#"{{"auth":{authed}}}"#)),
        ("GET", "/v1/empty") => (200, json, String::new()),
        _ => (404, json, r#"{"detail":"Not Found"}"#.to_string()),
    }
}

/// A project whose API port is the one the stand-in chap-core listens on.
fn served_project(sandbox: &Sandbox) -> PathBuf {
    let port = free_port();
    sandbox
        .init(&["--models", "none", "--api-port", &port.to_string()])
        .assert()
        .success();
    chap_core_server(port);
    sandbox.project()
}

#[test]
fn jobs_lists_what_chap_core_has_run_newest_first() {
    let sandbox = Sandbox::new();
    let dir = served_project(&sandbox);

    let out = chap_in(&sandbox, &dir, &["jobs"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).expect("the table is text");

    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with("ID  "), "{text}");
    for header in ["TYPE", "NAME", "STATUS", "STARTED", "DURATION"] {
        assert!(lines[0].contains(header), "{header} is missing:\n{text}");
    }
    // Newest first, and the ids are short because eight characters tell these
    // three apart.
    assert!(lines[1].starts_with("33333333..."), "{text}");
    assert!(lines[2].starts_with("22222222..."), "{text}");
    assert!(lines[3].starts_with("11111111..."), "{text}");
    assert!(
        !text.contains(RUNNING_ID),
        "the full id is not needed:\n{text}"
    );

    // A running job has no duration; a finished one has the time it took.
    assert!(lines[1].trim_end().ends_with('-'), "{text}");
    assert!(lines[2].trim_end().ends_with("10s"), "{text}");
    assert!(lines[3].trim_end().ends_with("41s"), "{text}");
    // And the STARTED column is a relative time.
    let ago = regex::Regex::new(r"\d+[smhd] ago").expect("a valid pattern");
    assert!(ago.is_match(lines[1]), "{text}");

    // The line it adds up to, and the one thing to do about it.
    assert!(
        text.contains("3 jobs: 1 running, 1 done, 1 failed"),
        "{text}"
    );
    assert!(
        text.contains(&format!("run `chaps jobs logs {FAILED_ID}` to see why")),
        "{text}"
    );
}

#[test]
fn jobs_filters_limits_and_hands_back_the_raw_list_as_json() {
    let sandbox = Sandbox::new();
    let dir = served_project(&sandbox);

    // `--status` is a query parameter chap-core applies, and the closing line
    // counts what came back.
    let text = chap_in(&sandbox, &dir, &["jobs", "list", "--status", "success"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(text).expect("text");
    assert!(text.contains("eval-ewars"), "{text}");
    assert!(!text.contains("eval-bym"), "{text}");
    assert!(text.contains("1 job: 1 done"), "{text}");

    // `--limit` is applied after the ordering, so it is the newest N.
    let text = chap_in(&sandbox, &dir, &["jobs", "--limit", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(text).expect("text");
    assert!(text.contains("eval-live"), "{text}");
    assert!(!text.contains("eval-ewars"), "{text}");

    // `--json` is chap-core's own documents, in the table's order.
    let list = json_of(&mut chap_in(&sandbox, &dir, &["--json", "jobs"]));
    assert_eq!(list[0]["id"], RUNNING_ID);
    assert_eq!(list[1]["id"], FAILED_ID);
    assert_eq!(list[2]["id"], DONE_ID);
    // Fields this CLI never renders survive the round trip.
    assert_eq!(list[0]["prediction_setup_id"], 1);
    assert_eq!(list[2]["result"], "3");
}

#[test]
fn jobs_says_where_a_job_would_come_from_when_there_are_none() {
    let sandbox = Sandbox::new();
    // A chap-core that has run nothing answers with an empty list, and the
    // deployment is pointed at the port it answers on.
    let empty = server("application/json", "[]");
    let dir = sandbox.home.path().join("chaps-empty");
    let mut init = sandbox.chap();
    init.arg("init")
        .arg(&dir)
        .args(["--models", "none", "--api-port", &empty.to_string()]);
    init.assert().success();

    chap_in(&sandbox, &dir, &["jobs"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "no jobs yet; a backtest or prediction started from the Modeling App",
        ));
}

#[test]
fn jobs_logs_prints_the_log_on_stdout_and_the_reason_on_stderr() {
    let sandbox = Sandbox::new();
    let dir = served_project(&sandbox);

    let assert = chap_in(&sandbox, &dir, &["jobs", "logs", FAILED_ID])
        .assert()
        .success();
    let out = assert.get_output();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // stdout is the log and nothing else, so it can be piped.
    assert!(stdout.contains("Starting backtest for model"), "{stdout}");
    assert!(stdout.contains("--- stderr ---"), "{stdout}");
    assert!(!stdout.contains("job 22222222"), "{stdout}");

    // stderr says which job it is, and what the stderr section blames.
    assert!(
        stderr.contains(&format!(
            "job {FAILED_ID} FAILURE (create_backtest eval-bym)"
        )),
        "{stderr}"
    );
    assert!(
        stderr.contains("stderr: Error in predict_chap() : the inla program crashed"),
        "{stderr}"
    );

    // `--tail` cuts the log from the end, and keeps the hint.
    let assert = chap_in(&sandbox, &dir, &["jobs", "logs", FAILED_ID, "--tail", "2"])
        .assert()
        .success();
    let out = assert.get_output();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.lines().count(), 2, "{stdout}");
    assert!(stdout.contains("Execution halted"), "{stdout}");
    assert!(!stdout.contains("Starting backtest"), "{stdout}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("stderr: Error in predict_chap()"),
        "the hint is read from the whole log, not from the tail"
    );

    // A job that worked gets no hint at all.
    let assert = chap_in(&sandbox, &dir, &["jobs", "logs", DONE_ID])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("SUCCESS"), "{stderr}");
    assert!(!stderr.contains("stderr:"), "{stderr}");
}

#[test]
fn jobs_takes_an_id_prefix_and_says_when_it_matches_nothing() {
    let sandbox = Sandbox::new();
    let dir = served_project(&sandbox);

    // Eight characters is what the table printed, so eight characters work.
    chap_in(&sandbox, &dir, &["jobs", "show", "11111111"])
        .assert()
        .success()
        .stdout(predicates::str::contains(DONE_ID))
        .stdout(predicates::str::contains("Database result  4"))
        .stdout(predicates::str::contains("row 4 in chap-core's database"));

    // `-v` says which job the prefix landed on.
    chap_in(&sandbox, &dir, &["-v", "jobs", "show", "11111111"])
        .assert()
        .success()
        .stderr(predicates::str::contains(format!("matched {DONE_ID}")));

    // An id that names nothing is an error with the way back in it.
    chap_in(&sandbox, &dir, &["jobs", "show", "zzzz"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains(
            "job zzzz not found; run `chaps jobs` to list them",
        ));

    // And one that names several says so rather than picking one.
    chap_in(&sandbox, &dir, &["jobs", "logs", ""])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains("not found"));
}

#[test]
fn jobs_cancel_and_delete_report_what_chap_core_said() {
    let sandbox = Sandbox::new();
    let dir = served_project(&sandbox);

    chap_in(&sandbox, &dir, &["jobs", "cancel", RUNNING_ID])
        .assert()
        .success()
        .stdout(predicates::str::contains("Job cancelled"))
        .stdout(predicates::str::contains("run `chaps jobs`"));

    chap_in(&sandbox, &dir, &["jobs", "delete", DONE_ID])
        .assert()
        .success()
        .stdout(predicates::str::contains("Job deleted"));

    // chap-core refuses to forget a job it is still working on; the verb that
    // does apply is named.
    chap_in(&sandbox, &dir, &["jobs", "delete", RUNNING_ID])
        .assert()
        .failure()
        .code(1)
        .stderr(predicates::str::contains(format!(
            "job {RUNNING_ID} is still running; cancel it first"
        )));
}

#[test]
fn api_pretty_prints_json_and_reads_a_json_string_as_text() {
    let sandbox = Sandbox::new();
    let dir = served_project(&sandbox);
    let url = read(&dir.join(".env"));
    let port = url
        .lines()
        .find_map(|line| line.strip_prefix("CHAP_API_PORT="))
        .expect("the api port is in .env")
        .trim()
        .to_string();
    let url = format!("http://127.0.0.1:{port}");

    // An object comes back indented with two spaces.
    let text = chap_in(&sandbox, &dir, &["api", "get", "/v1/whoami"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(text).expect("text"),
        "{\n  \"auth\": false\n}\n"
    );

    // A JSON string is its text, which is what makes a log readable.
    let text = chap_in(
        &sandbox,
        &dir,
        &["api", "GET", &format!("/v1/jobs/{DONE_ID}")],
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    assert_eq!(String::from_utf8(text).expect("text"), "SUCCESS\n");

    // `--raw` is the bytes as they arrived, quotes and all.
    let text = chap_in(
        &sandbox,
        &dir,
        &["api", "GET", &format!("/v1/jobs/{DONE_ID}"), "--raw"],
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    assert_eq!(String::from_utf8(text).expect("text"), "\"SUCCESS\"");

    // A body that is not JSON is printed as it came.
    chap_in(&sandbox, &dir, &["api", "GET", "/v1/text"])
        .assert()
        .success()
        .stdout("plain text, not JSON\n");

    // Outside a project `--url` is the address, and it works there.
    chap_in(
        &sandbox,
        sandbox.home.path(),
        &["api", "GET", "/v1/whoami", "--url", &url],
    )
    .assert()
    .success()
    .stdout(predicates::str::contains("\"auth\": false"));

    // Without one there is nothing to aim at, and the error says both ways out.
    chap_in(&sandbox, sandbox.home.path(), &["api", "GET", "/v1/whoami"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--url"));
}

#[test]
fn api_sends_a_body_from_a_file_from_stdin_and_inline() {
    let sandbox = Sandbox::new();
    let dir = served_project(&sandbox);

    let body = sandbox.home.path().join("dataset.json");
    std::fs::write(&body, "{\"name\": \"eval\"}").unwrap();
    chap_in(
        &sandbox,
        &dir,
        &[
            "api",
            "POST",
            "/v1/echo",
            "--data",
            &format!("@{}", body.display()),
        ],
    )
    .assert()
    .success()
    .stdout(predicates::str::contains("\"name\": \"eval\""));

    chap_in(
        &sandbox,
        &dir,
        &["api", "PUT", "/v1/echo", "--data", r#"{"n":1}"#],
    )
    .assert()
    .success()
    .stdout(predicates::str::contains("\"n\": 1"));

    chap_in(&sandbox, &dir, &["api", "POST", "/v1/echo", "--data", "-"])
        .write_stdin("{\"from\":\"stdin\"}")
        .assert()
        .success()
        .stdout(predicates::str::contains("\"from\": \"stdin\""));

    // A body that is not JSON is caught here rather than at the other end.
    chap_in(
        &sandbox,
        &dir,
        &["api", "POST", "/v1/echo", "--data", "{n:1}"],
    )
    .assert()
    .failure()
    .code(2)
    .stderr(predicates::str::contains("not valid JSON"));

    // So is a method chaps does not send, and a path with no leading slash.
    chap_in(&sandbox, &dir, &["api", "BREW", "/v1/echo"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicates::str::contains("GET, POST, PUT, PATCH, DELETE"));
    chap_in(&sandbox, &dir, &["api", "GET", "v1/echo"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicates::str::contains("starts with `/`"));
}

#[test]
fn api_exits_one_on_an_http_error_and_two_when_chap_core_is_not_there() {
    let sandbox = Sandbox::new();
    let dir = served_project(&sandbox);

    // The status line is on stderr and the body is still on stdout: a 404
    // from chap-core says what it did not find.
    let assert = chap_in(&sandbox, &dir, &["api", "GET", "/v1/jobs/nope"])
        .assert()
        .failure()
        .code(1);
    let out = assert.get_output();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Job 'nope' not found"),
        "the body is the answer"
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "HTTP 404 Not Found"
    );

    // Nothing listening at all is a different exit code, with the sentence
    // `chaps status` uses.
    chap_in(
        &sandbox,
        &dir,
        &[
            "api",
            "GET",
            "/v1/jobs",
            "--url",
            "http://127.0.0.1:9",
            "--timeout",
            "2",
        ],
    )
    .assert()
    .failure()
    .code(2)
    .stderr(predicates::str::contains("is not responding"))
    .stderr(predicates::str::contains("run `chaps status`"));
}

#[test]
fn api_and_jobs_send_the_token_this_deployment_holds() {
    let sandbox = Sandbox::new();
    let dir = served_project(&sandbox);

    // Nothing is protected yet, so nothing is sent.
    chap_in(&sandbox, &dir, &["api", "GET", "/v1/whoami"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"auth\": false"));

    sandbox.auth(&["enable"]).assert().success();
    chap_in(&sandbox, &dir, &["api", "GET", "/v1/whoami"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"auth\": true"));

    // And `-v` narrates the header without the value in it.
    let token = env_value(&sandbox.env(), "CHAP_API_TOKEN")
        .expect("a token")
        .to_string();
    let assert = chap_in(&sandbox, &dir, &["-v", "api", "GET", "/v1/whoami"])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();
    assert!(stderr.contains("Authorization: Bearer <token>"), "{stderr}");
    assert!(!stderr.contains(&token), "the token leaked:\n{stderr}");
}

#[test]
fn auth_token_prints_the_token_and_nothing_else() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "none"]).assert().success();

    // Authentication is off to begin with: stdout stays empty, the sentence
    // is on stderr, and the exit code stops a script.
    let assert = chap_in(&sandbox, &sandbox.project(), &["auth", "token"])
        .assert()
        .failure()
        .code(1);
    let out = assert.get_output();
    assert!(out.stdout.is_empty(), "stdout has to stay empty");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("API authentication is off in this deployment; run `chaps auth enable`"),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );

    sandbox.auth(&["enable"]).assert().success();
    let token = env_value(&sandbox.env(), "CHAP_API_TOKEN")
        .expect("a token")
        .to_string();

    // On, and stdout is the token with one newline after it: nothing a
    // `$(...)` would have to strip.
    let out = chap_in(&sandbox, &sandbox.project(), &["auth", "token"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(String::from_utf8(out).expect("text"), format!("{token}\n"));

    // `--json` says the same thing as a document, either way.
    let value = json_of(&mut chap_in(
        &sandbox,
        &sandbox.project(),
        &["--json", "auth", "token"],
    ));
    assert_eq!(value["token"], token);

    sandbox.auth(&["disable"]).assert().success();
    let value = json_of(&mut chap_in(
        &sandbox,
        &sandbox.project(),
        &["--json", "auth", "token"],
    ));
    assert_eq!(value["token"], Json::Null);

    // And outside a deployment it says which one it could not find.
    chap_in(&sandbox, sandbox.home.path(), &["auth", "token"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not a chaps project"));
}

// ------------------------------------------------------------ models test ---

/// A project with the three stand-in models enabled, pointed at a chap-core
/// that answers the `models test --backtest` path.
fn tested_project(sandbox: &Sandbox) -> (PathBuf, u16) {
    let port = free_port();
    sandbox
        .init(&[
            "--models",
            "chapkit_ewars_model,chapkit_rwanda_malaria_bym_model,auto_arima_chapkit",
            "--api-port",
            &port.to_string(),
        ])
        .assert()
        .success();
    chap_core_server(port);
    (sandbox.project(), port)
}

/// What the stand-in was asked to do, read back through `chaps api`.
fn recorded(sandbox: &Sandbox, dir: &Path) -> Json {
    json_of(&mut chap_in(
        sandbox,
        dir,
        &["--json", "api", "GET", "/v1/recorded"],
    ))
}

#[test]
fn models_test_backtest_reports_scores_a_failure_and_a_skip() {
    let sandbox = Sandbox::new();
    let (dir, _) = tested_project(&sandbox);

    let out = chap_in(&sandbox, &dir, &["models", "test", "--all", "--backtest"])
        .assert()
        // One model failed, so the run did.
        .failure()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).expect("the rows are text");

    // The header says which of the two levels this was.
    assert!(
        text.starts_with("testing 3 models (through chap-core:"),
        "{text}"
    );
    // A model that backtested: the three scores, in chap-core's own numbers.
    assert!(
        text.contains("chapkit-ewars-model                 pass"),
        "{text}"
    );
    assert!(text.contains("crps 20.0  mae 29.0  rmse 34.9"), "{text}");
    // A model whose prediction crashed: the reason out of the job log, and
    // the command that shows the whole of it.
    assert!(
        text.contains("chapkit-rwanda-malaria-bym-model    FAIL"),
        "{text}"
    );
    assert!(
        text.contains("the inla program crashed"),
        "the stderr hint is the reason:\n{text}"
    );
    assert!(
        text.contains(&format!("run `chaps jobs logs bt-{FAILING_MODEL}`")),
        "{text}"
    );
    // A model whose chapkit predates the route: a skip that names the version
    // it needs and the one the service reports.
    assert!(
        text.contains("auto-arima-chapkit                  skip"),
        "{text}"
    );
    assert!(
        text.contains("needs chapkit 1.1.0, this one reports 1.0.0"),
        "{text}"
    );
    // And the line the run adds up to, with the failing model named.
    assert!(
        text.contains(
            "1 pass, 1 fail, 1 skipped; run \
             `chaps models test chapkit_rwanda_malaria_bym_model -v` for the full output"
        ),
        "{text}"
    );

    let recorded = recorded(&sandbox, &dir);
    // The dataset is named after the service and the moment, and the org
    // units arrived: `importedCount` is the count of features with a
    // top-level id, so a zero here would mean chaps forgot to set them.
    let names: Vec<&str> = recorded["datasets"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|name| name.as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        names.len(),
        2,
        "the skipped model posted nothing: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|name| name.starts_with("chaps-test-chapkit-ewars-model-")),
        "{names:?}"
    );
    // The backtest asked for is the small rolling one, against the dataset
    // the job wrote.
    let backtest = &recorded["backtests"][0];
    // The configured model chap-core would run, by its integer id: the row
    // named plainly after the service is archived, and the live ones are the
    // synced configs, of which the `test_config_` is the wrong answer.
    assert_eq!(backtest["modelId"], Json::from(PASSING_CONFIGURED));
    assert_eq!(backtest["datasetId"], Json::from(TEST_DATASET));
    assert_eq!(backtest["nPeriods"], Json::from(3));
    assert_eq!(backtest["nSplits"], Json::from(2));
    assert_eq!(backtest["stride"], Json::from(1));
    // Everything this run made is gone again, and nothing else was touched.
    let deleted: Vec<&str> = recorded["deleted"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|path| path.as_str().unwrap_or_default())
        .collect();
    assert!(
        deleted.contains(&format!("/v1/crud/backtests/{TEST_BACKTEST}").as_str()),
        "{deleted:?}"
    );
    assert!(
        deleted.contains(&format!("/v1/crud/datasets/{TEST_DATASET}").as_str()),
        "{deleted:?}"
    );
    assert!(
        deleted.iter().all(|path| path.starts_with("/v1/crud/")),
        "only its own rows: {deleted:?}"
    );
}

#[test]
fn models_test_keep_leaves_the_dataset_and_says_so() {
    let sandbox = Sandbox::new();
    let (dir, _) = tested_project(&sandbox);

    let out = chap_in(
        &sandbox,
        &dir,
        &[
            "models",
            "test",
            "chapkit_ewars_model",
            "--backtest",
            "--keep",
        ],
    )
    .assert()
    .success()
    .get_output()
    .clone();
    let text = String::from_utf8(out.stdout).expect("text");
    let notes = String::from_utf8(out.stderr).expect("text");
    assert!(text.contains("1 of 1 model pass"), "{text}");
    // A single row is not padded past its own name.
    assert!(text.contains("chapkit-ewars-model    pass"), "{text}");
    assert!(
        notes.contains(&format!(
            "kept backtest {TEST_BACKTEST} and dataset {TEST_DATASET}; remove them with \
             `chaps api DELETE /v1/crud/backtests/{TEST_BACKTEST}` then \
             `chaps api DELETE /v1/crud/datasets/{TEST_DATASET}`"
        )),
        "{notes}"
    );

    let recorded = recorded(&sandbox, &dir);
    assert_eq!(recorded["deleted"], Json::Array(Vec::new()));
    // One observation per value, and neither index column became one: three
    // periods times two locations times four feature columns.
    assert_eq!(recorded["observations"], Json::from(24));
    assert_eq!(recorded["sampled"], serde_json::json!([PASSING_MODEL]));
}

#[test]
fn models_test_json_is_one_object_per_model() {
    let sandbox = Sandbox::new();
    let (dir, _) = tested_project(&sandbox);

    let list = json_of(&mut chap_in(
        &sandbox,
        &dir,
        &[
            "--json",
            "models",
            "test",
            "chapkit_ewars_model",
            "--backtest",
        ],
    ));
    let row = &list[0];
    assert_eq!(row["id"], Json::from("chapkit_ewars_model"));
    assert_eq!(row["service_id"], Json::from(PASSING_MODEL));
    assert_eq!(row["level"], Json::from("backtest"));
    assert_eq!(row["result"], Json::from("pass"));
    assert!(row["seconds"].is_number(), "{row}");
    assert_eq!(row["summary"], Json::from("crps 20.0  mae 29.0  rmse 34.9"));
    assert_eq!(row["backtest_id"], Json::from(TEST_BACKTEST));
    assert_eq!(row["job_id"], Json::from(format!("bt-{PASSING_MODEL}")));
    // The scores are handed back whole, not just the three the row prints.
    assert_eq!(row["metrics"]["mape"], Json::from(12.98));
}

#[test]
fn models_test_needs_an_id_or_all_and_the_model_has_to_be_enabled() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();

    // Neither an id nor --all: a usage error that names both.
    chap_in(&sandbox, &dir, &["models", "test"])
        .assert()
        .code(2)
        .stderr(
            predicates::str::contains("name a model to test")
                .and(predicates::str::contains("--all")),
        );

    // An id the catalogue has never heard of.
    chap_in(&sandbox, &dir, &["models", "test", "nope"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("unknown model `nope`"));

    // An id that is a model, but not one this deployment enables.
    chap_in(&sandbox, &dir, &["models", "test", "auto_arima_chapkit"])
        .assert()
        .failure()
        .stderr(
            predicates::str::contains("auto_arima_chapkit is not enabled in this project").and(
                predicates::str::contains("chaps models enable auto_arima_chapkit"),
            ),
        );

    // And the two ways of saying which models cannot be combined.
    chap_in(
        &sandbox,
        &dir,
        &["models", "test", "chapkit_ewars_model", "--all"],
    )
    .assert()
    .failure()
    .stderr(predicates::str::contains("cannot be used with"));
}

/// A `docker` on PATH that answers the three questions the model level asks.
///
/// The model level is `docker compose exec` and nothing else, so there is no
/// way to cover it without either a Docker with the marketplace images pulled
/// or a stand-in. This is the stand-in: it reports one running service, prints
/// whatever `CHAPKIT_OUT` holds for `chapkit test`, and records every
/// invocation so a test can check what was asked and that the cleanup
/// happened.
#[cfg(unix)]
fn fake_docker(out: &str, code: i32) -> (TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let log = temp.path().join("calls.log");
    let chapkit = temp.path().join("chapkit.out");
    std::fs::write(&chapkit, out).expect("the chapkit output");
    let script = format!(
        "#!/bin/sh\n\
         echo \"$*\" >> {log}\n\
         case \"$*\" in\n\
         *' ps --format json'*) \
           printf '{{\"Service\":\"{PASSING_MODEL}\",\"State\":\"running\"}}\\n'; exit 0;;\n\
         *'chapkit test'*) cat {chapkit}; exit {code};;\n\
         *'-X DELETE'*) exit 0;;\n\
         *curl*) printf '[]'; exit 0;;\n\
         esac\n\
         exit 1\n",
        log = log.display(),
        chapkit = chapkit.display(),
    );
    let docker = bin.join("docker");
    std::fs::write(&docker, script).expect("the fake docker");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake docker");
    }
    (temp, log)
}

/// `chaps <args>` with the fake docker ahead of the real one on PATH.
#[cfg(unix)]
fn chap_with_docker(sandbox: &Sandbox, cwd: &Path, bin: &Path, args: &[&str]) -> Command {
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut cmd = chap_in(sandbox, cwd, args);
    cmd.env("PATH", path);
    cmd
}

#[cfg(unix)]
#[test]
fn models_test_runs_chapkit_test_in_the_container_and_cleans_up_after_it() {
    let sandbox = Sandbox::new();
    let (dir, _) = tested_project(&sandbox);
    let passed = "\
==================================================
TEST SUMMARY
==================================================
Elapsed time:          16.85s
Configs created:       1
Trainings completed:   1
Trainings failed:      0
Predictions completed: 1
Predictions failed:    0
Validations run:       2
Validations failed:    0

Result: ALL TESTS PASSED
";
    let (fake, log) = fake_docker(passed, 0);
    let bin = fake.path().join("bin");

    let out = chap_with_docker(&sandbox, &dir, &bin, &["models", "test", "--all"])
        .assert()
        // A skip is not a failure: two containers were not up, and that is a
        // question this run could not ask rather than a bad answer.
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).expect("the rows are text");

    assert!(
        text.starts_with("testing 3 models (model level; add --backtest"),
        "{text}"
    );
    assert!(
        text.contains("chapkit-ewars-model                 pass    0s   1 training, 1 prediction"),
        "{text}"
    );
    // The two that are not up say so, and say the way out.
    assert!(
        text.contains(
            "auto-arima-chapkit                  skip    0s   its container is not running"
        ),
        "{text}"
    );
    assert!(text.contains("run `chaps up`"), "{text}");
    assert!(text.contains("1 pass, 2 skipped"), "{text}");

    let calls = read(&log);
    // The invocation is the documented one, non-interactive, with the
    // deadline handed to chapkit as well as kept here.
    assert!(
        calls.contains(&format!(
            "exec -T {PASSING_MODEL} chapkit test --url http://127.0.0.1:8000 --timeout 300"
        )),
        "{calls}"
    );
    // The config `chapkit test` left behind is gone, deleted through the
    // service's own API because chap-core's proxy is read-only.
    assert!(
        calls.contains(&format!(
            "exec -T {PASSING_MODEL} curl -fsS -X DELETE \
             http://127.0.0.1:8000/api/v1/configs/{TEST_CONFIG}"
        )),
        "{calls}"
    );
    // The artifact went with it - chapkit cascades - so nothing asked for it.
    assert!(!calls.contains(TEST_ARTIFACT), "{calls}");
}

#[cfg(unix)]
#[test]
fn models_test_skips_an_image_that_has_no_chapkit_test() {
    let sandbox = Sandbox::new();
    let (dir, _) = tested_project(&sandbox);
    let (fake, _) = fake_docker(
        "OCI runtime exec failed: exec failed: unable to start container process: \
         exec: \"chapkit\": executable file not found in $PATH: unknown\n",
        1,
    );
    let bin = fake.path().join("bin");

    let out = chap_with_docker(
        &sandbox,
        &dir,
        &bin,
        &["models", "test", "chapkit_ewars_model"],
    )
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
    let text = String::from_utf8(out).expect("text");
    assert!(
        text.contains("skip") && text.contains("the image has no `chapkit test`"),
        "{text}"
    );
    // Which chapkit the service reports is the fact that says whether an
    // update would fix it.
    assert!(text.contains("the service reports chapkit 2.0.0"), "{text}");
    assert!(text.contains("--backtest"), "{text}");
    assert!(text.contains("0 pass, 1 skipped"), "{text}");
}

// ------------------------------------------- switching chap-core's tag ---

/// A `docker` on PATH that answers everything with success and nothing else.
///
/// `chaps update` ends in `docker compose pull`, and a test must not pull
/// anything: what it is about is the files the run writes before that, and
/// the closing line it prints after. Every call is logged so a test can check
/// that the pull was asked for at all.
#[cfg(unix)]
fn quiet_docker() -> (TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("a directory for the fake docker");
    let bin = temp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("a bin directory");
    let log = temp.path().join("calls.log");
    let docker = bin.join("docker");
    std::fs::write(
        &docker,
        format!("#!/bin/sh\necho \"$*\" >> {}\nexit 0\n", log.display()),
    )
    .expect("the fake docker");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("an executable fake docker");
    }
    (temp, bin, log)
}

/// A deployment pinned to the hub's newest release, and the fake docker the
/// update runs through.
#[cfg(unix)]
fn pinned_sandbox() -> (Sandbox, PathBuf, u16, TempDir, PathBuf) {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    let port = Hub::new().start();
    sandbox
        .online_init(port, &["--models", "none", "--chap-tag", "v2.3.1"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "chap-core: v2.3.1 (chap-core compose.ghcr.yml at v2.3.1)",
        ));
    assert!(read(&dir.join("compose.yml")).contains("CHAPS_TEST_REF: v2.3.1"));
    let (temp, bin, _) = quiet_docker();
    (sandbox, dir, port, temp, bin)
}

/// `chaps -C <project> update ...` against the hub, with the fake docker
/// ahead of the real one on PATH.
#[cfg(unix)]
fn online_update(sandbox: &Sandbox, port: u16, bin: &Path, args: &[&str]) -> Command {
    let mut cmd = sandbox.online(port);
    cmd.env(
        "PATH",
        format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    cmd.arg("update").args(args);
    cmd
}

#[cfg(unix)]
#[test]
fn update_switches_chap_core_to_a_moving_tag_and_back() {
    let (sandbox, dir, port, _temp, bin) = pinned_sandbox();

    // Forwards, onto a tag that is ahead of every release: no confirmation,
    // and the compose file of that branch comes with it.
    online_update(&sandbox, port, &bin, &["--chap-tag", "dev"])
        .assert()
        .success()
        .stdout(predicates::str::contains("chap-core  v2.3.1 -> dev"))
        .stdout(predicates::str::contains("updated chap-core v2.3.1 -> dev"))
        .stderr(predicates::str::contains("backup").not());

    let moved = state(&dir);
    assert_eq!(moved["chap_image_tag"], "dev");
    assert_eq!(moved["chap_compose_source"]["tag"], "dev");
    assert!(
        moved["chap_compose_source"]["url"]
            .as_str()
            .unwrap()
            .ends_with("/dhis2-chap/chap-core/dev/compose.ghcr.yml")
    );
    assert!(dir.join(".chaps/compose.chap-core.dev.yml").is_file());
    assert!(
        read(&dir.join(".chaps/compose.chap-core.dev.yml")).contains("CHAPS_TEST_REF: dev"),
        "the fetched copy is the dev one"
    );
    // The rendered base follows it, and so does the line compose reads.
    let base = read(&dir.join("compose.yml"));
    assert!(base.contains("CHAPS_TEST_REF: dev"), "{base}");
    assert!(
        base.contains("# chap-core compose.ghcr.yml at dev\n"),
        "{base}"
    );
    assert_eq!(env_value(&sandbox.env(), "CHAP_IMAGE_TAG"), Some("dev"));

    // Asking for the tag it already runs changes nothing and says so.
    online_update(&sandbox, port, &bin, &["--chap-tag", "dev"])
        .assert()
        .success()
        .stdout(predicates::str::contains(
            "chap-core  dev  already the pin, nothing to switch",
        ))
        .stdout(predicates::str::contains("already up to date"));
    assert_eq!(state(&dir)["chap_image_tag"], "dev");

    // And back to the release, which is the direction that needs an answer.
    online_update(&sandbox, port, &bin, &["--chap-tag", "v2.3.1", "--yes"])
        .assert()
        .success()
        .stderr(predicates::str::contains(
            "moving chap-core from dev to v2.3.1 can run an older schema against a database \
             migrated by the newer one; run `chaps backup create` first",
        ))
        .stdout(predicates::str::contains("chap-core  dev -> v2.3.1"))
        .stdout(predicates::str::contains("updated chap-core dev -> v2.3.1"));

    assert_eq!(state(&dir)["chap_image_tag"], "v2.3.1");
    assert_eq!(state(&dir)["chap_compose_source"]["tag"], "v2.3.1");
    assert!(read(&dir.join("compose.yml")).contains("CHAPS_TEST_REF: v2.3.1"));
    assert_eq!(env_value(&sandbox.env(), "CHAP_IMAGE_TAG"), Some("v2.3.1"));
}

#[cfg(unix)]
#[test]
fn a_backwards_switch_without_an_answer_is_refused() {
    let (sandbox, dir, port, _temp, bin) = pinned_sandbox();
    online_update(&sandbox, port, &bin, &["--chap-tag", "v2.3.0"])
        .assert()
        .failure()
        .stderr(predicates::str::contains(
            "moving chap-core from v2.3.1 to v2.3.0",
        ))
        .stderr(predicates::str::contains("not a terminal"))
        .stderr(predicates::str::contains("--yes"));
    // Refused before anything was written.
    assert_eq!(state(&dir)["chap_image_tag"], "v2.3.1");
    assert_eq!(env_value(&sandbox.env(), "CHAP_IMAGE_TAG"), Some("v2.3.1"));
}

#[cfg(unix)]
#[test]
fn update_refuses_a_chap_tag_that_was_never_released() {
    let (sandbox, dir, port, _temp, bin) = pinned_sandbox();
    let before = read(&dir.join("compose.yml"));
    online_update(&sandbox, port, &bin, &["--chap-tag", "v9.9.9"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("chap-core has no release v9.9.9"))
        .stderr(predicates::str::contains("chaps update --list-tags"));
    assert_eq!(state(&dir)["chap_image_tag"], "v2.3.1");
    assert_eq!(read(&dir.join("compose.yml")), before);

    // The two ways of deciding chap-core's tag cannot both be given.
    online_update(
        &sandbox,
        port,
        &bin,
        &["--chap-tag", "dev", "--pin-chap-core"],
    )
    .assert()
    .failure()
    .stderr(predicates::str::contains("cannot be used with"));
}

#[cfg(unix)]
#[test]
fn a_dry_run_switch_writes_nothing() {
    let (sandbox, dir, port, _temp, bin) = pinned_sandbox();
    let before = (
        read(&dir.join("compose.yml")),
        read(&dir.join(".chaps/project.yaml")),
        sandbox.env(),
    );

    online_update(&sandbox, port, &bin, &["--dry-run", "--chap-tag", "dev"])
        .assert()
        .success()
        .stdout(predicates::str::contains("chap-core  v2.3.1 -> dev"))
        .stdout(predicates::str::contains(
            "would update chap-core v2.3.1 -> dev; nothing written",
        ));

    assert_eq!(read(&dir.join("compose.yml")), before.0);
    assert_eq!(read(&dir.join(".chaps/project.yaml")), before.1);
    assert_eq!(sandbox.env(), before.2);
    assert!(!dir.join(".chaps/compose.chap-core.dev.yml").exists());

    // A dry run backwards says what it would cost and still writes nothing,
    // without an answer: there is nothing yet to confirm.
    online_update(&sandbox, port, &bin, &["--dry-run", "--chap-tag", "v2.3.0"])
        .assert()
        .success()
        .stderr(predicates::str::contains(
            "moving chap-core from v2.3.1 to v2.3.0",
        ));
    assert_eq!(read(&dir.join(".chaps/project.yaml")), before.1);
}

#[test]
fn list_tags_names_the_moving_tags_the_releases_and_the_pin() {
    let sandbox = Sandbox::new();
    let port = Hub::new().start();
    sandbox
        .online_init(port, &["--models", "none", "--chap-tag", "v2.3.0"])
        .assert()
        .success();

    let mut cmd = sandbox.online(port);
    cmd.args(["update", "--list-tags"]);
    let out = cmd.assert().success().get_output().stdout.clone();
    let text = String::from_utf8(out).expect("text");
    assert!(text.contains("TAG") && text.contains("KIND"), "{text}");
    assert!(
        text.contains("PUBLISHED") && text.contains("NOTE"),
        "{text}"
    );
    for row in [
        "dev     moving   2026-09-24  -",
        "master  moving   2026-09-24  -",
        "latest  moving   2026-09-21  -",
        "v2.3.1  release  2026-09-21  newest",
        "v2.3.0  release  2026-09-11  pinned",
    ] {
        assert!(text.contains(row), "missing row `{row}` in:\n{text}");
    }
    assert!(
        text.contains(
            "chap-core is pinned to v2.3.0; move it with `chaps update --chap-tag <TAG>`"
        ),
        "{text}"
    );

    // The same as JSON, which is the list a script reads.
    let mut cmd = sandbox.online(port);
    cmd.args(["--json", "update", "--list-tags"]);
    let list = json_of(&mut cmd);
    assert_eq!(list["pin"], "v2.3.0");
    assert_eq!(list["releases_listed"], true);
    let tags: Vec<&str> = list["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["tag"].as_str().unwrap())
        .collect();
    assert_eq!(tags, vec!["dev", "master", "latest", "v2.3.1", "v2.3.0"]);
    assert_eq!(list["tags"][3]["newest"], true);
    assert_eq!(list["tags"][4]["pinned"], true);

    // It writes nothing: the listing is a question, not a change.
    assert_eq!(state(&sandbox.project())["chap_image_tag"], "v2.3.0");
}

#[test]
fn list_tags_works_offline_with_what_it_has() {
    let sandbox = Sandbox::new();
    sandbox
        .init(&["--models", "none", "--chap-tag", "v1.2.3"])
        .assert()
        .success();

    let mut cmd = sandbox.chap();
    cmd.arg("-C")
        .arg(sandbox.project())
        .args(["update", "--list-tags"]);
    cmd.assert()
        .success()
        .stderr(predicates::str::contains(
            "--offline: the chap-core releases were not listed",
        ))
        .stdout(predicates::str::contains("dev     moving   -          -"))
        .stdout(predicates::str::contains(
            "v1.2.3  release  -          pinned",
        ))
        .stdout(predicates::str::contains("chap-core is pinned to v1.2.3"));

    // `--dry-run` has nothing to say about a command that writes nothing.
    let mut dry = sandbox.chap();
    dry.arg("-C")
        .arg(sandbox.project())
        .args(["update", "--list-tags", "--dry-run"]);
    dry.assert().success().stdout(predicates::str::contains(
        "v1.2.3  release  -          pinned",
    ));
}
