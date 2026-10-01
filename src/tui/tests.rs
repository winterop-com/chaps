use super::*;
use crate::project::ProjectState;
use crate::registry::load_embedded;
use crate::tui::app::Action;

/// A refresh keeps the unsaved edits, the filter and the cursor, because
/// re-fetching a catalogue is not a reason to lose what was toggled.
#[test]
fn what_is_carried_over_a_refresh_survives_a_new_catalogue() {
    let registry = load_embedded().expect("embedded snapshot parses");
    let mut app = App::new(&registry, &ProjectState::default());
    app.reduce(Action::ToggleTemplates);
    app.reduce(Action::Toggle);
    let toggled = app.selected_model().expect("a row is selected").id.clone();
    let carry = Carry::of(&app);
    assert_eq!(carry.edited.len(), 1);
    assert_eq!(carry.edited[0].0, toggled);

    // A fresh browser over the same catalogue: the edit comes back.
    let mut fresh = App::new(&registry, &ProjectState::default());
    assert!(!fresh.has_changes());
    carry.restore(&mut fresh);
    assert!(fresh.has_changes());
    assert!(fresh.show_templates);
    assert_eq!(fresh.selection().enable[0].id, toggled);
    assert_eq!(
        fresh.selected_model().map(|m| m.id.clone()),
        Some(toggled),
        "the cursor stays on the row it was on"
    );
}

/// A new catalogue says nothing about components, so a refresh has to
/// bring the whole wanted set back: it is counted in the header and saved
/// by the same `s`, and losing it to a re-fetch would be silent.
#[test]
fn a_refresh_keeps_the_component_edits_and_the_page() {
    use crate::components::Component;

    let registry = load_embedded().expect("embedded snapshot parses");
    let mut app = App::new(&registry, &ProjectState::default());
    app.reduce(Action::NextPage);
    app.reduce(Action::Down);
    app.reduce(Action::Toggle);
    assert_eq!(app.selected_component(), Component::Ocs);
    assert!(app.components.ocs.enabled);

    let carry = Carry::of(&app);
    let mut fresh = App::new(&registry, &ProjectState::default());
    assert!(!fresh.has_changes());
    carry.restore(&mut fresh);

    assert!(fresh.components.ocs.enabled);
    assert!(fresh.has_changes() && fresh.dirty);
    assert_eq!(fresh.counts().pending, 1);
    assert_eq!(fresh.page, app::Page::Components);
    assert_eq!(
        fresh.selected_component(),
        Component::Ocs,
        "the cursor stays on the row it was on"
    );
    assert!(
        fresh
            .selection()
            .components
            .expect("the set is selected")
            .ocs
            .enabled
    );
}
