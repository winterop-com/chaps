//! A registered model that chap-core has no configured model for.

use super::*;

fn row(name: &str, version: Option<&str>) -> crate::modeltest::ConfiguredModel {
    crate::modeltest::ConfiguredModel {
        id: 1,
        name: name.to_string(),
        archived: false,
        covariates: Vec::new(),
        version: version.map(str::to_string),
    }
}

#[test]
fn a_registered_model_without_a_configured_model_is_not_configured() {
    let registered = [
        registered("chapkit-ewars-model", 5),
        registered("auto-arima-chapkit", 5),
    ];
    let mut rows = model_rows(&enabled(), &registered, &BTreeSet::new(), NOW);
    // The ewars model has a configured model of an older version only.
    let configured = [
        row("chapkit-ewars-model:monthly_climate", Some("0.9.0")),
        row("auto-arima-chapkit", Some("1.0.0")),
    ];
    mark_unconfigured(&mut rows, &registered, &configured);
    assert_eq!(rows[0].state, ModelState::NotConfigured);
    assert_eq!(rows[0].state.label(), "registered, not configured");
    // It is something to do, and not a failure of `varde status`.
    assert!(!rows[0].state.is_problem());
    // A row that is not registered stays as it is.
    assert_eq!(rows[1].state, ModelState::NotRunning);
    assert_eq!(rows[2].state, ModelState::Registered);

    assert_eq!(
        hints(&rows, false, None),
        [
            "chapkit-ewars-model: chap-core has no configured model for it, so nothing can run \
             it; run `varde models configure`",
            "chapkit-rwanda-malaria-bym-model: start Chap with `varde up`, then `varde logs \
             chapkit-rwanda-malaria-bym-model`",
        ]
    );
    // A model that nothing can run is not one to test.
    assert_eq!(test_hint(&rows), None);
}
