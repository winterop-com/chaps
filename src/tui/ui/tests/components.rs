//! The components page as it is drawn: its table, dialogs and detail overlays.

use super::*;

/// A project state with OCS enabled beside chap-core.
fn state_with_ocs(port: Option<u16>) -> ProjectState {
    let mut state = ProjectState::default();
    state.components.set_enabled(Component::Ocs, true);
    state.components.ocs.port = port;
    state
}

/// A project state with OCS and DHIS2 enabled beside chap-core, DHIS2 on
/// its default port and its default seed.
fn state_with_dhis2() -> ProjectState {
    let mut state = state_with_ocs(Some(9000));
    state.components.set_enabled(Component::Dhis2, true);
    state
}

/// Put the browser on the components page, with the cursor on one of them.
fn on_components(app: &mut App, component: Component) {
    app.page = Page::Components;
    app.component_cursor = Component::ALL
        .iter()
        .position(|c| *c == component)
        .expect("every component is listed");
}

/// The second page: the title says which list it is, the box is named
/// after it, and the columns are the four `varde components list` prints.
#[test]
fn the_components_page_draws_the_columns_components_list_prints() {
    let registry = registry();
    let mut app = App::new(&registry, &state_with_ocs(Some(9000)));
    let before = render(&app, 120, 40);
    assert!(before.contains("varde · models"), "{before}");

    app.reduce(Action::NextPage);
    let screen = render(&app, 120, 40);
    assert!(screen.contains("varde · components"), "{screen}");
    assert!(
        screen.contains("4 components · 2 enabled · 0 pending"),
        "{screen}"
    );
    assert!(screen.contains("Components"), "{screen}");
    assert!(!screen.contains("Marketplace"), "{screen}");
    assert!(
        !screen.contains("CHAP-EWARS"),
        "the model list is not drawn"
    );

    let head = line_with(&screen, "COMPONENT");
    for column in ["COMPONENT", "STATE", "REACH", "WHAT IT IS"] {
        assert!(head.contains(column), "{head}");
    }
    // Every component has a row, and the headings sit over the cells.
    let core = line_with(&screen, "chap-core");
    assert_eq!(column_of(head, "COMPONENT"), column_of(core, "chap-core"));
    assert_eq!(column_of(head, "STATE"), column_of(core, "enabled"));
    assert_eq!(
        column_of(head, "REACH"),
        column_of(core, "http://localhost:8700"),
        "chap-core is reached at the API port"
    );
    let ocs = line_with(&screen, " ocs ");
    assert!(ocs.contains("✓ ocs"), "{ocs}");
    assert!(ocs.contains("http://localhost:9000"), "{ocs}");
    assert!(ocs.contains("Open Climate Service"), "{ocs}");
    let s3 = line_with(&screen, " s3 ");
    assert!(s3.contains("off"), "{s3}");
    assert!(s3.contains("RustFS"), "{s3}");
    assert!(!s3.contains('✓'), "{s3}");
    // The fourth row, off here: the name fits the column and the sentence
    // is what `components list` says.
    let dhis2 = line_with(&screen, " dhis2 ");
    assert!(dhis2.contains("off"), "{dhis2}");
    assert!(dhis2.contains("DHIS2 and its own database"), "{dhis2}");
    assert_eq!(
        column_of(head, "COMPONENT"),
        column_of(dhis2, "dhis2"),
        "the name column still holds the longest name"
    );

    // The strip under it describes the row, and the bar offers the page.
    assert!(screen.contains("i for details"), "{screen}");
    assert!(screen.contains("[tab] page"), "{screen}");
    assert!(!screen.contains("[v] channel"), "{screen}");
    assert!(!screen.contains("[t] templates"), "{screen}");
}

/// A pending component change is marked before it is saved, exactly as a
/// model's is, and the strip says what saving would do.
#[test]
fn a_pending_component_change_is_marked_and_listed() {
    let registry = registry();
    let mut app = App::new(&registry, &state_with_ocs(Some(9000)));
    on_components(&mut app, Component::S3);
    app.reduce(Action::Toggle);

    let screen = render(&app, 120, 40);
    let s3 = line_with(&screen, " s3 ");
    assert!(s3.contains("+ s3"), "{s3}");
    assert!(s3.contains("adding"), "{s3}");
    assert!(
        s3.contains("internal"),
        "the store publishes nothing by default"
    );
    assert!(screen.contains("1 pending change "), "{screen}");
    let listed = line_with(&screen, "enable, on the compose network");
    assert!(listed.contains("+ s3"), "{listed}");

    // Switching an enabled one off reads as a removal that keeps the data.
    on_components(&mut app, Component::Ocs);
    app.reduce(Action::Toggle);
    let screen = render(&app, 120, 40);
    let ocs = line_with(&screen, " ocs ");
    assert!(ocs.contains("- ocs"), "{ocs}");
    assert!(ocs.contains("removing"), "{ocs}");
    assert!(screen.contains("2 pending changes"), "{screen}");
    assert!(screen.contains("the ocs_data volume is kept"), "{screen}");
    // And the header counts both pages' worth.
    assert!(screen.contains("2 pending"), "{screen}");

    // The count survives the trip back to the model page, because one save
    // writes both.
    app.reduce(Action::PrevPage);
    let screen = render(&app, 120, 40);
    assert!(screen.contains("varde · models"), "{screen}");
    assert!(screen.contains("2 pending changes"), "{screen}");
}

/// The port dialog on a component: no range, because a component is not in
/// the model range, and no `auto`, because nothing allocates one for it.
#[test]
fn the_component_port_dialog_offers_a_number_or_none() {
    let registry = registry();
    let mut app = App::new(&registry, &state_with_ocs(Some(9000)));
    on_components(&mut app, Component::Ocs);
    app.reduce(Action::PortPrompt);

    let screen = render(&app, 120, 40);
    assert!(screen.contains("Host port for ocs"), "{screen}");
    assert!(screen.contains("port  › 9000_"), "{screen}");
    assert!(
        screen.contains("now: http://localhost:9000 · api port 8700 is taken"),
        "{screen}"
    );
    assert!(
        screen.contains("[enter] apply  [esc] cancel  [none] no host port"),
        "and it does not offer auto:\n{screen}"
    );
    assert!(!screen.contains("any free port"), "{screen}");
    assert!(!screen.contains("range 5001-5999"), "{screen}");

    // A refusal keeps the box up with the reason inside it.
    app.port_input.clear();
    for c in "auto".chars() {
        app.reduce(Action::PortChar(c));
    }
    app.reduce(Action::PortApply);
    let screen = render(&app, 120, 40);
    assert!(
        screen.contains("Host port for ocs"),
        "still open:\n{screen}"
    );
    assert!(screen.contains("model port range"), "{screen}");
}

/// The overlay is where the rest of a component lives: the file `sync`
/// renders, its data volume, the OCS config, and the two OCS settings this
/// page deliberately leaves to the CLI.
#[test]
fn the_component_overlay_names_the_files_and_the_settings_it_does_not_edit() {
    let registry = registry();
    let mut app = App::new(&registry, &state_with_ocs(Some(9000)));
    on_components(&mut app, Component::Ocs);
    app.reduce(Action::Info);

    let screen = render(&app, 120, 40);
    for needle in [
        "ocs",
        "enabled here",
        "Open Climate Service",
        "http://localhost:9000",
        "compose.ocs.yml · rendered from .varde/components.yaml by `varde sync`",
        "ocs_data · kept when the component is disabled",
        "ocs/climate-service.yaml · yours to edit",
        "not on this page:",
        "`varde components enable ocs --base-url URL`",
        "`varde components enable ocs --read-only`",
    ] {
        assert!(screen.contains(needle), "{needle} is missing:\n{screen}");
    }
    assert!(screen.contains("[esc] close"), "{screen}");
    assert!(
        !screen.contains("[o] open repository"),
        "a component has no repository:\n{screen}"
    );

    // chap-core's own: no volume of its own, and its port lives elsewhere.
    app.reduce(Action::Info);
    on_components(&mut app, Component::ChapCore);
    app.reduce(Action::Info);
    let screen = render(&app, 120, 40);
    assert!(
        screen.contains("compose.yml + compose.varde.yml"),
        "{screen}"
    );
    assert!(screen.contains("none of its own"), "{screen}");
    assert!(screen.contains("`varde down --volumes`"), "{screen}");
    assert!(screen.contains("host port is the API port"), "{screen}");
    assert!(
        !screen.contains("--base-url"),
        "an OCS-only setting:\n{screen}"
    );
}

/// The DHIS2 overlay, which carries what only it can say: that the config
/// file is not optional, that one of the three volumes is a cache, what the
/// seed is and when it applies, how long the first start takes, and that the
/// two settings no flag moves are a key in `.varde/components.yaml`.
#[test]
fn the_dhis2_overlay_names_its_config_its_cache_and_the_keys_no_flag_moves() {
    let registry = registry();
    let mut app = App::new(&registry, &state_with_dhis2());
    on_components(&mut app, Component::Dhis2);
    app.reduce(Action::Info);

    let screen = render(&app, 120, 40);
    for needle in [
        "dhis2",
        "enabled here",
        "http://localhost:8780",
        "compose.dhis2.yml · rendered from .varde/components.yaml by `varde",
        "dhis2_home, dhis2_db, dhis2_dump · kept when the component is disabled",
        "dhis2/dhis.conf · yours to edit, and DHIS2 will not start without it",
        "https://databases.dhis2.org/climate/laos/2.42/laos.sql.gz · restored",
        "once, into the database the first `varde up` creates",
        "dhis2_dump holds the downloaded dump rather than data",
        "minutes, not seconds: DHIS2 migrates its schema on the way up",
        "`varde logs dhis2` is where that shows",
        "not on this page:",
        "`seed:` in .varde/components.yaml, then `varde sync`",
        "`v` on this row, or `varde components enable dhis2 --tag TAG`",
        "the DHIS2 version, 2.42 here; it migrates a schema forward only",
        "backup create` first",
    ] {
        assert!(screen.contains(needle), "{needle} is missing:\n{screen}");
    }
    // The OCS block is OCS's alone, and DHIS2's is DHIS2's.
    assert!(
        !screen.contains("--base-url"),
        "an OCS-only setting:\n{screen}"
    );
    assert!(!screen.contains("climate-service.yaml"), "{screen}");

    app.reduce(Action::Info);
    on_components(&mut app, Component::Ocs);
    app.reduce(Action::Info);
    let ocs = render(&app, 120, 40);
    assert!(!ocs.contains("dhis.conf"), "{ocs}");
    assert!(!ocs.contains("first start"), "{ocs}");
}

/// The `seed` field's four answers: a dump, `none`, and a `default` on a
/// minor line varde publishes no dump for - which starts empty like `none`
/// and has to say why.
#[test]
fn the_seed_field_says_which_of_the_four_answers_this_deployment_gave() {
    let mut components = crate::components::Components::default();
    components.dhis2.enabled = true;

    assert_eq!(
        dhis2_seed_field(&components),
        "https://databases.dhis2.org/climate/laos/2.42/laos.sql.gz · restored once, \
             into the database the first `varde up` creates"
    );

    components.dhis2.seed = crate::components::Dhis2Seed::None;
    assert_eq!(
        dhis2_seed_field(&components),
        "none · dhis2_db starts empty and DHIS2 migrates a new database into it"
    );

    components.dhis2.seed = crate::components::Dhis2Seed::From("dumps/laos.sql.gz".into());
    assert!(
        dhis2_seed_field(&components).starts_with("dumps/laos.sql.gz · restored once,"),
        "{}",
        dhis2_seed_field(&components)
    );

    // A pinned minor with no published dump: the reason, not just the fact.
    components.dhis2.seed = crate::components::Dhis2Seed::Default;
    components.dhis2.image_tag = "2.40.1".to_string();
    assert_eq!(
        dhis2_seed_field(&components),
        "default · varde knows no dump for 2.40, so dhis2_db starts empty"
    );
}

/// `p` on the DHIS2 row behaves as it does on OCS: a number or `none`, no
/// `auto`, and the dialog names where the component answers today.
#[test]
fn the_port_dialog_treats_dhis2_as_it_treats_ocs() {
    let registry = registry();
    let mut app = App::new(&registry, &state_with_dhis2());
    on_components(&mut app, Component::Dhis2);
    app.reduce(Action::PortPrompt);

    let screen = render(&app, 120, 40);
    assert!(screen.contains("Host port for dhis2"), "{screen}");
    assert!(screen.contains("port  › 8780_"), "{screen}");
    assert!(
        screen.contains("now: http://localhost:8780 · api port 8700 is taken"),
        "{screen}"
    );
    assert!(
        screen.contains("[enter] apply  [esc] cancel  [none] no host port"),
        "and it does not offer auto:\n{screen}"
    );

    app.port_input.clear();
    for c in "auto".chars() {
        app.reduce(Action::PortChar(c));
    }
    app.reduce(Action::PortApply);
    let screen = render(&app, 120, 40);
    assert!(
        screen.contains("Host port for dhis2"),
        "still open:\n{screen}"
    );
    assert!(screen.contains("model port range"), "{screen}");
}

/// The overlay's `volume` field, including the shape no component has yet:
/// one volume is the field it has always been, and several are all listed,
/// because the field is there to say where the data is.
#[test]
fn the_volume_field_lists_every_volume_a_component_keeps() {
    assert_eq!(
        volume_field(&["ocs_data"]),
        "ocs_data · kept when the component is disabled"
    );
    assert_eq!(
        volume_field(&["a_data", "b_data"]),
        "a_data, b_data · kept when the component is disabled"
    );
    assert_eq!(
        volume_field(&[]),
        "none of its own · `varde down --volumes` removes this deployment's"
    );
}

/// The components page holds together at every terminal size the model
/// page has to.
#[test]
fn the_components_page_holds_from_a_tiny_terminal_to_a_huge_one() {
    let registry = registry();
    let mut app = App::new(&registry, &state_with_ocs(Some(9000)));
    app.reduce(Action::NextPage);
    for (width, height) in [(1, 1), (8, 3), (20, 5), (60, 16), (80, 24), (200, 60)] {
        let screen = render(&app, width, height);
        assert_eq!(screen.lines().count(), height as usize);
        for line in screen.lines() {
            assert_eq!(
                line.chars().count(),
                width as usize,
                "a line overflowed at {width}x{height}:\n{screen}"
            );
        }
    }
    // Eighty columns keeps the three columns that say what the row is.
    let eighty = render(&app, 80, 24);
    assert!(eighty.contains("chap-core"), "{eighty}");
    assert!(eighty.contains("http://localhost:9000"), "{eighty}");

    // And every overlay draws on top of it without panicking.
    for action in [
        Action::Info,
        Action::PortPrompt,
        Action::Help,
        Action::Palette,
    ] {
        let mut app = App::new(&registry, &state_with_ocs(Some(9000)));
        on_components(&mut app, Component::Ocs);
        app.reduce(action.clone());
        assert_ne!(app.mode, Mode::Browse, "{action:?} opened nothing");
        for (width, height) in [(1, 1), (10, 4), (30, 6), (60, 16), (80, 24)] {
            render(&app, width, height);
        }
    }
}

#[test]
fn narrow_component_tables_drop_columns_instead_of_wrapping() {
    let wide = component_columns(160);
    assert_eq!(wide.component, COMPONENT_W);
    assert_eq!(wide.state, STATE_W);
    assert_eq!(wide.reach, REACH_W);
    assert!(wide.what > 0, "{wide:?}");

    // Narrow: the sentence gives way first, then the reach narrows to its
    // short form, and the state is the last thing to go.
    let fifty = component_columns(50);
    assert_eq!(fifty.reach, REACH_MIN, "{fifty:?}");
    assert!(fifty.what > 0 && fifty.what < REACH_W, "{fifty:?}");
    assert_eq!(component_columns(30).reach, 0);
    assert_eq!(component_columns(30).state, STATE_W);
    assert_eq!(component_columns(14).state, 0);

    for inner in 0..200usize {
        let c = component_columns(inner);
        let used = MARKER_W
            + [c.component, c.state, c.reach, c.what]
                .iter()
                .map(|w| if *w > 0 { w + 1 } else { 0 })
                .sum::<usize>();
        assert!(
            used <= inner.max(MARKER_W) + 1,
            "{inner}: {c:?} needs {used}"
        );
    }
}

#[test]
fn zz_dump_components_page() {
    let registry = registry();
    let mut app = App::new(&registry, &state_with_ocs(Some(9000)));
    app.reduce(Action::NextPage);
    eprintln!("=== COMPONENTS 120x15 ===");
    eprintln!("{}", render(&app, 120, 15));
    on_components(&mut app, Component::S3);
    app.reduce(Action::Toggle);
    eprintln!("=== COMPONENTS PENDING 120x15 ===");
    eprintln!("{}", render(&app, 120, 15));
    app.reduce(Action::Info);
    eprintln!("=== COMPONENT OVERLAY 120x30 ===");
    eprintln!("{}", render(&app, 120, 30));
    app.reduce(Action::Info);
    on_components(&mut app, Component::Ocs);
    app.reduce(Action::Info);
    eprintln!("=== OCS OVERLAY 120x30 ===");
    eprintln!("{}", render(&app, 120, 30));

    // The DHIS2 row and its overlay, on a deployment that has one: the
    // component with the most to say behind `i`, and the one whose summary
    // strip carries the longest sentence.
    let mut app = App::new(&registry, &state_with_dhis2());
    on_components(&mut app, Component::Dhis2);
    eprintln!("=== COMPONENTS DHIS2 120x15 ===");
    eprintln!("{}", render(&app, 120, 15));
    app.reduce(Action::Info);
    eprintln!("=== DHIS2 OVERLAY 120x30 ===");
    eprintln!("{}", render(&app, 120, 30));
}

#[test]
fn zz_dump_docs_mock() {
    let registry = registry();
    let state = state_with_ewars(&registry, None);
    let app = App::new(&registry, &state);
    eprintln!("=== DOCS BROWSER 120x11 ===");
    eprintln!("{}", render(&app, 120, 11));
    let mut app = App::new(&registry, &ProjectState::default());
    while app.selected().map(|r| app.model(r).display_name.clone())
        != Some("Rwanda Malaria BYM".to_string())
    {
        app.reduce(Action::Down);
    }
    app.reduce(Action::Info);
    eprintln!("=== DOCS OVERLAY 120x40 ===");
    eprintln!("{}", render(&app, 120, 40));
}
