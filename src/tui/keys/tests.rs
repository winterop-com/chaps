use super::*;

/// The key bar as one line, the way it reads on screen.
fn bar(mode: Mode, pending: usize, filtering: bool) -> String {
    page_bar(Page::Models, mode, pending, filtering)
}

/// The same, for a named page.
fn page_bar(page: Page, mode: Mode, pending: usize, filtering: bool) -> String {
    keybar(page, mode, pending, filtering)
        .iter()
        .map(|hint| format!("{} {}", hint.key, hint.what))
        .collect::<Vec<String>>()
        .join("   ")
}

#[test]
fn every_mode_has_a_key_bar_that_names_its_way_out() {
    assert_eq!(
        bar(Mode::Browse, 0, false),
        "j/k move   tab page   space toggle   i info   p port   v channel   \
             t templates   / filter   s save   ctrl+k commands   ? help   q quit"
    );
    // The components page drops the keys a component has no use for and
    // gains the one that opens its web interface.
    assert_eq!(
        page_bar(Page::Components, Mode::Browse, 0, false),
        "j/k move   tab page   space toggle   i info   o open   v dhis2 version   p port   \
             s save   ctrl+k commands   ? help   q quit"
    );
    assert_eq!(
        bar(Mode::Filter, 0, false),
        "type to filter   enter keep   esc clear"
    );
    assert_eq!(
        bar(Mode::ConfirmQuit, 0, false),
        "y discard the changes and quit   n keep editing"
    );
    assert_eq!(bar(Mode::Help, 0, false), "? or esc closes this help");
    assert_eq!(
        bar(Mode::Info, 0, false),
        "esc close   j/k scroll   o open repository   c image ref"
    );
    assert_eq!(
        bar(Mode::Palette, 0, false),
        "esc close   up/down move   enter run"
    );
}

/// The bar is about what can be done now: unsaved changes turn saving into
/// a chip that counts them and add a way to throw them away, and a filter
/// offers to clear itself instead of hiding templates.
#[test]
fn the_key_bar_follows_what_is_pending_and_what_is_filtered() {
    let pending = bar(Mode::Browse, 2, false);
    assert!(pending.contains("s save 2 changes"), "{pending}");
    assert!(pending.contains("u discard"), "{pending}");
    assert!(!pending.contains("s save   "), "{pending}");
    assert_eq!(
        keybar(Page::Models, Mode::Browse, 2, false)
            .iter()
            .filter(|hint| hint.chip)
            .count(),
        1,
        "the save chip is the only filled thing on the bar"
    );
    assert!(bar(Mode::Browse, 1, false).contains("s save 1 change"));
    assert!(
        keybar(Page::Models, Mode::Browse, 0, false)
            .iter()
            .all(|h| !h.chip)
    );

    let filtering = bar(Mode::Browse, 0, true);
    assert!(filtering.contains("esc clear filter"), "{filtering}");
    assert!(!filtering.contains("t templates"), "{filtering}");
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn browse_action(code: KeyCode) -> Action {
    action_for(Mode::Browse, &key(code))
}

#[test]
fn browse_movement_keys() {
    assert_eq!(browse_action(KeyCode::Char('j')), Action::Down);
    assert_eq!(browse_action(KeyCode::Down), Action::Down);
    assert_eq!(browse_action(KeyCode::Char('k')), Action::Up);
    assert_eq!(browse_action(KeyCode::Up), Action::Up);
    assert_eq!(browse_action(KeyCode::Char('g')), Action::Top);
    assert_eq!(browse_action(KeyCode::Home), Action::Top);
    assert_eq!(browse_action(KeyCode::Char('G')), Action::Bottom);
    assert_eq!(browse_action(KeyCode::End), Action::Bottom);
    assert_eq!(browse_action(KeyCode::PageDown), Action::PageDown);
    assert_eq!(browse_action(KeyCode::PageUp), Action::PageUp);
    assert_eq!(
        action_for(Mode::Browse, &ctrl('d')),
        Action::PageDown,
        "ctrl-d pages down like it does in a pager"
    );
    assert_eq!(action_for(Mode::Browse, &ctrl('u')), Action::PageUp);
}

#[test]
fn browse_editing_keys() {
    assert_eq!(browse_action(KeyCode::Char(' ')), Action::Toggle);
    assert_eq!(browse_action(KeyCode::Char('p')), Action::PortPrompt);
    assert_eq!(browse_action(KeyCode::Char('P')), Action::RemovePort);
    assert_eq!(browse_action(KeyCode::Char('v')), Action::ChannelPrompt);
    assert_eq!(browse_action(KeyCode::Char('t')), Action::ToggleTemplates);
    assert_eq!(browse_action(KeyCode::Char('/')), Action::StartFilter);
    assert_eq!(browse_action(KeyCode::Char('s')), Action::Save);
    assert_eq!(browse_action(KeyCode::Char('u')), Action::Discard);
    assert_eq!(browse_action(KeyCode::Char('o')), Action::Open);
    assert_eq!(browse_action(KeyCode::Char('c')), Action::ImageRef);
    assert_eq!(browse_action(KeyCode::Char('?')), Action::Help);
    assert_eq!(browse_action(KeyCode::Char('q')), Action::Quit);
    assert_eq!(browse_action(KeyCode::Esc), Action::ClearFilterOrQuit);
}

/// Enter is the second way into the details rather than a second way to
/// save: `s` is the one key that writes anything.
#[test]
fn enter_and_i_open_the_details_and_only_s_saves() {
    assert_eq!(browse_action(KeyCode::Char('i')), Action::Info);
    assert_eq!(browse_action(KeyCode::Enter), Action::Info);
    assert_eq!(browse_action(KeyCode::Char('s')), Action::Save);
    for code in [
        KeyCode::Char('i'),
        KeyCode::Char('q'),
        KeyCode::Esc,
        KeyCode::Enter,
    ] {
        assert_eq!(action_for(Mode::Info, &key(code)), Action::Info);
    }
    assert_eq!(
        action_for(Mode::Info, &key(KeyCode::Char('j'))),
        Action::Down
    );
    assert_eq!(action_for(Mode::Info, &key(KeyCode::Char('k'))), Action::Up);
    assert_eq!(
        action_for(Mode::Info, &key(KeyCode::Char('o'))),
        Action::Open
    );
    assert_eq!(
        action_for(Mode::Info, &key(KeyCode::Char('c'))),
        Action::ImageRef
    );
    assert_eq!(
        action_for(Mode::Info, &key(KeyCode::Char(' '))),
        Action::None,
        "nothing behind the overlay is reachable through it"
    );
}

/// The channel dialog is picked from, not cycled through.
#[test]
fn the_channel_dialog_moves_picks_and_applies() {
    assert_eq!(
        action_for(Mode::Channel, &key(KeyCode::Char('j'))),
        Action::Down
    );
    assert_eq!(
        action_for(Mode::Channel, &key(KeyCode::Char('k'))),
        Action::Up
    );
    assert_eq!(
        action_for(Mode::Channel, &key(KeyCode::Char('s'))),
        Action::ChannelChar('s')
    );
    assert_eq!(
        action_for(Mode::Channel, &key(KeyCode::Char('L'))),
        Action::ChannelChar('L')
    );
    assert_eq!(
        action_for(Mode::Channel, &key(KeyCode::Enter)),
        Action::ChannelApply
    );
    assert_eq!(
        action_for(Mode::Channel, &key(KeyCode::Esc)),
        Action::FilterCancel
    );
}

/// Both dialogs carry their own key line, and the bar under them says the
/// one thing that is always true.
#[test]
fn a_dialog_names_its_keys_and_the_bar_under_it_says_cancel() {
    assert_eq!(bar(Mode::Port, 0, false), "esc cancel");
    assert_eq!(bar(Mode::Channel, 2, true), "esc cancel");
    let port: Vec<String> = port_dialog_keys(Page::Models)
        .iter()
        .map(|h| format!("{} {}", h.key, h.what))
        .collect();
    assert_eq!(
        port.join("   "),
        "enter apply   esc cancel   auto any free port   none no host port"
    );
    let channel: Vec<String> = channel_dialog_keys()
        .iter()
        .map(|h| format!("{} {}", h.key, h.what))
        .collect();
    assert_eq!(
        channel.join("   "),
        "enter apply   esc cancel   j/k move   s/l pick one"
    );
    for hint in port_dialog_keys(Page::Models)
        .iter()
        .chain(channel_dialog_keys().iter())
    {
        assert!(!hint.chip, "a dialog's keys are not chips");
        for c in ['[', ']', '{', '}', '|', '\\'] {
            assert!(!hint.key.contains(c) && !hint.what.contains(c));
        }
    }
}

/// The palette opens where an editor and a file finder put it, and closes
/// from inside itself; while it is up every printable key is query text.
#[test]
fn the_palette_opens_on_ctrl_k_and_ctrl_p_and_types_text() {
    assert_eq!(action_for(Mode::Browse, &ctrl('k')), Action::Palette);
    assert_eq!(action_for(Mode::Browse, &ctrl('p')), Action::Palette);
    assert_eq!(
        action_for(Mode::Palette, &key(KeyCode::Char('q'))),
        Action::PaletteChar('q'),
        "q is a letter of the query, not a way out"
    );
    assert_eq!(
        action_for(Mode::Palette, &key(KeyCode::Char(' '))),
        Action::PaletteChar(' ')
    );
    assert_eq!(
        action_for(Mode::Palette, &key(KeyCode::Backspace)),
        Action::PaletteBackspace
    );
    assert_eq!(
        action_for(Mode::Palette, &key(KeyCode::Enter)),
        Action::PaletteRun
    );
    assert_eq!(
        action_for(Mode::Palette, &key(KeyCode::Esc)),
        Action::FilterCancel
    );
    assert_eq!(action_for(Mode::Palette, &key(KeyCode::Down)), Action::Down);
    assert_eq!(action_for(Mode::Palette, &key(KeyCode::Up)), Action::Up);
    assert_eq!(action_for(Mode::Palette, &ctrl('k')), Action::Palette);
}

#[test]
fn unbound_browse_keys_do_nothing() {
    assert_eq!(browse_action(KeyCode::Char('x')), Action::None);
    assert_eq!(browse_action(KeyCode::F(5)), Action::None);
    assert_eq!(
        browse_action(KeyCode::Tab),
        Action::NextPage,
        "Tab is the other page now"
    );
    assert_eq!(
        action_for(
            Mode::Browse,
            &KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)
        ),
        Action::PrevPage,
        "and shift-Tab walks them the other way"
    );
    assert_eq!(
        action_for(Mode::Browse, &ctrl('v')),
        Action::None,
        "a modifier must not fall through to the plain binding"
    );
    assert_eq!(
        action_for(Mode::Browse, &ctrl('n')),
        Action::Down,
        "ctrl-n stays `next line`; only plain p publishes a port"
    );
    assert_eq!(
        action_for(
            Mode::Browse,
            &KeyEvent::new(KeyCode::Char('t'), KeyModifiers::ALT)
        ),
        Action::None,
        "alt is how this layout types brackets, not a binding"
    );
}

#[test]
fn shift_still_reaches_the_uppercase_binding() {
    assert_eq!(
        action_for(
            Mode::Browse,
            &KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT)
        ),
        Action::Bottom
    );
    assert_eq!(
        action_for(
            Mode::Browse,
            &KeyEvent::new(KeyCode::Char('?'), KeyModifiers::SHIFT)
        ),
        Action::Help
    );
}

#[test]
fn filter_mode_types_text() {
    assert_eq!(
        action_for(Mode::Filter, &key(KeyCode::Char('e'))),
        Action::FilterChar('e')
    );
    assert_eq!(
        action_for(Mode::Filter, &key(KeyCode::Char('/'))),
        Action::FilterChar('/'),
        "every character is text while filtering"
    );
    assert_eq!(
        action_for(Mode::Filter, &key(KeyCode::Char('j'))),
        Action::FilterChar('j')
    );
    assert_eq!(
        action_for(Mode::Filter, &key(KeyCode::Backspace)),
        Action::FilterBackspace
    );
    assert_eq!(
        action_for(Mode::Filter, &key(KeyCode::Enter)),
        Action::FilterDone
    );
    assert_eq!(
        action_for(Mode::Filter, &key(KeyCode::Esc)),
        Action::FilterCancel
    );
    assert_eq!(action_for(Mode::Filter, &key(KeyCode::Down)), Action::Down);
    assert_eq!(action_for(Mode::Filter, &key(KeyCode::Up)), Action::Up);
    assert_eq!(action_for(Mode::Filter, &ctrl('u')), Action::FilterCancel);
    assert_eq!(action_for(Mode::Filter, &ctrl('x')), Action::None);
}

#[test]
fn confirm_mode_only_answers_the_question() {
    assert_eq!(
        action_for(Mode::ConfirmQuit, &key(KeyCode::Char('y'))),
        Action::ConfirmYes
    );
    assert_eq!(
        action_for(Mode::ConfirmQuit, &key(KeyCode::Char('Y'))),
        Action::ConfirmYes
    );
    assert_eq!(
        action_for(Mode::ConfirmQuit, &key(KeyCode::Char('n'))),
        Action::ConfirmNo
    );
    assert_eq!(
        action_for(Mode::ConfirmQuit, &key(KeyCode::Esc)),
        Action::ConfirmNo
    );
    assert_eq!(
        action_for(Mode::ConfirmQuit, &key(KeyCode::Char('j'))),
        Action::None
    );
}

#[test]
fn help_mode_closes_on_the_obvious_keys() {
    for code in [
        KeyCode::Char('?'),
        KeyCode::Char('q'),
        KeyCode::Char(' '),
        KeyCode::Esc,
        KeyCode::Enter,
    ] {
        assert_eq!(action_for(Mode::Help, &key(code)), Action::Help);
    }
    assert_eq!(
        action_for(Mode::Help, &key(KeyCode::Char('t'))),
        Action::None
    );
}

#[test]
fn ctrl_c_always_gets_out() {
    assert_eq!(action_for(Mode::Browse, &ctrl('c')), Action::Quit);
    assert_eq!(action_for(Mode::Filter, &ctrl('c')), Action::Quit);
    assert_eq!(action_for(Mode::Help, &ctrl('c')), Action::Quit);
    assert_eq!(
        action_for(Mode::ConfirmQuit, &ctrl('c')),
        Action::ConfirmYes,
        "ctrl-c at the confirmation must not be a dead end"
    );
}

#[test]
fn no_binding_needs_alt_on_a_norwegian_layout() {
    let awkward = ['[', ']', '{', '}', '|', '\\'];
    for c in awkward {
        assert_eq!(
            action_for(Mode::Browse, &key(KeyCode::Char(c))),
            Action::None,
            "{c} needs Alt on a Norwegian layout and must not be bound"
        );
    }
    for (keys, _) in help_entries() {
        for c in awkward {
            assert!(!keys.contains(c), "{keys} documents an Alt-only character");
        }
    }
    for mode in [
        Mode::Browse,
        Mode::Filter,
        Mode::ConfirmQuit,
        Mode::Help,
        Mode::Info,
        Mode::Palette,
        Mode::Port,
    ] {
        for c in awkward {
            // The brackets the renderer draws around a key are decoration
            // and not a binding; what the bar names must be typeable.
            assert!(
                !bar(mode, 2, true).contains(c),
                "the key bar for {mode:?} documents an Alt-only character"
            );
        }
    }
}
