//! TUI state and the pure `App::reduce(Action) -> Option<Outcome>` reducer.
//!
//! Nothing here touches the terminal: the browser is a state machine that
//! takes [`Action`]s and ends by producing a [`Selection`], which the caller
//! applies through [`crate::compose::apply()`]. That is what makes the browser
//! testable without a tty.
//!
//! There are two pages, [`Page::Models`] and [`Page::Components`], and one
//! selection: every key that acts on a row acts on the page that is up, and
//! saving carries both pages' changes.

use crate::components::{Component, Components, models_need_chap_core};
use crate::compose::{EnableRequest, PortRequest, Selection};
use crate::project::{EnabledModel, ProjectState};
use crate::registry::{Channel, Model, Registry, Version, VersionSelector};
use std::cell::Cell;
use std::collections::BTreeMap;

/// Rows a page key moves the cursor by.
pub const PAGE_JUMP: usize = 10;

/// Footer note shown after enabling a template.
pub const TEMPLATE_WARNING: &str = "templates are not for real forecasts";

/// Footer note shown when a template row is toggled while templates are hidden.
pub const TEMPLATE_HIDDEN_HINT: &str = "press t to show templates first";

/// Footer note shown when `p` is pressed on a row that is not enabled.
pub const PUBLISH_NEEDS_ENABLED_HINT: &str = "enable the model first (space), then press p";

/// Footer note shown when `u` is pressed with nothing to discard.
pub const NOTHING_TO_DISCARD_HINT: &str = "there is nothing to discard";

/// Footer note shown after `u` threw the pending changes away.
pub const DISCARDED_HINT: &str = "the pending changes are gone; nothing was written";

/// Footer note shown when `p` is pressed on a component that is not enabled.
pub const PORT_NEEDS_COMPONENT_HINT: &str = "enable the component first (space), then press p";

/// Footer note shown when `p` is pressed on `chap-core`.
///
/// chap-core's host port is the API port, which lives in `project.yaml` rather
/// than in the component block, so this page is not where it is edited.
pub const CORE_PORT_IS_API_PORT: &str = "chap-core's host port is the API port; run `chaps init --api-port PORT --force`, \
     or set CHAP_API_PORT in .env";

/// Why the component port prompt refuses `auto`.
pub const COMPONENT_HAS_NO_AUTO_PORT: &str =
    "`auto` picks from the model port range; type a number, or none";

/// The documentation the palette's "open the documentation" opens, per page.
pub const DOCS_CHAPTER: &str = "models.html";
pub const COMPONENTS_DOCS_CHAPTER: &str = "components.html";

/// The channels the dialog offers, in the order it lists them.
pub const CHANNELS: [Channel; 2] = [Channel::Stable, Channel::Latest];

/// Which list the browser is showing.
///
/// Two pages rather than one table: a component has no version, no maturity
/// and no channel, and `space` on a model resolves a version out of the
/// registry where `space` on a component only flips a flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Models,
    Components,
}

impl Page {
    /// The pages, in the order `Tab` walks them.
    pub const ALL: [Page; 2] = [Page::Models, Page::Components];

    /// What the title bar calls it.
    pub fn title(self) -> &'static str {
        match self {
            Page::Models => "models",
            Page::Components => "components",
        }
    }

    /// What the box around the list is called.
    pub fn pane(self) -> &'static str {
        match self {
            Page::Models => "Marketplace",
            Page::Components => "Components",
        }
    }
}

/// Which sub-state the browser is in; it decides both key mapping and what is
/// drawn on top of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Browse,
    Filter,
    ConfirmQuit,
    Help,
    /// The full details of the row under the cursor, on top of the list.
    Info,
    /// The command palette.
    Palette,
    /// The dialog `p` opens, asking for a host port.
    Port,
    /// The dialog `v` opens, asking which channel to follow.
    Channel,
}

/// Everything the browser can be asked to do, independent of key bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Top,
    Bottom,
    PageUp,
    PageDown,
    Toggle,
    /// Go to the next page, which `Tab` does.
    NextPage,
    /// Go to the previous one, which shift-Tab does.
    PrevPage,
    /// Open the port prompt on the row under the cursor.
    PortPrompt,
    PortChar(char),
    PortBackspace,
    /// Take what the prompt holds, or say why it cannot be taken.
    PortApply,
    /// Take the host port off the row under the cursor.
    RemovePort,
    /// Open the channel dialog on the row under the cursor.
    ChannelPrompt,
    /// Pick a channel by its first letter.
    ChannelChar(char),
    /// Follow the channel the dialog's cursor is on.
    ChannelApply,
    ToggleTemplates,
    StartFilter,
    FilterChar(char),
    FilterBackspace,
    FilterDone,
    FilterCancel,
    /// Esc in the list: drop an active filter, or leave when there is none.
    ClearFilterOrQuit,
    /// Open the details overlay, or close it again.
    Info,
    /// Open the command palette.
    Palette,
    PaletteChar(char),
    PaletteBackspace,
    /// Run the command under the palette's cursor.
    PaletteRun,
    /// Throw the pending changes away.
    Discard,
    /// Hand the selected model's repository to the platform opener.
    OpenRepository,
    /// Put the selected model's image reference on the status line.
    ImageRef,
    Save,
    Quit,
    ConfirmYes,
    ConfirmNo,
    Help,
    None,
}

/// How the browser ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Apply what [`App::selection`] returns.
    Save,
    /// Leave the project untouched.
    Quit,
}

/// Something the reducer cannot do itself because it leaves the process.
///
/// The reducer stays pure by recording the wish; [`crate::tui::run_tui`] is
/// what actually spawns an opener or goes back to the marketplace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Hand this URL to the platform's opener.
    Open(String),
    /// Re-fetch the catalogue, the way `chaps registry update` does.
    Refresh,
    /// Write a picture of the next frame to the working directory.
    ///
    /// The next one, not this one: the frame that is still on screen has the
    /// palette over it, and nobody wants a screenshot of the menu they used
    /// to take it.
    Screenshot,
}

/// One catalogue entry as the browser tracks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Index into [`Registry::models`].
    pub model_idx: usize,
    pub enabled: bool,
    /// Channel the row would be pinned to when enabled.
    pub channel: Channel,
    /// Host port from `.chaps/models.yaml`, for rows that are already enabled
    /// and publish one.
    pub port: Option<u16>,
    /// The host port the row should end up with. Starts out matching
    /// [`Row::port`]; `p` edits it, and saving turns the difference into a
    /// [`PortRequest`].
    pub want: PortWant,
}

impl Row {
    /// Whether the row asks for a host port of its own at all.
    #[cfg(test)]
    pub fn publishes(&self) -> bool {
        self.want != PortWant::None
    }
}

/// What a row asks for in its PORT column.
///
/// The browser cannot pick a port itself - only [`crate::compose::apply()`]
/// knows what the compose files and the machine have taken - so `Auto` is a
/// question the save answers, and `Exact` is one the save has to honour or
/// fail on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortWant {
    /// No host port: chap-core's proxy is the way in.
    #[default]
    None,
    /// The lowest free port in the project's range, picked when saving.
    Auto,
    /// This port, or a save that says who has it.
    Exact(u16),
}

impl PortWant {
    /// What the PORT column and the prompt show for it.
    pub fn text(&self) -> String {
        match self {
            PortWant::None => String::new(),
            PortWant::Auto => "auto".to_string(),
            PortWant::Exact(port) => port.to_string(),
        }
    }
}

/// One row of the components page, in the columns `chaps components list`
/// prints: COMPONENT, STATE, REACH, WHAT IT IS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentLine {
    pub component: Component,
    /// Whether this session's set has it on.
    pub enabled: bool,
    /// Whether the project had it on when the browser opened.
    pub recorded: bool,
    /// Where it is reached: its own host port, the compose network, or `-`
    /// for a component this deployment does not have.
    pub reach: String,
    pub summary: &'static str,
}

/// Header counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub total: usize,
    pub enabled: usize,
    pub pending: usize,
}

/// What one pending change would do, as the summary strip lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// A model this project does not run yet.
    Add,
    /// A model it runs, with a different channel or host port.
    Update,
    /// A model it runs and would stop running.
    Remove,
}

/// One line of the pending strip: the mark, the model, and what saving does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    pub name: String,
    pub detail: String,
}

/// One command the palette can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandId {
    Toggle,
    SetPort,
    RemovePort,
    SetChannel,
    Templates,
    Filter,
    /// Go to the other page.
    Page,
    /// Turn one named component on or off, whichever page is up.
    Component(Component),
    Save,
    Discard,
    Refresh,
    Repository,
    Screenshot,
    Docs,
    Help,
    Quit,
}

/// A palette entry: what it says, what key does the same thing, and where the
/// filter matched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub id: CommandId,
    /// The whole line, with the selected model named where that helps.
    pub label: String,
    /// The key that does the same thing from the list, or `""`.
    pub key: &'static str,
    /// The name the palette's own footer strip lists it under.
    pub short: &'static str,
    /// Where the filter matched `label`, as character offsets.
    pub hit: Option<(usize, usize)>,
}

/// The browser's whole state.
pub struct App<'a> {
    pub registry: &'a Registry,
    pub rows: Vec<Row>,
    /// Indices into [`App::rows`], in display order.
    pub visible: Vec<usize>,
    /// Index into [`App::visible`].
    pub cursor: usize,
    pub filter: String,
    pub mode: Mode,
    /// Which list is up. Every key that acts on a row acts on this page's.
    pub page: Page,
    /// The component set the session wants, which saving turns into
    /// [`Selection::components`].
    pub components: Components,
    /// `.chaps/components.yaml` as it was when the browser opened; the wanted
    /// set is the diff against this.
    pub initial_components: Components,
    /// Index into [`Component::ALL`], the components page's cursor.
    pub component_cursor: usize,
    pub show_templates: bool,
    /// `.chaps/models.yaml` as it was when the browser opened; the selection is the
    /// diff against this.
    pub initial: BTreeMap<String, EnabledModel>,
    /// The project's model port range, which the prompt refuses to leave.
    pub port_range: (u16, u16),
    /// chap-core's own host port, which no model may take.
    pub api_port: u16,
    /// What is being typed into the port prompt.
    pub port_input: String,
    /// Why the last thing typed into it was refused.
    pub port_error: Option<String>,
    pub dirty: bool,
    /// Transient footer note, cleared by the next action.
    pub message: Option<String>,
    /// First line of the details overlay that is on screen.
    pub info_scroll: usize,
    /// How far the overlay can scroll, which only the renderer knows because
    /// it depends on the terminal's size. The one thing drawing writes.
    pub info_max: Cell<usize>,
    /// Which row the channel dialog is on, as an index into [`CHANNELS`].
    pub channel_cursor: usize,
    pub palette_query: String,
    /// Index into [`App::palette_matches`].
    pub palette_cursor: usize,
    /// What the reducer wants the caller to do outside the terminal.
    pub effect: Option<Effect>,
}

impl<'a> App<'a> {
    /// Build the browser state from the catalogue and the project's state file.
    pub fn new(registry: &'a Registry, state: &ProjectState) -> App<'a> {
        let mut rows: Vec<Row> = registry
            .models
            .iter()
            .enumerate()
            .map(|(model_idx, model)| {
                let enabled = state.models.get(&model.id);
                let port = enabled.and_then(|e| e.host_port);
                Row {
                    model_idx,
                    enabled: enabled.is_some(),
                    channel: enabled.and_then(|e| e.channel).unwrap_or(Channel::Stable),
                    port,
                    want: port.map(PortWant::Exact).unwrap_or_default(),
                }
            })
            .collect();
        // Maturity, not the marketplace index's file order: the first row a
        // reader lands on should be the one most likely to be worth running.
        // Nothing the browser does changes a model's maturity, so this order
        // holds for the whole session and a toggle never moves the cursor.
        rows.sort_by_key(|row| registry.models[row.model_idx].order_key());

        // A project that already runs a template should not hide it.
        let show_templates = rows
            .iter()
            .any(|r| r.enabled && registry.models[r.model_idx].is_template());

        let mut app = App {
            registry,
            rows,
            visible: Vec::new(),
            cursor: 0,
            filter: String::new(),
            mode: Mode::Browse,
            page: Page::Models,
            components: state.components.clone(),
            initial_components: state.components.clone(),
            component_cursor: 0,
            show_templates,
            initial: state.models.clone(),
            port_range: state.port_range,
            api_port: state.api_port,
            port_input: String::new(),
            port_error: None,
            channel_cursor: 0,
            dirty: false,
            message: None,
            info_scroll: 0,
            info_max: Cell::new(0),
            palette_query: String::new(),
            palette_cursor: 0,
            effect: None,
        };
        app.refilter();
        app
    }

    /// What the reducer asked the caller to do, once.
    pub fn take_effect(&mut self) -> Option<Effect> {
        self.effect.take()
    }

    /// Apply one action. `Some(_)` ends the browser.
    pub fn reduce(&mut self, action: Action) -> Option<Outcome> {
        if action != Action::None {
            self.message = None;
        }
        match self.mode {
            Mode::Help => self.reduce_help(action),
            Mode::ConfirmQuit => self.reduce_confirm(action),
            Mode::Filter => self.reduce_filter(action),
            Mode::Port => self.reduce_port(action),
            Mode::Channel => self.reduce_channel(action),
            Mode::Info => self.reduce_info(action),
            Mode::Palette => self.reduce_palette(action),
            Mode::Browse => self.reduce_browse(action),
        }
    }

    /// The port prompt: one line of text, taken by Enter and dropped by Esc.
    /// A refusal keeps the prompt up with the reason on it, so a mistyped
    /// port is corrected rather than typed again from nothing.
    fn reduce_port(&mut self, action: Action) -> Option<Outcome> {
        match action {
            Action::PortChar(c) => {
                self.port_input.push(c);
                self.port_error = None;
            }
            Action::PortBackspace => {
                self.port_input.pop();
                self.port_error = None;
            }
            Action::PortApply => self.apply_port_prompt(),
            Action::FilterCancel | Action::Quit => {
                self.mode = Mode::Browse;
                self.port_input.clear();
                self.port_error = None;
            }
            _ => {}
        }
        None
    }

    /// The channel dialog: two rows, picked with j/k or with the first letter
    /// of the one wanted, taken by Enter.
    fn reduce_channel(&mut self, action: Action) -> Option<Outcome> {
        match action {
            Action::Down => self.channel_cursor = (self.channel_cursor + 1).min(1),
            Action::Up => self.channel_cursor = self.channel_cursor.saturating_sub(1),
            Action::ChannelChar(c) => match c.to_ascii_lowercase() {
                's' => self.channel_cursor = 0,
                'l' => self.channel_cursor = 1,
                _ => {}
            },
            Action::ChannelApply => {
                let channel = CHANNELS[self.channel_cursor.min(1)];
                self.mode = Mode::Browse;
                self.set_channel(channel);
            }
            Action::FilterCancel | Action::Quit => self.mode = Mode::Browse,
            _ => {}
        }
        None
    }

    /// The details overlay: it scrolls, it opens a repository, and it closes.
    fn reduce_info(&mut self, action: Action) -> Option<Outcome> {
        let last = self.info_max.get();
        match action {
            Action::Info | Action::Quit | Action::FilterCancel => {
                self.mode = Mode::Browse;
                self.info_scroll = 0;
            }
            Action::Down => self.info_scroll = (self.info_scroll + 1).min(last),
            Action::Up => self.info_scroll = self.info_scroll.saturating_sub(1),
            Action::PageDown => self.info_scroll = (self.info_scroll + PAGE_JUMP).min(last),
            Action::PageUp => self.info_scroll = self.info_scroll.saturating_sub(PAGE_JUMP),
            Action::Top => self.info_scroll = 0,
            Action::Bottom => self.info_scroll = last,
            Action::OpenRepository if self.page == Page::Models => self.open_repository(),
            Action::ImageRef if self.page == Page::Models => self.show_image_ref(),
            _ => {}
        }
        None
    }

    /// The command palette: type to narrow, Enter to run, Esc to leave.
    fn reduce_palette(&mut self, action: Action) -> Option<Outcome> {
        match action {
            Action::PaletteChar(c) => {
                self.palette_query.push(c);
                self.palette_cursor = 0;
            }
            Action::PaletteBackspace => {
                self.palette_query.pop();
                self.palette_cursor = 0;
            }
            Action::Down => {
                let last = self.palette_matches().len().saturating_sub(1);
                self.palette_cursor = (self.palette_cursor + 1).min(last);
            }
            Action::Up => self.palette_cursor = self.palette_cursor.saturating_sub(1),
            Action::PaletteRun => return self.run_command(),
            Action::Palette | Action::FilterCancel => self.close_palette(),
            Action::Quit => return Some(Outcome::Quit),
            _ => {}
        }
        None
    }

    fn close_palette(&mut self) {
        self.mode = Mode::Browse;
        self.palette_query.clear();
        self.palette_cursor = 0;
    }

    fn reduce_help(&mut self, action: Action) -> Option<Outcome> {
        // Any of the keys that could mean "close" closes; nothing else applies
        // while the overlay is up.
        if matches!(
            action,
            Action::Help | Action::Quit | Action::FilterCancel | Action::ConfirmNo | Action::Save
        ) {
            self.mode = Mode::Browse;
        }
        None
    }

    /// Run the command under the palette's cursor, then leave the palette.
    fn run_command(&mut self) -> Option<Outcome> {
        let command = self.palette_matches().get(self.palette_cursor).cloned()?;
        self.close_palette();
        match command.id {
            CommandId::Toggle => self.toggle(),
            CommandId::SetPort => self.open_port_prompt(),
            CommandId::RemovePort => self.remove_port(),
            CommandId::SetChannel => self.open_channel_prompt(),
            CommandId::Templates => {
                self.show_templates = !self.show_templates;
                self.refilter();
            }
            CommandId::Filter => self.mode = Mode::Filter,
            CommandId::Page => self.turn_page(1),
            CommandId::Component(component) => {
                // The cursor follows, so the strip and the footer talk about
                // the component that just moved.
                if let Some(at) = Component::ALL.iter().position(|c| *c == component) {
                    self.component_cursor = at;
                }
                self.toggle_component(component);
            }
            CommandId::Save => return Some(Outcome::Save),
            CommandId::Discard => self.discard(),
            CommandId::Refresh => self.effect = Some(Effect::Refresh),
            CommandId::Repository => self.open_repository(),
            CommandId::Screenshot => self.effect = Some(Effect::Screenshot),
            CommandId::Docs => {
                let chapter = match self.page {
                    Page::Models => DOCS_CHAPTER,
                    Page::Components => COMPONENTS_DOCS_CHAPTER,
                };
                self.effect = Some(Effect::Open(format!("{}{chapter}", crate::cli::DOCS_URL)))
            }
            CommandId::Help => self.mode = Mode::Help,
            CommandId::Quit => return self.quit(),
        }
        None
    }

    /// Every command the palette offers, in the order it lists them.
    ///
    /// The row under the cursor is named where a command acts on it, so the
    /// palette reads as a sentence about what is selected rather than as a
    /// menu of verbs. The page decides which rows those are: the entries that
    /// resolve a version or filter a catalogue belong to the models page, and
    /// on the components page the same three keys act on a component.
    pub fn commands(&self) -> Vec<Command> {
        let entry = |id, label: String, key, short| Command {
            id,
            label,
            key,
            short,
            hit: None,
        };
        let other = match self.page {
            Page::Models => Page::Components,
            Page::Components => Page::Models,
        };
        let mut commands = vec![entry(
            CommandId::Page,
            format!("Go to the {} page", other.title()),
            "tab",
            "Other page",
        )];

        match self.page {
            Page::Models => {
                let name = self
                    .selected()
                    .map(|row| self.model(row).display_name.clone())
                    .unwrap_or_else(|| "the selected model".to_string());
                commands.extend([
                    entry(
                        CommandId::Toggle,
                        format!("Enable or disable {name}"),
                        "space",
                        "Toggle model",
                    ),
                    entry(
                        CommandId::SetPort,
                        format!("Set a host port for {name}"),
                        "p",
                        "Set port",
                    ),
                    entry(
                        CommandId::RemovePort,
                        format!("Remove the host port of {name}"),
                        "P",
                        "Remove port",
                    ),
                    entry(
                        CommandId::SetChannel,
                        format!("Set the channel of {name}: stable or latest"),
                        "v",
                        "Set channel",
                    ),
                    entry(
                        CommandId::Templates,
                        "Show or hide templates".to_string(),
                        "t",
                        "Templates",
                    ),
                    entry(
                        CommandId::Filter,
                        "Filter the model list".to_string(),
                        "/",
                        "Filter",
                    ),
                ]);
            }
            Page::Components => {
                let name = self.selected_component().name();
                commands.extend([
                    entry(
                        CommandId::Toggle,
                        format!("Enable or disable the {name} component"),
                        "space",
                        "Toggle component",
                    ),
                    entry(
                        CommandId::SetPort,
                        format!("Set a host port for the {name} component"),
                        "p",
                        "Set port",
                    ),
                    entry(
                        CommandId::RemovePort,
                        format!("Remove the host port of the {name} component"),
                        "P",
                        "Remove port",
                    ),
                ]);
            }
        }

        // Every component by name, from either page: this is where someone
        // looking for OCS or an object store finds out the browser has them.
        for component in Component::ALL {
            commands.push(entry(
                CommandId::Component(*component),
                format!("Turn the {} component on or off", component.name()),
                "",
                component.name(),
            ));
        }

        commands.extend([
            entry(
                CommandId::Save,
                "Save the changes and apply them".to_string(),
                "s",
                "Save changes",
            ),
            entry(
                CommandId::Discard,
                "Discard the pending changes".to_string(),
                "u",
                "Discard changes",
            ),
            entry(
                CommandId::Refresh,
                "Refresh the registry from the marketplace".to_string(),
                "",
                "Refresh registry",
            ),
        ]);
        if self.page == Page::Models {
            let name = self
                .selected()
                .map(|row| self.model(row).display_name.clone())
                .unwrap_or_else(|| "the selected model".to_string());
            commands.push(entry(
                CommandId::Repository,
                format!("Open the repository of {name}"),
                "o",
                "Open repository",
            ));
        }
        commands.extend([
            entry(
                CommandId::Screenshot,
                "Save a screenshot (SVG)".to_string(),
                "",
                "Screenshot",
            ),
            entry(
                CommandId::Docs,
                "Open the chaps documentation".to_string(),
                "",
                "Open docs",
            ),
            entry(
                CommandId::Help,
                "Show every key the browser binds".to_string(),
                "?",
                "Help",
            ),
            entry(CommandId::Quit, "Quit the browser".to_string(), "q", "Quit"),
        ]);
        commands
    }

    /// The commands the palette's filter leaves, each carrying where it hit.
    ///
    /// Case-insensitive substring: enough to find a command by typing a word
    /// out of the middle of it, and short enough that it cannot surprise.
    pub fn palette_matches(&self) -> Vec<Command> {
        let needle = self.palette_query.to_lowercase();
        self.commands()
            .into_iter()
            .filter_map(|mut command| {
                if needle.is_empty() {
                    return Some(command);
                }
                let haystack = command.label.to_lowercase();
                let byte = haystack.find(&needle)?;
                let start = haystack[..byte].chars().count();
                command.hit = Some((start, start + needle.chars().count()));
                Some(command)
            })
            .collect()
    }

    fn reduce_confirm(&mut self, action: Action) -> Option<Outcome> {
        match action {
            Action::ConfirmYes => return Some(Outcome::Quit),
            Action::ConfirmNo | Action::FilterCancel => self.mode = Mode::Browse,
            Action::Save => {
                self.mode = Mode::Browse;
                return Some(Outcome::Save);
            }
            _ => {}
        }
        None
    }

    fn reduce_filter(&mut self, action: Action) -> Option<Outcome> {
        match action {
            Action::FilterChar(c) => {
                self.filter.push(c);
                self.refilter();
            }
            Action::FilterBackspace => {
                self.filter.pop();
                self.refilter();
            }
            Action::FilterDone => self.mode = Mode::Browse,
            Action::FilterCancel => {
                self.filter.clear();
                self.refilter();
                self.mode = Mode::Browse;
            }
            Action::Up => self.move_by(-1),
            Action::Down => self.move_by(1),
            Action::Quit => return Some(Outcome::Quit),
            _ => {}
        }
        None
    }

    fn reduce_browse(&mut self, action: Action) -> Option<Outcome> {
        match action {
            Action::Up => self.move_by(-1),
            Action::Down => self.move_by(1),
            Action::Top => self.move_by(-(self.row_count() as isize)),
            Action::Bottom => self.move_by(self.row_count() as isize),
            Action::PageUp => self.move_by(-(PAGE_JUMP as isize)),
            Action::PageDown => self.move_by(PAGE_JUMP as isize),
            Action::NextPage => self.turn_page(1),
            Action::PrevPage => self.turn_page(-1),
            Action::Toggle => self.toggle(),
            Action::PortPrompt => self.open_port_prompt(),
            Action::RemovePort => self.remove_port(),
            // A component follows no channel, so the dialog belongs to the
            // model page alone.
            Action::ChannelPrompt if self.page == Page::Models => self.open_channel_prompt(),
            // Nor is a component filtered or hidden: three rows need neither.
            Action::ToggleTemplates if self.page == Page::Models => {
                self.show_templates = !self.show_templates;
                self.refilter();
            }
            Action::StartFilter if self.page == Page::Models => self.mode = Mode::Filter,
            Action::ClearFilterOrQuit => {
                if self.filter.is_empty() {
                    return self.quit();
                }
                self.filter.clear();
                self.refilter();
            }
            Action::Info => {
                // The components page always has a row to describe; the model
                // list can be filtered down to none.
                if self.page == Page::Components || self.selected().is_some() {
                    self.mode = Mode::Info;
                    self.info_scroll = 0;
                }
            }
            Action::Palette => {
                self.mode = Mode::Palette;
                self.palette_query.clear();
                self.palette_cursor = 0;
            }
            Action::Discard => {
                if self.has_changes() {
                    self.discard();
                    self.message = Some(DISCARDED_HINT.to_string());
                } else {
                    self.message = Some(NOTHING_TO_DISCARD_HINT.to_string());
                }
            }
            // Both are about a marketplace model: a component has no
            // repository of its own and no image reference to copy.
            Action::OpenRepository if self.page == Page::Models => self.open_repository(),
            Action::ImageRef if self.page == Page::Models => self.show_image_ref(),
            Action::Help => self.mode = Mode::Help,
            Action::Save => return Some(Outcome::Save),
            Action::Quit => return self.quit(),
            _ => {}
        }
        None
    }

    /// Leave, asking first when there is something unsaved to lose.
    fn quit(&mut self) -> Option<Outcome> {
        if self.dirty {
            self.mode = Mode::ConfirmQuit;
            None
        } else {
            Some(Outcome::Quit)
        }
    }

    /// Put every row back where the project state had it, on both pages: `u`
    /// means "nothing I did this session", not "nothing on this page".
    fn discard(&mut self) {
        for row in &mut self.rows {
            let recorded = self.initial.get(&self.registry.models[row.model_idx].id);
            row.enabled = recorded.is_some();
            row.channel = recorded.and_then(|e| e.channel).unwrap_or(Channel::Stable);
            row.port = recorded.and_then(|e| e.host_port);
            row.want = row.port.map(PortWant::Exact).unwrap_or_default();
        }
        self.components = self.initial_components.clone();
        self.dirty = false;
        self.refilter();
    }

    /// Go to another page. With two of them either direction flips, which is
    /// what `Tab` and shift-Tab both come to.
    fn turn_page(&mut self, delta: isize) {
        let at = Page::ALL
            .iter()
            .position(|page| *page == self.page)
            .unwrap_or(0) as isize;
        let next = (at + delta).rem_euclid(Page::ALL.len() as isize) as usize;
        self.page = Page::ALL[next];
    }

    /// How many rows the page under the cursor has.
    fn row_count(&self) -> usize {
        match self.page {
            Page::Models => self.visible.len(),
            Page::Components => Component::ALL.len(),
        }
    }

    /// The component the components page's cursor is on.
    pub fn selected_component(&self) -> Component {
        Component::ALL[self.component_cursor.min(Component::ALL.len() - 1)]
    }

    /// Where a component is reached, as the REACH column prints it.
    ///
    /// The same three answers `chaps components list` gives: its own host port,
    /// the compose network, or a dash for a component this deployment does not
    /// have. chap-core's port is the API port, which lives in `project.yaml`
    /// rather than in the component block.
    pub fn component_reach(&self, component: Component) -> String {
        if !self.components.is_enabled(component) {
            return "-".to_string();
        }
        match component {
            Component::ChapCore => format!("http://localhost:{}", self.api_port),
            Component::Ocs => self.components.ocs_reach(),
            Component::S3 => match self.components.s3.port {
                Some(port) => format!("http://localhost:{port}"),
                None => "internal".to_string(),
            },
        }
    }

    /// Every row of the components page, in [`Component::ALL`] order.
    pub fn component_lines(&self) -> Vec<ComponentLine> {
        Component::ALL
            .iter()
            .map(|component| ComponentLine {
                component: *component,
                enabled: self.components.is_enabled(*component),
                recorded: self.initial_components.is_enabled(*component),
                reach: self.component_reach(*component),
                summary: component.summary(),
            })
            .collect()
    }

    /// The host port the wanted set records for a component, whether it is on
    /// or not: what the prompt opens with, and what a toggle brings back.
    fn component_port(&self, component: Component) -> Option<u16> {
        match component {
            Component::ChapCore => None,
            Component::Ocs => self.components.ocs.port,
            Component::S3 => self.components.s3.port,
        }
    }

    fn set_component_port(&mut self, component: Component, port: Option<u16>) {
        match component {
            Component::ChapCore => {}
            Component::Ocs => self.components.ocs.port = port,
            Component::S3 => self.components.s3.port = port,
        }
    }

    /// Turn one component on or off, or say why it cannot go off.
    ///
    /// Turning chap-core off with models enabled is refused in the words
    /// `chaps components disable chap-core` uses, because it is the same
    /// dependency: a model service registers with chap-core.
    fn toggle_component(&mut self, component: Component) {
        let wanted = !self.components.is_enabled(component);
        if !wanted && component == Component::ChapCore {
            let staying: Vec<String> = self
                .rows
                .iter()
                .filter(|row| row.enabled)
                .map(|row| self.model(row).id.clone())
                .collect();
            if !staying.is_empty() {
                self.message = Some(models_need_chap_core(&staying));
                return;
            }
        }
        // The port it publishes is left alone, so a component switched off and
        // on again in one session comes back exactly as the project has it.
        self.components.set_enabled(component, wanted);
        self.dirty = self.has_changes();
    }

    /// Ask the caller to open the selected model's repository.
    fn open_repository(&mut self) {
        let Some(row) = self.selected() else {
            return;
        };
        let url = self.model(row).source.repository.clone();
        self.effect = Some(Effect::Open(url));
    }

    /// Put the selected model's image reference on the status line, where it
    /// can be read off and copied by hand.
    fn show_image_ref(&mut self) {
        let Some(row) = self.selected() else {
            return;
        };
        let model = self.model(row);
        let reference = match self.resolved(row) {
            Some(version) => crate::compose::image_ref(&model.source.image, &version.image_tag),
            None => model.source.image.clone(),
        };
        self.message = Some(reference);
    }

    /// What saving would do, one line per model, for the summary strip.
    pub fn changes(&self) -> Vec<Change> {
        let mut changes = Vec::new();
        for row in &self.rows {
            let model = self.model(row);
            let name = model.display_name.clone();
            match self.initial.get(&model.id) {
                None => {
                    if row.enabled {
                        let version = self
                            .resolved(row)
                            .map(|v| v.version.clone())
                            .unwrap_or_else(|| "-".to_string());
                        let detail = match row.want {
                            PortWant::None => {
                                format!("enable at {version}, via chap-core")
                            }
                            PortWant::Auto => {
                                format!("enable at {version} on an automatic port")
                            }
                            PortWant::Exact(port) => {
                                format!("enable at {version} on port {port}")
                            }
                        };
                        changes.push(Change {
                            kind: ChangeKind::Add,
                            name,
                            detail,
                        });
                    }
                }
                Some(previous) => {
                    if !row.enabled {
                        changes.push(Change {
                            kind: ChangeKind::Remove,
                            name,
                            detail: "disable · the data volume is kept".to_string(),
                        });
                        continue;
                    }
                    let mut moved: Vec<String> = Vec::new();
                    if row.channel != previous.channel.unwrap_or(Channel::Stable) {
                        let version = self
                            .resolved(row)
                            .map(|v| format!(" ({})", v.version))
                            .unwrap_or_default();
                        moved.push(format!("follow {}{version}", row.channel.as_str()));
                    }
                    match port_change(row.want, previous.host_port) {
                        Some(PortRequest::None) => moved.push("remove the host port".into()),
                        Some(PortRequest::Auto) => moved.push("port auto".into()),
                        Some(PortRequest::Fixed(port)) => {
                            moved.push(format!("publish port {port}"))
                        }
                        None => {}
                    }
                    if !moved.is_empty() {
                        changes.push(Change {
                            kind: ChangeKind::Update,
                            name,
                            detail: moved.join(", "),
                        });
                    }
                }
            }
        }
        changes.extend(self.component_changes());
        changes
    }

    /// What saving would do to the component set, one line per component.
    ///
    /// Listed after the models, in [`Component::ALL`] order, so the strip reads
    /// the way the two pages are ordered.
    pub fn component_changes(&self) -> Vec<Change> {
        let mut changes = Vec::new();
        for component in Component::ALL {
            let name = component.name().to_string();
            let was = self.initial_components.is_enabled(*component);
            let now = self.components.is_enabled(*component);
            match (was, now) {
                (false, true) => changes.push(Change {
                    kind: ChangeKind::Add,
                    name,
                    detail: match self.components.port_of(*component) {
                        Some(port) => format!("enable on port {port}"),
                        None => "enable, on the compose network".to_string(),
                    },
                }),
                (true, false) => changes.push(Change {
                    kind: ChangeKind::Remove,
                    name,
                    detail: match component.volume() {
                        Some(volume) => format!("disable · the {volume} volume is kept"),
                        None => "disable · its own volumes are kept".to_string(),
                    },
                }),
                // Still on, and the port under it may have moved.
                (true, true) => {
                    let before = self.initial_components.port_of(*component);
                    let now = self.components.port_of(*component);
                    if before != now {
                        changes.push(Change {
                            kind: ChangeKind::Update,
                            name,
                            detail: match now {
                                Some(port) => format!("publish port {port}"),
                                None => "remove the host port".to_string(),
                            },
                        });
                    }
                }
                (false, false) => {}
            }
        }
        changes
    }

    /// The row under the cursor, if the list is not empty.
    pub fn selected(&self) -> Option<&Row> {
        self.visible.get(self.cursor).map(|i| &self.rows[*i])
    }

    /// The catalogue entry a row describes.
    pub fn model(&self, row: &Row) -> &'a Model {
        &self.registry.models[row.model_idx]
    }

    /// The catalogue entry under the cursor.
    #[cfg(test)]
    pub fn selected_model(&self) -> Option<&'a Model> {
        self.selected().map(|row| self.model(row))
    }

    /// The version a row's channel currently points at, when it resolves.
    pub fn resolved(&self, row: &Row) -> Option<&'a Version> {
        self.model(row)
            .resolve(&VersionSelector::Channel(row.channel))
            .ok()
    }

    /// What the project recorded for a row when the browser opened.
    pub fn recorded(&self, row: &Row) -> Option<&EnabledModel> {
        self.initial.get(&self.model(row).id)
    }

    /// Header counters: catalogue size, enabled rows, pending changes.
    ///
    /// The pending count is both pages' worth, because one `s` writes both and
    /// a counter that only saw the page in front of you would be a way to lose
    /// the other one.
    pub fn counts(&self) -> Counts {
        let selection = self.selection();
        Counts {
            total: self.rows.len(),
            enabled: self.rows.iter().filter(|r| r.enabled).count(),
            pending: selection.enable.len()
                + selection.disable.len()
                + self.component_changes().len(),
        }
    }

    /// The diff against the project state the browser opened with.
    ///
    /// A row is enabled when it was not before, or when the user changed its
    /// channel or whether it publishes a host port; a row that was enabled and
    /// is not any more is disabled. Rows nobody touched produce nothing, so
    /// saving an untouched browser is a no-op.
    pub fn selection(&self) -> Selection {
        let mut selection = Selection::default();
        for row in &self.rows {
            let model = self.model(row);
            match self.initial.get(&model.id) {
                None => {
                    if row.enabled {
                        // A row enabled in this session publishes a port only
                        // if `p` asked for one.
                        let port = match row.want {
                            PortWant::None => None,
                            PortWant::Auto => Some(PortRequest::Auto),
                            PortWant::Exact(port) => Some(PortRequest::Fixed(port)),
                        };
                        selection.enable.push(request(model, row, port));
                    }
                }
                Some(previous) => {
                    if !row.enabled {
                        selection.disable.push(model.id.clone());
                        continue;
                    }
                    let channel_moved = row.channel != previous.channel.unwrap_or(Channel::Stable);
                    let port = port_change(row.want, previous.host_port);
                    if channel_moved || port.is_some() {
                        // Pressing `p` asks for a host port, not for a new
                        // version: only a toggle or a new channel resolves the
                        // registry again. Without this, publishing a port
                        // would move a model that follows `latest` onto
                        // whatever that points at today, and take an exact pin
                        // off its version altogether.
                        let mut req = request(model, row, port);
                        req.keep_version = !channel_moved;
                        selection.enable.push(req);
                    }
                }
            }
        }
        // The component set rides on the same selection, so one `s` writes
        // both pages. It is only set when the session moved something: apply
        // would treat an identical set as a no-op, but a selection that says
        // nothing is what makes `chaps ui` able to say "no changes".
        if self.components != self.initial_components {
            selection.components = Some(self.components.clone());
        }
        selection
    }

    /// Whether the selection would change anything, on either page.
    pub fn has_changes(&self) -> bool {
        let selection = self.selection();
        !selection.enable.is_empty()
            || !selection.disable.is_empty()
            || selection.components.is_some()
    }

    fn toggle(&mut self) {
        if self.page == Page::Components {
            self.toggle_component(self.selected_component());
            return;
        }
        let Some(&row_idx) = self.visible.get(self.cursor) else {
            return;
        };
        let is_template = self.model(&self.rows[row_idx]).is_template();
        if is_template && !self.show_templates {
            self.message = Some(TEMPLATE_HIDDEN_HINT.to_string());
            return;
        }
        // A model registers with chap-core, so a session that has switched
        // chap-core off says so now rather than at the save, where the
        // refusal would arrive after the browser had closed.
        if !self.rows[row_idx].enabled && !self.components.chap_core.enabled {
            self.message = Some(crate::components::MODELS_NEED_CHAP_CORE.to_string());
            return;
        }

        let row = &mut self.rows[row_idx];
        row.enabled = !row.enabled;
        let now_enabled = row.enabled;
        if !now_enabled {
            // Turning a model off drops its port with it, so turning it back
            // on in the same session does not silently re-publish.
            row.want = PortWant::None;
        }
        if is_template && now_enabled {
            self.message = Some(TEMPLATE_WARNING.to_string());
        }
        self.dirty = self.has_changes();
    }

    /// Open the port prompt on the row under the cursor, prefilled with what
    /// it asks for today.
    ///
    /// The browser does not pick the port: `auto` leaves that to
    /// [`crate::compose::apply()`], which knows what the compose files and the
    /// machine have taken, and an exact port is a claim the save has to
    /// honour or fail on.
    fn open_port_prompt(&mut self) {
        if self.page == Page::Components {
            self.open_component_port_prompt();
            return;
        }
        let Some(&row_idx) = self.visible.get(self.cursor) else {
            return;
        };
        if !self.rows[row_idx].enabled {
            self.message = Some(PUBLISH_NEEDS_ENABLED_HINT.to_string());
            return;
        }
        self.port_input = match self.rows[row_idx].want {
            // Pressing p and Enter on a row with no port still means "any
            // free one", which is what it has always meant.
            PortWant::None => PortWant::Auto.text(),
            want => want.text(),
        };
        self.port_error = None;
        self.mode = Mode::Port;
    }

    /// Open the prompt on the component under the cursor, prefilled with the
    /// port it publishes today.
    ///
    /// chap-core is refused rather than prompted: its host port is the API
    /// port, and `project.yaml` is where that one lives.
    fn open_component_port_prompt(&mut self) {
        let component = self.selected_component();
        if component == Component::ChapCore {
            self.message = Some(CORE_PORT_IS_API_PORT.to_string());
            return;
        }
        if !self.components.is_enabled(component) {
            self.message = Some(PORT_NEEDS_COMPONENT_HINT.to_string());
            return;
        }
        self.port_input = match self.component_port(component) {
            Some(port) => port.to_string(),
            None => String::new(),
        };
        self.port_error = None;
        self.mode = Mode::Port;
    }

    /// Read what was typed, and either take it or say why not.
    fn apply_port_prompt(&mut self) {
        if self.page == Page::Components {
            self.apply_component_port_prompt();
            return;
        }
        let Some(&row_idx) = self.visible.get(self.cursor) else {
            self.mode = Mode::Browse;
            return;
        };
        let taken: Vec<u16> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(i, row)| *i != row_idx && row.enabled)
            .filter_map(|(_, row)| match row.want {
                PortWant::Exact(port) => Some(port),
                _ => None,
            })
            .collect();
        match parse_port(&self.port_input, self.port_range, self.api_port, &taken) {
            Ok(want) => {
                self.rows[row_idx].want = want;
                self.port_error = None;
                self.port_input.clear();
                self.mode = Mode::Browse;
                self.dirty = self.has_changes();
            }
            // The prompt stays up with the reason on it, so the number can be
            // corrected rather than retyped from nothing.
            Err(why) => self.port_error = Some(why),
        }
    }

    /// The component prompt's answer: a number, or nothing at all.
    fn apply_component_port_prompt(&mut self) {
        let component = self.selected_component();
        // Every other port this session has claimed, so two components cannot
        // be sent to the same one.
        let mut taken: Vec<(u16, String)> = Vec::new();
        for other in Component::ALL.iter().filter(|c| **c != component) {
            if let Some(port) = self.components.port_of(*other) {
                taken.push((port, format!("the {} component", other.name())));
            }
        }
        for row in self.rows.iter().filter(|row| row.enabled) {
            if let PortWant::Exact(port) = row.want {
                taken.push((port, format!("the model {}", self.model(row).id)));
            }
        }
        match parse_component_port(&self.port_input, self.api_port, &taken) {
            Ok(port) => {
                self.set_component_port(component, port);
                self.port_error = None;
                self.port_input.clear();
                self.mode = Mode::Browse;
                self.dirty = self.has_changes();
            }
            Err(why) => self.port_error = Some(why),
        }
    }

    /// Take the host port off the row under the cursor, which is what the
    /// palette's second port command asks for.
    fn remove_port(&mut self) {
        if self.page == Page::Components {
            let component = self.selected_component();
            if component == Component::ChapCore {
                self.message = Some(CORE_PORT_IS_API_PORT.to_string());
                return;
            }
            if !self.components.is_enabled(component) {
                self.message = Some(PORT_NEEDS_COMPONENT_HINT.to_string());
                return;
            }
            self.set_component_port(component, None);
            self.dirty = self.has_changes();
            return;
        }
        let Some(&row_idx) = self.visible.get(self.cursor) else {
            return;
        };
        if !self.rows[row_idx].enabled {
            self.message = Some(PUBLISH_NEEDS_ENABLED_HINT.to_string());
            return;
        }
        self.rows[row_idx].want = PortWant::None;
        self.dirty = self.has_changes();
    }

    /// Open the channel dialog on the row under the cursor, with its cursor
    /// on the channel the row follows today.
    fn open_channel_prompt(&mut self) {
        let Some(row) = self.selected() else {
            return;
        };
        self.channel_cursor = CHANNELS
            .iter()
            .position(|c| *c == row.channel)
            .unwrap_or_default();
        self.mode = Mode::Channel;
    }

    fn set_channel(&mut self, channel: Channel) {
        let Some(&row_idx) = self.visible.get(self.cursor) else {
            return;
        };
        self.rows[row_idx].channel = channel;
        self.dirty = self.has_changes();
    }

    fn move_by(&mut self, delta: isize) {
        if self.page == Page::Components {
            let last = (Component::ALL.len() - 1) as isize;
            let next = (self.component_cursor as isize).saturating_add(delta);
            self.component_cursor = next.clamp(0, last) as usize;
            return;
        }
        if self.visible.is_empty() {
            self.cursor = 0;
            return;
        }
        let last = (self.visible.len() - 1) as isize;
        let next = (self.cursor as isize).saturating_add(delta);
        self.cursor = next.clamp(0, last) as usize;
    }

    /// Recompute [`App::visible`], keeping the cursor on the same row when it
    /// survives the new filter and clamping it into range otherwise.
    pub fn refilter(&mut self) {
        let keep = self.visible.get(self.cursor).copied();
        let registry = self.registry;
        let needle = self.filter.to_lowercase();
        let show_templates = self.show_templates;

        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                let model = &registry.models[row.model_idx];
                // Templates are scaffolding: hidden unless asked for, or
                // already enabled in this project.
                if model.is_template() && !show_templates && !row.enabled {
                    return false;
                }
                matches(model, &needle)
            })
            .map(|(idx, _)| idx)
            .collect();

        self.cursor = match keep.and_then(|k| self.visible.iter().position(|v| *v == k)) {
            Some(position) => position,
            None => self.cursor.min(self.visible.len().saturating_sub(1)),
        };
    }
}

fn request(model: &Model, row: &Row, port: Option<PortRequest>) -> EnableRequest {
    EnableRequest {
        id: model.id.clone(),
        selector: VersionSelector::Channel(row.channel),
        port,
        // Data dirs and users keep whatever the project already has; the
        // browser does not edit them. A toggled row is enabled afresh, so its
        // user is read off the image like any other `models enable`.
        data_dir: None,
        user: None,
        user_from: None,
        allow_template: model.is_template(),
        // The caller decides: a row the user toggled or re-channelled resolves
        // the registry again, a row that only changed its port does not.
        keep_version: false,
    }
}

/// The port request for a row whose wish may have moved, or `None` when it
/// still says what the project recorded.
fn port_change(want: PortWant, recorded: Option<u16>) -> Option<PortRequest> {
    match (want, recorded) {
        (PortWant::None, None) => None,
        (PortWant::None, Some(_)) => Some(PortRequest::None),
        (PortWant::Auto, _) => Some(PortRequest::Auto),
        // The port it already has is not a change; any other one is.
        (PortWant::Exact(port), Some(had)) if port == had => None,
        (PortWant::Exact(port), _) => Some(PortRequest::Fixed(port)),
    }
}

/// What the port prompt accepts: a number in the project's range, `auto`, or
/// nothing at all.
///
/// The reasons are the ones `chaps models expose` gives, minus the two only a
/// save can answer - a port another compose file claims, and a port something
/// on this machine is listening on - which [`crate::compose::apply()`] checks
/// when the selection is applied.
fn parse_port(
    input: &str,
    range: (u16, u16),
    api_port: u16,
    taken: &[u16],
) -> std::result::Result<PortWant, String> {
    let text = input.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("none") {
        return Ok(PortWant::None);
    }
    if text.eq_ignore_ascii_case("auto") {
        return Ok(PortWant::Auto);
    }
    let port: u16 = text
        .parse()
        .map_err(|_| format!("`{text}` is not a port; type a number, auto, or none"))?;
    // The API port is named before the range, because it is usually outside
    // it and "outside the range" is not the reason worth giving for it.
    if port == api_port {
        return Err(format!("port {port} is chap-core's own API port"));
    }
    if port < range.0 || port > range.1 {
        return Err(format!(
            "port {port} is outside this project's range {}-{}",
            range.0, range.1
        ));
    }
    if taken.contains(&port) {
        return Err(format!("port {port} is already taken by another model"));
    }
    Ok(PortWant::Exact(port))
}

/// What the port prompt accepts on the components page: a number, or nothing
/// at all.
///
/// `auto` is not one of them. It means "the lowest free port in this project's
/// model range", which is a question the save answers for a model service; a
/// component publishes a well-known port of its own and is not in that range,
/// so the prompt says what to type instead of picking a number nobody asked
/// for. The range itself is not checked either, for the same reason.
fn parse_component_port(
    input: &str,
    api_port: u16,
    taken: &[(u16, String)],
) -> std::result::Result<Option<u16>, String> {
    let text = input.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    if text.eq_ignore_ascii_case("auto") {
        return Err(COMPONENT_HAS_NO_AUTO_PORT.to_string());
    }
    let port: u16 = text
        .parse()
        .map_err(|_| format!("`{text}` is not a port; type a number, or none"))?;
    if port == 0 {
        return Err("`0` is not a port; type a number, or none".to_string());
    }
    if port == api_port {
        return Err(format!("port {port} is chap-core's own API port"));
    }
    if let Some((_, holder)) = taken.iter().find(|(other, _)| *other == port) {
        return Err(format!("port {port} is already taken by {holder}"));
    }
    Ok(Some(port))
}

/// Case-insensitive substring match over the fields a user would type.
fn matches(model: &Model, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    model.id.to_lowercase().contains(needle)
        || model.service_id.to_lowercase().contains(needle)
        || model.display_name.to_lowercase().contains(needle)
        || model.summary.to_lowercase().contains(needle)
}

#[cfg(test)]
mod tests {
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
            ("8000", "chap-core's own API port"),
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

        app.reduce(Action::OpenRepository);
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
        assert_eq!(app.commands().len(), 18);

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
        assert_eq!(lines[1].reach, "http://localhost:9000");
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
        assert_eq!(changes[0].detail, "enable on port 9000");

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
            Some(9000),
            "its port is left alone, so switching it back on restores it"
        );
    }

    /// The one hard dependency between components, in the words `chaps
    /// components disable chap-core` uses.
    #[test]
    fn chap_core_cannot_be_switched_off_while_a_model_is_enabled() {
        let registry = registry();
        let state = state_with(&registry, EWARS, Some(Channel::Stable));
        let mut app = App::new(&registry, &state);
        focus_component(&mut app, Component::ChapCore);

        app.reduce(Action::Toggle);
        assert_eq!(
            app.message.as_deref(),
            Some(models_need_chap_core(&[EWARS.to_string()]).as_str())
        );
        assert!(app.components.chap_core.enabled, "it is still on");
        assert!(!app.has_changes());

        // Disabling the model first is what clears the way, and both changes
        // ride on one selection.
        app.page = Page::Models;
        focus(&mut app, EWARS);
        app.reduce(Action::Toggle);
        focus_component(&mut app, Component::ChapCore);
        app.reduce(Action::Toggle);
        assert!(!app.components.chap_core.enabled);
        let selection = app.selection();
        assert_eq!(selection.disable, vec![EWARS.to_string()]);
        assert!(!selection.components.expect("a set").chap_core.enabled);
        assert_eq!(app.counts().pending, 2);
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
        assert_eq!(app.port_input, "9000", "prefilled with what it publishes");

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
        ask_for_port(&mut app, "9000");
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

        // Turning chap-core off from the palette is refused just as `space` is.
        let state = state_with(&registry, EWARS, Some(Channel::Stable));
        let mut app = App::new(&registry, &state);
        app.reduce(Action::Palette);
        for c in "turn the chap-core".chars() {
            app.reduce(Action::PaletteChar(c));
        }
        app.reduce(Action::PaletteRun);
        assert!(app.components.chap_core.enabled);
        assert!(
            app.message
                .as_deref()
                .unwrap_or_default()
                .contains("chap-core cannot be disabled"),
            "{:?}",
            app.message
        );
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

    /// The dependency in the other direction, caught while the browser is
    /// still up: a session with chap-core switched off cannot enable a model,
    /// because the selection it would produce is one apply refuses.
    #[test]
    fn a_model_cannot_be_enabled_while_this_session_has_chap_core_off() {
        let registry = registry();
        let mut app = App::new(&registry, &empty_state());
        focus_component(&mut app, Component::ChapCore);
        app.reduce(Action::Toggle);
        assert!(!app.components.chap_core.enabled);

        app.page = Page::Models;
        focus(&mut app, EWARS);
        app.reduce(Action::Toggle);
        assert_eq!(
            app.message.as_deref(),
            Some(crate::components::MODELS_NEED_CHAP_CORE)
        );
        assert!(!app.selected().unwrap().enabled);
        assert!(app.selection().enable.is_empty());

        // Switching chap-core back on is what clears the way.
        focus_component(&mut app, Component::ChapCore);
        app.reduce(Action::Toggle);
        app.page = Page::Models;
        focus(&mut app, EWARS);
        app.reduce(Action::Toggle);
        assert!(app.selected().unwrap().enabled);
        let selection = app.selection();
        assert_eq!(selection.enable.len(), 1);
        assert!(
            selection.components.is_none(),
            "chap-core is back where the project had it, so the set says nothing"
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
            Action::OpenRepository,
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
}
