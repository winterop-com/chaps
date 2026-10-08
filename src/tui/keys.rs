//! Key bindings, mapped to actions for the reducer.
//!
//! Keys avoid `[`, `]`, `{`, `}`, `|` and `\`, which need Alt on a Norwegian
//! keyboard layout.

use crate::tui::app::{Action, Mode, Page};
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
        Mode::Channel | Mode::Dhis2Version => channel(key),
        Mode::Configs => configs(key),
        Mode::ConfigForm => form(key, ctrl),
        Mode::ConfigConfirm => confirm(key),
    }
}

/// The configured models of one model: move, add, archive, ask again, back.
fn configs(key: &KeyEvent) -> Action {
    if key.modifiers.contains(KeyModifiers::ALT) {
        return Action::None;
    }
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => Action::Down,
        KeyCode::Char('k') | KeyCode::Up => Action::Up,
        KeyCode::Char('g') | KeyCode::Home => Action::Top,
        KeyCode::Char('G') | KeyCode::End => Action::Bottom,
        KeyCode::Char('a') => Action::ConfigAdd,
        KeyCode::Char('d') => Action::ConfigArchive,
        KeyCode::Char('r') => Action::ConfigReload,
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('m') => Action::FilterCancel,
        _ => Action::None,
    }
}

/// The form: every printable key is text in the field; the arrows and Tab
/// move between the fields.
fn form(key: &KeyEvent, ctrl: bool) -> Action {
    if ctrl {
        return match key.code {
            KeyCode::Char('n') => Action::Down,
            _ => Action::None,
        };
    }
    match key.code {
        KeyCode::Esc => Action::FilterCancel,
        KeyCode::Enter => Action::FormSubmit,
        KeyCode::Backspace => Action::FormBackspace,
        KeyCode::Tab | KeyCode::Down => Action::Down,
        KeyCode::BackTab | KeyCode::Up => Action::Up,
        KeyCode::Char(c) => Action::FormChar(c),
        _ => Action::None,
    }
}

/// The channel dialog: two rows, so j/k, the first letter, Enter and Esc are
/// the whole of it.
fn channel(key: &KeyEvent) -> Action {
    match key.code {
        KeyCode::Esc => Action::FilterCancel,
        KeyCode::Enter => Action::ChannelApply,
        KeyCode::Char('j') | KeyCode::Down => Action::Down,
        KeyCode::Char('k') | KeyCode::Up => Action::Up,
        KeyCode::Char(c) => Action::ChannelChar(c),
        _ => Action::None,
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
        // Tab walks the pages, shift-Tab walks them the other way; with two
        // pages either one is "the other list".
        KeyCode::Tab => Action::NextPage,
        KeyCode::BackTab => Action::PrevPage,
        KeyCode::Char(' ') => Action::Toggle,
        KeyCode::Char('p') => Action::PortPrompt,
        // Shift-P is the prompt's answer without the prompt: the one thing
        // there is nothing to type for.
        KeyCode::Char('P') => Action::RemovePort,
        KeyCode::Char('v') => Action::ChannelPrompt,
        KeyCode::Char('t') => Action::ToggleTemplates,
        KeyCode::Char('/') => Action::StartFilter,
        // Enter is the second way into the details, next to `i`; saving is
        // `s`, which is the one key that writes anything.
        KeyCode::Char('i') | KeyCode::Enter => Action::Info,
        KeyCode::Char('u') => Action::Discard,
        // One key, one meaning - hand this row to a browser - and each page
        // decides what that is: a model's repository, a component's web
        // interface.
        KeyCode::Char('o') => Action::Open,
        KeyCode::Char('c') => Action::ImageRef,
        KeyCode::Char('m') => Action::Configs,
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
        KeyCode::Char('o') => Action::Open,
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
        ("tab / shift-tab", "the other page: models or components"),
        ("space", "enable or disable the row"),
        ("i / Enter", "the full details, which j/k scroll"),
        ("p", "publish a host port: a number, auto, or none"),
        ("P", "take the host port away"),
        ("v", "pick the channel; on the dhis2 component, its version"),
        ("t", "show or hide templates"),
        ("/", "filter; Enter keeps it, Esc clears it"),
        ("s", "save the changes; `varde up` applies them"),
        ("u", "discard the pending changes"),
        ("o", "open the repository, or the web interface"),
        ("c", "show the model's image reference"),
        ("m", "the configured models of the model in chap-core"),
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

/// The key bar under the list for a page and a mode.
///
/// `pending` and `filtering` are what the browser is in the middle of: with
/// changes to save the bar grows a save chip and a way to throw them away,
/// and with a filter on it offers to clear it rather than to hide templates.
pub fn keybar(page: Page, mode: Mode, pending: usize, filtering: bool) -> Vec<Hint> {
    match mode {
        Mode::Browse => {
            // The rank is what the bar gives up first when the terminal is
            // too narrow for all of it: the two display toggles before the
            // two things that act on the row under the cursor. `tab` outranks
            // both, because it is the only way to the other list.
            let mut hints = vec![hint("j/k", "move", 10), hint("tab", "page", 7)];
            hints.push(hint("space", "toggle", 8));
            hints.push(hint("i", "info", 5));
            if page == Page::Models {
                hints.push(hint("m", "configs", 0));
            }
            // Only the components page has something of its own to open: a
            // model's repository is named on its details overlay's bar, where
            // `o` already sits.
            if page == Page::Components {
                hints.push(hint("o", "open", 5));
                hints.push(hint("v", "dhis2 version", 2));
            }
            hints.push(hint("p", "port", 3));
            // Neither belongs to a component: it follows no channel, and a
            // handful of rows is not a list to filter.
            if page == Page::Models {
                hints.push(hint("v", "channel", 2));
                if filtering {
                    hints.push(hint("esc", "clear filter", 7));
                } else {
                    hints.push(hint("t", "templates", 0));
                    hints.push(hint("/", "filter", 1));
                }
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
        // A component has no image reference to copy, and what `o` opens for it
        // is its web interface rather than a repository.
        Mode::Info if page == Page::Components => vec![
            hint("esc", "close", 9),
            hint("j/k", "scroll", 8),
            hint("o", "open", 4),
        ],
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
        Mode::Configs => vec![
            hint("esc", "back", 9),
            hint("j/k", "move", 8),
            hint("a", "add", 9),
            hint("d", "archive", 7),
            hint("r", "reload", 5),
        ],
        Mode::ConfigForm => vec![
            hint("esc", "cancel", 9),
            hint("tab/arrows", "field", 8),
            hint("enter", "save", 9),
        ],
        Mode::ConfigConfirm => vec![hint("y", "yes", 9), hint("n", "no", 9)],
        // Both dialogs carry their own key line; the bar under them says the
        // one thing that is true wherever the cursor is.
        Mode::Port | Mode::Channel | Mode::Dhis2Version => vec![hint("esc", "cancel", 9)],
    }
}

/// The key line inside the port dialog.
///
/// `auto` is only offered on the models page: it means the lowest free port in
/// the project's model range, which a component is not in.
pub fn port_dialog_keys(page: Page) -> Vec<Hint> {
    let mut hints = vec![hint("enter", "apply", 9), hint("esc", "cancel", 9)];
    if page == Page::Models {
        hints.push(hint("auto", "any free port", 5));
    }
    hints.push(hint("none", "no host port", 5));
    hints
}

/// The key line inside the DHIS2 version dialog.
pub fn dhis2_version_dialog_keys() -> Vec<Hint> {
    vec![
        hint("enter", "apply", 9),
        hint("esc", "cancel", 9),
        hint("j/k", "move", 6),
    ]
}

/// The key line inside the channel dialog.
pub fn channel_dialog_keys() -> Vec<Hint> {
    vec![
        hint("enter", "apply", 9),
        hint("esc", "cancel", 9),
        hint("j/k", "move", 6),
        hint("s/l", "pick one", 5),
    ]
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests;
