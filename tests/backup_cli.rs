//! End-to-end tests for `chaps backup create` and `chaps backup restore`.
//!
//! Every run is `--offline` with its own cache directory, and every one of
//! them stays away from Docker: the parts of a backup that need a daemon are
//! switched off with `--no-db --no-models --no-components`, and the restores
//! are `--files-only`. What is tested here is what the archive holds and what
//! a restore does to the deployment it is restored into.

use assert_cmd::Command;
use serde_json::Value as Json;
use serde_yaml_ng::Value as Yaml;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A cache directory plus the directory the projects are written into.
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

    /// `chaps --offline <args..>`, run from `cwd`.
    fn chap(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("chaps").expect("the chaps binary is built");
        cmd.env("CHAPS_CACHE_DIR", self.cache.path())
            .current_dir(cwd)
            .arg("--offline")
            .args(args);
        cmd
    }

    /// `chaps init <home>/<name> --models none --api-port <free>`, with the
    /// arguments appended. Returns the project directory.
    fn init(&self, name: &str, args: &[&str]) -> PathBuf {
        let dir = self.home.path().join(name);
        let port = free_port().to_string();
        let mut cmd = Command::cargo_bin("chaps").expect("the chaps binary is built");
        cmd.env("CHAPS_CACHE_DIR", self.cache.path())
            .current_dir(self.home.path())
            .arg("--offline")
            .arg("init")
            .arg(&dir)
            .args(["--models", "none", "--api-port", &port])
            .args(args);
        cmd.assert().success();
        dir
    }

    /// An empty directory the archives are written to.
    fn archives(&self) -> PathBuf {
        let dir = self.home.path().join("archives");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// A port the kernel handed out, and that nothing is listening on.
///
/// Bound only long enough to learn which port is free, so the CLI's own probe
/// finds nothing there. Ports already handed out are remembered, so two calls
/// never name the same one.
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

/// The member names inside an archive, as `tar -tzf` prints them.
fn members(archive: &Path) -> Vec<String> {
    let out = std::process::Command::new("tar")
        .args(["-tzf", archive.to_str().unwrap()])
        .output()
        .expect("tar is on PATH");
    assert!(out.status.success(), "tar -tzf {}", archive.display());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim_end_matches('/').to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// The archive's manifest, as JSON, read without unpacking it.
fn manifest(archive: &Path) -> Json {
    let out = std::process::Command::new("tar")
        .args(["-xzf", archive.to_str().unwrap(), "-O", "manifest.yaml"])
        .output()
        .expect("tar is on PATH");
    assert!(out.status.success(), "reading the manifest");
    serde_json::to_value(serde_yaml_ng::from_slice::<Yaml>(&out.stdout).unwrap()).unwrap()
}

/// `.chaps/project.yaml` as JSON.
fn state(dir: &Path) -> Json {
    let body = read(&dir.join(".chaps/project.yaml"));
    serde_json::to_value(serde_yaml_ng::from_str::<Yaml>(&body).unwrap()).unwrap()
}

/// The compose project name a deployment records.
fn identity(dir: &Path) -> String {
    state(dir)["compose_project"]
        .as_str()
        .expect("a recorded compose project name")
        .to_string()
}

/// The top-level `name:` of a rendered compose file.
fn compose_name(dir: &Path, file: &str) -> String {
    let body = read(&dir.join(file));
    serde_yaml_ng::from_str::<Yaml>(&body).unwrap()["name"]
        .as_str()
        .expect("a name: key")
        .to_string()
}

/// `chaps backup create`, with Docker left out of it.
fn backup(sandbox: &Sandbox, dir: &Path, out: &Path, extra: &[&str]) -> PathBuf {
    let mut args = vec![
        "backup",
        "create",
        "--no-db",
        "--no-models",
        "--out",
        out.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    sandbox.chap(dir, &args).assert().success();
    only_archive(out)
}

#[test]
fn a_backup_holds_the_ocs_instance_config() {
    let sandbox = Sandbox::new();
    let dir = sandbox.init("chapx", &["--with", "ocs"]);
    let config = "ocs/climate-service.yaml";
    assert!(dir.join(config).is_file(), "init --with ocs scaffolds it");

    // The operator's own edit to the file, which nothing but a backup can
    // bring back: sync only ever creates a missing one.
    let edited = format!("{}\n# restored-marker\n", read(&dir.join(config)));
    std::fs::write(dir.join(config), &edited).unwrap();

    let out = sandbox.archives();
    let archive = backup(&sandbox, &dir, &out, &["--no-components"]);

    let members = members(&archive);
    assert!(
        members.contains(&format!("files/{config}")),
        "the OCS config is in the archive: {members:?}"
    );

    let manifest = manifest(&archive);
    let files: Vec<&str> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(files.contains(&config), "the manifest lists it: {files:?}");
    // The file list is ordered: .env, then .chaps/**, then ocs/**, then the
    // compose files.
    let at = |name: &str| files.iter().position(|f| *f == name);
    assert!(at(".chaps/project.yaml") < at(config));
    assert!(at(config) < at("compose.yml"));

    // The component is recorded even though its volume was not captured, so
    // the archive says what the deployment was made of.
    assert_eq!(manifest["components"][0]["name"], "ocs");
    assert_eq!(manifest["components"][0]["volume"], "ocs_data");
    assert_eq!(manifest["components"][0]["skipped"], "--no-components");
    assert!(manifest["components"][0]["path"].is_null());

    // And the edit round-trips: restoring the archive brings the file back.
    std::fs::write(dir.join(config), "# thrown away\n").unwrap();
    sandbox
        .chap(
            &dir,
            &[
                "backup",
                "restore",
                archive.to_str().unwrap(),
                "--files-only",
                "--yes",
            ],
        )
        .assert()
        .success();
    assert_eq!(read(&dir.join(config)), edited);
}

#[test]
fn a_backup_of_a_deployment_without_ocs_holds_no_components() {
    let sandbox = Sandbox::new();
    let dir = sandbox.init("chapx", &[]);
    let archive = backup(&sandbox, &dir, &sandbox.archives(), &[]);

    let manifest = manifest(&archive);
    assert!(
        manifest["components"].as_array().unwrap().is_empty(),
        "chap-core keeps its state in the database and the model volumes"
    );
    assert!(
        !members(&archive)
            .iter()
            .any(|m| m.starts_with("components/"))
    );
}

#[test]
fn restoring_into_a_second_deployment_keeps_that_deployments_identity() {
    let sandbox = Sandbox::new();
    let source = sandbox.init("chapx", &["--with", "ocs"]);
    let archive = backup(&sandbox, &source, &sandbox.archives(), &["--no-components"]);
    let taken_from = identity(&source);

    let target = sandbox.init("chapy", &[]);
    let kept = identity(&target);
    assert_ne!(kept, taken_from, "two deployments, two identities");

    let text = String::from_utf8(
        sandbox
            .chap(
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

    // Said before it happened, and again afterwards.
    assert!(
        text.contains(&format!("the archive's own ({taken_from}) is not adopted")),
        "{text}"
    );
    assert_eq!(text.matches("identity").count(), 2, "{text}");

    // The state that arrived is the archive's, except for the one field that
    // says which containers and volumes belong to this deployment.
    assert_eq!(identity(&target), kept);
    assert_eq!(
        state(&target)["api_port"],
        state(&source)["api_port"],
        "the API port is the archive's, like the .env that sets it"
    );
    assert_eq!(read(&target.join(".env")), read(&source.join(".env")));

    // And the re-rendered compose files carry the name this deployment keeps,
    // so `docker compose` still finds its own containers.
    for file in ["compose.chaps.yml", "compose.marketplace.yml"] {
        assert_eq!(compose_name(&target, file), kept, "{file}");
    }
    // The component the archive brought with it is enabled here now, so its
    // compose file is rendered and is in this deployment's `-f` list. That is
    // what every step after the files restore has to work from.
    assert!(target.join("compose.ocs.yml").is_file());
    assert!(target.join("ocs/climate-service.yaml").is_file());
    assert_eq!(
        state(&target)["compose_files"],
        serde_json::json!([
            "compose.yml",
            "compose.chaps.yml",
            "compose.ocs.yml",
            "compose.marketplace.yml"
        ])
    );
}

#[test]
fn adopt_identity_takes_the_archives_name_over() {
    let sandbox = Sandbox::new();
    let source = sandbox.init("chapx", &[]);
    let archive = backup(&sandbox, &source, &sandbox.archives(), &[]);
    let taken_from = identity(&source);

    let target = sandbox.init("chapy", &[]);
    assert_ne!(identity(&target), taken_from);

    let text = String::from_utf8(
        sandbox
            .chap(
                &target,
                &[
                    "backup",
                    "restore",
                    archive.to_str().unwrap(),
                    "--files-only",
                    "--adopt-identity",
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

    assert!(
        text.contains(&format!(
            "identity  compose project {taken_from}, taken over from the archive"
        )),
        "{text}"
    );
    assert_eq!(identity(&target), taken_from);
    assert_eq!(compose_name(&target, "compose.chaps.yml"), taken_from);
}

#[test]
fn the_restore_flags_that_narrow_a_run_refuse_the_combinations_that_make_no_sense() {
    let sandbox = Sandbox::new();
    let dir = sandbox.init("chapx", &[]);
    let archive = backup(&sandbox, &dir, &sandbox.archives(), &[]);
    let archive = archive.to_str().unwrap().to_string();

    for flags in [
        vec!["--files-only", "--no-components"],
        vec!["--db-only", "--no-components"],
        // Nothing writes project.yaml in a --db-only run, so there is no
        // identity to adopt.
        vec!["--db-only", "--adopt-identity"],
    ] {
        let mut args = vec!["backup", "restore", &archive, "--yes"];
        args.extend_from_slice(&flags);
        sandbox
            .chap(&dir, &args)
            .assert()
            .failure()
            .stderr(predicates::str::contains("cannot be used with"));
    }
}
