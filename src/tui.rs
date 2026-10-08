//! The ratatui browser: the marketplace on one page, the components this
//! deployment is made of on the other.
//!
//! The TUI only produces a [`Selection`], applying it is done by the caller
//! through [`crate::compose::apply()`], so the write path stays shared with
//! `init`, `enable` and `disable` - the component set rides on the same
//! selection, which is why one `s` writes both pages.

pub mod app;
mod chap;
pub mod form;
pub mod keys;
pub mod screenshot;
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
        // The port compose publishes, which `.env` can move away from the
        // recorded one; the page shows it and the port prompt keeps off it.
        app.api_port = project.effective_api_port();
        if let Some(carry) = carry.take() {
            carry.restore(&mut app);
        }

        match event_loop(&mut terminal, &mut app, &theme, project, &current)? {
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

/// Run the configured model form alone, for `varde models configs add` and
/// `update` at a terminal. Enter hands the form to `check`, which gives the
/// draft or the reason the form shows; Esc and ctrl-c give `None`.
pub fn run_form(
    model: &str,
    mut form: form::Form,
    mut check: impl FnMut(&form::Form) -> std::result::Result<crate::configs::Draft, String>,
) -> Result<Option<crate::configs::Draft>> {
    let theme = theme::Theme::detect();
    let mut terminal = TerminalGuard::open()?;
    loop {
        terminal
            .inner
            .draw(|frame| ui::draw_form(frame, model, &form, &theme))?;
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match form.apply(keys::action_for(app::Mode::ConfigForm, &key)) {
            form::Event::Edited => {}
            form::Event::Cancel => return Ok(None),
            form::Event::Submit => match check(&form) {
                Ok(draft) => return Ok(Some(draft)),
                Err(why) => form.error = Some(why),
            },
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
fn event_loop(
    terminal: &mut TerminalGuard,
    app: &mut App,
    theme: &theme::Theme,
    project: &Project,
    registry: &Registry,
) -> Result<Exit> {
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
            Some(Effect::Open(url)) => app.message = Some(crate::open::launch(&url)),
            Some(Effect::Screenshot) => {
                // Draw the frame the palette is no longer on, and shoot that
                // one; the message about the file lands on the frame after.
                let saved = {
                    let frame = terminal.inner.draw(|frame| ui::draw(frame, app, theme))?;
                    screenshot::save(frame.buffer, theme)
                };
                app.message = Some(saved);
            }
            Some(Effect::LoadConfigs { model, service }) => {
                // The request blocks: draw the page that says it is out.
                terminal.inner.draw(|frame| ui::draw(frame, app, theme))?;
                app.configs_loaded(chap::load(project, registry, &model, &service));
            }
            Some(Effect::CreateConfig {
                model,
                service,
                draft,
            }) => {
                terminal.inner.draw(|frame| ui::draw(frame, app, theme))?;
                let (message, load) = chap::create(project, registry, &model, &service, &draft);
                app.configs_done(message, load);
            }
            Some(Effect::UpdateConfig {
                model,
                service,
                old,
                draft,
            }) => {
                terminal.inner.draw(|frame| ui::draw(frame, app, theme))?;
                let (message, load) =
                    chap::update(project, registry, &model, &service, old, &draft);
                app.configs_done(message, load);
            }
            Some(Effect::ArchiveConfig {
                model,
                service,
                id,
                variant,
            }) => {
                terminal.inner.draw(|frame| ui::draw(frame, app, theme))?;
                let (message, load) =
                    chap::archive(project, registry, &model, &service, id, &variant);
                app.configs_done(message, load);
            }
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
    /// The component set the session wants. A new catalogue says nothing about
    /// components, so it comes back whole - and a refresh that dropped it would
    /// throw away edits the header was still counting.
    components: crate::components::Components,
    page: app::Page,
    component_cursor: usize,
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
                            || r.channel != row.channel
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
            components: app.components.clone(),
            page: app.page,
            component_cursor: app.component_cursor,
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
        app.components = self.components;
        app.page = self.page;
        app.component_cursor = self.component_cursor;
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
                 use `varde models enable` and `varde models disable` in a script."
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
mod tests;
