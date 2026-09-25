//! The ratatui model browser.
//!
//! Owned by agent C. The TUI only produces a [`Selection`], applying it is
//! done by the caller through [`crate::compose::apply`], so the write path
//! stays shared with `init`, `enable` and `disable`.

pub mod app;
pub mod keys;
pub mod theme;
pub mod ui;

use crate::commands::Ctx;
use crate::compose::Selection;
use crate::error::Result;
use crate::project::Project;
use crate::registry::Registry;
use app::{App, Effect, Outcome};
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;

/// Run the browser. Returns `Ok(None)` when the user quits without saving.
///
/// The catalogue is owned here rather than borrowed for the whole run because
/// the palette can go back to the marketplace for a fresh one: a refresh
/// builds a new [`App`] over the new catalogue and puts the unsaved edits back
/// on the rows they belong to.
pub fn run_tui(ctx: &Ctx, project: &Project, registry: &Registry) -> Result<Option<Selection>> {
    let theme = theme::Theme::detect();
    let mut terminal = TerminalGuard::open()?;
    let mut current = registry.clone();
    let mut carry: Option<Carry> = None;

    loop {
        let mut app = App::new(&current, &project.state);
        if let Some(carry) = carry.take() {
            carry.restore(&mut app);
        }

        match event_loop(&mut terminal, &mut app, &theme)? {
            Exit::Save => return Ok(Some(app.selection())),
            Exit::Quit => return Ok(None),
            Exit::Refresh => {
                let mut kept = Carry::of(&app);
                // The fetch blocks, so say what is happening before it starts.
                app.message = Some("refreshing the registry ...".to_string());
                terminal.inner.draw(|frame| ui::draw(frame, &app, &theme))?;
                match crate::commands::refreshed_registry_for(ctx, Some(project)) {
                    Ok(fresh) => {
                        kept.message = Some(format!(
                            "registry refreshed: {} models from the marketplace",
                            fresh.models.len()
                        ));
                        current = fresh;
                    }
                    Err(err) => {
                        kept.message = Some(format!("could not refresh the registry: {err}"));
                    }
                }
                // A failed fetch may have warned on stderr, straight through
                // the frame; a clear makes the next draw a whole one.
                terminal.inner.clear()?;
                carry = Some(kept);
            }
        }
    }
}

/// How one pass over the browser ended.
enum Exit {
    Save,
    Quit,
    Refresh,
}

/// Draw, read a key, reduce, until something ends the pass.
fn event_loop(terminal: &mut TerminalGuard, app: &mut App, theme: &theme::Theme) -> Result<Exit> {
    loop {
        terminal.inner.draw(|frame| ui::draw(frame, app, theme))?;

        // Resizes and mouse events only mean "draw again"; key releases and
        // repeats would otherwise toggle a row twice on Windows.
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let outcome = app.reduce(keys::action_for(app.mode, &key));
        match app.take_effect() {
            Some(Effect::Refresh) => return Ok(Exit::Refresh),
            Some(Effect::Open(url)) => app.message = Some(open(&url)),
            None => {}
        }
        match outcome {
            Some(Outcome::Save) => return Ok(Exit::Save),
            Some(Outcome::Quit) => return Ok(Exit::Quit),
            None => {}
        }
    }
}

/// The unsaved edits and where the user was looking, so a refresh does not
/// throw them away.
struct Carry {
    /// Rows that differ from the project state, by model id.
    edited: Vec<(String, app::Row)>,
    cursor_id: Option<String>,
    filter: String,
    show_templates: bool,
    message: Option<String>,
}

impl Carry {
    fn of(app: &App) -> Carry {
        let edited = app
            .rows
            .iter()
            .filter(|row| {
                let recorded = app.recorded(row);
                recorded.is_some() != row.enabled
                    || recorded.is_some_and(|r| {
                        r.host_port.map(app::PortWant::Exact).unwrap_or_default() != row.want
                            || r.channel.unwrap_or(crate::registry::Channel::Stable) != row.channel
                    })
            })
            .map(|row| (app.model(row).id.clone(), row.clone()))
            .collect();
        Carry {
            edited,
            cursor_id: app.selected().map(|row| app.model(row).id.clone()),
            filter: app.filter.clone(),
            show_templates: app.show_templates,
            message: app.message.clone(),
        }
    }

    /// Put what survives the new catalogue back: an id it no longer lists is
    /// dropped, because there is nothing left to enable.
    fn restore(self, app: &mut App) {
        for (id, edited) in self.edited {
            if let Some(idx) = app.rows.iter().position(|row| app.model(row).id == id) {
                let model_idx = app.rows[idx].model_idx;
                app.rows[idx] = app::Row {
                    model_idx,
                    ..edited
                };
            }
        }
        app.show_templates = self.show_templates;
        app.filter = self.filter;
        app.message = self.message;
        app.dirty = app.has_changes();
        app.refilter();
        if let Some(id) = self.cursor_id
            && let Some(position) = app
                .visible
                .iter()
                .position(|row_idx| app.model(&app.rows[*row_idx]).id == id)
        {
            app.cursor = position;
        }
    }
}

/// Hand a URL to whatever this platform opens URLs with.
///
/// Best effort on purpose: there is no dependency to do it properly, the
/// browser must not block on the child, and a machine with no opener at all
/// (a server over ssh, which is where chaps mostly runs) has to hear the URL
/// instead of nothing.
fn open(url: &str) -> String {
    let (command, args) = opener();

    let spawned = std::process::Command::new(command)
        .args(args)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match spawned {
        Ok(_) => format!("opening {url}"),
        Err(_) => format!("no `{command}` on this machine: {url}"),
    }
}

/// The command this platform opens URLs with, and the arguments before the
/// URL itself.
fn opener() -> (&'static str, &'static [&'static str]) {
    if cfg!(target_os = "macos") {
        ("open", &[])
    } else if cfg!(target_os = "windows") {
        // The empty argument is `start`'s window title, which it would
        // otherwise read the URL as.
        ("cmd", &["/C", "start", ""])
    } else {
        ("xdg-open", &[])
    }
}

/// Owns the terminal so that every exit path puts it back: an early `?`, a
/// normal return, or a panic while drawing.
struct TerminalGuard {
    inner: DefaultTerminal,
}

impl TerminalGuard {
    fn open() -> Result<TerminalGuard> {
        // `try_init` also installs a panic hook that restores the terminal, so
        // a panic inside a widget does not leave the shell in raw mode.
        let inner = ratatui::try_init().map_err(|e| {
            anyhow::anyhow!(
                "could not take over the terminal: {e}. \
                 The model browser needs an interactive terminal; \
                 use `chaps models enable` and `chaps models disable` in a script."
            )
        })?;
        Ok(TerminalGuard { inner })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectState;
    use crate::registry::load_embedded;
    use crate::tui::app::Action;

    /// A refresh keeps the unsaved edits, the filter and the cursor, because
    /// re-fetching a catalogue is not a reason to lose what was toggled.
    #[test]
    fn what_is_carried_over_a_refresh_survives_a_new_catalogue() {
        let registry = load_embedded().expect("embedded snapshot parses");
        let mut app = App::new(&registry, &ProjectState::default());
        app.reduce(Action::ToggleTemplates);
        app.reduce(Action::Toggle);
        let toggled = app.selected_model().expect("a row is selected").id.clone();
        let carry = Carry::of(&app);
        assert_eq!(carry.edited.len(), 1);
        assert_eq!(carry.edited[0].0, toggled);

        // A fresh browser over the same catalogue: the edit comes back.
        let mut fresh = App::new(&registry, &ProjectState::default());
        assert!(!fresh.has_changes());
        carry.restore(&mut fresh);
        assert!(fresh.has_changes());
        assert!(fresh.show_templates);
        assert_eq!(fresh.selection().enable[0].id, toggled);
        assert_eq!(
            fresh.selected_model().map(|m| m.id.clone()),
            Some(toggled),
            "the cursor stays on the row it was on"
        );
    }

    /// Spawning the opener is not tested - it would open a browser on the
    /// machine running the suite - but the command it would spawn is.
    #[test]
    fn every_platform_names_an_opener() {
        let (command, args) = opener();
        assert!(!command.is_empty());
        assert!(args.iter().all(|arg| !arg.contains("://")));
    }
}
