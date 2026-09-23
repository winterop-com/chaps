//! Human and `--json` output.
//!
//! Every command renders through [`Out`] so `--json` is uniform: the same
//! value is either pretty-printed as JSON or handed to a human formatter.

use crate::error::Result;
use serde::Serialize;
use std::io::Write;
use std::time::Duration;

/// Width human prose is wrapped to.
pub const WRAP_WIDTH: usize = 80;

/// Output mode for one CLI invocation.
#[derive(Debug, Clone, Copy, Default)]
pub struct Out {
    pub json: bool,
}

impl Out {
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

    /// Render a table with two spaces between columns and no trailing
    /// whitespace. Returns the text, ending in a newline when non-empty.
    ///
    /// Cells are left-aligned and padded to the widest cell in their column.
    /// The last column is never padded, so no line ends in a space and the
    /// output pastes cleanly into a terminal or a bug report.
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
            push_row(&mut out, headers.iter().copied(), &widths);
        }
        for row in rows {
            push_row(&mut out, row.iter().map(String::as_str), &widths);
        }
        out
    }

    /// Render an error: a JSON object under `--json`, otherwise `error: ...`
    /// followed by one indented line per cause. The result has no trailing
    /// newline.
    pub fn error(&self, err: &anyhow::Error) -> String {
        let causes: Vec<String> = err.chain().skip(1).map(|c| c.to_string()).collect();
        if self.json {
            let value = serde_json::json!({
                "error": err.to_string(),
                "causes": causes,
            });
            serde_json::to_string_pretty(&value)
                .unwrap_or_else(|_| format!("{{\"error\":\"{}\"}}", err))
        } else {
            let mut out = format!("error: {err}");
            for cause in causes {
                out.push_str(&format!("\n  caused by: {cause}"));
            }
            out
        }
    }
}

/// Print a one-line warning to stderr.
///
/// Warnings never go to stdout: a `--json` consumer must be able to pipe
/// stdout straight into a parser.
pub fn warn(message: &str) {
    eprintln!("warning: {message}");
}

/// Render `label  value` pairs with the labels padded to a common width.
///
/// Entries whose value is empty are skipped, and a value containing newlines
/// keeps the value column on its continuation lines. The result ends in a
/// newline when it is non-empty.
pub fn fields(indent: usize, entries: &[(&str, String)]) -> String {
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
                out.push_str(key);
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

fn push_row<'a>(out: &mut String, cells: impl Iterator<Item = &'a str>, widths: &[usize]) {
    let cells: Vec<&str> = cells.collect();
    let last = cells.len().saturating_sub(1);
    for (i, cell) in cells.iter().enumerate() {
        out.push_str(cell);
        if i != last {
            let pad = widths[i].saturating_sub(display_width(cell)) + 2;
            out.push_str(&" ".repeat(pad));
        }
    }
    out.push('\n');
}

/// Column width in characters. Good enough for the identifiers and short
/// labels the tables hold; no attempt at full grapheme or East Asian width.
fn display_width(s: &str) -> usize {
    s.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn human_error_lists_causes() {
        let out = Out { json: false };
        let err = anyhow::anyhow!("root cause")
            .context("middle")
            .context("top");
        assert_eq!(
            out.error(&err),
            "error: top\n  caused by: middle\n  caused by: root cause"
        );
    }

    #[test]
    fn json_error_is_an_object_with_causes() {
        let out = Out { json: true };
        let err = anyhow::anyhow!("root cause").context("top");
        let value: serde_json::Value = serde_json::from_str(&out.error(&err)).unwrap();
        assert_eq!(value["error"], "top");
        assert_eq!(value["causes"], serde_json::json!(["root cause"]));
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
}
