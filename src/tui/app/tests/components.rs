//! The browser's components page: its rows, ports, palette entries and keys.

use super::*;

/// Put the cursor on a component and make sure the page is up.
fn focus_component(app: &mut App, component: Component) {
    app.page = Page::Components;
    app.component_cursor = Component::ALL
        .iter()
        .position(|c| *c == component)
        .expect("every component is listed");
}

/// `Tab` is the only way between the two lists, and going there leaves the
/// model page exactly as it was.
#[test]
fn tab_walks_the_pages_and_the_model_page_is_untouched() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    assert_eq!(app.page, Page::Models);
    app.reduce(Action::Down);
    let on = app.selected_model().map(|m| m.id.clone());
    let visible = app.visible.len();

    assert!(app.reduce(Action::NextPage).is_none());
    assert_eq!(app.page, Page::Components);
    assert_eq!(app.component_cursor, 0);
    // Movement on the components page moves the components page's cursor.
    app.reduce(Action::Down);
    assert_eq!(app.component_cursor, 1);
    app.reduce(Action::Bottom);
    assert_eq!(app.component_cursor, Component::ALL.len() - 1);
    app.reduce(Action::Down);
    assert_eq!(
        app.component_cursor,
        Component::ALL.len() - 1,
        "there are only three"
    );
    app.reduce(Action::Top);
    assert_eq!(app.component_cursor, 0);

    // Back, and nothing about the model list moved.
    app.reduce(Action::NextPage);
    assert_eq!(app.page, Page::Models);
    assert_eq!(app.selected_model().map(|m| m.id.clone()), on);
    assert_eq!(app.visible.len(), visible);
    assert!(!app.has_changes());

    // shift-Tab is the other direction, which with two pages is the same
    // flip.
    app.reduce(Action::PrevPage);
    assert_eq!(app.page, Page::Components);
    app.reduce(Action::PrevPage);
    assert_eq!(app.page, Page::Models);
}

/// The rows the components page draws, in the columns `varde components
/// list` prints them in.
#[test]
fn the_component_rows_say_what_each_one_is_and_where_it_is_reached() {
    let registry = registry();
    let mut state = empty_state();
    state.components.set_enabled(Component::Ocs, true);
    let app = App::new(&registry, &state);

    let lines = app.component_lines();
    assert_eq!(
        lines
            .iter()
            .map(|l| l.component)
            .collect::<Vec<Component>>(),
        Component::ALL.to_vec()
    );
    // chap-core's port is the API port, which lives in project.yaml.
    assert_eq!(
        lines[0].reach,
        format!("http://localhost:{}", state.api_port)
    );
    assert!(lines[0].enabled && lines[0].recorded);
    assert_eq!(lines[1].reach, "http://localhost:8790");
    assert_eq!(lines[1].summary, Component::Ocs.summary());
    assert_eq!(lines[2].reach, "-", "a component this deployment has not");
    assert!(!lines[2].enabled);

    // An OCS instance behind a proxy names the proxy where the address
    // would be, which is what `Components::ocs_reach` does for the CLI.
    let mut app = App::new(&registry, &state);
    app.components.ocs.port = None;
    app.components.ocs.base_url = Some("https://ocs.example.org".to_string());
    assert_eq!(
        app.component_lines()[1].reach,
        "internal (proxy: https://ocs.example.org)"
    );
    // And the object store says where it is reached from inside.
    app.components.set_enabled(Component::S3, true);
    assert_eq!(app.component_lines()[2].reach, "internal");
    app.components.s3.port = Some(18091);
    assert_eq!(app.component_lines()[2].reach, "http://localhost:18091");
}

/// `space` on a component is the whole of adding one: the selection carries
/// the wanted set, and the models beside it are untouched.
#[test]
fn space_on_ocs_selects_the_component_set_that_has_it() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus_component(&mut app, Component::Ocs);

    assert!(app.reduce(Action::Toggle).is_none());
    assert!(app.dirty);
    assert!(app.components.ocs.enabled);

    let selection = app.selection();
    assert!(selection.enable.is_empty() && selection.disable.is_empty());
    let wanted = selection.components.expect("the component set is selected");
    assert!(wanted.ocs.enabled);
    assert!(wanted.chap_core.enabled, "chap-core is left where it was");
    assert!(!wanted.s3.enabled);
    assert_eq!(app.counts().pending, 1);

    // The strip says what saving would do, in the same shape a model does.
    let changes = app.changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, ChangeKind::Add);
    assert_eq!(changes[0].name, "ocs");
    assert_eq!(changes[0].detail, "enable on port 8790");

    // Off again is back where the project had it, so there is nothing to
    // save and nothing to say about components.
    app.reduce(Action::Toggle);
    assert!(!app.has_changes());
    assert!(app.selection().components.is_none());
    assert_eq!(app.counts().pending, 0);
}

/// Turning a component off is a removal, and it says what is kept.
#[test]
fn switching_a_component_off_is_a_removal_that_keeps_the_volume() {
    let registry = registry();
    let mut state = empty_state();
    state.components.set_enabled(Component::Ocs, true);
    let mut app = App::new(&registry, &state);
    focus_component(&mut app, Component::Ocs);

    app.reduce(Action::Toggle);
    let changes = app.changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, ChangeKind::Remove);
    assert_eq!(changes[0].detail, "disable · the ocs_data volume is kept");
    let wanted = app.selection().components.expect("a set is selected");
    assert!(!wanted.ocs.enabled);
    assert_eq!(
        wanted.ocs.port,
        Some(8790),
        "its port is left alone, so switching it back on restores it"
    );
}

/// The removal detail on its own, including the shapes no component has
/// yet: one volume reads as the one it is, and several are all named, since
/// each is a name the operator would have to remove by hand.
#[test]
fn the_removal_detail_names_every_volume_that_stays() {
    assert_eq!(
        disable_detail(&[]),
        "disable · its own volumes are kept",
        "chap-core's are upstream's"
    );
    assert_eq!(
        disable_detail(&["ocs_data"]),
        "disable · the ocs_data volume is kept"
    );
    assert_eq!(
        disable_detail(&["a_data", "b_data", "c_data"]),
        "disable · the a_data, b_data, c_data volumes are kept"
    );
}

/// chap-core can go off with a model enabled: the model stays, and the
/// selection carries the component set that says it runs on its own.
#[test]
fn chap_core_can_be_switched_off_while_a_model_is_enabled() {
    let registry = registry();
    let state = state_with(&registry, EWARS, Some(Channel::Stable));
    let mut app = App::new(&registry, &state);
    focus_component(&mut app, Component::ChapCore);

    app.reduce(Action::Toggle);
    assert!(!app.components.chap_core.enabled);
    let selection = app.selection();
    assert!(selection.disable.is_empty(), "the model stays");
    assert!(!selection.components.expect("a set").chap_core.enabled);
}

/// `p` on a component: a number or `none`, never `auto`, and never on
/// chap-core - whose host port is the API port.
#[test]
fn the_component_port_prompt_takes_a_number_or_none() {
    let registry = registry();
    let mut state = empty_state();
    state.components.set_enabled(Component::Ocs, true);
    let mut app = App::new(&registry, &state);
    focus_component(&mut app, Component::Ocs);

    app.reduce(Action::PortPrompt);
    assert_eq!(app.mode, Mode::Port);
    assert_eq!(app.port_input, "8790", "prefilled with what it publishes");

    ask_for_port(&mut app, "none");
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.components.ocs.port, None);
    let wanted = app.selection().components.expect("a set is selected");
    assert_eq!(wanted.ocs.port, None, "no host port survives the save");
    assert!(wanted.ocs.enabled, "and it is still enabled");
    let changes = app.changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, ChangeKind::Update);
    assert_eq!(changes[0].detail, "remove the host port");

    // A number is taken as it is: a component is not in the model range,
    // so the range is not checked.
    ask_for_port(&mut app, "18090");
    assert_eq!(app.components.ocs.port, Some(18090));
    assert_eq!(app.changes()[0].detail, "publish port 18090");

    // And `auto` is refused with what to type instead.
    ask_for_port(&mut app, "auto");
    assert_eq!(app.mode, Mode::Port);
    assert_eq!(app.port_error.as_deref(), Some(COMPONENT_HAS_NO_AUTO_PORT));
    assert_eq!(app.components.ocs.port, Some(18090), "nothing was taken");
    app.reduce(Action::FilterCancel);

    // `P` takes the port away without a prompt, as it does for a model.
    app.reduce(Action::RemovePort);
    assert_eq!(app.components.ocs.port, None);
}

/// The DHIS2 row goes through the same three accessors OCS does, so `p`,
/// `P` and the REACH cell read its own `port:` and nothing else.
#[test]
fn the_dhis2_row_publishes_and_unpublishes_like_the_ocs_one() {
    let registry = registry();
    let mut state = empty_state();
    state.components.set_enabled(Component::Dhis2, true);
    let mut app = App::new(&registry, &state);
    focus_component(&mut app, Component::Dhis2);
    assert_eq!(
        app.component_reach(Component::Dhis2),
        "http://localhost:8780"
    );

    app.reduce(Action::PortPrompt);
    assert_eq!(app.port_input, "8780", "prefilled with what it publishes");
    ask_for_port(&mut app, "none");
    assert_eq!(app.components.dhis2.port, None);
    assert_eq!(
        app.component_reach(Component::Dhis2),
        "internal",
        "a DHIS2 behind a proxy is reached somewhere else, not nowhere"
    );
    let wanted = app.selection().components.expect("a set is selected");
    assert_eq!(wanted.dhis2.port, None);
    assert!(wanted.dhis2.enabled);

    ask_for_port(&mut app, "18080");
    assert_eq!(app.components.dhis2.port, Some(18080));
    assert_eq!(app.changes()[0].detail, "publish port 18080");
    ask_for_port(&mut app, "auto");
    assert_eq!(app.port_error.as_deref(), Some(COMPONENT_HAS_NO_AUTO_PORT));
    app.reduce(Action::FilterCancel);

    // A component that is off is asked to be enabled first, as `s3` is.
    app.components.set_enabled(Component::Dhis2, false);
    app.reduce(Action::PortPrompt);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.message.as_deref(), Some(PORT_NEEDS_COMPONENT_HINT));
    assert_eq!(
        app.component_reach(Component::Dhis2),
        "-",
        "a component this deployment does not have reaches nothing"
    );
}

#[test]
fn the_component_port_prompt_refuses_chap_core_and_a_component_that_is_off() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());

    focus_component(&mut app, Component::ChapCore);
    app.reduce(Action::PortPrompt);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.message.as_deref(), Some(CORE_PORT_IS_API_PORT));
    app.reduce(Action::RemovePort);
    assert_eq!(app.message.as_deref(), Some(CORE_PORT_IS_API_PORT));

    focus_component(&mut app, Component::S3);
    app.reduce(Action::PortPrompt);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.message.as_deref(), Some(PORT_NEEDS_COMPONENT_HINT));
    assert!(!app.has_changes());
}

/// What the component prompt accepts, away from the reducer.
#[test]
fn the_component_port_prompt_parses_a_number_and_nothing_else() {
    let taken = vec![(18090u16, "the s3 component".to_string())];
    let ok = |text: &str| parse_component_port(text, 8000, &taken).expect(text);
    assert_eq!(ok(""), None);
    assert_eq!(ok(" none "), None);
    assert_eq!(ok("NONE"), None);
    assert_eq!(ok(" 9000 "), Some(9000));
    // Nothing about the model range: a component is not in it.
    assert_eq!(ok("80"), Some(80));

    let why = |text: &str| parse_component_port(text, 8000, &taken).expect_err(text);
    assert_eq!(why("auto"), COMPONENT_HAS_NO_AUTO_PORT);
    assert!(why("8000").contains("API port"));
    assert!(why("18090").contains("already taken by the s3 component"));
    assert!(why("0").contains("not a port"));
    assert!(why("nine thousand").contains("not a port"));
}

/// Two components cannot be sent to one port, and neither can a component
/// and a model.
#[test]
fn a_component_port_another_row_asks_for_is_refused() {
    let registry = registry();
    let mut state = empty_state();
    state.components.set_enabled(Component::Ocs, true);
    state.components.set_enabled(Component::S3, true);
    let mut app = App::new(&registry, &state);

    focus_component(&mut app, Component::S3);
    ask_for_port(&mut app, "8790");
    assert_eq!(app.mode, Mode::Port);
    assert!(
        app.port_error
            .as_deref()
            .unwrap_or_default()
            .contains("already taken by the ocs component"),
        "{:?}",
        app.port_error
    );
    ask_for_port(&mut app, "18091");
    assert_eq!(app.components.s3.port, Some(18091));

    // A model's published port is taken too.
    app.page = Page::Models;
    focus(&mut app, EWARS);
    app.reduce(Action::Toggle);
    ask_for_port(&mut app, "5010");
    focus_component(&mut app, Component::S3);
    ask_for_port(&mut app, "5010");
    assert!(
        app.port_error
            .as_deref()
            .unwrap_or_default()
            .contains(&format!("already taken by the model {EWARS}")),
        "{:?}",
        app.port_error
    );
}

/// `u` is "nothing I did this session", both pages included, and the quit
/// confirmation counts both too.
#[test]
fn discarding_and_quitting_account_for_both_pages() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, EWARS);
    app.reduce(Action::Toggle);
    focus_component(&mut app, Component::Ocs);
    app.reduce(Action::Toggle);
    assert_eq!(app.counts().pending, 2);

    // A component change alone is enough to be asked before quitting.
    let mut only_component = App::new(&registry, &empty_state());
    focus_component(&mut only_component, Component::S3);
    only_component.reduce(Action::Toggle);
    assert!(only_component.dirty);
    assert!(only_component.reduce(Action::Quit).is_none());
    assert_eq!(only_component.mode, Mode::ConfirmQuit);

    app.reduce(Action::Discard);
    assert_eq!(app.message.as_deref(), Some(DISCARDED_HINT));
    assert!(!app.has_changes());
    assert!(!app.components.ocs.enabled);
    assert!(app.selection().components.is_none());
}

/// The palette is where someone who has never pressed Tab finds the page
/// and the components on it.
#[test]
fn the_palette_reaches_the_other_page_and_every_component() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::Palette);
    for c in "components page".chars() {
        app.reduce(Action::PaletteChar(c));
    }
    let matched = app.palette_matches();
    assert_eq!(matched.len(), 1, "{matched:?}");
    assert_eq!(matched[0].id, CommandId::Page);
    assert_eq!(matched[0].key, "tab");
    app.reduce(Action::PaletteRun);
    assert_eq!(app.page, Page::Components);
    assert_eq!(app.mode, Mode::Browse);
    // And from there it offers the way back.
    assert!(
        app.commands()
            .iter()
            .any(|c| c.label == "Go to the models page")
    );

    // Every component by name, from either page.
    for (query, component) in [
        ("turn the ocs", Component::Ocs),
        ("turn the s3", Component::S3),
        ("turn the dhis2", Component::Dhis2),
    ] {
        let mut app = App::new(&registry, &empty_state());
        app.reduce(Action::Palette);
        for c in query.chars() {
            app.reduce(Action::PaletteChar(c));
        }
        let matched = app.palette_matches();
        assert_eq!(matched.len(), 1, "{query}: {matched:?}");
        assert_eq!(matched[0].id, CommandId::Component(component));
        app.reduce(Action::PaletteRun);
        assert_eq!(app.page, Page::Models, "it does not move the page");
        assert!(app.components.is_enabled(component), "{query} turned it on");
        assert_eq!(
            app.selected_component(),
            component,
            "the cursor followed, so the page talks about it"
        );
    }

    // Turning chap-core off from the palette works with a model enabled,
    // just as `space` does.
    let state = state_with(&registry, EWARS, Some(Channel::Stable));
    let mut app = App::new(&registry, &state);
    app.reduce(Action::Palette);
    for c in "turn the chap-core".chars() {
        app.reduce(Action::PaletteChar(c));
    }
    app.reduce(Action::PaletteRun);
    assert!(!app.components.chap_core.enabled);
}

/// On the components page the three row keys act on a component, and the
/// entries that only make sense for a catalogue are not offered.
#[test]
fn the_palette_follows_the_page_it_was_opened_from() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus_component(&mut app, Component::Ocs);
    let labels: Vec<String> = app.commands().iter().map(|c| c.label.clone()).collect();
    assert!(
        labels.contains(&"Enable or disable the ocs component".to_string()),
        "{labels:?}"
    );
    assert!(
        labels.contains(&"Set a host port for the ocs component".to_string()),
        "{labels:?}"
    );
    for gone in [
        "Show or hide templates",
        "Filter the model list",
        "Open the repository of CHAP-EWARS",
    ] {
        assert!(
            !labels.iter().any(|label| label == gone),
            "{gone} is offered on the components page: {labels:?}"
        );
    }

    // And the page's own docs chapter is what the palette opens.
    app.reduce(Action::Palette);
    for c in "varde documentation".chars() {
        app.reduce(Action::PaletteChar(c));
    }
    app.reduce(Action::PaletteRun);
    assert_eq!(
        app.take_effect(),
        Some(Effect::Open(format!(
            "{}{COMPONENTS_DOCS_CHAPTER}",
            crate::cli::DOCS_URL
        )))
    );
}

/// `o` on a published component asks the caller to open it, and every
/// other answer goes in the footer instead: the reducer stays pure, so the
/// whole of it is testable without a terminal.
#[test]
fn o_opens_a_published_component_and_says_why_when_it_cannot() {
    let registry = registry();
    let mut state = empty_state();
    state.api_port = 18000;
    state.components.set_enabled(Component::Dhis2, true);
    state.components.dhis2.port = Some(18080);
    let mut app = App::new(&registry, &state);

    // A component with a host port: the web interface at its root.
    focus_component(&mut app, Component::Dhis2);
    app.reduce(Action::Open);
    assert_eq!(
        app.take_effect(),
        Some(Effect::Open("http://localhost:18080".to_string()))
    );

    // chap-core: the API answers JSON everywhere but `/docs`, and the port
    // is this project's own rather than 8000.
    focus_component(&mut app, Component::ChapCore);
    app.reduce(Action::Open);
    assert_eq!(
        app.take_effect(),
        Some(Effect::Open("http://localhost:18000/docs".to_string()))
    );

    // A component this deployment does not have: nothing is opened, and the
    // footer names the key that adds it.
    focus_component(&mut app, Component::Ocs);
    app.reduce(Action::Open);
    assert!(app.take_effect().is_none());
    assert_eq!(app.message.as_deref(), Some(OPEN_NEEDS_COMPONENT));

    // The object store has no web interface, on or off.
    focus_component(&mut app, Component::S3);
    app.reduce(Action::Open);
    assert!(app.take_effect().is_none());
    assert_eq!(app.message.as_deref(), Some(crate::open::NO_WEB_INTERFACE));
}

/// A component with no host port is reached inside the deployment, so `o`
/// says where and names `p` rather than opening a port nothing is on.
#[test]
fn o_on_a_component_with_no_host_port_names_the_internal_address() {
    let registry = registry();
    let mut state = empty_state();
    state.components.set_enabled(Component::Ocs, true);
    state.components.ocs.port = None;
    let mut app = App::new(&registry, &state);
    focus_component(&mut app, Component::Ocs);

    app.reduce(Action::Open);
    assert!(app.take_effect().is_none(), "nothing is opened");
    let message = app.message.clone().expect("a footer note");
    assert!(message.contains("http://ocs:9000"), "{message}");
    assert!(message.contains("p publishes one"), "{message}");
}

/// A row this session only just enabled has no compose file and nothing
/// running behind it, so its address is a plan: `o` says what is missing
/// instead of opening it.
#[test]
fn o_on_a_pending_component_asks_for_the_save_first() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus_component(&mut app, Component::Dhis2);
    app.reduce(Action::Toggle);
    assert!(app.components.dhis2.enabled && !app.initial_components.dhis2.enabled);

    app.reduce(Action::Open);
    assert!(app.take_effect().is_none());
    assert_eq!(app.message.as_deref(), Some(OPEN_NEEDS_SAVING));
}

/// A port changed but not saved is not the port anything is published on,
/// so `o` opens the one that is.
#[test]
fn o_opens_the_port_this_deployment_publishes_not_the_pending_one() {
    let registry = registry();
    let mut state = empty_state();
    state.components.set_enabled(Component::Ocs, true);
    state.components.ocs.port = Some(9000);
    let mut app = App::new(&registry, &state);
    focus_component(&mut app, Component::Ocs);
    app.components.ocs.port = Some(9010);

    app.reduce(Action::Open);
    assert_eq!(
        app.take_effect(),
        Some(Effect::Open("http://localhost:9000".to_string())),
        "the wanted port is a plan until `s` and `varde up`"
    );
}

/// The palette is where the key is discoverable, so the components page
/// offers it there too, on the row under the cursor.
#[test]
fn the_palette_offers_the_web_interface_on_the_components_page() {
    let registry = registry();
    let mut state = empty_state();
    state.components.set_enabled(Component::Dhis2, true);
    let mut app = App::new(&registry, &state);
    focus_component(&mut app, Component::Dhis2);

    let entry = app
        .commands()
        .into_iter()
        .find(|command| command.id == CommandId::Web)
        .expect("the components page offers it");
    assert_eq!(entry.label, "Open the web interface of the dhis2 component");
    assert_eq!(entry.key, "o");

    // Running it from the palette is the same thing the key does.
    app.reduce(Action::Palette);
    for c in "web interface".chars() {
        app.reduce(Action::PaletteChar(c));
    }
    app.reduce(Action::PaletteRun);
    assert_eq!(
        app.take_effect(),
        Some(Effect::Open("http://localhost:8780".to_string()))
    );

    // The models page has a repository to open instead, and does not offer
    // a component's web interface at all.
    app.page = Page::Models;
    assert!(
        !app.commands()
            .iter()
            .any(|command| command.id == CommandId::Web)
    );
}

/// A session with chap-core switched off can still enable a model; it
/// will run on its own.
#[test]
fn a_model_can_be_enabled_while_this_session_has_chap_core_off() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus_component(&mut app, Component::ChapCore);
    app.reduce(Action::Toggle);
    assert!(!app.components.chap_core.enabled);

    app.page = Page::Models;
    focus(&mut app, EWARS);
    app.reduce(Action::Toggle);
    assert!(app.selected().unwrap().enabled);
    let selection = app.selection();
    assert_eq!(selection.enable.len(), 1);
    assert!(!selection.components.expect("a set").chap_core.enabled);
}

/// `v` on the `dhis2` component picks its version: the minor lines with a
/// seed dump plus the one in force, the change rides on the component
/// set, and moving off the recorded version says DHIS2 migrates forward.
#[test]
fn the_dhis2_version_is_picked_from_the_components_page() {
    let registry = registry();
    let mut state = empty_state();
    state.components.set_enabled(Component::Dhis2, true);
    state.components.dhis2.image_tag = "2.42".to_string();
    let mut app = App::new(&registry, &state);
    focus_component(&mut app, Component::Dhis2);

    app.reduce(Action::ChannelPrompt);
    assert_eq!(app.mode, Mode::Dhis2Version);
    assert_eq!(app.dhis2_versions, vec!["2.43", "2.42", "2.41"]);
    assert_eq!(app.dhis2_version_cursor, 1, "it opens on the one in force");

    app.reduce(Action::Down);
    app.reduce(Action::ChannelApply);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.components.dhis2.image_tag, "2.41");
    let note = app.message.clone().unwrap_or_default();
    assert!(note.contains("migrates a schema forward only"), "{note}");
    let wanted = app.selection().components.expect("a set");
    assert_eq!(wanted.dhis2.image_tag, "2.41");

    // Back to the recorded one: nothing to save, nothing to warn about.
    app.reduce(Action::ChannelPrompt);
    app.reduce(Action::Up);
    app.reduce(Action::ChannelApply);
    assert!(app.message.is_none());
    assert!(!app.has_changes());

    // A tag varde has no seed for is still offered while it is in force.
    assert_eq!(
        dhis2_versions("2.40.3"),
        vec!["2.40.3", "2.43", "2.42", "2.41"]
    );
}

/// A DHIS2 that is off has no version to pick yet, and says how to turn it on.
#[test]
fn a_dhis2_that_is_off_says_so_instead_of_offering_versions() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus_component(&mut app, Component::Dhis2);
    app.reduce(Action::ChannelPrompt);
    assert_eq!(app.mode, Mode::Browse);
    assert!(
        app.message
            .as_deref()
            .unwrap_or_default()
            .contains("turn dhis2 on first")
    );
}

/// The keys that belong to a catalogue do nothing on the components page:
/// a component follows no channel and three rows are not filtered.
#[test]
fn the_model_only_keys_are_inert_on_the_components_page() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus_component(&mut app, Component::Ocs);
    for action in [
        Action::ChannelPrompt,
        Action::StartFilter,
        Action::ToggleTemplates,
        Action::ImageRef,
    ] {
        app.reduce(action.clone());
        assert_eq!(app.mode, Mode::Browse, "{action:?} opened something");
        assert!(app.take_effect().is_none(), "{action:?} left the terminal");
    }
    assert!(!app.show_templates);
    assert!(app.filter.is_empty());
    assert!(!app.has_changes());

    // `i` always has a row to describe here, however the model list is
    // filtered.
    app.reduce(Action::Info);
    assert_eq!(app.mode, Mode::Info);
}
