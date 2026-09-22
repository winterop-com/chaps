//! Human and `--json` output.
//!
//! Every command renders through [`Out`] so `--json` is uniform: the same
//! value is either pretty-printed as JSON or handed to a human formatter.

use crate::error::Result;
use serde::Serialize;
use std::io::Write;

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
}
