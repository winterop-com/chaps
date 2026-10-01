//! What saving would do: the pending changes the strip lists, the counters,
//! and the [`Selection`] handed to [`crate::compose::apply()`].

use super::{App, Change, ChangeKind, Counts, PortWant, Row};
use crate::components::Component;
use crate::compose::{EnableRequest, PortRequest, Selection};
use crate::registry::{Channel, Model, VersionSelector};

impl App<'_> {
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
pub(super) fn port_change(want: PortWant, recorded: Option<u16>) -> Option<PortRequest> {
    match (want, recorded) {
        (PortWant::None, None) => None,
        (PortWant::None, Some(_)) => Some(PortRequest::None),
        (PortWant::Auto, _) => Some(PortRequest::Auto),
        // The port it already has is not a change; any other one is.
        (PortWant::Exact(port), Some(had)) if port == had => None,
        (PortWant::Exact(port), _) => Some(PortRequest::Fixed(port)),
    }
}

/// What a component's removal says about the data it leaves behind.
///
/// Every volume is named, because every one of them is a name the operator
/// would have to type to remove it, and a component with two volumes that only
/// owned up to one would be the change strip understating what stays on the
/// disk. Nothing to name at all is chap-core, whose volumes are upstream's.
pub(super) fn disable_detail(volumes: &[&str]) -> String {
    match volumes {
        [] => "disable · its own volumes are kept".to_string(),
        [volume] => format!("disable · the {volume} volume is kept"),
        volumes => format!("disable · the {} volumes are kept", volumes.join(", ")),
    }
}
