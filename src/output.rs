//! Human and `--json` output, and the one place that knows about colour.
//!
//! Every command renders through [`Out`] so `--json` is uniform: the same
//! value is either pretty-printed as JSON or handed to a human formatter.
//! [`Out`] also carries the two facts that decide how the human rendering
//! looks: whether stdout is a terminal, and whether colour is wanted. Nothing
//! else in the crate calls into `console`.

use crate::error::Result;
use console::{Style, measure_text_width};
use serde::Serialize;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

/// Width human prose is wrapped to.
pub const WRAP_WIDTH: usize = 80;

/// Widest a panel is drawn, however wide the terminal is: a box the width of
/// an ultrawide terminal is a worse read, not a better one.
pub const PANEL_MAX_WIDTH: usize = 100;

/// Narrowest panel worth drawing; below this the box is all frame.
const PANEL_MIN_WIDTH: usize = 24;

/// Say nothing but the answer.
pub const QUIET: u8 = 0;
/// `-v`: narrate what runs.
pub const VERBOSE: u8 = 1;
/// `-d`: narrate what runs and what came back.
pub const DEBUG: u8 = 2;

/// How much of a response body a `-d` line prints.
pub const MAX_TRACE_BODY: usize = 2048;

/// What a panel is about, which is all the border colour says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    Error,
    Warning,
    Ok,
    /// Neither good nor bad: the CLI's own colour.
    Accent,
}

impl Default for PanelKind {
    /// A box with nothing to report is just a box.
    fn default() -> PanelKind {
        PanelKind::Accent
    }
}

/// Output mode for one CLI invocation.
///
/// `Default` is the plain, colourless, non-terminal mode: that is what the
/// unit tests render against, and what a pipe gets.
#[derive(Debug, Clone, Copy, Default)]
pub struct Out {
    pub json: bool,
    /// Paint with ANSI styles. Implies [`Out::tty`] in practice, but the two
    /// are tracked apart so `NO_COLOR` on a terminal still gets the boxes.
    pub color: bool,
    /// Whether stdout is a terminal. Panels are only drawn when it is, so a
    /// script that parses `chaps` output sees exactly the plain lines it
    /// always saw.
    pub tty: bool,
    /// [`QUIET`], [`VERBOSE`] or [`DEBUG`]. Tracing never reaches stdout, so
    /// raising it can never change what a caller parses.
    pub verbosity: u8,
}

impl Out {
    /// The real environment: colour when stdout is a terminal, `NO_COLOR` is
    /// unset and `--no-color` was not given. `--json` is never coloured, so
    /// its bytes are the same everywhere.
    pub fn detect(json: bool, no_color: bool) -> Out {
        let tty = std::io::stdout().is_terminal();
        Out {
            json,
            color: !json && tty && !no_color && std::env::var_os("NO_COLOR").is_none(),
            tty,
            verbosity: QUIET,
        }
    }

    /// `-v` or `-d` was given. `-d` implies `-v`: there is no way to ask for
    /// the bodies without the requests they belong to.
    pub fn is_verbose(&self) -> bool {
        self.verbosity >= VERBOSE
    }

    /// `-d` was given.
    pub fn is_debug(&self) -> bool {
        self.verbosity >= DEBUG
    }

    /// Narrate one step under `-v`, on stderr, dimmed.
    pub fn verbose(&self, message: &str) {
        if self.is_verbose() {
            trace(message);
        }
    }

    /// Narrate one detail under `-d`, on stderr, dimmed.
    pub fn debug(&self, message: &str) {
        if self.is_debug() {
            trace(message);
        }
    }

    /// Print `value` as pretty JSON under `--json`, otherwise print `human()`.
    ///
    /// The human string is printed as one line with a trailing newline added
    /// when it does not already end in one; an empty string prints nothing.
    pub fn emit<T: Serialize>(&self, value: &T, human: impl FnOnce() -> String) -> Result<()> {
        let stdout = std::io::stdout();
        let mut w = stdout.lock();
        if self.json {
            let body = serde_json::to_string_pretty(value)?;
            writeln!(w, "{body}")?;
        } else {
            let body = human();
            if !body.is_empty() {
                if body.ends_with('\n') {
                    write!(w, "{body}")?;
                } else {
                    writeln!(w, "{body}")?;
                }
            }
        }
        w.flush()?;
        Ok(())
    }

    /// Apply a style, or hand the text back untouched when colour is off.
    fn paint(&self, text: &str, style: Style) -> String {
        if self.color {
            // `console` would otherwise re-decide for itself whether the
            // stream is a terminal; the decision was already made in
            // `Out::detect`, and the tests depend on it being ours.
            style.force_styling(true).apply_to(text).to_string()
        } else {
            text.to_string()
        }
    }

    /// A section or document heading.
    pub fn heading(&self, text: &str) -> String {
        self.paint(text, Style::new().cyan().bold())
    }

    /// Something that worked, is up, or is on.
    pub fn ok(&self, text: &str) -> String {
        self.paint(text, Style::new().green())
    }

    /// Something that needs attention but is not a failure.
    pub fn warn(&self, text: &str) -> String {
        self.paint(text, Style::new().yellow())
    }

    /// Something that failed, is down, or is off.
    pub fn bad(&self, text: &str) -> String {
        self.paint(text, Style::new().red())
    }

    /// Secondary text: paths, ages, labels, things that are already known.
    pub fn dim(&self, text: &str) -> String {
        self.paint(text, Style::new().dim())
    }

    /// A command the reader is meant to type.
    pub fn cmd(&self, text: &str) -> String {
        self.paint(text, Style::new().bold())
    }

    /// A label in front of a value.
    pub fn key(&self, text: &str) -> String {
        self.paint(text, Style::new().cyan())
    }

    /// A value worth reading twice: a URL, a token, a port.
    pub fn value(&self, text: &str) -> String {
        self.paint(text, Style::new().bold())
    }

    /// Bold every `` `backticked` `` span in a sentence, backticks included,
    /// so the plain rendering is byte-for-byte what it always was.
    pub fn backticks(&self, text: &str) -> String {
        if !self.color {
            return text.to_string();
        }
        let mut out = String::new();
        let mut rest = text;
        while let Some(open) = rest.find('`') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('`') else { break };
            out.push_str(&rest[..open]);
            out.push_str(&self.cmd(&rest[open..open + close + 2]));
            rest = &after[close + 1..];
        }
        out.push_str(rest);
        out
    }

    /// Draw `body` in a rounded box titled `title`, sized to the terminal.
    ///
    /// The border carries the only colour: a panel is a shape first, so it
    /// still reads as one under `NO_COLOR`.
    pub fn panel(&self, title: &str, body: &str, kind: PanelKind) -> String {
        self.panel_at(title, body, kind, terminal_width())
    }

    /// [`Out::panel`] at an explicit width, for tests and fixed layouts.
    pub fn panel_at(&self, title: &str, body: &str, kind: PanelKind, width: usize) -> String {
        let width = width.clamp(PANEL_MIN_WIDTH, PANEL_MAX_WIDTH);
        let border = match kind {
            PanelKind::Error => Style::new().red(),
            PanelKind::Warning => Style::new().yellow(),
            PanelKind::Ok => Style::new().green(),
            PanelKind::Accent => Style::new().cyan(),
        };
        // `│ ` on the left and ` │` on the right.
        let inner = width - 4;

        let mut title_text = if title.is_empty() {
            String::new()
        } else {
            format!(" {title} ")
        };
        // A title longer than the box would push the closing corner off the
        // line; cut it rather than let the frame break.
        if title_text.chars().count() + 3 > width {
            title_text = title_text.chars().take(width - 3).collect();
        }
        let fill = width - 3 - title_text.chars().count();

        let mut out = String::new();
        out.push_str(&self.paint("╭─", border.clone()));
        if !title_text.is_empty() {
            out.push_str(&self.paint(&title_text, border.clone().bold()));
        }
        out.push_str(&self.paint(&format!("{}╮", "─".repeat(fill)), border.clone()));
        out.push('\n');

        for line in wrap_hard(body, inner) {
            let pad = inner.saturating_sub(measure_text_width(&line));
            out.push_str(&self.paint("│", border.clone()));
            out.push(' ');
            out.push_str(&line);
            out.push_str(&" ".repeat(pad));
            out.push(' ');
            out.push_str(&self.paint("│", border.clone()));
            out.push('\n');
        }

        out.push_str(&self.paint(&format!("╰{}╯", "─".repeat(width - 2)), border));
        out
    }

    /// A panel on a terminal, `plain` anywhere else.
    ///
    /// Pipes and scripts keep the exact line they have always parsed; only a
    /// human at a terminal gets the box.
    pub fn panel_or(&self, title: &str, body: &str, kind: PanelKind, plain: &str) -> String {
        if self.tty {
            self.panel(title, body, kind)
        } else {
            plain.to_string()
        }
    }

    /// Render a table with two spaces between columns and no trailing
    /// whitespace. Returns the text, ending in a newline when non-empty.
    ///
    /// Cells are left-aligned and padded to the widest cell in their column,
    /// measured with the ANSI escapes stripped so a coloured cell lines up
    /// with a plain one. The last column is never padded, so no line ends in a
    /// space and the output pastes cleanly into a terminal or a bug report.
    ///
    /// With colour on the header is bold and gets a rule under it; with colour
    /// off the table is exactly the plain grid it always was.
    pub fn table(&self, headers: &[&str], rows: &[Vec<String>]) -> String {
        if headers.is_empty() && rows.is_empty() {
            return String::new();
        }
        let columns = headers
            .len()
            .max(rows.iter().map(Vec::len).max().unwrap_or(0));
        let mut widths = vec![0usize; columns];
        for (i, h) in headers.iter().enumerate() {
            widths[i] = widths[i].max(display_width(h));
        }
        for row in rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(display_width(cell));
            }
        }

        let mut out = String::new();
        if !headers.is_empty() {
            let bold: Vec<String> = headers
                .iter()
                .map(|h| self.paint(h, Style::new().bold()))
                .collect();
            push_row(&mut out, bold.iter().map(String::as_str), &widths);
            if self.color {
                let rule: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
                out.push_str(&self.dim(rule.join("  ").trim_end()));
                out.push('\n');
            }
        }
        for row in rows {
            push_row(&mut out, row.iter().map(String::as_str), &widths);
        }
        out
    }

    /// Render an error: a JSON object under `--json`, a red `Error` panel on a
    /// terminal, and otherwise `error: ...` followed by one indented line per
    /// cause. The result has no trailing newline.
    pub fn error(&self, err: &anyhow::Error) -> String {
        let causes: Vec<String> = err.chain().skip(1).map(|c| c.to_string()).collect();
        if self.json {
            let value = serde_json::json!({
                "error": err.to_string(),
                "causes": causes,
            });
            return serde_json::to_string_pretty(&value)
                .unwrap_or_else(|_| format!("{{\"error\":\"{}\"}}", err));
        }

        let mut plain = format!("error: {err}");
        for cause in &causes {
            plain.push_str(&format!("\n  caused by: {cause}"));
        }
        if !self.tty {
            return plain;
        }

        // A hint ("run `chaps init` first") is the line the reader acts on, so
        // it gets a line of its own inside the box instead of trailing off the
        // end of the sentence.
        let mut body: Vec<String> = hint_lines(&err.to_string())
            .iter()
            .map(|line| self.backticks(line))
            .collect();
        for cause in &causes {
            body.push(format!(
                "{} {}",
                self.dim("caused by:"),
                self.backticks(cause)
            ));
        }
        self.panel("Error", &body.join("\n"), PanelKind::Error)
    }
}

/// Remember `--no-color` for the stderr side, which has no [`Out`] to consult.
static NO_COLOR_FLAG: AtomicBool = AtomicBool::new(false);

/// The same, for `-v` and `-d`: the docker runner and the HTTP helpers sit
/// several layers below the command that owns the [`Out`].
static LEVEL: AtomicU8 = AtomicU8::new(QUIET);

/// Record the global `-v` / `-d` flags. Called once, from `Ctx::from_cli`.
pub fn set_verbosity(verbose: bool, debug: bool) -> u8 {
    let level = if debug {
        DEBUG
    } else if verbose {
        VERBOSE
    } else {
        QUIET
    };
    LEVEL.store(level, Ordering::Relaxed);
    level
}

/// Whether `-v` (or `-d`) was given.
pub fn verbose_enabled() -> bool {
    LEVEL.load(Ordering::Relaxed) >= VERBOSE
}

/// Whether `-d` was given.
pub fn debug_enabled() -> bool {
    LEVEL.load(Ordering::Relaxed) >= DEBUG
}

/// Narrate one step under `-v`, for code with no [`Out`] in reach.
pub fn verbose(message: &str) {
    if verbose_enabled() {
        trace(message);
    }
}

/// Narrate one detail under `-d`, for code with no [`Out`] in reach.
pub fn debug(message: &str) {
    if debug_enabled() {
        trace(message);
    }
}

/// One trace line: stderr, dimmed, never stdout.
///
/// stdout belongs to the answer; a `--json` consumer has to be able to pipe it
/// into a parser however loud the CLI is being.
fn trace(message: &str) {
    for line in message.split('\n') {
        if stderr_color() {
            let styled = Style::new().dim().force_styling(true).apply_to(line);
            eprintln!("{styled}");
        } else {
            eprintln!("{line}");
        }
    }
}

/// A response body, cut to [`MAX_TRACE_BODY`] bytes on a character boundary.
pub fn trace_body(body: &str) -> String {
    if body.len() <= MAX_TRACE_BODY {
        return body.to_string();
    }
    let mut end = MAX_TRACE_BODY;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}... ({} bytes total)", &body[..end], body.len())
}

/// Record the global `--no-color` flag. Called once, from `Ctx::from_cli`.
pub fn set_no_color(flag: bool) {
    NO_COLOR_FLAG.store(flag, Ordering::Relaxed);
}

/// Whether a warning on stderr may be coloured.
fn stderr_color() -> bool {
    !NO_COLOR_FLAG.load(Ordering::Relaxed)
        && std::env::var_os("NO_COLOR").is_none()
        && std::io::stderr().is_terminal()
}

/// Print a one-line warning to stderr, with a yellow `warning:` in front of it
/// when stderr is a terminal.
///
/// Warnings never go to stdout: a `--json` consumer must be able to pipe
/// stdout straight into a parser. They are never panels either - a warning is
/// something noticed in passing, not the answer to the command.
pub fn warn(message: &str) {
    if stderr_color() {
        let label = Style::new().yellow().bold().apply_to("warning:");
        eprintln!("{label} {message}");
    } else {
        eprintln!("warning: {message}");
    }
}

/// Print one dimmed line to stderr.
///
/// For something worth knowing that is not about the command that was run, and
/// that nothing is wrong with: the update notice is the only caller. Dimmed
/// rather than labelled `warning:`, and on stderr for the same reason warnings
/// are, so a `--json` consumer's stdout stays parseable.
pub fn notice(message: &str) {
    if stderr_color() {
        eprintln!("{}", Style::new().dim().apply_to(message));
    } else {
        eprintln!("{message}");
    }
}

/// Render `label  value` pairs with the labels padded to a common width.
///
/// Entries whose value is empty are skipped, and a value containing newlines
/// keeps the value column on its continuation lines. The result ends in a
/// newline when it is non-empty.
pub fn fields(indent: usize, entries: &[(&str, String)]) -> String {
    fields_with(indent, entries, &|label| label.to_string())
}

/// [`fields`] with the labels run through `style` as they are written.
///
/// The column is measured on the plain label, so a styled label lines up with
/// an unstyled one.
pub fn fields_with(
    indent: usize,
    entries: &[(&str, String)],
    style: &dyn Fn(&str) -> String,
) -> String {
    let width = entries
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, _)| display_width(k))
        .max()
        .unwrap_or(0);
    let lead = " ".repeat(indent);
    let mut out = String::new();
    for (key, value) in entries.iter().filter(|(_, v)| !v.is_empty()) {
        for (i, line) in value.split('\n').enumerate() {
            if i == 0 {
                out.push_str(&lead);
                out.push_str(&style(key));
                out.push_str(&" ".repeat(width - display_width(key) + 2));
            } else {
                out.push_str(&lead);
                out.push_str(&" ".repeat(width + 2));
            }
            out.push_str(line.trim_end());
            // A blank continuation line must not carry the indent with it.
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
        }
    }
    out
}

/// Split a sentence at the `; ` in front of a hint, so the thing to do next
/// can be put on a line of its own.
///
/// A hint is what the messages in this CLI have always looked like: it either
/// starts with ``run `...` `` or tells you to start something with something.
pub fn hint_lines(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(at) = rest
        .match_indices("; ")
        .find(|(i, _)| is_hint(&rest[i + 2..]))
        .map(|(i, _)| i)
    {
        parts.push(rest[..at].trim_end().to_string());
        rest = &rest[at + 2..];
    }
    parts.push(rest.to_string());
    parts
}

fn is_hint(tail: &str) -> bool {
    tail.starts_with("run `") || (tail.starts_with("start ") && tail.contains(" with "))
}

/// Greedily wrap `text` to `width` columns, preserving explicit line breaks.
///
/// A word longer than `width` is left on a line of its own rather than split,
/// which keeps URLs and image references copy-pasteable.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if display_width(&current) + 1 + display_width(word) <= width {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(std::mem::take(&mut current));
                current.push_str(word);
            }
        }
        lines.push(current);
    }
    lines
}

/// [`wrap`], then break anything still too wide. Inside a box a long word has
/// to be cut: the alternative is a frame with a hole in it.
fn wrap_hard(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for line in wrap(text, width) {
        if measure_text_width(&line) <= width {
            lines.push(line);
            continue;
        }
        let mut current = String::new();
        for c in line.chars() {
            if measure_text_width(&current) + 1 > width {
                lines.push(std::mem::take(&mut current));
            }
            current.push(c);
        }
        lines.push(current);
    }
    lines
}

/// Wrap `text` and join it back into a block, one line per wrapped line.
pub fn wrapped(text: &str, width: usize) -> String {
    wrap(text, width).join("\n")
}

/// A rough, human-readable age: "42 seconds", "3 hours", "2 days".
///
/// Only the largest unit is shown; this labels a cache entry, so minutes of
/// precision on a two-day-old snapshot would be noise.
pub fn human_age(age: Duration) -> String {
    let (value, unit) = age_parts(age);
    if value == 1 {
        format!("1 {unit}")
    } else {
        format!("{value} {unit}s")
    }
}

/// The same age as a table cell: "42s ago", "3h ago", "2d ago".
///
/// [`human_age`]'s units, abbreviated: a column of ages has to stay narrow
/// enough that the columns after it are still readable.
pub fn ago(age: Duration) -> String {
    let (value, unit) = age_parts(age);
    format!("{value}{} ago", &unit[..1])
}

/// `HH:MM` of a Unix timestamp on this machine's clock.
///
/// Local time, because the only thing done with one of these is to compare it
/// with the clock in front of the reader: "wait until 15:04" has to mean
/// their 15:04. The offset comes from this machine's own zone file; where
/// there is none to read - Windows keeps its zone somewhere else - the time
/// is UTC, which is the closest this can get without a calendar dependency.
pub fn local_clock(unix: u64) -> String {
    clock_of(unix, local_offset(unix as i64))
}

/// [`local_clock`] with the offset handed in, so the formatting is testable
/// on a machine in any time zone.
fn clock_of(unix: u64, offset: i64) -> String {
    let local = (unix as i64).saturating_add(offset).max(0) as u64;
    let (_, _, _, hour, minute, _) = crate::backup::utc_parts(local);
    format!("{hour:02}:{minute:02}")
}

/// This machine's UTC offset in seconds at `unix`, and 0 where nothing here
/// can say what it is.
fn local_offset(unix: i64) -> i64 {
    std::fs::read(LOCALTIME)
        .ok()
        .and_then(|bytes| tzif_offset(&bytes, unix))
        .unwrap_or(0)
}

/// The zone file every Unix keeps the machine's own zone in, as a symlink
/// into the zoneinfo database. Absent on Windows, which is one of the two
/// ways [`local_offset`] ends up with nothing to read.
const LOCALTIME: &str = "/etc/localtime";

/// The UTC offset a TZif file (RFC 8536) gives for `unix`, in seconds.
///
/// Written out rather than taken from a calendar crate, for the same reason
/// [`crate::chapcore::sha256_hex`] is: it is one function, it is only ever
/// used to print a clock time, and the release targets keep the dependency
/// set they have.
///
/// A version 2 or later file carries the whole table twice - once with
/// 32-bit transition times, once with 64-bit ones - and the modern `zic`
/// leaves the first copy empty, so the second block is the one to read.
fn tzif_offset(bytes: &[u8], unix: i64) -> Option<i64> {
    /// Magic, version and the reserved bytes, before the six counts.
    const COUNTS_AT: usize = 20;
    /// The whole header: the counts are six 32-bit numbers.
    const HEADER: usize = COUNTS_AT + 6 * 4;

    let u32_at = |at: usize| -> Option<u32> {
        bytes
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_be_bytes)
    };
    // `isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt`, in that order.
    let counts = |start: usize| -> Option<[u32; 6]> {
        if bytes.get(start..start + 4)? != b"TZif" {
            return None;
        }
        let mut out = [0u32; 6];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = u32_at(start + COUNTS_AT + i * 4)?;
        }
        Some(out)
    };

    // Where the block to read starts, and how wide its transition times are.
    let (start, width) = match *bytes.get(4)? >= b'2' {
        false => (0, 4usize),
        true => {
            let [isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt] = counts(0)?;
            let first = HEADER
                + timecnt as usize * 5
                + typecnt as usize * 6
                + charcnt as usize
                + leapcnt as usize * 8
                + isstdcnt as usize
                + isutcnt as usize;
            (first, 8usize)
        }
    };

    let [_, _, _, timecnt, typecnt, _] = counts(start)?;
    let times = start + HEADER;
    let indices = times + timecnt as usize * width;
    let types = indices + timecnt as usize;

    // The last transition at or before `unix` decides which local time type
    // is in force; before the first one, the file's first type is.
    let mut which = 0usize;
    for i in 0..timecnt as usize {
        let at = times + i * width;
        let when = match width {
            8 => i64::from_be_bytes(bytes.get(at..at + 8)?.try_into().ok()?),
            _ => i64::from(i32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?)),
        };
        if when > unix {
            break;
        }
        which = *bytes.get(indices + i)? as usize;
    }
    if which >= typecnt as usize {
        return None;
    }
    let at = types + which * 6;
    Some(i64::from(i32::from_be_bytes(
        bytes.get(at..at + 4)?.try_into().ok()?,
    )))
}

/// The largest whole unit of an age, as `(value, singular unit name)`.
///
/// One place decides where a duration stops being seconds, so [`human_age`]
/// and [`ago`] can never disagree about it.
fn age_parts(age: Duration) -> (u64, &'static str) {
    let secs = age.as_secs();
    match secs {
        0..=59 => (secs, "second"),
        60..=3599 => (secs / 60, "minute"),
        3600..=86_399 => (secs / 3600, "hour"),
        _ => (secs / 86_400, "day"),
    }
}

/// How wide a panel may be here. Off a terminal `console` answers with its own
/// default, which is the same 80 columns the prose is wrapped to.
fn terminal_width() -> usize {
    console::Term::stdout().size().1 as usize
}

fn push_row<'a>(out: &mut String, cells: impl Iterator<Item = &'a str>, widths: &[usize]) {
    let cells: Vec<&str> = cells.collect();
    let last = cells.len().saturating_sub(1);
    let start = out.len();
    for (i, cell) in cells.iter().enumerate() {
        out.push_str(cell);
        if i != last {
            let pad = widths[i].saturating_sub(display_width(cell)) + 2;
            out.push_str(&" ".repeat(pad));
        }
    }
    // A row whose last cells are empty would otherwise end in the padding of
    // the columns before them, and no line of this CLI's output ends in
    // whitespace. Only spaces this function itself added can be here.
    while out.len() > start && out.ends_with(' ') {
        out.pop();
    }
    out.push('\n');
}

/// Column width in characters, with any ANSI escapes discounted, so a styled
/// cell occupies the same number of columns as the text inside it.
fn display_width(s: &str) -> usize {
    measure_text_width(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terminal that wants colour.
    fn colored() -> Out {
        Out {
            color: true,
            tty: true,
            ..Out::default()
        }
    }

    /// A terminal that does not: `NO_COLOR`, or `--no-color`.
    fn plain_tty() -> Out {
        Out {
            tty: true,
            ..Out::default()
        }
    }

    #[test]
    fn table_pads_columns_and_trims_line_ends() {
        let out = Out::default();
        let text = out.table(
            &["ID", "PORT"],
            &[
                vec!["chapkit_ewars_model".into(), "5001".into()],
                vec!["auto_arima_chapkit".into(), "5002".into()],
            ],
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "ID                   PORT");
        assert_eq!(lines[1], "chapkit_ewars_model  5001");
        assert_eq!(lines[2], "auto_arima_chapkit   5002");
        assert!(lines.iter().all(|l| !l.ends_with(' ')));
        assert!(text.ends_with('\n'));

        // A row whose last cells are empty ends after its last word, not in
        // the padding of the columns before them.
        let text = out.table(
            &["VERSION", "STATUS", "CHAPKIT", "CHANGELOG"],
            &[vec![
                "sha-b1d6c31".into(),
                "unstable".into(),
                String::new(),
                String::new(),
            ]],
        );
        assert_eq!(text.lines().nth(1), Some("sha-b1d6c31  unstable"));
    }

    #[test]
    fn table_widens_a_column_to_fit_its_header() {
        let out = Out::default();
        let text = out.table(
            &["ID", "ASSESSED STATUS", "PORT"],
            &[vec!["a".into(), "green".into(), "5001".into()]],
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "ID  ASSESSED STATUS  PORT");
        assert_eq!(lines[1], "a   green            5001");
    }

    #[test]
    fn table_tolerates_a_short_row() {
        let out = Out::default();
        let text = out.table(&["A", "B"], &[vec!["only".into()]]);
        assert_eq!(text.lines().nth(1), Some("only"));
    }

    #[test]
    fn table_of_nothing_is_empty() {
        assert_eq!(Out::default().table(&[], &[]), "");
    }

    #[test]
    fn a_styled_cell_occupies_the_width_of_its_text() {
        let out = colored();
        let text = out.table(
            &["ID", "STATUS", "PORT"],
            &[
                vec!["a".into(), out.ok("green"), "5001".into()],
                vec!["bbbb".into(), out.bad("red"), "5002".into()],
            ],
        );
        // Row 0 is the header, row 1 the rule colour adds, then the body.
        let rows: Vec<String> = text
            .lines()
            .skip(2)
            .map(|l| console::strip_ansi_codes(l).to_string())
            .collect();
        assert_eq!(rows[0], "a     green   5001");
        assert_eq!(rows[1], "bbbb  red     5002");
    }

    #[test]
    fn colour_adds_a_rule_under_the_header_and_plain_does_not() {
        let rows = vec![vec!["a".into(), "5001".into()]];
        assert!(colored().table(&["ID", "PORT"], &rows).contains('─'));
        assert!(!plain_tty().table(&["ID", "PORT"], &rows).contains('─'));
        assert!(!Out::default().table(&["ID", "PORT"], &rows).contains('─'));
    }

    #[test]
    fn styling_helpers_are_a_no_op_without_colour() {
        let out = Out::default();
        assert_eq!(out.heading("Next"), "Next");
        assert_eq!(out.ok("up"), "up");
        assert_eq!(out.warn("slow"), "slow");
        assert_eq!(out.bad("down"), "down");
        assert_eq!(out.dim("path"), "path");
        assert_eq!(out.cmd("chaps up"), "chaps up");
        assert_eq!(out.key("API"), "API");
        assert_eq!(out.value("8000"), "8000");
        assert_eq!(out.backticks("run `chaps up`"), "run `chaps up`");
    }

    #[test]
    fn styling_helpers_emit_escapes_with_colour() {
        let out = colored();
        let painted = out.ok("up");
        assert!(painted.contains('\u{1b}'), "{painted:?}");
        assert_eq!(console::strip_ansi_codes(&painted), "up");
        let hint = out.backticks("start it with `chaps up`");
        assert!(hint.contains('\u{1b}'));
        assert_eq!(
            console::strip_ansi_codes(&hint),
            "start it with `chaps up`",
            "the backticks stay, so the plain reading never changes"
        );
    }

    #[test]
    fn tracing_is_off_until_a_flag_turns_it_on() {
        let quiet = Out::default();
        assert!(!quiet.is_verbose());
        assert!(!quiet.is_debug());

        let verbose = Out {
            verbosity: VERBOSE,
            ..Out::default()
        };
        assert!(verbose.is_verbose());
        assert!(!verbose.is_debug(), "-v does not print bodies");

        let debug = Out {
            verbosity: DEBUG,
            ..Out::default()
        };
        assert!(debug.is_debug());
        assert!(debug.is_verbose(), "-d implies -v");
    }

    #[test]
    fn set_verbosity_maps_the_flags_onto_the_levels() {
        assert_eq!(set_verbosity(false, false), QUIET);
        assert_eq!(set_verbosity(true, false), VERBOSE);
        assert_eq!(set_verbosity(false, true), DEBUG, "-d implies -v");
        assert_eq!(set_verbosity(true, true), DEBUG);
        // Leave the process as quiet as the other tests expect it.
        set_verbosity(false, false);
    }

    #[test]
    fn a_traced_body_is_cut_at_a_character_boundary() {
        let short = "{\"status\":\"ok\"}";
        assert_eq!(trace_body(short), short);

        let long = "æ".repeat(MAX_TRACE_BODY);
        let cut = trace_body(&long);
        assert!(cut.ends_with(&format!("({} bytes total)", long.len())));
        assert!(cut.starts_with('æ'));
        assert!(
            cut.len() < long.len(),
            "a body that does not fit is cut, not printed whole"
        );
    }

    #[test]
    fn detect_turns_colour_off_for_json_and_no_color() {
        // stdout is not a terminal under `cargo test`, which is the case that
        // matters most: piped output is never painted.
        assert!(!Out::detect(false, false).color);
        assert!(!Out::detect(true, false).color);
        assert!(!Out::detect(false, true).color);
        assert!(!Out::detect(false, false).tty);
        assert!(!Out::detect(true, false).tty || Out::detect(true, false).json);
    }

    #[test]
    fn a_panel_is_a_box_of_exactly_the_asked_for_width() {
        let out = plain_tty();
        let text = out.panel_at("Error", "CHAP is not running", PanelKind::Error, 40);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], format!("╭─ Error {}╮", "─".repeat(30)));
        assert_eq!(
            lines[1],
            format!("│ CHAP is not running{} │", " ".repeat(17))
        );
        assert_eq!(lines[2], format!("╰{}╯", "─".repeat(38)));
        assert!(lines.iter().all(|l| l.chars().count() == 40));
        assert!(!text.ends_with('\n'), "the caller adds the newline");
    }

    #[test]
    fn a_panel_wraps_its_body_on_words() {
        let out = plain_tty();
        let text = out.panel_at(
            "Not running",
            "CHAP is not running\nstart it with `chaps up`",
            PanelKind::Warning,
            30,
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], format!("╭─ Not running {}╮", "─".repeat(14)));
        assert_eq!(
            lines[1],
            format!("│ CHAP is not running{} │", " ".repeat(7))
        );
        assert_eq!(
            lines[2],
            format!("│ start it with `chaps up`{} │", " ".repeat(2))
        );
        assert_eq!(lines[3], format!("╰{}╯", "─".repeat(28)));
        assert!(lines.iter().all(|l| l.chars().count() == 30));
    }

    #[test]
    fn a_panel_breaks_a_word_too_long_for_the_box() {
        let out = plain_tty();
        let text = out.panel_at("", "ghcr.io/dhis2-chap/chapkit-r-inla", PanelKind::Ok, 24);
        assert!(text.lines().all(|l| l.chars().count() == 24), "{text}");
        let body: String = text
            .lines()
            .skip(1)
            .take_while(|l| l.starts_with('│'))
            .map(|l| l[4..l.len() - 4].trim_end().to_string())
            .collect();
        assert_eq!(body, "ghcr.io/dhis2-chap/chapkit-r-inla");
    }

    #[test]
    fn a_panel_never_grows_past_the_maximum() {
        let text = plain_tty().panel_at("Error", "boom", PanelKind::Error, 400);
        assert!(
            text.lines().all(|l| l.chars().count() == PANEL_MAX_WIDTH),
            "{text}"
        );
    }

    #[test]
    fn a_long_title_is_cut_rather_than_breaking_the_frame() {
        let text = plain_tty().panel_at(
            "a title far wider than this little box",
            "x",
            PanelKind::Accent,
            24,
        );
        assert!(text.lines().all(|l| l.chars().count() == 24), "{text}");
    }

    #[test]
    fn a_panel_only_paints_the_frame_when_colour_is_on() {
        let plain = plain_tty().panel_at("Error", "boom", PanelKind::Error, 30);
        assert!(!plain.contains('\u{1b}'));
        let painted = colored().panel_at("Error", "boom", PanelKind::Error, 30);
        assert!(painted.contains('\u{1b}'));
        assert_eq!(
            console::strip_ansi_codes(&painted).to_string(),
            plain,
            "colour changes nothing about the shape"
        );
    }

    #[test]
    fn panel_or_falls_back_to_the_plain_line_off_a_terminal() {
        let plain = "CHAP is not running; start it with `chaps up`";
        assert_eq!(
            Out::default().panel_or("Not running", "x", PanelKind::Warning, plain),
            plain
        );
        assert!(
            plain_tty()
                .panel_or("Not running", "x", PanelKind::Warning, plain)
                .starts_with("╭─ Not running")
        );
    }

    #[test]
    fn human_error_lists_causes() {
        let out = Out::default();
        let err = anyhow::anyhow!("root cause")
            .context("middle")
            .context("top");
        assert_eq!(
            out.error(&err),
            "error: top\n  caused by: middle\n  caused by: root cause"
        );
    }

    #[test]
    fn an_error_on_a_terminal_is_a_panel_with_the_hint_on_its_own_line() {
        let out = plain_tty();
        let err = anyhow::anyhow!("compose files are out of date with .chaps/; run `chaps sync`");
        let text = out.error(&err);
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("╭─ Error "), "{text}");
        assert!(lines[1].contains("compose files are out of date with .chaps/"));
        assert!(
            !lines[1].contains("chaps sync"),
            "the hint moved down a line"
        );
        assert!(lines[2].contains("run `chaps sync`"));
        assert!(lines[3].starts_with('╰'));
    }

    #[test]
    fn json_error_is_an_object_with_causes() {
        let out = Out {
            json: true,
            ..Out::default()
        };
        let err = anyhow::anyhow!("root cause").context("top");
        let value: serde_json::Value = serde_json::from_str(&out.error(&err)).unwrap();
        assert_eq!(value["error"], "top");
        assert_eq!(value["causes"], serde_json::json!(["root cause"]));
    }

    #[test]
    fn hint_lines_split_only_in_front_of_a_hint() {
        assert_eq!(
            hint_lines("CHAP is not running; start it with `chaps up`"),
            vec!["CHAP is not running", "start it with `chaps up`"]
        );
        assert_eq!(
            hint_lines("compose files are out of date with .chaps/; run `chaps sync`"),
            vec![
                "compose files are out of date with .chaps/",
                "run `chaps sync`"
            ]
        );
        assert_eq!(
            hint_lines("one thing; another thing"),
            vec!["one thing; another thing"],
            "a plain semicolon is not a hint"
        );
        assert_eq!(
            hint_lines("a; b; run `x`"),
            vec!["a; b", "run `x`"],
            "only the semicolon in front of the hint splits"
        );
    }

    #[test]
    fn fields_align_labels_and_skip_empty_values() {
        let text = fields(
            2,
            &[
                ("id", "chapkit_ewars_model".into()),
                ("service", "chapkit-ewars-model".into()),
                ("citation", String::new()),
                ("covariates", "required: rainfall\ndefaults: none".into()),
            ],
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "  id          chapkit_ewars_model");
        assert_eq!(lines[1], "  service     chapkit-ewars-model");
        assert_eq!(lines[2], "  covariates  required: rainfall");
        assert_eq!(lines[3], "              defaults: none");
        assert!(lines.iter().all(|l| !l.ends_with(' ')));
        assert!(!text.contains("citation"));
    }

    #[test]
    fn styled_labels_keep_the_same_column() {
        let out = colored();
        let entries = [
            ("id", "chapkit_ewars_model".to_string()),
            ("service", "chapkit-ewars-model".to_string()),
        ];
        let styled = fields_with(2, &entries, &|label| out.dim(label));
        assert_eq!(
            console::strip_ansi_codes(&styled).to_string(),
            fields(2, &entries)
        );
    }

    #[test]
    fn wrap_breaks_on_words_and_keeps_paragraphs() {
        assert_eq!(
            wrap("one two three four", 9),
            vec!["one two", "three", "four"]
        );
        assert_eq!(wrap("a\n\nb", 10), vec!["a", "", "b"]);
        assert_eq!(
            wrap("ghcr.io/dhis2-chap/chapkit-r-inla", 10),
            vec!["ghcr.io/dhis2-chap/chapkit-r-inla"],
            "a long word is never split"
        );
    }

    #[test]
    fn human_age_uses_the_largest_unit() {
        assert_eq!(human_age(Duration::from_secs(0)), "0 seconds");
        assert_eq!(human_age(Duration::from_secs(1)), "1 second");
        assert_eq!(human_age(Duration::from_secs(90)), "1 minute");
        assert_eq!(human_age(Duration::from_secs(3 * 3600)), "3 hours");
        assert_eq!(human_age(Duration::from_secs(50 * 3600)), "2 days");
    }

    #[test]
    fn ago_abbreviates_the_same_units() {
        assert_eq!(ago(Duration::from_secs(0)), "0s ago");
        assert_eq!(ago(Duration::from_secs(12)), "12s ago");
        assert_eq!(ago(Duration::from_secs(59)), "59s ago");
        assert_eq!(ago(Duration::from_secs(180)), "3m ago");
        assert_eq!(ago(Duration::from_secs(3 * 3600)), "3h ago");
        assert_eq!(ago(Duration::from_secs(50 * 3600)), "2d ago");
        // The same boundary human_age uses, so the two never disagree.
        assert_eq!(ago(Duration::from_secs(60)), "1m ago");
        assert_eq!(human_age(Duration::from_secs(60)), "1 minute");
    }

    /// 2026-09-25T13:04:00Z, which is the hour a rate-limit reset lands on.
    const RESET: u64 = 1_758_805_440;

    #[test]
    fn a_clock_time_is_the_local_hour_and_minute() {
        assert_eq!(clock_of(RESET, 0), "13:04");
        // Two hours east and eight hours west of it.
        assert_eq!(clock_of(RESET, 2 * 3600), "15:04");
        assert_eq!(clock_of(RESET, -8 * 3600), "05:04");
        // An offset that is not a whole hour, and one that crosses midnight.
        assert_eq!(clock_of(RESET, 5 * 3600 + 1800), "18:34");
        assert_eq!(clock_of(RESET, 11 * 3600), "00:04");
        // A clock before 1970 is not a thing this prints.
        assert_eq!(clock_of(0, -3600), "00:00");
        // Whatever this machine's zone is, the answer is a clock.
        assert!(
            regex::Regex::new(r"^\d{2}:\d{2}$")
                .unwrap()
                .is_match(&local_clock(RESET)),
            "{}",
            local_clock(RESET)
        );
    }

    /// The zone file reader, against a version 1 file built here: two
    /// transitions, and a type table with the offsets they select.
    #[test]
    fn the_zone_file_gives_the_offset_in_force() {
        fn tzif(version: u8, transitions: &[(i32, u8)], offsets: &[i32]) -> Vec<u8> {
            let mut out = b"TZif".to_vec();
            out.push(version);
            out.extend(std::iter::repeat_n(0u8, 15));
            for count in [
                0u32,
                0,
                0,
                transitions.len() as u32,
                offsets.len() as u32,
                0,
            ] {
                out.extend(count.to_be_bytes());
            }
            for (when, _) in transitions {
                out.extend(when.to_be_bytes());
            }
            for (_, which) in transitions {
                out.push(*which);
            }
            for offset in offsets {
                out.extend(offset.to_be_bytes());
                out.push(0);
                out.push(0);
            }
            out
        }

        // Standard time until the first transition, summer time after it,
        // standard time again after the second.
        let file = tzif(
            b'\0',
            &[(1_743_296_400, 1), (1_761_440_400, 0)],
            &[3600, 7200],
        );
        assert_eq!(tzif_offset(&file, 1_700_000_000), Some(3600), "before both");
        assert_eq!(tzif_offset(&file, RESET as i64), Some(7200), "in between");
        assert_eq!(tzif_offset(&file, 1_800_000_000), Some(3600), "after both");

        // A zone that never changes carries no transitions at all.
        let fixed = tzif(b'\0', &[], &[-18_000]);
        assert_eq!(tzif_offset(&fixed, RESET as i64), Some(-18_000));

        // Anything that is not a zone file, and a truncated one, are nothing
        // rather than a wrong hour.
        assert_eq!(tzif_offset(b"not a zone file at all", RESET as i64), None);
        assert_eq!(tzif_offset(&file[..30], RESET as i64), None);
        assert_eq!(tzif_offset(&[], 0), None);
    }

    /// A version 2 file carries the 32-bit table first and the one that is
    /// actually read second; `zic` leaves the first empty, and reading it
    /// would put every machine on UTC.
    #[test]
    fn a_version_2_zone_file_is_read_from_its_second_block() {
        let mut out = b"TZif2".to_vec();
        out.extend(std::iter::repeat_n(0u8, 15));
        // The empty 32-bit block: no transitions, one type of no interest.
        for count in [0u32, 0, 0, 0, 1, 0] {
            out.extend(count.to_be_bytes());
        }
        out.extend(0i32.to_be_bytes());
        out.extend([0u8, 0]);
        // The 64-bit block, with the offset that has to win.
        out.extend(b"TZif2");
        out.extend(std::iter::repeat_n(0u8, 15));
        for count in [0u32, 0, 0, 1, 2, 0] {
            out.extend(count.to_be_bytes());
        }
        out.extend(1_743_296_400i64.to_be_bytes());
        out.push(1);
        for offset in [3600i32, 7200] {
            out.extend(offset.to_be_bytes());
            out.extend([0u8, 0]);
        }

        assert_eq!(tzif_offset(&out, RESET as i64), Some(7200));
        assert_eq!(tzif_offset(&out, 1_700_000_000), Some(3600));
    }
}
