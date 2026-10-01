//! Human and `--json` output, and the one place that knows about colour.
//!
//! Every command renders through [`Out`] so `--json` is uniform: the same
//! value is either pretty-printed as JSON or handed to a human formatter.
//! [`Out`] also carries the two facts that decide how the human rendering
//! looks: whether stdout is a terminal, and whether colour is wanted. Nothing
//! else in the crate calls into `console`.

mod clock;

pub use clock::{ago, human_age, local_clock};

use crate::error::Result;
use console::{Style, measure_text_width};
use serde::Serialize;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Width human prose is wrapped to.
pub const WRAP_WIDTH: usize = 80;

/// Say nothing but the answer.
pub const QUIET: u8 = 0;
/// `-v`: narrate what runs.
pub const VERBOSE: u8 = 1;
/// `-d`: narrate what runs and what came back.
pub const DEBUG: u8 = 2;

/// How much of a response body a `-d` line prints.
pub const MAX_TRACE_BODY: usize = 2048;

/// Output mode for one CLI invocation.
///
/// `Default` is the plain, colourless, non-terminal mode: that is what the
/// unit tests render against, and what a pipe gets.
#[derive(Debug, Clone, Copy, Default)]
pub struct Out {
    pub json: bool,
    /// Paint with ANSI styles. Implies [`Out::tty`] in practice, but the two
    /// are tracked apart so `NO_COLOR` on a terminal still gets a terminal's
    /// behaviour (prompts, the update notice) without the colour.
    pub color: bool,
    /// Whether stdout is a terminal. It changes behaviour, never the words: a
    /// terminal gets the same lines a pipe does, so what a person pastes is
    /// what a script parses.
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

    /// Render an error: a JSON object under `--json`, otherwise `error: ...`
    /// followed by one indented line per cause, on a terminal too (only the
    /// label is coloured), so the fix on the line pastes as it reads. The
    /// result has no trailing newline.
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

        let mut text = format!(
            "{} {}",
            self.paint("error:", Style::new().red().bold()),
            self.backticks(&err.to_string())
        );
        for cause in &causes {
            text.push_str(&format!(
                "\n  {} {}",
                self.dim("caused by:"),
                self.backticks(cause)
            ));
        }
        text
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

/// A request or response body, with any bearer token masked, cut to
/// [`MAX_TRACE_BODY`] bytes on a character boundary.
///
/// The mask is here rather than at each caller because a body is where a token
/// travels without anyone deciding it should: the DHIS2 route payload carries
/// chap-core's token in its `auth` headers, and `--debug` output ends up in bug
/// reports.
pub fn trace_body(body: &str) -> String {
    let body = mask_bearer(body);
    if body.len() <= MAX_TRACE_BODY {
        return body;
    }
    let mut end = MAX_TRACE_BODY;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}... ({} bytes total)", &body[..end], body.len())
}

/// `Bearer <anything up to a quote or whitespace>` with the value replaced.
fn mask_bearer(text: &str) -> String {
    const WORD: &str = "Bearer ";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(WORD) {
        out.push_str(&rest[..at + WORD.len()]);
        rest = &rest[at + WORD.len()..];
        let end = rest
            .find(|c: char| c == '"' || c == '\\' || c.is_whitespace())
            .unwrap_or(rest.len());
        if end > 0 {
            out.push_str("<redacted>");
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
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

/// Wrap `text` and join it back into a block, one line per wrapped line.
pub fn wrapped(text: &str, width: usize) -> String {
    wrap(text, width).join("\n")
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
mod tests;
