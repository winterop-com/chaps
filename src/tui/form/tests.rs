use super::*;
use serde_json::json;

fn template() -> Template {
    configs::parse_template(&json!({
        "id": 30,
        "name": "svc",
        "requiredCovariates": ["population"],
        "userOptions": {
            "n_lags": {"type": "integer", "default": 3},
            "method": {"enum": ["fast", "exact"], "default": "fast"}
        }
    }))
    .expect("a template")
}

fn current() -> Config {
    let rows = json!([{"id": 4, "name": "svc:weekly",
        "additionalContinuousCovariates": ["rainfall"],
        "userOptionValues": {"n_lags": 3}}]);
    configs::configs_of(&rows, "svc").remove(0)
}

#[test]
fn keys_edit_the_field_under_the_cursor_and_end_the_form() {
    let mut form = form_for(&template());
    assert_eq!(form.apply(Action::FormChar('w')), Event::Edited);
    assert_eq!(form.apply(Action::Down), Event::Edited);
    form.apply(Action::FormChar('7'));
    form.apply(Action::FormChar('8'));
    form.apply(Action::FormBackspace);
    assert_eq!(form.fields[0].input, "w");
    assert_eq!(form.fields[1].input, "7");
    form.apply(Action::Up);
    assert_eq!(form.cursor, 0);
    assert_eq!(form.apply(Action::FormSubmit), Event::Submit);
    assert_eq!(form.apply(Action::FilterCancel), Event::Cancel);
    // ctrl-c is Quit, and leaves too.
    assert_eq!(form.apply(Action::Quit), Event::Cancel);
}

#[test]
fn covariates_are_a_field_only_where_the_template_takes_some() {
    let mut template = template();
    let labels =
        |form: &Form| -> Vec<String> { form.fields.iter().map(|f| f.label.clone()).collect() };
    assert_eq!(labels(&form_for(&template)), ["name", "method", "n_lags"]);
    template.allowed_covariates = vec!["rainfall".to_string()];
    let form = form_for(&template);
    assert_eq!(labels(&form).last().map(String::as_str), Some("covariates"));
    let about = &form.fields.last().expect("a field").description;
    assert!(about.contains("Only rainfall are allowed."), "{about}");
    assert!(about.contains("Every run gets population."), "{about}");
}

#[test]
fn an_update_holds_the_values_it_has_and_keeps_them() {
    let mut template = template();
    template.free_covariates = true;
    let form = form_for_update(&template, &current());
    assert_eq!(form.fixed_name.as_deref(), Some("weekly"));
    assert!(form.fields.iter().all(|f| f.role != Role::Name));
    let lags = form
        .fields
        .iter()
        .find(|f| f.label == "n_lags")
        .expect("n_lags");
    assert_eq!(lags.input, "3");
    assert!(lags.had);
    let covariates = form.fields.last().expect("covariates");
    assert_eq!(covariates.input, "rainfall");
    // Unchanged, the draft is the configured model as it is, though its
    // n_lags is the default.
    let draft = draft_of(&form, &template, &[], "m").expect("a draft");
    assert_eq!(draft.variant, "weekly");
    assert_eq!(draft.values, current().values);
    assert_eq!(draft.covariates, current().covariates);
}

#[test]
fn a_new_value_at_its_default_is_left_out_and_a_bad_one_refused() {
    let mut form = form_for(&template());
    form.fields[0].input = "w".to_string();
    form.fields[2].input = "3".to_string();
    let draft = draft_of(&form, &template(), &[], "m").expect("a draft");
    assert!(draft.values.is_empty());
    form.fields[2].input = "x".to_string();
    assert!(draft_of(&form, &template(), &[], "m").is_err());
    form.fields[0].input = String::new();
    form.fields[2].input = String::new();
    assert!(draft_of(&form, &template(), &[], "m").is_err());
}
