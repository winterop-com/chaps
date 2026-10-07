//! What a command says when it is done, as lines with a level.
//!
//! A person sees the info lines and the warnings. `-v` adds the hints: the
//! next commands and the background that a person does not always need.
//! `--json` carries every line with its level, next to the command's value.

use super::{Out, Style};
use crate::error::Result;
use serde::Serialize;
use std::io::Write;

/// How much a line matters to the person who ran the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// What the command did or found. Always shown.
    Info,
    /// A next command or background. Shown under `-v`.
    Hint,
    /// Something that needs attention. Always shown, on stderr.
    Warning,
}

/// One line of a [`Report`].
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub level: Level,
    pub text: String,
}

/// The lines a command prints when it is done.
#[derive(Debug, Default)]
pub struct Report {
    messages: Vec<Message>,
}

impl Report {
    /// What the command did or found.
    pub fn info(&mut self, text: impl Into<String>) -> &mut Self {
        self.push(Level::Info, text)
    }

    /// A next command or background, shown under `-v`.
    pub fn hint(&mut self, text: impl Into<String>) -> &mut Self {
        self.push(Level::Hint, text)
    }

    /// Something that needs attention.
    pub fn warning(&mut self, text: impl Into<String>) -> &mut Self {
        self.push(Level::Warning, text)
    }

    fn push(&mut self, level: Level, text: impl Into<String>) -> &mut Self {
        self.messages.push(Message {
            level,
            text: text.into(),
        });
        self
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Every line, with `hint:` and `warning:` in front of those levels,
    /// for the tests that read a rendering.
    #[cfg(test)]
    pub(crate) fn text(&self) -> String {
        self.messages
            .iter()
            .map(|message| match message.level {
                Level::Info => format!("{}\n", message.text),
                Level::Hint => format!("hint: {}\n", message.text),
                Level::Warning => format!("warning: {}\n", message.text),
            })
            .collect()
    }

    /// The human rendering: the text for stdout and the text for stderr.
    pub(crate) fn render(&self, verbose: bool, color: bool) -> (String, String) {
        let paint = |text: &str, style: Style| match color {
            true => style.force_styling(true).apply_to(text).to_string(),
            false => text.to_string(),
        };
        let mut stdout = String::new();
        let mut stderr = String::new();
        for message in &self.messages {
            match message.level {
                Level::Info => {
                    stdout.push_str(&message.text);
                    stdout.push('\n');
                }
                Level::Hint if verbose => {
                    stdout.push_str(&paint(
                        &format!("hint: {}", message.text),
                        Style::new().dim(),
                    ));
                    stdout.push('\n');
                }
                Level::Hint => {}
                Level::Warning => {
                    stderr.push_str(&paint("warning:", Style::new().yellow().bold()));
                    stderr.push(' ');
                    stderr.push_str(&message.text);
                    stderr.push('\n');
                }
            }
        }
        (stdout, stderr)
    }
}

impl Out {
    /// Print what a command did. Under `--json`, print `value` with a
    /// `messages` list of every line and its level; otherwise print the
    /// lines that the verbosity lets through.
    pub fn report<T: Serialize>(&self, value: &T, build: impl FnOnce(&mut Report)) -> Result<()> {
        self.report_with(value, None, build)
    }

    /// [`Out::report`] for a command that changed something: under `--json`
    /// the document also says `"ok": true`, the counterpart of the `"ok": false`
    /// an error carries, so a caller can branch on one field.
    pub fn report_ok<T: Serialize>(
        &self,
        value: &T,
        build: impl FnOnce(&mut Report),
    ) -> Result<()> {
        self.report_with(value, Some(true), build)
    }

    fn report_with<T: Serialize>(
        &self,
        value: &T,
        ok: Option<bool>,
        build: impl FnOnce(&mut Report),
    ) -> Result<()> {
        let mut report = Report::default();
        build(&mut report);
        if self.json {
            let mut value = serde_json::to_value(value)?;
            if let serde_json::Value::Object(map) = &mut value {
                if let Some(ok) = ok {
                    map.insert("ok".to_string(), serde_json::Value::Bool(ok));
                }
                map.insert(
                    "messages".to_string(),
                    serde_json::to_value(report.messages())?,
                );
            }
            return self.emit(&value, String::new);
        }
        let (stdout, stderr) = report.render(self.shows_hints(), self.color);
        if !stderr.is_empty() {
            eprint!("{stderr}");
        }
        let out = std::io::stdout();
        let mut w = out.lock();
        write!(w, "{stdout}")?;
        w.flush()?;
        Ok(())
    }
}
