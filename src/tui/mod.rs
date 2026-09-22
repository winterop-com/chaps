//! The ratatui model browser.
//!
//! Owned by agent C. The TUI only produces a [`Selection`], applying it is
//! done by the caller through [`crate::compose::apply`], so the write path
//! stays shared with `init`, `enable` and `disable`.

pub mod app;
pub mod keys;
pub mod ui;

use crate::commands::Ctx;
use crate::compose::Selection;
use crate::error::Result;
use crate::project::Project;
use crate::registry::Registry;
use app::{App, Outcome};
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;

/// Run the browser. Returns `Ok(None)` when the user quits without saving.
pub fn run_tui(_ctx: &Ctx, project: &Project, registry: &Registry) -> Result<Option<Selection>> {
    let mut app = App::new(registry, &project.state);
    let mut terminal = TerminalGuard::open()?;

    loop {
        terminal.inner.draw(|frame| ui::draw(frame, &app))?;

        // Resizes and mouse events only mean "draw again"; key releases and
        // repeats would otherwise toggle a row twice on Windows.
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match app.reduce(keys::action_for(app.mode, &key)) {
            Some(Outcome::Save) => return Ok(Some(app.selection())),
            Some(Outcome::Quit) => return Ok(None),
            None => {}
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
