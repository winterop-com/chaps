use super::*;
use clap::ValueEnum;

fn all() -> Vec<Template> {
    Template::value_variants().to_vec()
}

fn rendered(template: Template, with_validation: bool) -> Vec<(&'static str, String)> {
    let names = names("My Model").unwrap();
    render(&names, template, with_validation).unwrap()
}

fn file<'a>(files: &'a [(&str, String)], path: &str) -> &'a str {
    files
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, body)| body.as_str())
        .unwrap_or_else(|| panic!("{path} is not in the project"))
}

#[test]
fn names_come_from_the_directory_name() {
    let n = names("My Dengue_Model").unwrap();
    assert_eq!(n.name, "My Dengue_Model");
    assert_eq!(n.service_id, "my-dengue-model");
    assert_eq!(n.model_id, "my_dengue_model");
    assert_eq!(n.class_name, "MyDengueModel");
}

#[test]
fn names_collapse_separators_and_need_a_letter_first() {
    assert_eq!(names("a--b..c").unwrap().service_id, "a-b-c");
    assert_eq!(names("2024-ewars").unwrap().service_id, "model-2024-ewars");
    assert_eq!(names("2024-ewars").unwrap().class_name, "Model2024Ewars");
}

#[test]
fn names_refuse_characters_a_model_name_cannot_have() {
    let err = names("my\"model").unwrap_err().to_string();
    assert!(err.contains("the character `\"`"), "{err}");
    assert!(names("--").is_err());
}

#[test]
fn the_template_names_are_the_option_values() {
    for template in all() {
        let value = template.to_possible_value().unwrap();
        assert_eq!(value.get_name(), template.as_str());
    }
}

#[test]
fn every_template_renders_with_no_template_syntax_left() {
    for template in all() {
        for with_validation in [false, true] {
            for (path, body) in rendered(template, with_validation) {
                if path == ".github/workflows/publish.yml" {
                    assert!(body.contains("${{ github.repository }}"));
                    continue;
                }
                assert!(
                    !body.contains("{{") && !body.contains("{%"),
                    "{} {path} has template syntax left",
                    template.as_str()
                );
                assert!(body.ends_with('\n'), "{path} has no last newline");
                // Python puts two blank lines between top-level definitions.
                let gap = if path.ends_with(".py") {
                    "\n\n\n\n"
                } else {
                    "\n\n\n"
                };
                assert!(!body.contains(gap), "{path} has too many blank lines");
            }
        }
    }
}

#[test]
fn a_chapkit_service_has_main_py_and_pins_chapkit() {
    let files = rendered(Template::FnPy, false);
    let main = file(&files, "main.py");
    assert!(main.contains("class MyModelConfig(BaseConfig):"));
    assert!(main.contains("id=\"my-model\""));
    assert!(main.contains("FunctionalModelRunner"));
    assert!(!main.contains("on_validate_train"));
    assert!(file(&files, "pyproject.toml").contains(CHAPKIT_REQUIREMENT));
    assert!(file(&files, "Dockerfile").contains("FROM ghcr.io/dhis2-chap/chapkit-py:latest"));
    assert!(!files.iter().any(|(p, _)| p.starts_with("scripts/")));
}

#[test]
fn with_validation_adds_the_hooks() {
    for template in [Template::FnPy, Template::ShellR] {
        let files = rendered(template, true);
        let main = file(&files, "main.py");
        assert!(main.contains("async def on_validate_train("));
        assert!(main.contains("on_validate_train=on_validate_train,"));
    }
}

#[test]
fn a_shell_service_runs_its_scripts() {
    let files = rendered(Template::ShellR, false);
    let main = file(&files, "main.py");
    assert!(main.contains(
        "\"Rscript scripts/train.R --data {data_file} --model model.rds --config config.yml\""
    ));
    assert!(file(&files, "Dockerfile").contains("COPY scripts/ ./scripts/"));
    assert!(file(&files, "scripts/train.R").contains("saveRDS"));
    assert!(!file(&files, "pyproject.toml").contains("pandas"));
}

#[test]
fn an_mlproject_has_no_chapkit_code() {
    let files = rendered(Template::MlprojectPy, false);
    assert!(!files.iter().any(|(p, _)| *p == "main.py"));
    let mlproject: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(file(&files, "MLproject")).unwrap();
    assert_eq!(mlproject["name"], "my-model");
    assert_eq!(mlproject["meta_data"]["display_name"], "My Model");
    assert_eq!(mlproject["user_options"]["window"]["default"], 3);
    let train = mlproject["entry_points"]["train"]["command"]
        .as_str()
        .unwrap();
    assert_eq!(
        train,
        "python scripts/train.py --data {train_data} --model {model} --config {model_config}"
    );
    let docker = file(&files, "Dockerfile");
    assert!(docker.contains("FROM ghcr.io/dhis2-chap/chapkit-py-cli:latest"));
    assert!(docker.contains("CMD [\"chapkit\", \"mlproject\", \"run\", \".\""));
    assert!(!file(&files, "pyproject.toml").contains("chapkit"));
}

#[test]
fn an_r_mlproject_has_no_python_files() {
    let files = rendered(Template::MlprojectR, false);
    for path in ["pyproject.toml", ".python-version", "main.py"] {
        assert!(!files.iter().any(|(p, _)| *p == path), "{path} is there");
    }
    assert!(file(&files, "Dockerfile").contains("chapkit-r-cli:latest"));
}

#[test]
fn the_inla_types_pin_amd64() {
    for template in [Template::ShellRInla, Template::MlprojectRInla] {
        let files = rendered(template, false);
        assert!(file(&files, "Dockerfile").contains("FROM --platform=${BASE_PLATFORM}"));
        assert!(file(&files, "README.md").contains("about 2 GB"));
    }
    let files = rendered(Template::ShellR, false);
    assert!(!file(&files, "Dockerfile").contains("--platform"));
}

#[test]
fn the_readme_gives_the_commands_for_this_project() {
    let files = rendered(Template::ShellPy, false);
    let readme = file(&files, "README.md");
    assert!(readme.contains("-t my-model:dev ."));
    assert!(readme.contains("varde models add my-model:dev"));
    assert!(readme.contains("varde models test my_model --backtest"));
    assert!(readme.contains("varde models configs add my_model --name long --set window=6"));
}

#[test]
fn write_refuses_a_directory_that_is_not_empty() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("keep.txt"), "x").unwrap();
    let err = write(tmp.path(), &[("a.txt", "a".to_string())])
        .unwrap_err()
        .to_string();
    assert!(err.contains("already exists and is not empty"), "{err}");
    assert!(!tmp.path().join("a.txt").exists());
}

#[test]
fn write_fills_an_empty_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let written = write(
        tmp.path(),
        &[(".github/workflows/publish.yml", "x\n".to_string())],
    )
    .unwrap();
    assert_eq!(
        written,
        vec![tmp.path().join(".github/workflows/publish.yml")]
    );
}
