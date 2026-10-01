//! The reducer's per-mode halves, and moving about the list.

use super::{
    Action, App, CHANNELS, DISCARDED_HINT, Mode, NOTHING_TO_DISCARD_HINT, Outcome, PAGE_JUMP, Page,
    PortWant,
};
use crate::components::Component;
use crate::registry::{Channel, Model};

impl App<'_> {
    /// The port prompt: one line of text, taken by Enter and dropped by Esc.
    /// A refusal keeps the prompt up with the reason on it, so a mistyped
    /// port is corrected rather than typed again from nothing.
    pub(super) fn reduce_port(&mut self, action: Action) -> Option<Outcome> {
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
    pub(super) fn reduce_channel(&mut self, action: Action) -> Option<Outcome> {
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
    pub(super) fn reduce_dhis2_version(&mut self, action: Action) -> Option<Outcome> {
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
    pub(super) fn reduce_info(&mut self, action: Action) -> Option<Outcome> {
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
    pub(super) fn reduce_palette(&mut self, action: Action) -> Option<Outcome> {
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

    pub(super) fn close_palette(&mut self) {
        self.mode = Mode::Browse;
        self.palette_query.clear();
        self.palette_cursor = 0;
    }

    pub(super) fn reduce_help(&mut self, action: Action) -> Option<Outcome> {
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

    pub(super) fn reduce_confirm(&mut self, action: Action) -> Option<Outcome> {
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

    pub(super) fn reduce_filter(&mut self, action: Action) -> Option<Outcome> {
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

    pub(super) fn reduce_browse(&mut self, action: Action) -> Option<Outcome> {
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
    pub(super) fn quit(&mut self) -> Option<Outcome> {
        if self.dirty {
            self.mode = Mode::ConfirmQuit;
            None
        } else {
            Some(Outcome::Quit)
        }
    }

    /// Put every row back where the project state had it, on both pages: `u`
    /// means "nothing I did this session", not "nothing on this page".
    pub(super) fn discard(&mut self) {
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
    pub(super) fn turn_page(&mut self, delta: isize) {
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
