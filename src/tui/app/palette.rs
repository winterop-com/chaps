//! The command palette: what it offers, what its filter leaves, and running
//! the entry under its cursor.

use super::{
    App, COMPONENTS_DOCS_CHAPTER, Command, CommandId, DOCS_CHAPTER, Effect, Mode, Outcome, Page,
};
use crate::components::Component;

impl App<'_> {
    /// Run the command under the palette's cursor, then leave the palette.
    pub(super) fn run_command(&mut self) -> Option<Outcome> {
        let command = self.palette_matches().get(self.palette_cursor).cloned()?;
        self.close_palette();
        match command.id {
            CommandId::Toggle => self.toggle(),
            CommandId::SetPort => self.open_port_prompt(),
            CommandId::RemovePort => self.remove_port(),
            CommandId::SetChannel => self.open_channel_prompt(),
            CommandId::Configs => self.open_configs(),
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
                        CommandId::Configs,
                        format!("Show the configured models of {name} in chap-core"),
                        "m",
                        "Configs",
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
                "Save the changes; `varde up` applies them".to_string(),
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
                "Open the varde documentation".to_string(),
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
}
