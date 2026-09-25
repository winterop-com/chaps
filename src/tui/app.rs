//! TUI state and the pure `App::reduce(Action) -> Option<Outcome>` reducer.
//!
//! Owned by agent C.
//!
//! Nothing here touches the terminal: the browser is a state machine that
//! takes [`Action`]s and ends by producing a
//! [`Selection`](crate::compose::Selection), which the caller applies through
//! [`crate::compose::apply`]. That is what makes the browser testable without
//! a tty.

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

/// The documentation the palette's "open the documentation" opens.
pub const DOCS_CHAPTER: &str = "models.html";

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
    TogglePublish,
    CycleChannel,
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
    /// Whether the row should publish a host port at all. Starts out matching
    /// [`Row::port`]; `p` toggles it, and saving turns the difference into a
    /// [`PortRequest`].
    pub publish: bool,
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
    Save,
    Discard,
    Refresh,
    Repository,
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
    pub show_templates: bool,
    /// `.chaps/models.yaml` as it was when the browser opened; the selection is the
    /// diff against this.
    pub initial: BTreeMap<String, EnabledModel>,
    pub dirty: bool,
    /// Transient footer note, cleared by the next action.
    pub message: Option<String>,
    /// First line of the details overlay that is on screen.
    pub info_scroll: usize,
    /// How far the overlay can scroll, which only the renderer knows because
    /// it depends on the terminal's size. The one thing drawing writes.
    pub info_max: Cell<usize>,
    pub palette_query: String,
    /// Index into [`App::palette_matches`].
    pub palette_cursor: usize,
    /// What the reducer wants the caller to do outside the terminal.
    pub effect: Option<Effect>,
}

impl<'a> App<'a> {
    /// Build the browser state from the catalogue and the project's state file.
    pub fn new(registry: &'a Registry, state: &ProjectState) -> App<'a> {
        let rows: Vec<Row> = registry
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
                    publish: port.is_some(),
                }
            })
            .collect();

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
            show_templates,
            initial: state.models.clone(),
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
            Mode::Info => self.reduce_info(action),
            Mode::Palette => self.reduce_palette(action),
            Mode::Browse => self.reduce_browse(action),
        }
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
            Action::OpenRepository => self.open_repository(),
            Action::ImageRef => self.show_image_ref(),
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
            CommandId::SetPort => self.set_publish(true),
            CommandId::RemovePort => self.set_publish(false),
            CommandId::SetChannel => self.cycle_channel(),
            CommandId::Templates => {
                self.show_templates = !self.show_templates;
                self.refilter();
            }
            CommandId::Filter => self.mode = Mode::Filter,
            CommandId::Save => return Some(Outcome::Save),
            CommandId::Discard => self.discard(),
            CommandId::Refresh => self.effect = Some(Effect::Refresh),
            CommandId::Repository => self.open_repository(),
            CommandId::Docs => {
                self.effect = Some(Effect::Open(format!(
                    "{}{DOCS_CHAPTER}",
                    crate::cli::DOCS_URL
                )))
            }
            CommandId::Help => self.mode = Mode::Help,
            CommandId::Quit => return self.quit(),
        }
        None
    }

    /// Every command the palette offers, in the order it lists them.
    ///
    /// The selected model is named where a command acts on it, so the palette
    /// reads as a sentence about what is under the cursor rather than as a
    /// menu of verbs.
    pub fn commands(&self) -> Vec<Command> {
        let name = self
            .selected()
            .map(|row| self.model(row).display_name.clone())
            .unwrap_or_else(|| "the selected model".to_string());
        let entry = |id, label: String, key, short| Command {
            id,
            label,
            key,
            short,
            hit: None,
        };
        vec![
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
                "p",
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
            entry(
                CommandId::Repository,
                format!("Open the repository of {name}"),
                "o",
                "Open repository",
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
        ]
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
            Action::Top => self.cursor = 0,
            Action::Bottom => self.cursor = self.visible.len().saturating_sub(1),
            Action::PageUp => self.move_by(-(PAGE_JUMP as isize)),
            Action::PageDown => self.move_by(PAGE_JUMP as isize),
            Action::Toggle => self.toggle(),
            Action::TogglePublish => self.toggle_publish(),
            Action::CycleChannel => self.cycle_channel(),
            Action::ToggleTemplates => {
                self.show_templates = !self.show_templates;
                self.refilter();
            }
            Action::StartFilter => self.mode = Mode::Filter,
            Action::ClearFilterOrQuit => {
                if self.filter.is_empty() {
                    return self.quit();
                }
                self.filter.clear();
                self.refilter();
            }
            Action::Info => {
                if self.selected().is_some() {
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
            Action::OpenRepository => self.open_repository(),
            Action::ImageRef => self.show_image_ref(),
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

    /// Put every row back where the project state had it.
    fn discard(&mut self) {
        for row in &mut self.rows {
            let recorded = self.initial.get(&self.registry.models[row.model_idx].id);
            row.enabled = recorded.is_some();
            row.channel = recorded.and_then(|e| e.channel).unwrap_or(Channel::Stable);
            row.port = recorded.and_then(|e| e.host_port);
            row.publish = row.port.is_some();
        }
        self.dirty = false;
        self.refilter();
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
                        let reach = if row.publish {
                            "with a host port"
                        } else {
                            "internal (no host port)"
                        };
                        changes.push(Change {
                            kind: ChangeKind::Add,
                            name,
                            detail: format!("enable at {version}, {reach}"),
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
                    match port_change(row.publish, previous.host_port) {
                        Some(PortRequest::None) => moved.push("stop publishing a host port".into()),
                        Some(_) => moved.push("publish a host port".into()),
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
    pub fn counts(&self) -> Counts {
        let selection = self.selection();
        Counts {
            total: self.rows.len(),
            enabled: self.rows.iter().filter(|r| r.enabled).count(),
            pending: selection.enable.len() + selection.disable.len(),
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
                        // if `p` was pressed on it too.
                        let port = row.publish.then_some(PortRequest::Auto);
                        selection.enable.push(request(model, row, port));
                    }
                }
                Some(previous) => {
                    if !row.enabled {
                        selection.disable.push(model.id.clone());
                        continue;
                    }
                    let channel_moved = row.channel != previous.channel.unwrap_or(Channel::Stable);
                    let port = port_change(row.publish, previous.host_port);
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
        selection
    }

    /// Whether the selection would change anything.
    pub fn has_changes(&self) -> bool {
        let selection = self.selection();
        !selection.enable.is_empty() || !selection.disable.is_empty()
    }

    fn toggle(&mut self) {
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
            row.publish = false;
        }
        if is_template && now_enabled {
            self.message = Some(TEMPLATE_WARNING.to_string());
        }
        self.dirty = self.has_changes();
    }

    /// Toggle whether the row under the cursor publishes a host port.
    ///
    /// The port itself is not chosen here: saving asks for
    /// [`PortRequest::Auto`], and the allocator picks the lowest one that is
    /// free both in the compose files and on this machine.
    fn toggle_publish(&mut self) {
        let Some(&row_idx) = self.visible.get(self.cursor) else {
            return;
        };
        let want = !self.rows[row_idx].publish;
        self.set_publish(want);
    }

    /// The same, said in one direction: what the palette's two port commands
    /// ask for, where "set a host port" must not take one away.
    fn set_publish(&mut self, publish: bool) {
        let Some(&row_idx) = self.visible.get(self.cursor) else {
            return;
        };
        if !self.rows[row_idx].enabled {
            self.message = Some(PUBLISH_NEEDS_ENABLED_HINT.to_string());
            return;
        }
        self.rows[row_idx].publish = publish;
        self.dirty = self.has_changes();
    }

    fn cycle_channel(&mut self) {
        let Some(&row_idx) = self.visible.get(self.cursor) else {
            return;
        };
        let row = &mut self.rows[row_idx];
        row.channel = match row.channel {
            Channel::Stable => Channel::Latest,
            Channel::Latest => Channel::Stable,
        };
        self.dirty = self.has_changes();
    }

    fn move_by(&mut self, delta: isize) {
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

/// The port request for a row whose publish flag may have moved, or `None`
/// when it still says what the project recorded.
fn port_change(publish: bool, recorded: Option<u16>) -> Option<PortRequest> {
    match (publish, recorded) {
        (true, None) => Some(PortRequest::Auto),
        (false, Some(_)) => Some(PortRequest::None),
        _ => None,
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
        assert!(!app.selected().unwrap().publish);
        assert!(!request.allow_template);
        assert_eq!(app.counts().pending, 1);
    }

    #[test]
    fn p_asks_for_an_automatic_port_and_takes_one_away_again() {
        let registry = registry();

        // A model that publishes nothing: `p` asks for a port.
        let state = state_with_port(&registry, EWARS, Some(Channel::Stable), None);
        let mut app = App::new(&registry, &state);
        focus(&mut app, EWARS);
        assert!(!app.selected().unwrap().publish);

        assert!(app.reduce(Action::TogglePublish).is_none());
        assert!(app.selected().unwrap().publish);
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

        // Pressing it again is back where we started, so nothing to apply.
        app.reduce(Action::TogglePublish);
        assert!(!app.has_changes());

        // A model that does publish one: `p` takes it away.
        let state = state_with_port(&registry, EWARS, Some(Channel::Stable), Some(5001));
        let mut app = App::new(&registry, &state);
        focus(&mut app, EWARS);
        assert!(app.selected().unwrap().publish);
        assert_eq!(app.selected().unwrap().port, Some(5001));

        app.reduce(Action::TogglePublish);
        let selection = app.selection();
        assert_eq!(selection.enable.len(), 1);
        assert_eq!(selection.enable[0].port, Some(PortRequest::None));
        assert!(selection.disable.is_empty());
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

        app.reduce(Action::TogglePublish);
        let selection = app.selection();
        assert_eq!(selection.enable.len(), 1);
        assert_eq!(selection.enable[0].port, Some(PortRequest::Auto));
        assert!(
            selection.enable[0].keep_version,
            "`p` asks for a port, not for an upgrade"
        );

        // Cycling the channel is a request about the version, port and all.
        app.reduce(Action::CycleChannel);
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
        app.reduce(Action::TogglePublish);
        assert!(!app.selection().enable[0].keep_version);
    }

    #[test]
    fn p_on_a_row_that_is_not_enabled_only_hints() {
        let registry = registry();
        let mut app = App::new(&registry, &empty_state());
        focus(&mut app, ARIMA);

        app.reduce(Action::TogglePublish);
        assert_eq!(app.message.as_deref(), Some(PUBLISH_NEEDS_ENABLED_HINT));
        assert!(!app.selected().unwrap().publish);
        assert!(!app.has_changes());

        // Enabled first, then published: one request carrying both.
        app.reduce(Action::Toggle);
        app.reduce(Action::TogglePublish);
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

        // Off, then on again: the model comes back internal rather than
        // silently re-publishing a port.
        app.reduce(Action::Toggle);
        assert!(!app.selected().unwrap().publish);
        app.reduce(Action::Toggle);
        assert!(app.selected().unwrap().enabled);
        assert!(!app.selected().unwrap().publish);
        let selection = app.selection();
        assert_eq!(selection.enable.len(), 1);
        assert_eq!(selection.enable[0].port, Some(PortRequest::None));
    }

    #[test]
    fn a_port_change_is_only_a_change_when_it_differs_from_the_record() {
        assert_eq!(port_change(true, None), Some(PortRequest::Auto));
        assert_eq!(port_change(false, Some(5001)), Some(PortRequest::None));
        assert_eq!(port_change(true, Some(5001)), None, "already published");
        assert_eq!(port_change(false, None), None, "already internal");
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
    fn cycling_the_channel_of_an_enabled_row_re_pins_it() {
        let registry = registry();
        let state = state_with(&registry, EWARS, Some(Channel::Stable));
        let mut app = App::new(&registry, &state);
        focus(&mut app, EWARS);

        app.reduce(Action::CycleChannel);
        assert_eq!(app.selected().unwrap().channel, Channel::Latest);
        let selection = app.selection();
        assert!(selection.disable.is_empty());
        assert_eq!(selection.enable.len(), 1);
        assert_eq!(selection.enable[0].id, EWARS);
        assert_eq!(
            selection.enable[0].selector,
            VersionSelector::Channel(Channel::Latest)
        );

        // Cycling back is once again a no-op.
        app.reduce(Action::CycleChannel);
        assert!(!app.has_changes());
    }

    #[test]
    fn cycling_a_row_nobody_enabled_changes_nothing() {
        let registry = registry();
        let mut app = App::new(&registry, &empty_state());
        focus(&mut app, ARIMA);
        app.reduce(Action::CycleChannel);
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
        app.reduce(Action::CycleChannel);
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
        assert_eq!(app.commands().len(), 13);

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

        // The two port commands are directional, so neither undoes the other.
        app.reduce(Action::PaletteRun);
        assert_eq!(app.mode, Mode::Browse);
        assert!(app.palette_query.is_empty());
        assert!(app.selected().unwrap().publish, "set a host port did that");

        app.reduce(Action::Palette);
        for c in "set a host".chars() {
            app.reduce(Action::PaletteChar(c));
        }
        app.reduce(Action::PaletteRun);
        assert!(app.selected().unwrap().publish, "and asking again keeps it");

        app.reduce(Action::Palette);
        for c in "remove the host".chars() {
            app.reduce(Action::PaletteChar(c));
        }
        app.reduce(Action::PaletteRun);
        assert!(!app.selected().unwrap().publish);
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
        app.reduce(Action::CycleChannel);
        assert_eq!(app.counts().pending, 2);

        app.reduce(Action::Discard);
        assert_eq!(app.message.as_deref(), Some(DISCARDED_HINT));
        assert!(!app.has_changes());
        assert!(!app.dirty);
        focus(&mut app, EWARS);
        assert!(app.selected().unwrap().enabled);
        assert_eq!(app.selected().unwrap().port, Some(5001));
        assert!(app.selected().unwrap().publish);
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
        app.reduce(Action::CycleChannel);
        app.reduce(Action::TogglePublish);

        let changes = app.changes();
        assert_eq!(changes.len(), 2);
        let added = changes
            .iter()
            .find(|c| c.kind == ChangeKind::Add)
            .expect("the new model is an addition");
        assert!(added.detail.starts_with("enable at "), "{added:?}");
        assert!(
            added.detail.ends_with("internal (no host port)"),
            "{added:?}"
        );

        let updated = changes
            .iter()
            .find(|c| c.kind == ChangeKind::Update)
            .expect("the re-pinned model is an update");
        assert_eq!(updated.name, "CHAP-EWARS");
        assert!(updated.detail.contains("follow latest"), "{updated:?}");
        assert!(
            updated.detail.contains("publish a host port"),
            "{updated:?}"
        );

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
