use super::*;
use crate::registry::load_embedded;

const EWARS: &str = "chapkit_ewars_model";
const ARIMA: &str = "auto_arima_chapkit";
const TEMPLATE: &str = "chapkit_minimalist_example_py";

fn registry() -> Registry {
    load_embedded().expect("embedded snapshot parses")
}

/// Entries in the embedded snapshot, and how many of those are models
/// rather than templates. Read off the snapshot so a marketplace refresh
/// moves the counts below with it.
fn total(registry: &Registry) -> usize {
    registry.models.len()
}

fn without_templates(registry: &Registry) -> usize {
    registry.deployable().count()
}

fn empty_state() -> ProjectState {
    ProjectState::default()
}

fn state_with(registry: &Registry, id: &str, channel: Option<Channel>) -> ProjectState {
    state_with_port(registry, id, channel, None)
}

/// The same, with an explicit host port for the enabled model.
fn state_with_port(
    registry: &Registry,
    id: &str,
    channel: Option<Channel>,
    host_port: Option<u16>,
) -> ProjectState {
    let model = registry.get(id).expect("vendored model");
    let version = model
        .resolve(&VersionSelector::Channel(
            channel.unwrap_or(Channel::Stable),
        ))
        .expect("channel resolves");
    let mut state = ProjectState::default();
    state.models.insert(
        model.id.clone(),
        EnabledModel {
            service_id: model.service_id.clone(),
            image: model.source.image.clone(),
            image_tag: version.image_tag.clone(),
            version: version.version.clone(),
            channel,
            host_port,
            data_dir: "/work/data".to_string(),
            user: "chapkit:chapkit".to_string(),
            user_from: Default::default(),
            platform: None,
            compose_file: format!("compose.{}.yml", model.service_id),
        },
    );
    state
}

/// Pick a channel through the dialog the way `v` does.
fn pick_channel(app: &mut App, channel: Channel) {
    app.reduce(Action::ChannelPrompt);
    app.reduce(Action::ChannelChar(match channel {
        Channel::Stable => 's',
        Channel::Latest => 'l',
    }));
    app.reduce(Action::ChannelApply);
}

/// Type something into the port prompt and take it.
fn ask_for_port(app: &mut App, text: &str) {
    app.reduce(Action::PortPrompt);
    app.port_input.clear();
    for c in text.chars() {
        app.reduce(Action::PortChar(c));
    }
    app.reduce(Action::PortApply);
}

/// Put the cursor on a model id; panics when it is not visible.
fn focus(app: &mut App, id: &str) {
    let position = app
        .visible
        .iter()
        .position(|row_idx| app.model(&app.rows[*row_idx]).id == id)
        .unwrap_or_else(|| panic!("{id} is not visible"));
    app.cursor = position;
}

#[test]
fn a_fresh_browser_shows_models_but_not_templates() {
    let registry = registry();
    let app = App::new(&registry, &empty_state());
    assert_eq!(app.rows.len(), total(&registry));
    assert_eq!(
        app.visible.len(),
        without_templates(&registry),
        "the two templates are hidden"
    );
    assert!(!app.show_templates);
    assert_eq!(app.mode, Mode::Browse);
    assert!(!app.dirty);
    assert_eq!(
        app.counts(),
        Counts {
            total: total(&registry),
            enabled: 0,
            pending: 0
        }
    );
}

#[test]
fn templates_appear_after_toggling_them_on() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    assert!(app.reduce(Action::ToggleTemplates).is_none());
    assert!(app.show_templates);
    assert_eq!(app.visible.len(), total(&registry));
    app.reduce(Action::ToggleTemplates);
    assert_eq!(app.visible.len(), without_templates(&registry));
}

#[test]
fn toggling_a_row_marks_the_browser_dirty_and_enables_it() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, EWARS);

    assert!(app.reduce(Action::Toggle).is_none());
    assert!(app.dirty);
    assert!(app.selected().unwrap().enabled);

    let selection = app.selection();
    assert_eq!(selection.disable.len(), 0);
    assert_eq!(selection.enable.len(), 1);
    let request = &selection.enable[0];
    assert_eq!(request.id, EWARS);
    assert_eq!(
        request.selector,
        VersionSelector::Channel(Channel::Stable),
        "a fresh row follows stable"
    );
    assert!(
        request.port.is_none(),
        "a fresh row publishes nothing, which is apply's default"
    );
    assert!(!app.selected().unwrap().publishes());
    assert!(!request.allow_template);
    assert_eq!(app.counts().pending, 1);
}

/// `p` opens a prompt prefilled with what the row asks for today, and
/// Enter on it is the old "give me any free port".
#[test]
fn p_prompts_for_a_port_and_auto_is_what_enter_takes() {
    let registry = registry();

    // A model that publishes nothing: the prompt opens on `auto`.
    let state = state_with_port(&registry, EWARS, Some(Channel::Stable), None);
    let mut app = App::new(&registry, &state);
    focus(&mut app, EWARS);
    assert!(!app.selected().unwrap().publishes());

    assert!(app.reduce(Action::PortPrompt).is_none());
    assert_eq!(app.mode, Mode::Port);
    assert_eq!(app.port_input, "auto");
    assert!(app.reduce(Action::PortApply).is_none());
    assert_eq!(app.mode, Mode::Browse);

    assert_eq!(app.selected().unwrap().want, PortWant::Auto);
    assert!(app.dirty);
    let selection = app.selection();
    assert!(selection.disable.is_empty());
    assert_eq!(selection.enable.len(), 1);
    assert_eq!(selection.enable[0].id, EWARS);
    assert_eq!(selection.enable[0].port, Some(PortRequest::Auto));
    assert_eq!(
        selection.enable[0].selector,
        VersionSelector::Channel(Channel::Stable),
        "the pin does not move because a port did"
    );
    assert!(
        selection.enable[0].keep_version,
        "and the version the project recorded is kept, not re-resolved"
    );

    // Saying none is back where we started, so nothing to apply.
    ask_for_port(&mut app, "none");
    assert!(!app.has_changes());

    // A model that does publish one: the prompt opens on its number.
    let state = state_with_port(&registry, EWARS, Some(Channel::Stable), Some(5001));
    let mut app = App::new(&registry, &state);
    focus(&mut app, EWARS);
    assert!(app.selected().unwrap().publishes());
    assert_eq!(app.selected().unwrap().port, Some(5001));
    app.reduce(Action::PortPrompt);
    assert_eq!(app.port_input, "5001");
    app.reduce(Action::PortApply);
    assert!(
        !app.has_changes(),
        "taking back the port it already has changes nothing"
    );

    // An empty line takes the port away.
    ask_for_port(&mut app, "");
    let selection = app.selection();
    assert_eq!(selection.enable.len(), 1);
    assert_eq!(selection.enable[0].port, Some(PortRequest::None));
    assert!(selection.disable.is_empty());
}

/// The prompt takes a number, the way `chaps models expose --port` does.
#[test]
fn the_port_prompt_takes_a_number_auto_or_none() {
    let registry = registry();
    let state = state_with_port(&registry, EWARS, Some(Channel::Stable), None);
    let mut app = App::new(&registry, &state);
    focus(&mut app, EWARS);

    ask_for_port(&mut app, "5010");
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.selected().unwrap().want, PortWant::Exact(5010));
    assert_eq!(
        app.selection().enable[0].port,
        Some(PortRequest::Fixed(5010))
    );
    assert!(app.selection().enable[0].keep_version);

    ask_for_port(&mut app, "AUTO");
    assert_eq!(app.selected().unwrap().want, PortWant::Auto);
    ask_for_port(&mut app, "None");
    assert_eq!(app.selected().unwrap().want, PortWant::None);
    assert_eq!(app.selection().enable.len(), 0, "it had none to begin with");

    // Esc leaves the row alone whatever was typed.
    app.reduce(Action::PortPrompt);
    for c in "5010".chars() {
        app.reduce(Action::PortChar(c));
    }
    app.reduce(Action::PortBackspace);
    app.reduce(Action::FilterCancel);
    assert_eq!(app.mode, Mode::Browse);
    assert!(app.port_input.is_empty());
    assert_eq!(app.selected().unwrap().want, PortWant::None);
}

/// A refused port keeps the prompt up with the reason on it: the number is
/// corrected, not typed again from nothing.
#[test]
fn the_port_prompt_says_why_it_refuses_and_stays_open() {
    let registry = registry();
    let state = state_with_port(&registry, EWARS, Some(Channel::Stable), None);
    let mut app = App::new(&registry, &state);
    focus(&mut app, EWARS);

    for (typed, reason) in [
        ("80", "outside this project's range"),
        ("6000", "outside this project's range"),
        ("8700", "chap-core's own API port"),
        ("five thousand", "is not a port"),
    ] {
        ask_for_port(&mut app, typed);
        assert_eq!(app.mode, Mode::Port, "{typed} left the prompt");
        let why = app.port_error.clone().unwrap_or_default();
        assert!(why.contains(reason), "{typed}: {why}");
        assert_eq!(app.port_input, typed, "what was typed is still there");
        assert!(!app.has_changes());
        app.reduce(Action::FilterCancel);
    }

    // A port another enabled row is already asking for is refused too.
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, EWARS);
    app.reduce(Action::Toggle);
    ask_for_port(&mut app, "5010");
    focus(&mut app, ARIMA);
    app.reduce(Action::Toggle);
    ask_for_port(&mut app, "5010");
    assert_eq!(app.mode, Mode::Port);
    assert!(
        app.port_error
            .as_deref()
            .unwrap_or_default()
            .contains("already taken by another model"),
        "{:?}",
        app.port_error
    );
    ask_for_port(&mut app, "5011");
    assert_eq!(app.selected().unwrap().want, PortWant::Exact(5011));
}

/// The prompt only makes sense on a model this project runs.
#[test]
fn the_port_prompt_asks_for_the_model_to_be_enabled_first() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, ARIMA);
    app.reduce(Action::PortPrompt);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.message.as_deref(), Some(PUBLISH_NEEDS_ENABLED_HINT));
}

/// A port change and a channel change are two different requests. The
/// first has to keep the version the deployment is running - `state_with`
/// records an exact pin as `channel: None`, which is what re-resolving
/// would silently turn into "whatever stable points at today" - and the
/// second is asking for a new one.
#[test]
fn a_port_only_change_keeps_the_version_and_a_channel_change_does_not() {
    let registry = registry();
    let state = state_with_port(&registry, EWARS, None, None);
    let mut app = App::new(&registry, &state);
    focus(&mut app, EWARS);

    ask_for_port(&mut app, "auto");
    let selection = app.selection();
    assert_eq!(selection.enable.len(), 1);
    assert_eq!(selection.enable[0].port, Some(PortRequest::Auto));
    assert!(
        selection.enable[0].keep_version,
        "`p` asks for a port, not for an upgrade"
    );

    // Cycling the channel is a request about the version, port and all.
    pick_channel(&mut app, Channel::Latest);
    let selection = app.selection();
    assert_eq!(selection.enable.len(), 1);
    assert_eq!(
        selection.enable[0].selector,
        VersionSelector::Channel(Channel::Latest)
    );
    assert_eq!(selection.enable[0].port, Some(PortRequest::Auto));
    assert!(!selection.enable[0].keep_version);

    // A row this session enabled has no recorded version to keep.
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, EWARS);
    app.reduce(Action::Toggle);
    ask_for_port(&mut app, "auto");
    assert!(!app.selection().enable[0].keep_version);
}

#[test]
fn p_on_a_row_that_is_not_enabled_only_hints() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, ARIMA);

    app.reduce(Action::PortPrompt);
    assert_eq!(app.message.as_deref(), Some(PUBLISH_NEEDS_ENABLED_HINT));
    assert!(!app.selected().unwrap().publishes());
    assert!(!app.has_changes());

    // Enabled first, then published: one request carrying both.
    app.reduce(Action::Toggle);
    ask_for_port(&mut app, "auto");
    let selection = app.selection();
    assert_eq!(selection.enable.len(), 1);
    assert_eq!(selection.enable[0].port, Some(PortRequest::Auto));
}

#[test]
fn disabling_a_published_row_forgets_the_port_too() {
    let registry = registry();
    let state = state_with_port(&registry, EWARS, Some(Channel::Stable), Some(5001));
    let mut app = App::new(&registry, &state);
    focus(&mut app, EWARS);

    // Off, then on again: the model comes back reachable only through
    // chap-core rather than silently re-publishing a port.
    app.reduce(Action::Toggle);
    assert!(!app.selected().unwrap().publishes());
    app.reduce(Action::Toggle);
    assert!(app.selected().unwrap().enabled);
    assert!(!app.selected().unwrap().publishes());
    let selection = app.selection();
    assert_eq!(selection.enable.len(), 1);
    assert_eq!(selection.enable[0].port, Some(PortRequest::None));
}

#[test]
fn a_port_change_is_only_a_change_when_it_differs_from_the_record() {
    assert_eq!(port_change(PortWant::Auto, None), Some(PortRequest::Auto));
    assert_eq!(
        port_change(PortWant::None, Some(5001)),
        Some(PortRequest::None)
    );
    assert_eq!(
        port_change(PortWant::Exact(5001), Some(5001)),
        None,
        "it already has that one"
    );
    assert_eq!(
        port_change(PortWant::Exact(5010), Some(5001)),
        Some(PortRequest::Fixed(5010)),
        "a different one is a claim to make"
    );
    assert_eq!(
        port_change(PortWant::Exact(5010), None),
        Some(PortRequest::Fixed(5010))
    );
    assert_eq!(port_change(PortWant::None, None), None, "already proxied");
}

/// What the prompt accepts, away from the reducer.
#[test]
fn the_port_prompt_parses_the_three_things_it_takes() {
    let range = (5001, 5999);
    let ok = |text: &str| parse_port(text, range, 8000, &[5010]).expect(text);
    assert_eq!(ok(""), PortWant::None);
    assert_eq!(ok("  "), PortWant::None);
    assert_eq!(ok("none"), PortWant::None);
    assert_eq!(ok("NONE"), PortWant::None);
    assert_eq!(ok("auto"), PortWant::Auto);
    assert_eq!(ok(" 5011 "), PortWant::Exact(5011));

    let why = |text: &str| parse_port(text, range, 8000, &[5010]).expect_err(text);
    assert!(why("5000").contains("5001-5999"));
    assert!(why("6000").contains("5001-5999"));
    assert!(why("8000").contains("API port"));
    assert!(why("5010").contains("another model"));
    assert!(why("abc").contains("not a port"));
    assert!(why("-1").contains("not a port"));
}

#[test]
fn toggling_back_leaves_nothing_to_apply() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, EWARS);
    app.reduce(Action::Toggle);
    app.reduce(Action::Toggle);

    assert!(!app.dirty);
    let selection = app.selection();
    assert!(selection.enable.is_empty() && selection.disable.is_empty());
    assert_eq!(app.counts().pending, 0);
}

#[test]
fn disabling_a_recorded_model_yields_the_disable_list() {
    let registry = registry();
    let state = state_with(&registry, EWARS, Some(Channel::Stable));
    let mut app = App::new(&registry, &state);
    focus(&mut app, EWARS);
    assert!(app.selected().unwrap().enabled);
    assert_eq!(app.selected().unwrap().port, None);

    app.reduce(Action::Toggle);
    let selection = app.selection();
    assert_eq!(selection.disable, vec![EWARS.to_string()]);
    assert!(selection.enable.is_empty());
    assert!(app.dirty);
}

#[test]
fn picking_a_channel_for_an_enabled_row_re_pins_it() {
    let registry = registry();
    let state = state_with(&registry, EWARS, Some(Channel::Stable));
    let mut app = App::new(&registry, &state);
    focus(&mut app, EWARS);

    pick_channel(&mut app, Channel::Latest);
    assert_eq!(app.selected().unwrap().channel, Channel::Latest);
    let selection = app.selection();
    assert!(selection.disable.is_empty());
    assert_eq!(selection.enable.len(), 1);
    assert_eq!(selection.enable[0].id, EWARS);
    assert_eq!(
        selection.enable[0].selector,
        VersionSelector::Channel(Channel::Latest)
    );

    // Picking the one it started on is once again a no-op.
    pick_channel(&mut app, Channel::Stable);
    assert_eq!(app.selected().unwrap().channel, Channel::Stable);
    assert!(!app.has_changes());

    // And Esc takes nothing.
    app.reduce(Action::ChannelPrompt);
    assert_eq!(app.mode, Mode::Channel);
    assert_eq!(app.channel_cursor, 0, "it opens on the one in force");
    app.reduce(Action::Down);
    assert_eq!(app.channel_cursor, 1);
    app.reduce(Action::Down);
    assert_eq!(app.channel_cursor, 1, "there are only two");
    app.reduce(Action::FilterCancel);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.selected().unwrap().channel, Channel::Stable);
    assert!(!app.has_changes());
}

#[test]
fn a_row_nobody_enabled_takes_a_channel_but_changes_nothing() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, ARIMA);
    pick_channel(&mut app, Channel::Latest);
    assert_eq!(app.selected().unwrap().channel, Channel::Latest);
    assert!(!app.has_changes(), "a disabled row has nothing to apply");
}

#[test]
fn an_untouched_browser_saves_nothing() {
    let registry = registry();
    let state = state_with(&registry, EWARS, Some(Channel::Stable));
    let app = App::new(&registry, &state);
    assert!(!app.has_changes());
    assert_eq!(app.counts().enabled, 1);
}

#[test]
fn an_exact_pin_survives_a_save_that_does_not_touch_it() {
    let registry = registry();
    // channel: None means .chaps/models.yaml holds an exact version pin.
    let state = state_with(&registry, EWARS, None);
    let app = App::new(&registry, &state);
    assert!(
        !app.has_changes(),
        "opening and saving must not convert a pin into a channel"
    );
}

#[test]
fn filtering_narrows_the_list_and_keeps_the_cursor_valid() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::Bottom);
    let last = app.cursor;
    assert!(last > 0);

    app.reduce(Action::StartFilter);
    assert_eq!(app.mode, Mode::Filter);
    for c in "ewars".chars() {
        app.reduce(Action::FilterChar(c));
    }
    assert_eq!(app.filter, "ewars");
    assert_eq!(app.visible.len(), 1);
    assert!(app.cursor < app.visible.len());
    assert_eq!(app.selected_model().unwrap().id, EWARS);

    // Backspacing widens it again.
    app.reduce(Action::FilterBackspace);
    assert_eq!(app.filter, "ewar");
    assert_eq!(app.visible.len(), 1);

    app.reduce(Action::FilterDone);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.filter, "ewar", "Enter keeps the filter");
}

#[test]
fn escaping_the_filter_clears_it() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::StartFilter);
    for c in "arima".chars() {
        app.reduce(Action::FilterChar(c));
    }
    assert_eq!(app.visible.len(), 1);
    app.reduce(Action::FilterCancel);
    assert_eq!(app.mode, Mode::Browse);
    assert!(app.filter.is_empty());
    assert_eq!(app.visible.len(), without_templates(&registry));
}

#[test]
fn a_filter_that_matches_nothing_leaves_no_selection() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::StartFilter);
    for c in "zzzz".chars() {
        app.reduce(Action::FilterChar(c));
    }
    assert!(app.visible.is_empty());
    assert_eq!(app.cursor, 0);
    assert!(app.selected().is_none());
    // Movement and toggling on an empty list must not panic.
    app.reduce(Action::FilterDone);
    app.reduce(Action::Down);
    app.reduce(Action::Toggle);
    pick_channel(&mut app, Channel::Latest);
    assert!(!app.has_changes());
}

#[test]
fn enabling_a_template_warns_and_allows_it() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::ToggleTemplates);
    focus(&mut app, TEMPLATE);
    app.reduce(Action::Toggle);

    assert_eq!(app.message.as_deref(), Some(TEMPLATE_WARNING));
    let selection = app.selection();
    assert_eq!(selection.enable.len(), 1);
    assert!(
        selection.enable[0].allow_template,
        "apply must accept the template the browser showed"
    );
}

#[test]
fn a_project_running_a_template_shows_templates_from_the_start() {
    let registry = registry();
    let state = state_with(&registry, TEMPLATE, Some(Channel::Stable));
    let app = App::new(&registry, &state);
    assert!(app.show_templates);
    assert_eq!(app.visible.len(), total(&registry));
}

#[test]
fn movement_stays_inside_the_list() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::Up);
    assert_eq!(app.cursor, 0);
    app.reduce(Action::PageUp);
    assert_eq!(app.cursor, 0);

    app.reduce(Action::PageDown);
    assert_eq!(app.cursor, app.visible.len() - 1);
    app.reduce(Action::Down);
    assert_eq!(app.cursor, app.visible.len() - 1);
    app.reduce(Action::Top);
    assert_eq!(app.cursor, 0);
    app.reduce(Action::Bottom);
    assert_eq!(app.cursor, app.visible.len() - 1);
}

#[test]
fn quitting_a_dirty_browser_asks_first() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, EWARS);
    app.reduce(Action::Toggle);

    assert!(app.reduce(Action::Quit).is_none());
    assert_eq!(app.mode, Mode::ConfirmQuit);

    // Answering "no" returns to the list with the change intact.
    assert!(app.reduce(Action::ConfirmNo).is_none());
    assert_eq!(app.mode, Mode::Browse);
    assert!(app.dirty);

    app.reduce(Action::Quit);
    assert_eq!(app.reduce(Action::ConfirmYes), Some(Outcome::Quit));
}

#[test]
fn quitting_a_clean_browser_leaves_at_once() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    assert_eq!(app.reduce(Action::Quit), Some(Outcome::Quit));
}

#[test]
fn saving_ends_the_browser_from_browse_mode() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, ARIMA);
    app.reduce(Action::Toggle);
    assert_eq!(app.reduce(Action::Save), Some(Outcome::Save));
    assert_eq!(app.selection().enable[0].id, ARIMA);
}

#[test]
fn help_swallows_everything_until_it_closes() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::Help);
    assert_eq!(app.mode, Mode::Help);

    app.reduce(Action::Down);
    assert_eq!(app.cursor, 0, "movement is inert behind the overlay");
    app.reduce(Action::Toggle);
    assert!(!app.has_changes(), "toggling is inert behind the overlay");

    app.reduce(Action::Help);
    assert_eq!(app.mode, Mode::Browse);
}

#[test]
fn the_footer_note_clears_on_the_next_action() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::ToggleTemplates);
    focus(&mut app, TEMPLATE);
    app.reduce(Action::Toggle);
    assert!(app.message.is_some());
    app.reduce(Action::Down);
    assert!(app.message.is_none());
}

/// `i` opens the details over the list and swallows the keys that would
/// otherwise move it; only scrolling and the way out get through.
#[test]
fn the_details_overlay_opens_scrolls_within_its_bounds_and_closes() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, EWARS);
    app.reduce(Action::Info);
    assert_eq!(app.mode, Mode::Info);
    assert_eq!(app.info_scroll, 0);

    // Scrolling is clamped by what the last frame could show.
    app.info_max.set(3);
    for _ in 0..10 {
        app.reduce(Action::Down);
    }
    assert_eq!(app.info_scroll, 3, "it never scrolls past the end");
    app.reduce(Action::Up);
    assert_eq!(app.info_scroll, 2);
    app.reduce(Action::Top);
    assert_eq!(app.info_scroll, 0);
    app.reduce(Action::Bottom);
    assert_eq!(app.info_scroll, 3);
    app.reduce(Action::PageUp);
    assert_eq!(app.info_scroll, 0);

    // The list underneath is untouched.
    app.reduce(Action::Toggle);
    assert!(!app.has_changes(), "toggling is inert behind the overlay");

    app.reduce(Action::FilterCancel);
    assert_eq!(app.mode, Mode::Browse);
    assert_eq!(app.info_scroll, 0, "it opens at the top next time");

    // An empty list has nothing to describe, so nothing opens.
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::StartFilter);
    for c in "zzzz".chars() {
        app.reduce(Action::FilterChar(c));
    }
    app.reduce(Action::FilterDone);
    app.reduce(Action::Info);
    assert_eq!(app.mode, Mode::Browse);
}

/// `o` and `c` are about the model, not about the project: neither writes
/// anything, and both say what they did.
#[test]
fn the_overlay_keys_open_a_repository_and_show_an_image_reference() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, EWARS);

    app.reduce(Action::Open);
    let expected = registry.get(EWARS).unwrap().source.repository.clone();
    assert_eq!(app.take_effect(), Some(Effect::Open(expected)));
    assert!(app.take_effect().is_none(), "an effect is taken once");

    app.reduce(Action::ImageRef);
    let message = app.message.clone().expect("the reference is on the line");
    assert!(message.starts_with("ghcr.io/chap-models/chapkit_ewars_model:"));
    assert!(!app.has_changes());
}

#[test]
fn the_palette_narrows_by_substring_and_runs_what_it_matched() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    focus(&mut app, EWARS);
    // A host port is only a question for a model that is enabled.
    app.reduce(Action::Toggle);
    app.reduce(Action::Palette);
    assert_eq!(app.mode, Mode::Palette);
    assert_eq!(app.palette_matches().len(), app.commands().len());
    // Fifteen entries that do not depend on the component set, plus one
    // toggle per component, so a fifth component moves this number and
    // nothing else in the palette has to be counted again.
    assert_eq!(app.commands().len(), 15 + Component::ALL.len());
    assert_eq!(app.commands().len(), 19);

    for c in "PORT".chars() {
        app.reduce(Action::PaletteChar(c));
    }
    let matched = app.palette_matches();
    assert_eq!(matched.len(), 2, "case does not matter: {matched:?}");
    assert_eq!(matched[0].id, CommandId::SetPort);
    assert_eq!(matched[1].id, CommandId::RemovePort);
    let (from, to) = matched[0].hit.expect("the match is marked");
    assert_eq!(
        matched[0]
            .label
            .chars()
            .skip(from)
            .take(to - from)
            .collect::<String>(),
        "port"
    );
    assert!(matched[0].label.contains("CHAP-EWARS"), "{matched:?}");

    // "Set a host port" opens the same prompt `p` does; "remove" needs no
    // prompt, so the two are not each other's undo by accident.
    app.reduce(Action::PaletteRun);
    assert_eq!(app.mode, Mode::Port, "the palette opened the prompt");
    assert_eq!(app.port_input, "auto");
    app.port_input.clear();
    for c in "5010".chars() {
        app.reduce(Action::PortChar(c));
    }
    app.reduce(Action::PortApply);
    assert_eq!(app.selected().unwrap().want, PortWant::Exact(5010));

    app.reduce(Action::Palette);
    for c in "set a host".chars() {
        app.reduce(Action::PaletteChar(c));
    }
    app.reduce(Action::PaletteRun);
    assert_eq!(app.port_input, "5010", "prefilled with what it has");
    app.reduce(Action::FilterCancel);
    assert_eq!(app.selected().unwrap().want, PortWant::Exact(5010));

    app.reduce(Action::Palette);
    for c in "remove the host".chars() {
        app.reduce(Action::PaletteChar(c));
    }
    app.reduce(Action::PaletteRun);
    assert_eq!(app.mode, Mode::Browse, "removing needs no prompt");
    assert!(!app.selected().unwrap().publishes());
}

#[test]
fn the_palette_moves_backspaces_and_leaves_without_running_anything() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::Palette);
    for c in "port".chars() {
        app.reduce(Action::PaletteChar(c));
    }
    app.reduce(Action::Down);
    assert_eq!(app.palette_cursor, 1);
    app.reduce(Action::Down);
    assert_eq!(app.palette_cursor, 1, "the cursor stops at the last match");
    app.reduce(Action::Up);
    assert_eq!(app.palette_cursor, 0);

    app.reduce(Action::PaletteBackspace);
    assert_eq!(app.palette_query, "por");
    app.reduce(Action::FilterCancel);
    assert_eq!(app.mode, Mode::Browse);
    assert!(app.palette_query.is_empty());
    assert!(!app.has_changes(), "leaving the palette runs nothing");

    // A query nothing matches runs nothing either.
    app.reduce(Action::Palette);
    for c in "zzzz".chars() {
        app.reduce(Action::FilterChar(c));
    }
    for c in "zzzz".chars() {
        app.reduce(Action::PaletteChar(c));
    }
    assert!(app.palette_matches().is_empty());
    assert!(app.reduce(Action::PaletteRun).is_none());
    assert_eq!(app.mode, Mode::Palette);
}

/// The commands that leave the terminal do not do it themselves: they ask
/// the caller, which is what keeps the reducer pure.
#[test]
fn the_palette_asks_the_caller_for_the_things_it_cannot_do() {
    let registry = registry();
    let run = |query: &str| {
        let mut app = App::new(&registry, &empty_state());
        app.reduce(Action::Palette);
        for c in query.chars() {
            app.reduce(Action::PaletteChar(c));
        }
        assert_eq!(app.palette_matches().len(), 1, "{query} is ambiguous");
        let outcome = app.reduce(Action::PaletteRun);
        (app.effect.clone(), app.mode, outcome)
    };

    assert_eq!(run("refresh the registry").0, Some(Effect::Refresh));
    assert_eq!(
        run("chaps documentation").0,
        Some(Effect::Open(format!(
            "{}{DOCS_CHAPTER}",
            crate::cli::DOCS_URL
        )))
    );
    assert!(matches!(
        run("open the repository").0,
        Some(Effect::Open(_))
    ));
    assert_eq!(run("every key").1, Mode::Help);
    assert_eq!(run("filter the model").1, Mode::Filter);
    assert_eq!(run("save the changes").2, Some(Outcome::Save));
    assert_eq!(run("quit the browser").2, Some(Outcome::Quit));
    assert!(run("show or hide").0.is_none());
}

#[test]
fn discarding_puts_every_row_back_where_the_project_had_it() {
    let registry = registry();
    let state = state_with_port(&registry, EWARS, Some(Channel::Stable), Some(5001));
    let mut app = App::new(&registry, &state);
    focus(&mut app, EWARS);
    app.reduce(Action::Toggle);
    focus(&mut app, ARIMA);
    app.reduce(Action::Toggle);
    pick_channel(&mut app, Channel::Latest);
    assert_eq!(app.counts().pending, 2);

    app.reduce(Action::Discard);
    assert_eq!(app.message.as_deref(), Some(DISCARDED_HINT));
    assert!(!app.has_changes());
    assert!(!app.dirty);
    focus(&mut app, EWARS);
    assert!(app.selected().unwrap().enabled);
    assert_eq!(app.selected().unwrap().port, Some(5001));
    assert_eq!(app.selected().unwrap().want, PortWant::Exact(5001));
    focus(&mut app, ARIMA);
    assert!(!app.selected().unwrap().enabled);
    assert_eq!(app.selected().unwrap().channel, Channel::Stable);

    // With nothing pending it says so rather than doing nothing quietly.
    app.reduce(Action::Discard);
    assert_eq!(app.message.as_deref(), Some(NOTHING_TO_DISCARD_HINT));
}

/// Esc is the way out of a filter first and the way out of the browser
/// second, so it never loses a filter and a session in one press.
#[test]
fn esc_clears_a_filter_before_it_quits() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::StartFilter);
    for c in "arima".chars() {
        app.reduce(Action::FilterChar(c));
    }
    app.reduce(Action::FilterDone);
    assert_eq!(app.visible.len(), 1);

    assert!(app.reduce(Action::ClearFilterOrQuit).is_none());
    assert!(app.filter.is_empty());
    assert_eq!(app.visible.len(), without_templates(&registry));

    assert_eq!(
        app.reduce(Action::ClearFilterOrQuit),
        Some(Outcome::Quit),
        "with no filter left it is the way out"
    );

    // And it still asks before throwing changes away.
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::Toggle);
    assert!(app.reduce(Action::ClearFilterOrQuit).is_none());
    assert_eq!(app.mode, Mode::ConfirmQuit);
}

/// The pending strip is the summary of what saving writes, so it has to
/// tell an enable from a re-pin from a disable.
#[test]
fn the_change_list_says_what_each_pending_change_does() {
    let registry = registry();
    let state = state_with_port(&registry, EWARS, Some(Channel::Stable), None);
    let mut app = App::new(&registry, &state);
    assert!(app.changes().is_empty(), "an untouched browser has none");

    focus(&mut app, ARIMA);
    app.reduce(Action::Toggle);
    focus(&mut app, EWARS);
    pick_channel(&mut app, Channel::Latest);
    ask_for_port(&mut app, "5010");

    let changes = app.changes();
    assert_eq!(changes.len(), 2);
    let added = changes
        .iter()
        .find(|c| c.kind == ChangeKind::Add)
        .expect("the new model is an addition");
    assert!(added.detail.starts_with("enable at "), "{added:?}");
    assert!(added.detail.ends_with("via chap-core"), "{added:?}");

    let updated = changes
        .iter()
        .find(|c| c.kind == ChangeKind::Update)
        .expect("the re-pinned model is an update");
    assert_eq!(updated.name, "CHAP-EWARS");
    assert!(updated.detail.contains("follow latest"), "{updated:?}");
    assert!(updated.detail.contains("publish port 5010"), "{updated:?}");

    // Turning it off instead is a removal, and it says what is kept.
    app.reduce(Action::Toggle);
    let changes = app.changes();
    let removed = changes
        .iter()
        .find(|c| c.kind == ChangeKind::Remove)
        .expect("a disabled model is a removal");
    assert_eq!(removed.name, "CHAP-EWARS");
    assert_eq!(removed.detail, "disable · the data volume is kept");
    assert_eq!(changes.len(), 2);
    assert_eq!(app.counts().pending, 2, "and the header counts the same");
}

/// The list is ordered by how far a model can be trusted, not by
/// whatever order the marketplace index names its files in.
#[test]
fn the_list_is_ordered_by_maturity_then_by_name() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    let names = |app: &App| -> Vec<String> {
        app.visible
            .iter()
            .map(|i| app.model(&app.rows[*i]).display_name.clone())
            .collect()
    };
    assert_eq!(
        names(&app),
        vec![
            "CHAP-EWARS",
            "Simple Multistep",
            "Auto-ARIMA",
            "GHRmodel",
            "Rwanda Malaria BYM",
        ],
        "orange, orange, red, red, gray - and alphabetical inside each"
    );
    for pair in app.visible.windows(2) {
        let (a, b) = (app.model(&app.rows[pair[0]]), app.model(&app.rows[pair[1]]));
        assert!(
            a.assessed_status.rank() <= b.assessed_status.rank(),
            "{} came before {}",
            a.display_name,
            b.display_name
        );
    }

    // Templates are scaffolding, so they follow the models whatever
    // their own assessment says.
    app.reduce(Action::ToggleTemplates);
    let shown = names(&app);
    let first_template = shown
        .iter()
        .position(|name| name.contains("Minimalist"))
        .expect("the templates are listed");
    assert!(first_template >= 5, "templates come last: {shown:?}");

    // A filter narrows the list; it does not reorder it.
    app.reduce(Action::ToggleTemplates);
    app.reduce(Action::StartFilter);
    for c in "model".chars() {
        app.reduce(Action::FilterChar(c));
    }
    let filtered = names(&app);
    let mut expected = filtered.clone();
    expected.sort_by_key(|name| {
        let model = registry
            .models
            .iter()
            .find(|m| &m.display_name == name)
            .unwrap();
        model.order_key()
    });
    assert_eq!(filtered, expected, "the filter kept the order");
}

/// Nothing the browser does changes a model's maturity, so the cursor
/// stays on the row it was on when a toggle re-reads the list.
#[test]
fn toggling_a_row_never_moves_it() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::Down);
    let before = app.selected_model().map(|m| m.id.clone());
    app.reduce(Action::Toggle);
    assert_eq!(app.selected_model().map(|m| m.id.clone()), before);
    app.reduce(Action::Toggle);
    assert_eq!(app.selected_model().map(|m| m.id.clone()), before);
}

/// The palette is the only way to a screenshot, and it asks the caller
/// for it rather than writing the file from the reducer.
#[test]
fn the_screenshot_command_asks_the_caller_to_take_one() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    app.reduce(Action::Palette);
    for c in "screenshot".chars() {
        app.reduce(Action::PaletteChar(c));
    }
    let matched = app.palette_matches();
    assert_eq!(matched.len(), 1, "{matched:?}");
    assert_eq!(matched[0].id, CommandId::Screenshot);
    assert_eq!(matched[0].label, "Save a screenshot (SVG)");
    assert_eq!(matched[0].key, "", "no key binds it");
    assert!(app.reduce(Action::PaletteRun).is_none());
    assert_eq!(app.mode, Mode::Browse, "the palette is out of the picture");
    assert_eq!(app.take_effect(), Some(Effect::Screenshot));
}

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

/// The rows the components page draws, in the columns `chaps components
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
    for c in "chaps documentation".chars() {
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
        "the wanted port is a plan until `s` and `chaps up`"
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

    // A tag chaps has no seed for is still offered while it is in force.
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

#[test]
fn toggling_a_hidden_template_only_hints() {
    let registry = registry();
    let mut app = App::new(&registry, &empty_state());
    // Force a template row under the cursor without showing templates, the
    // way a stale cursor could.
    let template_row = app
        .rows
        .iter()
        .position(|r| app.model(r).id == TEMPLATE)
        .unwrap();
    app.visible.push(template_row);
    app.cursor = app.visible.len() - 1;

    app.reduce(Action::Toggle);
    assert_eq!(app.message.as_deref(), Some(TEMPLATE_HIDDEN_HINT));
    assert!(!app.has_changes());
}
