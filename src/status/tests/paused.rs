//! A container that docker reports paused, as an interrupted `varde backup
//! create` can leave it.

use super::*;

#[test]
fn a_paused_row_says_paused_and_names_the_way_out() {
    let mut report = report(
        ApiHealth::Off,
        &[],
        vec![component(ComponentState::NotRunning)],
    );
    report.models = vec![row(
        "chapkit-ewars-model",
        ModelState::NotRunning,
        "internal",
    )];
    let paused = BTreeSet::from(["ocs".to_string(), "chapkit-ewars-model".to_string()]);
    mark_paused(&mut report, &paused);
    assert_eq!(report.components[0].state, ComponentState::Paused);
    assert_eq!(report.components[0].state.label(), "paused");
    assert!(report.components[0].state.is_problem());
    assert_eq!(report.models[0].state, ModelState::Paused);

    assert_eq!(
        components_closing_line(&report.components),
        "1 of 1 component is paused; run `varde up` to resume it"
    );
    assert_eq!(
        closing_line(&report.models),
        "1 of 1 model is paused; run `varde up` to resume it"
    );
    assert_eq!(
        paused_line(&BTreeSet::from(["dhis2".to_string()])).as_deref(),
        Some("dhis2 is paused, so it does not answer; run `varde up` to resume it")
    );
    assert_eq!(
        paused_line(&paused).as_deref(),
        Some(
            "chapkit-ewars-model, ocs are paused, so they do not answer; run `varde up` to \
             resume them"
        )
    );
    assert_eq!(paused_line(&BTreeSet::new()), None);
}
