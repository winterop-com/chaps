//! Reading what `chapkit test` printed: the summary block, the phase a
//! failure belongs to, and the one clause of it worth a row.

use super::{MAX_SUMMARY, plural};

/// The heading chapkit prints above the block this parser reads.
pub const SUMMARY_MARKER: &str = "TEST SUMMARY";

/// The counts and the verdict of one `chapkit test` run.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TestSummary {
    pub elapsed: Option<f64>,
    pub configs_created: u32,
    pub trainings_completed: u32,
    pub trainings_failed: u32,
    pub predictions_completed: u32,
    pub predictions_failed: u32,
    pub validations_run: u32,
    pub validations_failed: u32,
    /// `Result: ALL TESTS PASSED`.
    pub passed: bool,
    /// Every `[FAILED] ...` line, in the order chapkit printed them.
    pub failures: Vec<String>,
}

impl TestSummary {
    /// The cell a passing run prints: what the model actually did.
    pub fn did(&self) -> String {
        format!(
            "{} {}, {} {}",
            self.trainings_completed,
            plural(self.trainings_completed as usize, "training"),
            self.predictions_completed,
            plural(self.predictions_completed as usize, "prediction")
        )
    }

    /// The cell a failing run prints: which phase, and why.
    pub fn why(&self) -> String {
        match self.failures.first() {
            Some(line) => {
                let phase = phase_of(line);
                match failure_reason(line) {
                    Some(reason) => format!("{phase}: {reason}"),
                    None => format!("{phase} failed"),
                }
            }
            None => {
                let mut parts = Vec::new();
                if self.trainings_failed > 0 {
                    parts.push(format!("{} training failed", self.trainings_failed));
                }
                if self.predictions_failed > 0 {
                    parts.push(format!("{} prediction failed", self.predictions_failed));
                }
                if parts.is_empty() {
                    parts.push("no training ran".to_string());
                }
                parts.join(", ")
            }
        }
    }
}

/// Read chapkit's summary block out of everything the command printed.
///
/// `None` when there is no block at all, which is how a `chapkit` that is not
/// in the image, or one that died before it got that far, comes back. Both
/// streams are searched: the counts go to stdout and the `[FAILED]` lines to
/// stderr, and a caller that kept them apart would have half the answer.
pub fn parse_summary(text: &str) -> Option<TestSummary> {
    if !text.contains(SUMMARY_MARKER) {
        return None;
    }
    let mut summary = TestSummary::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Elapsed time:") {
            summary.elapsed = rest.trim().trim_end_matches('s').parse::<f64>().ok();
        } else if let Some(n) = count_after(line, "Configs created:") {
            summary.configs_created = n;
        } else if let Some(n) = count_after(line, "Trainings completed:") {
            summary.trainings_completed = n;
        } else if let Some(n) = count_after(line, "Trainings failed:") {
            summary.trainings_failed = n;
        } else if let Some(n) = count_after(line, "Predictions completed:") {
            summary.predictions_completed = n;
        } else if let Some(n) = count_after(line, "Predictions failed:") {
            summary.predictions_failed = n;
        } else if let Some(n) = count_after(line, "Validations run:") {
            summary.validations_run = n;
        } else if let Some(n) = count_after(line, "Validations failed:") {
            summary.validations_failed = n;
        } else if let Some(rest) = line.strip_prefix("Result:") {
            summary.passed = rest.trim() == "ALL TESTS PASSED";
        } else if let Some(rest) = line.strip_prefix("[FAILED]") {
            summary.failures.push(rest.trim().to_string());
        }
    }
    Some(summary)
}

/// Whether the failure is that the image has no `chapkit test` to run.
///
/// Only ever asked once [`parse_summary`] has come back empty, because
/// `not found` is a phrase a model's own error message may well contain.
pub fn chapkit_missing(code: i32, text: &str) -> bool {
    if code == crate::docker::DOCKER_NOT_FOUND {
        return true;
    }
    let lower = text.to_ascii_lowercase();
    [
        "no such command",
        "executable file not found",
        "chapkit: not found",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// The word the row uses for the phase a `[FAILED]` line is about.
fn phase_of(line: &str) -> &'static str {
    let lower = line.to_ascii_lowercase();
    if lower.starts_with("training") || lower.contains("train script") {
        "train"
    } else if lower.starts_with("prediction") || lower.contains("predict script") {
        "predict"
    } else if lower.contains("validate") || lower.contains("validation") {
        "validate"
    } else if lower.starts_with("config") {
        "config"
    } else {
        "test"
    }
}

/// The one clause of a `[FAILED]` line worth putting in a row.
///
/// chapkit ends such a line with `stderr tail: <last five lines joined by
/// " | ">`, so the reason is in there and the rest is bookkeeping about
/// artifact ids. The rule for picking one of those five is [`crate::jobs`]'s,
/// which is the rule `varde jobs logs` already uses on a failed job's log.
fn failure_reason(line: &str) -> Option<String> {
    let tail = match line.split_once("stderr tail:") {
        Some((_, tail)) => tail,
        // No tail at all: whatever chapkit said after the job id is the
        // message, and there is nothing better to show.
        None => line.split_once(": ").map(|(_, rest)| rest)?,
    };
    let segments: Vec<&str> = tail
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }
    let at = crate::jobs::error_line(&segments).unwrap_or(segments.len() - 1);
    Some(crate::jobs::cut(&first_sentence(segments[at]), MAX_SUMMARY))
}

/// The one line of a job log most likely to say why it failed, for a log with
/// no `--- stderr ---` section in it.
///
/// [`crate::jobs::stderr_hint`] is the first thing to try, because a model's
/// own error stream beats chap-core's record of it. But a job that failed
/// inside chap-core - a covariate the configured model wants and the dataset
/// does not have, say - never reaches the model at all, and then the answer is
/// the last line of chap-core's own traceback rather than nothing.
pub fn log_hint(text: &str) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    // The tail only: an error line from the top of a long log is something
    // that was recovered from half an hour of work ago.
    let from = lines.len().saturating_sub(LOG_TAIL);
    let tail = &lines[from..];
    let at = crate::jobs::error_line(tail)?;
    Some(crate::jobs::cut(tail[at], MAX_SUMMARY))
}

/// How many lines from the end of a job log [`log_hint`] looks at.
const LOG_TAIL: usize = 20;

/// The first sentence of `text`, where a sentence ends on `. ` or `! `.
///
/// A model's error is usually one line with the reason at the front and a
/// path, a line number or a suggestion behind it; the front of it is what a
/// row has room for.
pub fn first_sentence(text: &str) -> String {
    let text = text.trim();
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if matches!(c, '.' | '!' | '?')
            && chars
                .peek()
                .is_some_and(|(_, next)| next.is_whitespace())
            // `chapkit 2.0.0 failed` has no full stop; `R 4.3.1` does, and
            // cutting there would leave a version number as the whole reason.
            && !text[..at].ends_with(|c: char| c.is_ascii_digit())
        {
            return text[..=at].trim_end_matches(['.', '!', '?']).to_string();
        }
    }
    text.to_string()
}

/// `Trainings failed:   0` as a number.
fn count_after(line: &str, label: &str) -> Option<u32> {
    line.strip_prefix(label)?.trim().parse::<u32>().ok()
}
