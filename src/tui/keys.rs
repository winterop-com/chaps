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
        Mode::Info => info(key, ctrl),
        Mode::Palette => palette(key, ctrl),
        Mode::Port => port(key, ctrl),
    }
}

/// The port prompt: a line of text, so every printable key is part of it.
fn port(key: &KeyEvent, ctrl: bool) -> Action {
    if ctrl {
        return match key.code {
            // Ctrl-U empties the line here as it does in a shell, rather than
            // leaving the prompt.
            KeyCode::Char('u') => Action::PortChar('\u{0}'),
            _ => Action::None,
        };
    }
    match key.code {
        KeyCode::Esc => Action::FilterCancel,
        KeyCode::Enter => Action::PortApply,
        KeyCode::Backspace => Action::PortBackspace,
        KeyCode::Char(c) => Action::PortChar(c),
        _ => Action::None,
    }
}

/// The palette's two openings: ctrl-k where an editor puts it, ctrl-p where a
/// file finder does.
fn opens_palette(code: KeyCode) -> bool {
    matches!(code, KeyCode::Char('k') | KeyCode::Char('p'))
}

fn browse(key: &KeyEvent, ctrl: bool) -> Action {
    if ctrl {
        return match key.code {
            KeyCode::Char('d') => Action::PageDown,
            KeyCode::Char('u') => Action::PageUp,
            KeyCode::Char('n') => Action::Down,
            code if opens_palette(code) => Action::Palette,
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
        KeyCode::Char('p') => Action::PortPrompt,
        // Shift-P is the prompt's answer without the prompt: the one thing
        // there is nothing to type for.
        KeyCode::Char('P') => Action::RemovePort,
        KeyCode::Char('v') => Action::CycleChannel,
        KeyCode::Char('t') => Action::ToggleTemplates,
        KeyCode::Char('/') => Action::StartFilter,
        // Enter is the second way into the details, next to `i`; saving is
        // `s`, which is the one key that writes anything.
        KeyCode::Char('i') | KeyCode::Enter => Action::Info,
        KeyCode::Char('u') => Action::Discard,
        KeyCode::Char('o') => Action::OpenRepository,
        KeyCode::Char('c') => Action::ImageRef,
        KeyCode::Char('s') => Action::Save,
        KeyCode::Char('?') => Action::Help,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Esc => Action::ClearFilterOrQuit,
        _ => Action::None,
    }
}

/// The details overlay: the movement keys scroll it, and the keys its own
/// footer names act on the model it describes.
fn info(key: &KeyEvent, ctrl: bool) -> Action {
    if ctrl {
        return match key.code {
            KeyCode::Char('d') => Action::PageDown,
            KeyCode::Char('u') => Action::PageUp,
            KeyCode::Char('n') => Action::Down,
            _ => Action::None,
        };
    }
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Action::Down,
        KeyCode::Char('k') | KeyCode::Up => Action::Up,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::Char('g') | KeyCode::Home => Action::Top,
        KeyCode::Char('G') | KeyCode::End => Action::Bottom,
        KeyCode::Char('o') => Action::OpenRepository,
        KeyCode::Char('c') => Action::ImageRef,
        KeyCode::Char('i') | KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => Action::Info,
        _ => Action::None,
    }
}

/// The command palette: every printable character is part of the query, so
/// only the keys that cannot be typed into it do anything else.
fn palette(key: &KeyEvent, ctrl: bool) -> Action {
    if ctrl {
        return match key.code {
            code if opens_palette(code) => Action::Palette,
            KeyCode::Char('n') => Action::Down,
            _ => Action::None,
        };
    }
    match key.code {
        KeyCode::Esc => Action::FilterCancel,
        KeyCode::Enter => Action::PaletteRun,
        KeyCode::Backspace => Action::PaletteBackspace,
        KeyCode::Up => Action::Up,
        KeyCode::Down => Action::Down,
        KeyCode::Char(c) => Action::PaletteChar(c),
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
        ("i / Enter", "the full details, which j/k scroll"),
        ("p", "publish a host port: a number, auto, or none"),
        ("P", "take the host port away"),
        ("v", "switch channel: stable or latest"),
        ("t", "show or hide templates"),
        ("/", "filter; Enter keeps it, Esc clears it"),
        ("s", "save and apply the changes"),
        ("u", "discard the pending changes"),
        ("o", "open the model's repository"),
        ("c", "show the model's image reference"),
        ("ctrl-k / ctrl-p", "the command palette"),
        ("?", "this help"),
        ("q", "quit, asking first when there are changes"),
        ("Esc", "clear the filter, or quit"),
    ]
}

/// One entry in the key bar: a key and what it does, drawn as `[key] what`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub key: &'static str,
    pub what: String,
    /// Drawn as a filled chip rather than as a key and a quiet word: the one
    /// thing on the bar that writes to disk, when there is something to write.
    pub chip: bool,
    /// What the bar gives up first when it does not fit: the lowest rank goes.
    pub rank: u8,
}

fn hint(key: &'static str, what: &str, rank: u8) -> Hint {
    Hint {
        key,
        what: what.to_string(),
        chip: false,
        rank,
    }
}

/// The key bar under the list for a mode.
///
/// `pending` and `filtering` are what the browser is in the middle of: with
/// changes to save the bar grows a save chip and a way to throw them away,
/// and with a filter on it offers to clear it rather than to hide templates.
pub fn keybar(mode: Mode, pending: usize, filtering: bool) -> Vec<Hint> {
    match mode {
        Mode::Browse => {
            // The rank is what the bar gives up first when the terminal is
            // too narrow for all of it: the two display toggles before the
            // two things that act on the model under the cursor.
            let mut hints = vec![
                hint("j/k", "move", 10),
                hint("space", "toggle", 8),
                hint("i", "info", 5),
                hint("p", "port", 3),
                hint("v", "channel", 2),
            ];
            if filtering {
                hints.push(hint("esc", "clear filter", 7));
            } else {
                hints.push(hint("t", "templates", 0));
                hints.push(hint("/", "filter", 1));
            }
            if pending > 0 {
                hints.push(Hint {
                    key: "s",
                    what: format!("save {pending} change{}", plural(pending)),
                    chip: true,
                    rank: 11,
                });
                hints.push(hint("u", "discard", 7));
            } else {
                hints.push(hint("s", "save", 11));
            }
            hints.push(hint("ctrl+k", "commands", 4));
            hints.push(hint("?", "help", 6));
            hints.push(hint("q", "quit", 9));
            hints
        }
        Mode::Filter => vec![
            hint("type", "to filter", 9),
            hint("enter", "keep", 9),
            hint("esc", "clear", 9),
        ],
        Mode::ConfirmQuit => vec![
            hint("y", "discard the changes and quit", 9),
            hint("n", "keep editing", 9),
        ],
        Mode::Help => vec![hint("? or esc", "closes this help", 9)],
        Mode::Info => vec![
            hint("esc", "close", 9),
            hint("j/k", "scroll", 8),
            hint("o", "open repository", 4),
            hint("c", "image ref", 3),
        ],
        Mode::Palette => vec![
            hint("esc", "close", 9),
            hint("up/down", "move", 8),
            hint("enter", "run", 9),
        ],
        Mode::Port => vec![
            hint("enter", "apply", 9),
            hint("esc", "cancel", 9),
            hint("auto", "any free port", 5),
            hint("none", "no host port", 5),
        ],
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key bar as one line, the way it reads on screen.
    fn bar(mode: Mode, pending: usize, filtering: bool) -> String {
        keybar(mode, pending, filtering)
            .iter()
            .map(|hint| format!("{} {}", hint.key, hint.what))
            .collect::<Vec<String>>()
            .join("   ")
    }

    #[test]
    fn every_mode_has_a_key_bar_that_names_its_way_out() {
        assert_eq!(
            bar(Mode::Browse, 0, false),
            "j/k move   space toggle   i info   p port   v channel   t templates   \
             / filter   s save   ctrl+k commands   ? help   q quit"
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
            keybar(Mode::Browse, 2, false)
                .iter()
                .filter(|hint| hint.chip)
                .count(),
            1,
            "the save chip is the only filled thing on the bar"
        );
        assert!(bar(Mode::Browse, 1, false).contains("s save 1 change"));
        assert!(keybar(Mode::Browse, 0, false).iter().all(|h| !h.chip));

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
        assert_eq!(browse_action(KeyCode::Char('v')), Action::CycleChannel);
        assert_eq!(browse_action(KeyCode::Char('t')), Action::ToggleTemplates);
        assert_eq!(browse_action(KeyCode::Char('/')), Action::StartFilter);
        assert_eq!(browse_action(KeyCode::Char('s')), Action::Save);
        assert_eq!(browse_action(KeyCode::Char('u')), Action::Discard);
        assert_eq!(browse_action(KeyCode::Char('o')), Action::OpenRepository);
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
            Action::OpenRepository
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
        assert_eq!(browse_action(KeyCode::Tab), Action::None);
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
}
