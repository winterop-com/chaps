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

use crate::components::{Component, Components};
use crate::compose::{EnableRequest, PortRequest, Selection};
use crate::open::Openable;
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

/// Footer note shown when `o` is pressed on a component this session has only
/// just enabled.
///
/// The address is a plan until the compose file exists and something is running
/// behind it, so the browser says what is missing rather than opening a port
/// nothing is on yet.
pub const OPEN_NEEDS_SAVING: &str = "press s to apply the change first, then `chaps up` starts it";

/// Footer note shown when `o` is pressed on a component this deployment does
/// not have.
pub const OPEN_NEEDS_COMPONENT: &str = "enable the component first (space), then save with s";

/// Footer note shown when `p` is pressed on `chap-core`.
///
/// chap-core's host port is the API port, which lives in `project.yaml` rather
/// than in the component block, so this page is not where it is edited.
pub const CORE_PORT_IS_API_PORT: &str =
    "chap-core's host port is the API port; set CHAP_API_PORT in `.env`";

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
    /// The dialog `v` opens on the `dhis2` component, asking which DHIS2
    /// version to run.
    Dhis2Version,
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
    /// Hand what the row under the cursor points at to the platform opener: a
    /// model's repository, or a component's web interface.
    Open,
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
    /// Open the selected component's web interface.
    Web,
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
    /// The DHIS2 versions the version dialog offers, and its cursor.
    pub dhis2_versions: Vec<String>,
    pub dhis2_version_cursor: usize,
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
            dhis2_versions: Vec::new(),
            dhis2_version_cursor: 0,
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
            Mode::Dhis2Version => self.reduce_dhis2_version(action),
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

    /// The DHIS2 version dialog: move, pick, or leave it.
    fn reduce_dhis2_version(&mut self, action: Action) -> Option<Outcome> {
        let last = self.dhis2_versions.len().saturating_sub(1);
        match action {
            Action::Down => self.dhis2_version_cursor = (self.dhis2_version_cursor + 1).min(last),
            Action::Up => self.dhis2_version_cursor = self.dhis2_version_cursor.saturating_sub(1),
            Action::ChannelApply => {
                self.mode = Mode::Browse;
                if let Some(tag) = self.dhis2_versions.get(self.dhis2_version_cursor).cloned() {
                    self.set_dhis2_version(tag);
                }
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
            Action::Open => self.open_selection(),
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
            CommandId::Web => self.open_component(),
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
                    entry(
                        CommandId::Web,
                        format!("Open the web interface of the {name} component"),
                        "o",
                        "Open web interface",
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
            // DHIS2 is the one component with versions to choose between.
            Action::ChannelPrompt if self.selected_component() == Component::Dhis2 => {
                self.open_dhis2_version_prompt()
            }
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
            // `o` means the same thing on both pages - hand what this row
            // points at to a browser - and each page says what that is. An
            // image reference belongs to a marketplace model alone.
            Action::Open => self.open_selection(),
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
    /// rather than in the component block, and OCS and DHIS2 answer through
    /// [`Components::ocs_reach`] and [`Components::dhis2_reach`], so the browser
    /// and `components list` cannot word the same state differently.
    ///
    /// [`Components::ocs_reach`]: crate::components::Components::ocs_reach
    /// [`Components::dhis2_reach`]: crate::components::Components::dhis2_reach
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
            Component::Dhis2 => self.components.dhis2_reach(),
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
            Component::Dhis2 => self.components.dhis2.port,
        }
    }

    fn set_component_port(&mut self, component: Component, port: Option<u16>) {
        match component {
            Component::ChapCore => {}
            Component::Ocs => self.components.ocs.port = port,
            Component::S3 => self.components.s3.port = port,
            Component::Dhis2 => self.components.dhis2.port = port,
        }
    }

    /// Turn one component on or off, or say why it cannot.
    ///
    /// chap-core can go off with models enabled: they keep running on their
    /// own, and the save publishes a host port for each one that had none.
    fn toggle_component(&mut self, component: Component) {
        let wanted = !self.components.is_enabled(component);
        if wanted
            && component == Component::Dhis2
            && let Some(external) = &self.components.dhis2_external
        {
            self.message = Some(crate::components::dhis2_external_refusal(&external.url));
            return;
        }
        // The port it publishes is left alone, so a component switched off and
        // on again in one session comes back exactly as the project has it.
        self.components.set_enabled(component, wanted);
        self.dirty = self.has_changes();
    }

    /// Hand what the row under the cursor points at to a browser: the page
    /// decides whether that is a repository or a web interface.
    fn open_selection(&mut self) {
        match self.page {
            Page::Models => self.open_repository(),
            Page::Components => self.open_component(),
        }
    }

    /// Ask the caller to open the selected model's repository.
    fn open_repository(&mut self) {
        let Some(row) = self.selected() else {
            return;
        };
        let url = self.model(row).source.repository.clone();
        self.effect = Some(Effect::Open(url));
    }

    /// Ask the caller to open the selected component's web interface, or say in
    /// the footer why there is none to open.
    ///
    /// Resolved against the *recorded* set, not the session's: what a browser
    /// can reach is what this deployment publishes now, and a row toggled or
    /// re-ported in this session is a plan until `s` has written it and
    /// `chaps up` has applied it. So a pending change is said rather than
    /// opened, and a pending port change opens the port that is actually
    /// published - which is the one the footer then names.
    fn open_component(&mut self) {
        let component = self.selected_component();
        let recorded = crate::open::resolve(component, &self.initial_components, &self.api_base());
        // A component this session turned on has no compose file yet, so its
        // address belongs to nothing: that is a different answer from a
        // component the deployment simply does not have.
        if recorded == Openable::Off && self.components.is_enabled(component) {
            self.message = Some(OPEN_NEEDS_SAVING.to_string());
            return;
        }
        match recorded {
            Openable::Url { url, .. } => self.effect = Some(Effect::Open(url)),
            Openable::Internal { inside } => {
                self.message = Some(format!(
                    "{} publishes no host port; it is reached at {inside} inside the deployment, \
                     and p publishes one",
                    component.name()
                ))
            }
            Openable::NoWeb => self.message = Some(crate::open::NO_WEB_INTERFACE.to_string()),
            Openable::Off => self.message = Some(OPEN_NEEDS_COMPONENT.to_string()),
        }
    }

    /// Where chap-core's API is reached from this machine, as this browser knows
    /// it.
    ///
    /// The recorded API port, which is exactly what the REACH column prints:
    /// the browser reads no `.env`, so a `CHAP_API_PORT` or `CHAP_ROOT_PATH`
    /// written there is `chaps open`'s to honour and not this page's.
    fn api_base(&self) -> String {
        format!("http://localhost:{}", self.api_port)
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
                    detail: disable_detail(component.volumes()),
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

    /// Open the version dialog on the `dhis2` component: the minor lines
    /// chaps has a seed for, plus whatever the deployment runs now, with the
    /// cursor on the one in force.
    fn open_dhis2_version_prompt(&mut self) {
        if !self.components.dhis2.enabled {
            self.message = Some(DHIS2_VERSION_NEEDS_ENABLED.to_string());
            return;
        }
        let current = self.components.dhis2.image_tag.clone();
        self.dhis2_versions = dhis2_versions(&current);
        self.dhis2_version_cursor = self
            .dhis2_versions
            .iter()
            .position(|v| *v == current)
            .unwrap_or_default();
        self.mode = Mode::Dhis2Version;
    }

    /// Move DHIS2 to `tag`, saying before the save that DHIS2 migrates a
    /// database forward only when the tag leaves what the project runs.
    fn set_dhis2_version(&mut self, tag: String) {
        let recorded = self.initial_components.dhis2.image_tag.clone();
        self.components.dhis2.image_tag = tag.clone();
        self.message =
            (tag != recorded).then(|| crate::components::dhis2_tag_change_note(&recorded, &tag));
        self.dirty = self.has_changes();
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

/// What `v` on a `dhis2` component that is off says.
const DHIS2_VERSION_NEEDS_ENABLED: &str = "turn dhis2 on first (space), then pick its version";

/// The DHIS2 versions the dialog offers: the ones chaps knows, newest first,
/// and `current` too when it is none of them.
pub fn dhis2_versions(current: &str) -> Vec<String> {
    let mut versions: Vec<String> = crate::components::DHIS2_VERSIONS
        .iter()
        .map(|v| v.to_string())
        .collect();
    if !versions.iter().any(|v| v == current) {
        versions.insert(0, current.to_string());
    }
    versions
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

/// What a component's removal says about the data it leaves behind.
///
/// Every volume is named, because every one of them is a name the operator
/// would have to type to remove it, and a component with two volumes that only
/// owned up to one would be the change strip understating what stays on the
/// disk. Nothing to name at all is chap-core, whose volumes are upstream's.
fn disable_detail(volumes: &[&str]) -> String {
    match volumes {
        [] => "disable · its own volumes are kept".to_string(),
        [volume] => format!("disable · the {volume} volume is kept"),
        volumes => format!("disable · the {} volumes are kept", volumes.join(", ")),
    }
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
mod tests;
