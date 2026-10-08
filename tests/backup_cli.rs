//! End-to-end tests for `varde backup create` and `varde backup restore`.
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

    /// `varde --offline <args..>`, run from `cwd`.
    fn chap(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("varde").expect("the varde binary is built");
        cmd.env("VARDE_CACHE_DIR", self.cache.path())
            // The data directory too: `init` records the deployment there, and
            // a test must not write the developer's own record.
            .env("VARDE_DATA_DIR", self.cache.path().join("data"))
            .env("VARDE_NO_DOCKER_PROBE", "1")
            .env_remove("GITHUB_TOKEN")
            .env_remove("GH_TOKEN")
            .current_dir(cwd)
            .arg("--offline")
            .args(args);
        cmd
    }

    /// `varde init <home>/<name> --models none --api-port <free>`, with the
    /// arguments appended. Returns the project directory.
    fn init(&self, name: &str, args: &[&str]) -> PathBuf {
        let dir = self.home.path().join(name);
        let port = free_port().to_string();
        let mut cmd = Command::cargo_bin("varde").expect("the varde binary is built");
        cmd.env("VARDE_CACHE_DIR", self.cache.path())
            // The data directory too: `init` records the deployment there, and
            // a test must not write the developer's own record.
            .env("VARDE_DATA_DIR", self.cache.path().join("data"))
            .env("VARDE_NO_DOCKER_PROBE", "1")
            .env_remove("GITHUB_TOKEN")
            .env_remove("GH_TOKEN")
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

/// One member's body, read out of the archive without unpacking it.
fn member_body(archive: &Path, member: &str) -> String {
    let out = std::process::Command::new("tar")
        .args(["-xzf", archive.to_str().unwrap(), "-O", member])
        .output()
        .expect("tar is on PATH");
    assert!(out.status.success(), "reading {member}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Every file under `dir`, as `prefix/...` paths.
fn files_under(dir: &Path, prefix: &str) -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = format!("{prefix}/{name}");
        if entry.path().is_dir() {
            found.extend(files_under(&entry.path(), &path));
        } else {
            found.push(path);
        }
    }
    found.sort();
    found
}

/// The directories a deployment keeps its own files in: everything in the
/// project root that is not the `.varde/` state.
///
/// Read off the disk rather than listed here, so a component that scaffolds a
/// directory of its own is covered by the tests that use this the day it is
/// added, without anybody having to remember it.
fn owned_dirs(dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name != ".varde")
        .collect();
    found.sort();
    found
}

/// The name of every component this build has, from `varde components list`.
fn component_names(sandbox: &Sandbox, dir: &Path) -> Vec<String> {
    let out = sandbox
        .chap(dir, &["components", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice::<Json>(&out).expect("a JSON listing")["components"]
        .as_array()
        .expect("a components array")
        .iter()
        .map(|row| row["name"].as_str().expect("a name").to_string())
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

/// `.varde/project.yaml` as JSON.
fn state(dir: &Path) -> Json {
    let body = read(&dir.join(".varde/project.yaml"));
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

/// `varde backup create`, with Docker left out of it.
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
    // The file list is ordered: .env, then .varde/**, then ocs/**, then the
    // compose files.
    let at = |name: &str| files.iter().position(|f| *f == name);
    assert!(at(".varde/project.yaml") < at(config));
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

/// Every file a deployment keeps of its own is in the archive, for every
/// component that has a directory - not only the one whose name somebody
/// remembered to write into the file list.
///
/// `dhis2/dhis.conf` was in no archive ever taken: `ocs/` was collected by name,
/// so DHIS2's directory was never going to be picked up, and a fifth
/// component's would have gone the same way. That file is written once and
/// never rewritten and DHIS2 throws on startup without it, so the reverse-proxy
/// pair, the Flyway settings and a hand-set encryption password were lost on
/// every restore with nothing reporting it.
///
/// The components are asked of the CLI and the directories are read off the
/// disk, so this fails for the next component that keeps files of its own
/// rather than waiting for somebody to add a test per component.
#[test]
fn a_backup_holds_every_file_a_component_keeps_of_its_own() {
    let sandbox = Sandbox::new();
    // Every component this build has, from the CLI rather than a list in here.
    let probe = sandbox.init("probe", &[]);
    let every = component_names(&sandbox, &probe).join(",");

    // `none` for the two host ports there is a flag for: this is a test about
    // files, and nothing in it should depend on which ports the machine running
    // it has free.
    let dir = sandbox.init(
        "chapx",
        &[
            "--with",
            &every,
            "--ocs-port",
            "none",
            "--dhis2-port",
            "none",
        ],
    );

    let owned = owned_dirs(&dir);
    for name in ["dhis2", "ocs"] {
        assert!(
            owned.contains(&name.to_string()),
            "init --with {every} scaffolds {name}/: {owned:?}"
        );
    }

    // A recognisable edit in every scaffolded file, plus a nested one beside it:
    // `ocs/plugins/` is collected recursively, and a component directory has to
    // behave the same whichever component it belongs to.
    const MARKER: &str = "# the-operators-own-edit";
    let mut expected: Vec<(String, String)> = Vec::new();
    for owned_dir in &owned {
        for rel in files_under(&dir.join(owned_dir), owned_dir) {
            let edited = format!("{}\n{MARKER}\n", read(&dir.join(&rel)));
            std::fs::write(dir.join(&rel), &edited).unwrap();
            expected.push((rel, edited));
        }
        let nested = format!("{owned_dir}/plugins/extra.py");
        std::fs::create_dir_all(dir.join(owned_dir).join("plugins")).unwrap();
        let body = format!("{MARKER}, nested\n");
        std::fs::write(dir.join(&nested), &body).unwrap();
        expected.push((nested, body));
    }
    for must in ["ocs/climate-service.yaml", "dhis2/dhis.conf"] {
        assert!(
            expected.iter().any(|(rel, _)| rel == must),
            "{must} is one of the files this covers: {expected:?}"
        );
    }

    let archive = backup(&sandbox, &dir, &sandbox.archives(), &["--no-components"]);
    let members = members(&archive);
    let manifest = manifest(&archive);
    let listed: Vec<&str> = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();

    for (rel, body) in &expected {
        assert!(
            members.contains(&format!("files/{rel}")),
            "{rel} is in the archive: {members:?}"
        );
        assert!(
            listed.contains(&rel.as_str()),
            "the manifest lists {rel}: {listed:?}"
        );
        assert_eq!(
            &member_body(&archive, &format!("files/{rel}")),
            body,
            "{rel} went in with the edit intact"
        );
    }

    // And every one of them comes back, which is the half that matters to an
    // operator: restore already writes whatever `files/` holds, so the archive
    // was the only thing between these files and a deployment rebuilt without
    // them. The edit rather than the whole body, because the re-render that
    // follows a files restore may add a key it manages to a file it wrote -
    // `plugins_dir` lands in `ocs/climate-service.yaml` now that there is an
    // `ocs/plugins/` - and that is `varde sync` doing its job, not the restore
    // losing anything.
    for (rel, _) in &expected {
        std::fs::write(dir.join(rel), "# thrown away\n").unwrap();
    }
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
    for (rel, _) in &expected {
        let restored = read(&dir.join(rel));
        assert!(restored.contains(MARKER), "{rel} was restored: {restored}");
    }
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
    let target_env = read(&target.join(".env"));

    let text = String::from_utf8(
        sandbox
            .chap(
                &target,
                &[
                    "-v",
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

    // Said in the plan before it happened, and again as a hint afterwards.
    let adopted = format!("the archive's own ({taken_from}) is not adopted");
    assert_eq!(text.matches(&adopted).count(), 2, "{text}");
    assert!(text.contains(&format!("hint: {kept} is kept;")), "{text}");

    // The state that arrived is the archive's, except for the one field that
    // says which containers and volumes belong to this deployment.
    assert_eq!(identity(&target), kept);
    assert_eq!(
        state(&target)["api_port"],
        state(&source)["api_port"],
        "the API port is the archive's, like the .env that sets it"
    );
    // The `.env` is the archive's but for the database password: this
    // deployment keeps its own `postgres` volume, whose role was created with
    // its own password, and `pg_restore` does not change a role. Taking the
    // archive's would leave chap-core unable to log in to its database.
    let password = |env: &str| {
        env.lines()
            .find(|line| line.starts_with("POSTGRES_PASSWORD="))
            .expect("a password line")
            .to_string()
    };
    let restored = read(&target.join(".env"));
    assert_eq!(password(&restored), password(&target_env));
    assert_ne!(password(&restored), password(&read(&source.join(".env"))));
    assert_eq!(
        restored.replace(&password(&restored), ""),
        read(&source.join(".env")).replace(&password(&read(&source.join(".env"))), "")
    );
    assert!(
        text.contains("kept this deployment's POSTGRES_PASSWORD in .env"),
        "{text}"
    );

    // And the re-rendered compose files carry the name this deployment keeps,
    // so `docker compose` still finds its own containers.
    for file in ["compose.varde.yml", "compose.marketplace.yml"] {
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
            "compose.varde.yml",
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

    let output = sandbox
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
        .clone();
    let text = String::from_utf8(output.stdout.clone()).expect("the report is text");

    assert!(
        text.contains(&format!(
            "identity    compose project {taken_from}, taken over from the archive"
        )),
        "{text}"
    );
    // Said once, in the closing lines; the plan line comes before them.
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    assert_eq!(
        format!("{text}{stderr}")
            .matches("taken over from the archive")
            .count(),
        2,
        "the plan line and one closing line:\n{text}{stderr}"
    );
    // chapx still records that name, so the two directories now share every
    // container and volume, and both the plan and the closing lines say so.
    let source_real = std::fs::canonicalize(&source).unwrap();
    let shared = format!(
        "{} also records the compose project name {taken_from}",
        source_real.display()
    );
    assert!(text.contains(&format!("  shared      {shared}")), "{text}");
    assert!(stderr.contains(&format!("warning: {shared}")), "{stderr}");
    // The takeover is recorded, so `varde doctor` does not take the older
    // volumes for an accident.
    assert_eq!(state(&target)["adopted_identity"], Json::Bool(true));
    assert_eq!(identity(&target), taken_from);
    assert_eq!(compose_name(&target, "compose.varde.yml"), taken_from);
    // Adopting the name is adopting the archive's volumes, and with them the
    // credentials they open with: the whole `.env` is the archive's.
    assert_eq!(read(&target.join(".env")), read(&source.join(".env")));
    assert!(!text.contains("kept this deployment's"), "{text}");
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
