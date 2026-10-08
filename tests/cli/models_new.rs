use crate::common::*;
use predicates::prelude::*;

#[test]
fn models_new_writes_a_project_and_says_what_to_do_next() {
    let sandbox = Sandbox::new();
    chap_in(&sandbox, sandbox.home.path(), &["models", "new", "dengue-lags"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "created dengue-lags in dengue-lags (fn-py template, service id dengue-lags)",
        ))
        .stdout(predicate::str::contains(
            "next: build the image as `dengue-lags/README.md` says, then run `varde run dengue-lags:dev`",
        ))
        .stdout(predicate::str::contains("hint:").not());
    let dir = sandbox.home.path().join("dengue-lags");
    for name in [
        "main.py",
        "pyproject.toml",
        "Dockerfile",
        "README.md",
        ".github/workflows/publish.yml",
        ".gitignore",
        ".dockerignore",
        ".python-version",
    ] {
        assert!(dir.join(name).is_file(), "{name} was not written");
    }
    assert!(read(&dir.join("main.py")).contains("class DengueLagsConfig(BaseConfig):"));
}

#[test]
fn models_new_json_names_the_files_and_the_ids() {
    let sandbox = Sandbox::new();
    let report = json_of(&mut chap_in(
        &sandbox,
        sandbox.home.path(),
        &[
            "--json",
            "models",
            "new",
            "Rain Model",
            "--template",
            "mlproject-r",
        ],
    ));
    assert_eq!(report["ok"], true);
    assert_eq!(report["template"], "mlproject-r");
    assert_eq!(report["service_id"], "rain-model");
    assert_eq!(report["model_id"], "rain_model");
    assert_eq!(
        report["base_image"],
        "ghcr.io/dhis2-chap/chapkit-r-cli:latest"
    );
    let files: Vec<&str> = report["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(files.contains(&"MLproject"));
    assert!(files.contains(&"scripts/train.R"));
    assert!(!files.contains(&"main.py"));
    let mlproject = yaml(&sandbox.home.path().join("Rain Model/MLproject"));
    assert_eq!(mlproject["name"], "rain-model");
}

#[test]
fn models_new_refuses_a_directory_with_files_in_it() {
    let sandbox = Sandbox::new();
    let dir = sandbox.home.path().join("taken");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "mine").unwrap();
    chap_in(&sandbox, sandbox.home.path(), &["models", "new", "taken"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "already exists and is not empty; give a new directory",
        ));
    assert!(!dir.join("main.py").exists());
}

#[test]
fn models_new_refuses_validation_hooks_for_an_mlproject() {
    let sandbox = Sandbox::new();
    chap_in(
        &sandbox,
        sandbox.home.path(),
        &[
            "models",
            "new",
            "x",
            "--template",
            "mlproject-py",
            "--with-validation",
        ],
    )
    .assert()
    .code(2)
    .stderr(predicate::str::contains(
        "the mlproject-py template has no main.py; use a chapkit service type",
    ));
    assert!(!sandbox.home.path().join("x").exists());
}

#[test]
fn models_new_does_not_need_a_deployment() {
    let sandbox = Sandbox::new();
    let mut cmd = sandbox.chap();
    cmd.args(["-C", "/nowhere", "models", "new", "free"])
        .assert()
        .success();
    assert!(sandbox.home.path().join("free/main.py").is_file());
}
