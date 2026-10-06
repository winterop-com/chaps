//! A picture of the browser, taken the way Textual takes one: the buffer that
//! was just drawn, written out as SVG.
//!
//! ratatui has no screenshot of its own and varde takes no dependency for
//! one: a terminal cell is a rectangle and a glyph, which is two SVG
//! elements. What lands in the file is what the terminal showed, cell for
//! cell, so the picture cannot drift from the screen it claims to be.
//!
//! The colours are the ones in the buffer. The browser draws in the sixteen
//! colours the terminal owns, and an SVG has no terminal to ask, so this is
//! the one place in the TUI that names a hex value: the Tango palette, on a
//! dark ground, which is what an ANSI 16 palette is drawn to be read on.

use crate::tui::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// One cell, in SVG user units: about what a 14px monospace glyph occupies.
const CELL_W: f32 = 8.4;
const CELL_H: f32 = 18.0;
/// Where the glyphs sit inside their row.
const BASELINE: f32 = 13.5;
const FONT_SIZE: f32 = 14.0;
/// No quotes in here: it goes straight into an attribute.
const FONT: &str = "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace";

/// The sixteen colours a terminal owns, as the Tango palette renders them.
const ANSI: [&str; 16] = [
    "#2e3436", "#cc0000", "#4e9a06", "#c4a000", "#3465a4", "#75507b", "#06989a", "#d3d7cf",
    "#555753", "#ef2929", "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#eeeeec",
];

/// Write a picture of `buffer` to the directory `varde ui` was started in,
/// and say what happened.
///
/// No directory is created and nothing is overwritten by name: the file is
/// named after the second it was taken in.
pub fn save(buffer: &Buffer, theme: &Theme) -> String {
    save_in(Path::new("."), buffer, theme, SystemTime::now())
}

/// [`save`], with the directory and the clock handed in, so a test can watch
/// it write a real file without moving the process into a temporary one.
fn save_in(dir: &Path, buffer: &Buffer, theme: &Theme, now: SystemTime) -> String {
    let name = file_name(now);
    match std::fs::write(dir.join(&name), svg(buffer, theme)) {
        Ok(()) => format!("saved {name}"),
        Err(err) => format!("could not save `{name}`: {err}"),
    }
}

/// `varde-ui-20260925-143012.svg`.
fn file_name(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default();
    format!("varde-ui-{}.svg", stamp(secs))
}

/// The whole buffer as one SVG document.
pub fn svg(buffer: &Buffer, theme: &Theme) -> String {
    let ground = hex(theme.background(), "#141618");
    let ink = hex(theme.foreground(), "#d8dce0");
    let width = buffer.area.width as f32 * CELL_W;
    let height = buffer.area.height as f32 * CELL_H;

    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" \
         viewBox=\"0 0 {} {}\" font-family=\"{FONT}\" font-size=\"{FONT_SIZE}\">\n",
        round(width),
        round(height),
        round(width),
        round(height)
    ));
    out.push_str(&format!(
        "<rect width=\"{}\" height=\"{}\" fill=\"{ground}\"/>\n",
        round(width),
        round(height)
    ));

    for y in 0..buffer.area.height {
        let row = row_of(buffer, y, &ground, &ink);
        for run in runs(&row, |cell| cell.bg.clone()) {
            if row[run.0].bg == ground {
                continue;
            }
            out.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>\n",
                round(run.0 as f32 * CELL_W),
                round(y as f32 * CELL_H),
                round((run.1 - run.0) as f32 * CELL_W),
                round(CELL_H),
                row[run.0].bg
            ));
        }

        let mut spans = String::new();
        for run in runs(&row, |cell| (cell.fg.clone(), cell.bold, cell.dim)) {
            let text: String = row[run.0..run.1]
                .iter()
                .map(|cell| cell.text.as_str())
                .collect();
            // A run that ends in blanks draws nothing with them, and a run
            // that is all blanks draws nothing at all.
            let text = text.trim_end();
            if text.is_empty() {
                continue;
            }
            let cell = &row[run.0];
            let weight = if cell.bold {
                " font-weight=\"bold\""
            } else {
                ""
            };
            let fade = if cell.dim { " opacity=\"0.65\"" } else { "" };
            spans.push_str(&format!(
                "<tspan x=\"{}\" fill=\"{}\"{weight}{fade}>{}</tspan>",
                round(run.0 as f32 * CELL_W),
                cell.fg,
                escape(text)
            ));
        }
        if spans.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "<text y=\"{}\" xml:space=\"preserve\">{spans}</text>\n",
            round(y as f32 * CELL_H + BASELINE)
        ));
    }

    out.push_str("</svg>\n");
    out
}

/// One cell of the picture: what it says and how it is painted, with the
/// terminal's defaults already resolved to something SVG can use.
struct Painted {
    text: String,
    fg: String,
    bg: String,
    bold: bool,
    dim: bool,
}

/// One row of the buffer, resolved.
fn row_of(buffer: &Buffer, y: u16, ground: &str, ink: &str) -> Vec<Painted> {
    (0..buffer.area.width)
        .map(|x| {
            let Some(cell) = buffer.cell((x, y)) else {
                return Painted {
                    text: " ".to_string(),
                    fg: ink.to_string(),
                    bg: ground.to_string(),
                    bold: false,
                    dim: false,
                };
            };
            let mut fg = hex(cell.fg, ink);
            let mut bg = hex(cell.bg, ground);
            // Reverse video is a colour swap once the defaults are resolved,
            // which is the only way to draw it without a terminal.
            if cell.modifier.contains(Modifier::REVERSED) {
                std::mem::swap(&mut fg, &mut bg);
            }
            Painted {
                // An empty symbol is the far half of a wide glyph; the near
                // half already carries both columns of text.
                text: cell.symbol().to_string(),
                fg,
                bg,
                bold: cell.modifier.contains(Modifier::BOLD),
                dim: cell.modifier.contains(Modifier::DIM),
            }
        })
        .collect()
}

/// Split a row into runs of cells that `key` cannot tell apart, as
/// `[from, to)` pairs.
fn runs<K: PartialEq>(row: &[Painted], key: impl Fn(&Painted) -> K) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    for i in 1..=row.len() {
        if i == row.len() || key(&row[i]) != key(&row[start]) {
            runs.push((start, i));
            start = i;
        }
    }
    runs
}

/// A colour an SVG can paint with. `Reset` is whatever the terminal would
/// have used, which only this module can decide.
fn hex(colour: Color, reset: &str) -> String {
    match colour {
        Color::Reset => reset.to_string(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(i) => indexed(i),
        Color::Black => ANSI[0].to_string(),
        Color::Red => ANSI[1].to_string(),
        Color::Green => ANSI[2].to_string(),
        Color::Yellow => ANSI[3].to_string(),
        Color::Blue => ANSI[4].to_string(),
        Color::Magenta => ANSI[5].to_string(),
        Color::Cyan => ANSI[6].to_string(),
        Color::Gray => ANSI[7].to_string(),
        Color::DarkGray => ANSI[8].to_string(),
        Color::LightRed => ANSI[9].to_string(),
        Color::LightGreen => ANSI[10].to_string(),
        Color::LightYellow => ANSI[11].to_string(),
        Color::LightBlue => ANSI[12].to_string(),
        Color::LightMagenta => ANSI[13].to_string(),
        Color::LightCyan => ANSI[14].to_string(),
        Color::White => ANSI[15].to_string(),
    }
}

/// An xterm 256 index: the sixteen, then the 6x6x6 cube, then the grey ramp.
fn indexed(i: u8) -> String {
    match i {
        0..=15 => ANSI[i as usize].to_string(),
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            let (r, g, b) = (level(i / 36), level((i % 36) / 6), level(i % 6));
            format!("#{r:02x}{g:02x}{b:02x}")
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            format!("#{v:02x}{v:02x}{v:02x}")
        }
    }
}

/// The three characters that would otherwise end an element or an attribute.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A coordinate without a trailing `.0`, so the file reads as a drawing
/// rather than as floating point.
fn round(v: f32) -> String {
    if (v - v.round()).abs() < f32::EPSILON {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

/// `YYYYMMDD-HHMMSS`, in UTC: a picture is named after the moment it was
/// taken, and varde carries no timezone database to say it in local time.
fn stamp(secs: u64) -> String {
    let (year, month, day) = civil((secs / 86_400) as i64);
    let rest = secs % 86_400;
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// Days since 1970-01-01 as a civil date, by Howard Hinnant's algorithm.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests;
