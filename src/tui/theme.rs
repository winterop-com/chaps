//! The browser's palette.
//!
//! Every colour the TUI draws comes from here, and every one of them is an
//! ANSI 16 colour rather than an RGB value: the terminal's own palette then
//! decides what "green" looks like, so the browser follows a light theme and a
//! dark theme without knowing which one it is in.
//!
//! `NO_COLOR` switches the whole thing to [`Theme::monochrome`], where bold,
//! dim and reverse carry the meaning instead.

use crate::registry::{AssessedStatus, VersionStatus};
use ratatui::style::{Color, Modifier, Style};

/// The colours the browser draws with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Titles, key letters, focus: the one colour that means "this is varde".
    pub accent: Color,
    /// Enabled, registered, verified.
    pub ok: Color,
    /// Pending, unstable, needs a look.
    pub warn: Color,
    /// Failed, yanked, off.
    pub bad: Color,
    /// Labels, ids, everything already known.
    pub dim: Color,
    /// The `[template]` marker, which is neither good nor bad, just different.
    pub template: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    /// No colour at all: shape, bold and reverse have to do the work.
    pub mono: bool,
}

impl Default for Theme {
    fn default() -> Theme {
        Theme {
            accent: Color::Cyan,
            ok: Color::Green,
            warn: Color::Yellow,
            bad: Color::Red,
            dim: Color::DarkGray,
            template: Color::Magenta,
            selection_bg: Color::Blue,
            selection_fg: Color::White,
            mono: false,
        }
    }
}

impl Theme {
    /// The palette for this terminal: the default one, or the monochrome one
    /// when `NO_COLOR` is set.
    pub fn detect() -> Theme {
        if std::env::var_os("NO_COLOR").is_some() {
            Theme::monochrome()
        } else {
            Theme::default()
        }
    }

    /// Every colour reset, so nothing but weight and reverse video is left.
    pub fn monochrome() -> Theme {
        Theme {
            accent: Color::Reset,
            ok: Color::Reset,
            warn: Color::Reset,
            bad: Color::Reset,
            dim: Color::Reset,
            template: Color::Reset,
            selection_bg: Color::Reset,
            selection_fg: Color::Reset,
            mono: true,
        }
    }

    /// What a picture of the browser is painted on.
    ///
    /// The TUI paints no background of its own - it inherits the terminal's -
    /// so [`crate::tui::screenshot`] has to name one, and an ANSI 16 palette
    /// is drawn to be read on a dark ground.
    pub fn background(&self) -> Color {
        Color::Rgb(0x14, 0x16, 0x18)
    }

    /// What a cell with no colour of its own is drawn in, in that picture.
    pub fn foreground(&self) -> Color {
        Color::Rgb(0xd8, 0xdc, 0xe0)
    }

    /// A foreground colour, or the weight that stands in for it without one.
    fn fg(&self, colour: Color, mono: Modifier) -> Style {
        if self.mono {
            Style::default().add_modifier(mono)
        } else {
            Style::default().fg(colour)
        }
    }

    /// Titles, focused borders, key letters.
    pub fn accent_style(&self) -> Style {
        self.fg(self.accent, Modifier::BOLD)
    }

    pub fn ok_style(&self) -> Style {
        self.fg(self.ok, Modifier::BOLD)
    }

    pub fn warn_style(&self) -> Style {
        self.fg(self.warn, Modifier::BOLD)
    }

    pub fn bad_style(&self) -> Style {
        self.fg(self.bad, Modifier::BOLD | Modifier::UNDERLINED)
    }

    pub fn dim_style(&self) -> Style {
        // Dim is dim in both worlds; it is the one signal that survives a
        // monochrome terminal unchanged.
        if self.mono {
            Style::default().add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(self.dim)
        }
    }

    pub fn template_style(&self) -> Style {
        self.fg(self.template, Modifier::ITALIC)
    }

    /// A label in front of a value.
    pub fn label_style(&self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    /// A column heading: quiet, but not so quiet it reads as data.
    pub fn column_style(&self) -> Style {
        self.dim_style().add_modifier(Modifier::BOLD)
    }

    /// A filled chip: the one thing on the key bar that writes to disk, when
    /// there is something to write. Reverse video stands in for the fill when
    /// there is no colour to fill it with.
    pub fn chip_style(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .bg(self.warn)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        }
    }

    /// The row under the cursor.
    pub fn selection_style(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
                .bg(self.selection_bg)
                .fg(self.selection_fg)
                .add_modifier(Modifier::BOLD)
        }
    }

    /// A pane border: the focused pane gets the accent, the other one fades.
    pub fn border_style(&self, focused: bool) -> Style {
        if focused {
            self.accent_style()
        } else {
            self.dim_style()
        }
    }

    /// A pane title, which follows its border.
    pub fn pane_title_style(&self, focused: bool) -> Style {
        self.border_style(focused).add_modifier(Modifier::BOLD)
    }

    /// A marketplace assessment.
    pub fn status_style(&self, status: AssessedStatus) -> Style {
        match status {
            AssessedStatus::Green => self.ok_style(),
            // There is no orange in a 16-colour palette, and "look before you
            // run it" is the same message yellow already carries.
            AssessedStatus::Yellow | AssessedStatus::Orange => self.warn_style(),
            AssessedStatus::Red => self.bad_style(),
            AssessedStatus::Gray => self.dim_style(),
        }
    }

    /// One version's status.
    pub fn version_style(&self, status: VersionStatus) -> Style {
        match status {
            VersionStatus::Verified => self.ok_style(),
            VersionStatus::Unstable | VersionStatus::Deprecated => self.warn_style(),
            VersionStatus::Yanked => self.bad_style().add_modifier(Modifier::CROSSED_OUT),
        }
    }
}

#[cfg(test)]
mod tests;
