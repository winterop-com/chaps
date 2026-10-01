//! Editing a row: toggling it, its host port, its channel, and the DHIS2
//! version, with the prompts that ask for them.

use super::{
    App, CHANNELS, COMPONENT_HAS_NO_AUTO_PORT, CORE_PORT_IS_API_PORT, Mode,
    PORT_NEEDS_COMPONENT_HINT, PUBLISH_NEEDS_ENABLED_HINT, Page, PortWant, TEMPLATE_HIDDEN_HINT,
    TEMPLATE_WARNING,
};
use crate::components::Component;
use crate::registry::Channel;

impl App<'_> {
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
    pub(super) fn toggle_component(&mut self, component: Component) {
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

    pub(super) fn toggle(&mut self) {
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
    pub(super) fn open_port_prompt(&mut self) {
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
    pub(super) fn apply_port_prompt(&mut self) {
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
    pub(super) fn remove_port(&mut self) {
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
    pub(super) fn open_channel_prompt(&mut self) {
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
    pub(super) fn open_dhis2_version_prompt(&mut self) {
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
    pub(super) fn set_dhis2_version(&mut self, tag: String) {
        let recorded = self.initial_components.dhis2.image_tag.clone();
        self.components.dhis2.image_tag = tag.clone();
        self.message =
            (tag != recorded).then(|| crate::components::dhis2_tag_change_note(&recorded, &tag));
        self.dirty = self.has_changes();
    }

    pub(super) fn set_channel(&mut self, channel: Channel) {
        let Some(&row_idx) = self.visible.get(self.cursor) else {
            return;
        };
        self.rows[row_idx].channel = channel;
        self.dirty = self.has_changes();
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

/// What the port prompt accepts: a number in the project's range, `auto`, or
/// nothing at all.
///
/// The reasons are the ones `chaps models expose` gives, minus the two only a
/// save can answer - a port another compose file claims, and a port something
/// on this machine is listening on - which [`crate::compose::apply()`] checks
/// when the selection is applied.
pub(super) fn parse_port(
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
pub(super) fn parse_component_port(
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
