use super::*;
use serde_json::json;

fn template() -> Template {
    configs::parse_template(&json!({
        "id": 30,
        "name": "svc",
        "requiredCovariates": ["population"],
        "allowFreeAdditionalContinuousCovariates": true,
        "userOptions": {
            "n_lags": {"type": "integer", "default": 3, "description": "Lags of the target."},
            "method": {"enum": ["fast", "exact"], "default": "fast"}
        }
    }))
    .expect("a template")
}

fn start() -> Draft {
    Draft {
        variant: String::new(),
        values: serde_json::Map::new(),
        covariates: Vec::new(),
    }
}

fn run(input: &str, existing: &[Config]) -> (Result<Draft>, String) {
    let mut input = std::io::Cursor::new(input.as_bytes().to_vec());
    let mut output: Vec<u8> = Vec::new();
    let draft = ask_all(&mut input, &mut output, &template(), existing, "m", start());
    (draft, String::from_utf8(output).expect("text"))
}

#[test]
fn enter_keeps_the_default_and_a_default_is_not_sent() {
    // name, method (Enter), n_lags (the default typed out), covariates.
    let (draft, output) = run("weekly\n\n3\nrainfall\n", &[]);
    let draft = draft.expect("a draft");
    assert_eq!(draft.variant, "weekly");
    assert!(draft.values.is_empty(), "{:?}", draft.values);
    assert_eq!(draft.covariates, ["rainfall"]);
    assert!(
        output.contains("method (one of fast, exact) [fast]: "),
        "{output}"
    );
    assert!(output.contains("n_lags: Lags of the target."), "{output}");
    assert!(output.contains("every run gets population"), "{output}");
}

#[test]
fn a_bad_answer_is_asked_again() {
    let (draft, output) = run("weekly\nslow\nexact\nfour\n5\n\n", &[]);
    let draft = draft.expect("a draft");
    assert_eq!(draft.values["method"], json!("exact"));
    assert_eq!(draft.values["n_lags"], json!(5));
    assert!(
        output.contains("`slow` is not a value of `method`"),
        "{output}"
    );
    assert!(
        output.contains("`four` is not a value of `n_lags`"),
        "{output}"
    );
}

#[test]
fn a_name_in_use_is_asked_again() {
    let rows = json!([{"id": 1, "name": "svc:weekly"}]);
    let existing = configs::configs_of(&rows, "svc");
    let (draft, output) = run("weekly\nother\n\n\n\n", &existing);
    assert_eq!(draft.expect("a draft").variant, "other");
    assert!(
        output.contains("it has these configured models: weekly"),
        "{output}"
    );
    assert!(
        output.contains("m has a configured model weekly already"),
        "{output}"
    );
}

#[test]
fn the_end_of_the_input_creates_nothing() {
    let (draft, _) = run("weekly\n", &[]);
    let err = draft.expect_err("stopped").to_string();
    assert!(err.contains("nothing was created"), "{err}");
}
