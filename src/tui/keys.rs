//! Key bindings, mapped to actions for the reducer.
//!
//! Keys avoid `[`, `]`, `{`, `}`, `|` and `\`, which need Alt on a Norwegian
//! keyboard layout.
//!
//! Owned by agent C.

use crate::tui::app::{Action, Mode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Translate a key press into an [`Action`] for the current [`Mode`].
///
/// Keys that mean nothing in a mode map to [`Action::None`], which the reducer
/// ignores.
pub fn action_for(mode: Mode, key: &KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // Ctrl-C always means "get me out of here", whatever is on screen.
    if ctrl && key.code == KeyCode::Char('c') {
        return match mode {
            Mode::ConfirmQuit => Action::ConfirmYes,
            _ => Action::Quit,
        };
    }

    match mode {
        Mode::Browse => browse(key, ctrl),
        Mode::Filter => filter(key, ctrl),
        Mode::ConfirmQuit => confirm(key),
        Mode::Help => help(key),
    }
}

fn browse(key: &KeyEvent, ctrl: bool) -> Action {
    if ctrl {
        return match key.code {
            KeyCode::Char('d') => Action::PageDown,
            KeyCode::Char('u') => Action::PageUp,
            KeyCode::Char('n') => Action::Down,
            // ctrl-p is "previous line"; plain p publishes a port.
            KeyCode::Char('p') => Action::Up,
            _ => Action::None,
        };
    }
    // Alt is how a Norwegian layout reaches the bracket characters; never let
    // it trigger a plain binding by accident.
    if key.modifiers.contains(KeyModifiers::ALT) {
        return Action::None;
    }
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Action::Down,
        KeyCode::Char('k') | KeyCode::Up => Action::Up,
        KeyCode::Char('g') | KeyCode::Home => Action::Top,
        KeyCode::Char('G') | KeyCode::End => Action::Bottom,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::Char(' ') => Action::Toggle,
        KeyCode::Char('p') => Action::TogglePublish,
        KeyCode::Char('v') => Action::CycleChannel,
        KeyCode::Char('t') => Action::ToggleTemplates,
        KeyCode::Char('/') => Action::StartFilter,
        KeyCode::Enter | KeyCode::Char('s') => Action::Save,
        KeyCode::Char('?') => Action::Help,
        KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
        _ => Action::None,
    }
}

fn filter(key: &KeyEvent, ctrl: bool) -> Action {
    if ctrl {
        return match key.code {
            // Ctrl-U is "clear the line" everywhere else, so keep the filter
            // but empty it rather than leaving the mode.
            KeyCode::Char('u') => Action::FilterCancel,
            _ => Action::None,
        };
    }
    match key.code {
        KeyCode::Esc => Action::FilterCancel,
        KeyCode::Enter => Action::FilterDone,
        KeyCode::Backspace => Action::FilterBackspace,
        KeyCode::Up => Action::Up,
        KeyCode::Down => Action::Down,
        // Alt is not filtered out here: it is how a Norwegian layout types
        // some characters, and every character is just text while filtering.
        KeyCode::Char(c) => Action::FilterChar(c),
        _ => Action::None,
    }
}

fn confirm(key: &KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => Action::ConfirmYes,
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Action::ConfirmNo,
        _ => Action::None,
    }
}

fn help(key: &KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('?')
        | KeyCode::Char('q')
        | KeyCode::Char(' ')
        | KeyCode::Esc
        | KeyCode::Enter => Action::Help,
        _ => Action::None,
    }
}

/// The bindings, as the help overlay lists them: `(keys, what they do)`.
pub fn help_entries() -> &'static [(&'static str, &'static str)] {
    &[
        ("j / k / arrows", "move the cursor"),
        ("g / G", "first / last model"),
        ("PageUp / PageDown", "jump a page"),
        ("ctrl-u / ctrl-d", "jump a page"),
        ("space", "enable or disable the model"),
        ("p", "publish a host port for it, or take it away"),
        ("v", "switch channel: stable or latest"),
        ("t", "show or hide templates"),
        ("/", "filter; Enter keeps it, Esc clears it"),
        ("Enter / s", "save and apply the changes"),
        ("?", "this help"),
        ("q / Esc", "quit, asking first when there are changes"),
    ]
}

/// The one-line key bar under the list, as `(keys, what they do)` so the
/// renderer can give the keys themselves a colour of their own.
pub fn keybar_entries(mode: Mode) -> &'static [(&'static str, &'static str)] {
    match mode {
        Mode::Browse => &[
            ("j/k", "move"),
            ("space", "toggle"),
            ("p", "port"),
            ("v", "channel"),
            ("t", "templates"),
            ("/", "filter"),
            ("s", "save"),
            ("?", "help"),
            ("q", "quit"),
        ],
        Mode::Filter => &[("type", "to filter"), ("Enter", "keep"), ("Esc", "clear")],
        Mode::ConfirmQuit => &[("y", "discard the changes and quit"), ("n", "keep editing")],
        Mode::Help => &[("? or Esc", "closes this help")],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key bar as one line, the way it reads on screen.
    fn keybar(mode: Mode) -> String {
        keybar_entries(mode)
            .iter()
            .map(|(key, what)| format!("{key} {what}"))
            .collect::<Vec<String>>()
            .join("   ")
    }

    #[test]
    fn every_mode_has_a_key_bar_that_names_its_way_out() {
        assert_eq!(
            keybar(Mode::Browse),
            "j/k move   space toggle   p port   v channel   t templates   \
             / filter   s save   ? help   q quit"
        );
        assert_eq!(
            keybar(Mode::Filter),
            "type to filter   Enter keep   Esc clear"
        );
        assert_eq!(
            keybar(Mode::ConfirmQuit),
            "y discard the changes and quit   n keep editing"
        );
        assert_eq!(keybar(Mode::Help), "? or Esc closes this help");
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
        assert_eq!(browse_action(KeyCode::Char('p')), Action::TogglePublish);
        assert_eq!(browse_action(KeyCode::Char('v')), Action::CycleChannel);
        assert_eq!(browse_action(KeyCode::Char('t')), Action::ToggleTemplates);
        assert_eq!(browse_action(KeyCode::Char('/')), Action::StartFilter);
        assert_eq!(browse_action(KeyCode::Char('s')), Action::Save);
        assert_eq!(browse_action(KeyCode::Enter), Action::Save);
        assert_eq!(browse_action(KeyCode::Char('?')), Action::Help);
        assert_eq!(browse_action(KeyCode::Char('q')), Action::Quit);
        assert_eq!(browse_action(KeyCode::Esc), Action::Quit);
    }

    #[test]
    fn unbound_browse_keys_do_nothing() {
        assert_eq!(browse_action(KeyCode::Char('x')), Action::None);
        assert_eq!(browse_action(KeyCode::F(5)), Action::None);
        assert_eq!(browse_action(KeyCode::Tab), Action::None);
        assert_eq!(
            action_for(Mode::Browse, &ctrl('v')),
            Action::None,
            "a modifier must not fall through to the plain binding"
        );
        assert_eq!(
            action_for(Mode::Browse, &ctrl('p')),
            Action::Up,
            "ctrl-p stays `previous line`; only plain p publishes a port"
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
        for mode in [Mode::Browse, Mode::Filter, Mode::ConfirmQuit, Mode::Help] {
            for c in awkward {
                assert!(
                    !keybar(mode).contains(c),
                    "the key bar for {mode:?} documents an Alt-only character"
                );
            }
        }
    }
}
