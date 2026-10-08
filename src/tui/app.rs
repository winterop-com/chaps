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
//!
//! [`Selection`]: crate::compose::Selection

pub mod configs;
mod edit;
mod notes;
mod open;
mod palette;
mod reduce;
mod selection;

use crate::components::{Component, Components};
use crate::project::{EnabledModel, ProjectState};
use crate::registry::{Channel, Model, Registry, Version, VersionSelector};
pub use notes::*;
use std::cell::Cell;
use std::collections::BTreeMap;

/// Rows a page key moves the cursor by.
pub const PAGE_JUMP: usize = 10;

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
    /// The configured models of the model `m` was pressed on.
    Configs,
    /// The form `a` opens there.
    ConfigForm,
    /// The question before chap-core is asked to create or archive one.
    ConfigConfirm,
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
    /// Open the configured models of the model under the cursor.
    Configs,
    ConfigAdd,
    ConfigArchive,
    ConfigReload,
    FormChar(char),
    FormBackspace,
    FormSubmit,
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
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Hand this URL to the platform's opener.
    Open(String),
    /// Re-fetch the catalogue, the way `varde registry update` does.
    Refresh,
    /// Write a picture of the next frame to the working directory.
    ///
    /// The next one, not this one: the frame that is still on screen has the
    /// palette over it, and nobody wants a screenshot of the menu they used
    /// to take it.
    Screenshot,
    /// Ask chap-core for the configured models of a model.
    LoadConfigs { model: String, service: String },
    /// Create one, which the user has confirmed.
    CreateConfig {
        model: String,
        service: String,
        draft: crate::configs::Draft,
    },
    /// Archive one, which the user has confirmed.
    ArchiveConfig {
        model: String,
        service: String,
        id: i64,
        variant: String,
    },
}

/// One catalogue entry as the browser tracks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Index into [`Registry::models`].
    pub model_idx: usize,
    pub enabled: bool,
    /// Channel the row follows when enabled. `None` is a row the project
    /// pinned to one exact version (`models enable --version`): it keeps that
    /// version until the channel dialog picks a channel.
    pub channel: Option<Channel>,
    /// Host port from `.varde/models.yaml`, for rows that are already enabled
    /// and publish one.
    pub port: Option<u16>,
    /// The host port the row should end up with. Starts out matching
    /// [`Row::port`]; `p` edits it, and saving turns the difference into a
    /// [`PortRequest`](crate::compose::PortRequest).
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

/// One row of the components page, in the columns `varde components list`
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
    /// Open the configured models of the selected model.
    Configs,
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
    /// [`Selection::components`](crate::compose::Selection::components).
    pub components: Components,
    /// `.varde/components.yaml` as it was when the browser opened; the wanted
    /// set is the diff against this.
    pub initial_components: Components,
    /// Index into [`Component::ALL`], the components page's cursor.
    pub component_cursor: usize,
    pub show_templates: bool,
    /// `.varde/models.yaml` as it was when the browser opened; the selection is the
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
    /// The configured models page, while it is open.
    pub configs: Option<configs::View>,
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
                    channel: recorded_channel(enabled),
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
            configs: None,
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
            Mode::Configs => self.reduce_configs(action),
            Mode::ConfigForm => self.reduce_config_form(action),
            Mode::ConfigConfirm => self.reduce_config_confirm(action),
            Mode::Browse => self.reduce_browse(action),
        }
    }

    /// The component the components page's cursor is on.
    pub fn selected_component(&self) -> Component {
        Component::ALL[self.component_cursor.min(Component::ALL.len() - 1)]
    }

    /// Where a component is reached, as the REACH column prints it.
    ///
    /// The same three answers `varde components list` gives: its own host port,
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
    /// A pinned row resolves to its pin, yanked or not: that is what runs.
    pub fn resolved(&self, row: &Row) -> Option<&'a Version> {
        match row.channel {
            Some(channel) => self
                .model(row)
                .resolve(&VersionSelector::Channel(channel))
                .ok(),
            None => self
                .recorded(row)
                .and_then(|e| self.model(row).version(&e.version)),
        }
    }

    /// The version a pinned row is pinned to, or `None` when it follows a
    /// channel.
    pub fn pinned(&self, row: &Row) -> Option<&str> {
        match row.channel {
            Some(_) => None,
            None => self.recorded(row).map(|e| e.version.as_str()),
        }
    }

    /// What the project recorded for a row when the browser opened.
    pub fn recorded(&self, row: &Row) -> Option<&EnabledModel> {
        self.initial.get(&self.model(row).id)
    }
}

/// The channel a row starts on: the one the project recorded, `None` for an
/// exact pin, and stable for a model the project does not run.
pub(super) fn recorded_channel(recorded: Option<&EnabledModel>) -> Option<Channel> {
    match recorded {
        Some(e) => e.channel,
        None => Some(Channel::Stable),
    }
}

#[cfg(test)]
mod tests;
