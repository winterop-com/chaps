use crate::common::*;
use serde_json::Value as Json;
use std::path::PathBuf;

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

    // The archive's `.env`, but for the one line its own `postgres` volume
    // decides: the role there was created with this deployment's password.
    let restored = read(&target.join(".env"));
    assert_eq!(password_line(&restored), password_line(&before));
    let archived = read(&source.join(".env"));
    assert_eq!(
        restored.replace(password_line(&restored), ""),
        archived.replace(password_line(&archived), "")
    );
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

/// A `.chaps/` state file the archive does not carry was not part of the
/// deployment it came from, so the restore takes this deployment's away
/// rather than mixing the two.
#[test]
fn restore_removes_optional_state_the_archive_does_not_have() {
    let sandbox = Sandbox::new();
    let source = sandbox.project();
    sandbox.init(&["--models", "none"]).assert().success();
    assert!(!source.join(".chaps/models-manual.yaml").exists());
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

    let target = sandbox.home.path().join("chapy");
    let mut init = sandbox.chap();
    init.arg("init").arg(&target).args(["--models", "none"]);
    init.assert().success();
    let manual = target.join(".chaps/models-manual.yaml");
    std::fs::write(&manual, "{}\n").unwrap();

    let report = json_of(&mut chap_in(
        &sandbox,
        &target,
        &[
            "--json",
            "backup",
            "restore",
            archive.to_str().unwrap(),
            "--files-only",
            "--yes",
        ],
    ));
    assert!(!manual.exists(), "the destination's definitions went");
    assert_eq!(
        report["removed_state"],
        serde_json::json!([".chaps/models-manual.yaml"])
    );
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
