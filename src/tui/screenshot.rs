//! A picture of the browser, taken the way Textual takes one: the buffer that
//! was just drawn, written out as SVG.
//!
//! ratatui has no screenshot of its own and chaps takes no dependency for
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

/// Write a picture of `buffer` to the directory `chaps ui` was started in,
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

/// `chaps-ui-20260925-143012.svg`.
fn file_name(now: SystemTime) -> String {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default();
    format!("chaps-ui-{}.svg", stamp(secs))
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
/// taken, and chaps carries no timezone database to say it in local time.
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
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Style;
    use std::time::Duration;

    fn buffer(width: u16, height: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, width, height))
    }

    #[test]
    fn a_buffer_becomes_a_picture_of_itself() {
        let mut buffer = buffer(20, 2);
        buffer.set_string(0, 0, "chaps · models", Style::default());
        buffer.set_string(0, 1, "7 models", Style::default().fg(Color::Cyan));
        let svg = svg(&buffer, &Theme::default());

        assert!(
            svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""),
            "{svg}"
        );
        assert!(svg.ends_with("</svg>\n"), "{svg}");
        // Twenty cells by two rows, at the cell size the font is set for.
        assert!(svg.contains("width=\"168\" height=\"36\""), "{svg}");
        // The ground is one rect, before anything is drawn on it.
        assert!(
            svg.contains(&format!(
                "<rect width=\"168\" height=\"36\" fill=\"{}\"/>",
                hex(Theme::default().background(), "#141618")
            )),
            "{svg}"
        );
        assert!(svg.contains(">chaps · models</tspan>"), "{svg}");
        assert!(
            svg.contains("fill=\"#06989a\">7 models</tspan>"),
            "cyan: {svg}"
        );
        // One text element per row that has anything on it.
        assert_eq!(svg.matches("<text").count(), 2, "{svg}");
    }

    #[test]
    fn the_three_characters_that_would_break_the_file_are_escaped() {
        let mut buffer = buffer(24, 1);
        buffer.set_string(0, 0, "a <b> & \"c\"", Style::default());
        let svg = svg(&buffer, &Theme::default());
        assert!(svg.contains("a &lt;b&gt; &amp; &quot;c&quot;"), "{svg}");
        assert!(!svg.contains("<b>"), "{svg}");
    }

    #[test]
    fn weight_colour_and_reverse_video_survive_the_trip() {
        let mut styled = buffer(24, 1);
        styled.set_string(0, 0, "MODEL", Theme::default().label_style());
        styled.set_string(6, 0, "chaps", Theme::default().accent_style());
        styled.set_string(12, 0, "dim", Theme::default().dim_style());
        let styled = svg(&styled, &Theme::default());
        assert!(
            styled.contains("fill=\"#d8dce0\" font-weight=\"bold\">MODEL</tspan>"),
            "a heading is bold in the terminal's own ink: {styled}"
        );
        assert!(
            styled.contains("fill=\"#06989a\">chaps</tspan>"),
            "the accent is cyan: {styled}"
        );
        assert!(
            styled.contains("fill=\"#555753\">dim</tspan>"),
            "dim is a colour here, not an opacity: {styled}"
        );

        // Reverse video has no terminal to ask, so it is drawn as the swap it
        // is: the row's colour becomes the ground under it.
        let mut reversed = buffer(12, 1);
        reversed.set_string(0, 0, "row", Theme::monochrome().selection_style());
        let reversed = svg(&reversed, &Theme::monochrome());
        assert!(
            reversed.contains("fill=\"#141618\">row</tspan>"),
            "the ground became the ink: {reversed}"
        );
        assert!(
            reversed
                .contains("<rect x=\"0\" y=\"0\" width=\"25.2\" height=\"18\" fill=\"#d8dce0\"/>"),
            "and the ink became the ground, painted under it: {reversed}"
        );
    }

    /// A row of nothing is not worth an element, and the picture of an empty
    /// screen is still a valid document.
    #[test]
    fn blank_rows_draw_nothing() {
        let svg = svg(&buffer(10, 3), &Theme::default());
        assert!(!svg.contains("<text"), "{svg}");
        assert_eq!(svg.matches("<rect").count(), 1, "only the ground: {svg}");
    }

    #[test]
    fn the_file_is_named_after_the_second_it_was_taken_in() {
        // 2026-09-25T14:30:12Z.
        let now = UNIX_EPOCH + Duration::from_secs(1_790_346_612);
        assert_eq!(file_name(now), "chaps-ui-20260925-143012.svg");
        assert_eq!(stamp(0), "19700101-000000");
        // A leap day, which is where a home-made calendar goes wrong.
        assert_eq!(stamp(1_709_164_800), "20240229-000000");
    }

    #[test]
    fn saving_writes_the_file_and_says_where() {
        let dir = tempfile::tempdir().unwrap();
        let mut buffer = buffer(16, 1);
        buffer.set_string(0, 0, "chaps", Style::default());
        let now = UNIX_EPOCH + Duration::from_secs(1_790_346_612);

        let message = save_in(dir.path(), &buffer, &Theme::default(), now);
        assert_eq!(message, "saved chaps-ui-20260925-143012.svg");
        let written =
            std::fs::read_to_string(dir.path().join("chaps-ui-20260925-143012.svg")).unwrap();
        assert!(written.contains(">chaps</tspan>"), "{written}");

        // A directory that is not there is a message, not a panic, and
        // nothing is created to make room for the file.
        let missing = dir.path().join("nope");
        let message = save_in(&missing, &buffer, &Theme::default(), now);
        assert!(
            message.starts_with("could not save `chaps-ui-"),
            "{message}"
        );
        assert!(!missing.exists(), "no directory was made");
    }

    /// A picture of the browser is a picture of whatever page was up: the
    /// buffer is the only thing this reads, so the components page needs no
    /// special case - which is worth a test rather than a comment.
    #[test]
    fn the_components_page_photographs_like_the_model_page() {
        use crate::project::ProjectState;
        use crate::tui::app::{Action, App};
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let registry = crate::registry::load_embedded().expect("embedded snapshot parses");
        let mut state = ProjectState::default();
        state
            .components
            .set_enabled(crate::components::Component::Ocs, true);
        let mut app = App::new(&registry, &state);
        app.reduce(Action::NextPage);

        let mut terminal = Terminal::new(TestBackend::new(120, 12)).expect("test backend starts");
        terminal
            .draw(|frame| crate::tui::ui::draw(frame, &app, &Theme::default()))
            .expect("draw");
        let svg = svg(terminal.backend().buffer(), &Theme::default());

        assert!(svg.starts_with("<svg "), "{svg}");
        // The title bar paints each part in its own colour, so the page's name
        // is a span of its own.
        assert!(svg.contains(">components</tspan>"), "{svg}");
        assert!(svg.contains("COMPONENT    STATE    REACH"), "{svg}");
        assert!(svg.contains("chap-core"), "{svg}");
        assert!(svg.contains(">http://localhost:9000</tspan>"), "{svg}");
        assert!(svg.ends_with("</svg>\n"), "{svg}");
    }

    #[test]
    fn every_kind_of_colour_becomes_a_hex_value() {
        assert_eq!(hex(Color::Reset, "#123456"), "#123456");
        assert_eq!(hex(Color::Rgb(1, 2, 3), "#000000"), "#010203");
        assert_eq!(hex(Color::Green, "#000000"), ANSI[2]);
        assert_eq!(hex(Color::Indexed(9), "#000000"), ANSI[9]);
        // The middle of the 6x6x6 cube, and the top of the grey ramp.
        assert_eq!(hex(Color::Indexed(16), "#000000"), "#000000");
        assert_eq!(hex(Color::Indexed(231), "#000000"), "#ffffff");
        assert_eq!(hex(Color::Indexed(255), "#000000"), "#eeeeee");
    }
}
