//! What `chaps models test` proves, and how it reads on a terminal.
//!
//! Registration is a heartbeat. A model that answers its healthcheck and pings
//! chap-core every thirty seconds can still be unable to produce a single
//! prediction - the user it runs as cannot write, a library is missing from
//! the image, the covariates it declares are not the ones it reads - and
//! neither `chaps status` nor `chaps doctor` can tell. The only way to know is
//! to make the model do the work.
//!
//! Everything here is pure: the parsing of what `chapkit test` printed, the
//! transposition of a sample frame into the observations chap-core's
//! `make-dataset` takes, and the rendering of a row. The `docker compose exec`
//! and the requests live in [`crate::commands::modeltest`], so every rule in
//! this file can be tested without docker and without a server.

use crate::error::Result;
use serde::Serialize;
use std::time::Duration;

/// Seconds one model gets at the model level before chaps gives up on it.
///
/// Passed to `chapkit test --timeout` as well, so chapkit abandons a single
/// job no later than chaps abandons the whole run. Five minutes is roughly
/// twenty times the slowest marketplace model's honest time.
pub const MODEL_TIMEOUT: u64 = 300;

/// Seconds one model gets at the backtest level.
///
/// A backtest is two chap-core jobs and `nSplits` fits of the model, so it is
/// an order of magnitude slower than the model level: the GHR model takes
/// north of two minutes on an idle laptop.
pub const BACKTEST_TIMEOUT: u64 = 900;

/// Org units in the generated sample data.
pub const SAMPLE_LOCATIONS: usize = 5;

/// Periods in the generated sample data, per org unit.
///
/// Three years of monthly data. A backtest needs at least
/// `nPeriods + (nSplits - 1) * stride + 1` periods; this is far above that,
/// and long enough for a model with a season or a lag to have something to
/// learn.
pub const SAMPLE_PERIODS: usize = 36;

/// The backtest chaps asks for: three periods ahead, two splits, stride one.
///
/// The smallest backtest that still exercises a rolling split, because this
/// is a check that the model runs and not a measurement of how well it does.
pub const BACKTEST_PERIODS: u32 = 3;
/// See [`BACKTEST_PERIODS`].
pub const BACKTEST_SPLITS: u32 = 2;
/// See [`BACKTEST_PERIODS`].
pub const BACKTEST_STRIDE: u32 = 1;

/// The chapkit release that first served `$generate-sample-data`.
pub const SAMPLE_DATA_CHAPKIT: &str = "1.1.0";

/// Spaces between the model column and the verdict.
const NAME_GAP: usize = 4;

/// Spaces between every other pair of columns.
const CELL_GAP: usize = 3;

/// The narrowest the time column gets, which is what `17s` needs.
///
/// A run that took single-digit seconds is right-aligned into it rather than
/// pushing its summary a character left of every other row's.
const TIME_WIDTH: usize = 3;

/// The metrics the backtest row prints, in this order.
///
/// Three of the dozen chap-core computes: the probabilistic score the
/// marketplace ranks on, and the two error measures that are read in the
/// units of the data.
pub const METRICS: &[&str] = &["crps", "mae", "rmse"];

/// How much of a failure line the summary cell keeps.
const MAX_SUMMARY: usize = 120;

/// The column of the sample frame that holds the period.
pub const TIME_COLUMN: &str = "time_period";

/// The column of the sample frame that holds the org unit.
pub const LOCATION_COLUMN: &str = "location";

/// Which of the two questions a run answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// The model service on its own, through `chapkit test` in its container.
    Model,
    /// The model through chap-core: a dataset, a backtest, and its metrics.
    Backtest,
}

/// How one model came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
    Skip,
}

impl Verdict {
    /// The word the row prints, which is upper-case only for a failure: a
    /// screen of `pass` lines should have one thing on it that catches an eye.
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Fail => "FAIL",
            Verdict::Skip => "skip",
        }
    }
}

/// One model's run: the row, and everything `--json` hands back.
#[derive(Debug, Clone, Serialize)]
pub struct Run {
    /// The marketplace id, which is what an operator types.
    pub id: String,
    /// The compose service name, which is what the row prints.
    pub service_id: String,
    pub level: Level,
    #[serde(rename = "result")]
    pub verdict: Verdict,
    /// Wall-clock seconds this model took.
    pub seconds: u64,
    /// The last cell of the row: what happened, in one clause.
    pub summary: String,
    /// The way out, printed indented under the row. `None` for a pass, which
    /// needs nothing done about it.
    pub detail: Option<String>,
    /// The chap-core job the backtest level ended on, for `chaps jobs logs`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backtest_id: Option<i64>,
    /// chap-core's `aggregateMetrics`, whole, for a backtest that finished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<serde_json::Value>,
}

impl Run {
    /// A run that has nothing but its name yet.
    pub fn new(id: &str, service_id: &str, level: Level) -> Run {
        Run {
            id: id.to_string(),
            service_id: service_id.to_string(),
            level,
            verdict: Verdict::Skip,
            seconds: 0,
            summary: String::new(),
            detail: None,
            job_id: None,
            backtest_id: None,
            metrics: None,
        }
    }

    /// Record a verdict, its one-clause summary and the way out.
    pub fn end(
        mut self,
        verdict: Verdict,
        summary: impl Into<String>,
        detail: Option<String>,
    ) -> Run {
        self.verdict = verdict;
        self.summary = summary.into();
        self.detail = detail;
        self
    }
}

/// The line a run opens with.
pub fn header(count: usize, level: Level) -> String {
    let what = format!("testing {count} {}", plural(count, "model"));
    match level {
        Level::Model => {
            format!("{what} (model level; add --backtest to run them through chap-core)")
        }
        Level::Backtest => {
            format!("{what} (through chap-core: a dataset, a backtest and its scores)")
        }
    }
}

/// The width of the model column: the longest service id there is.
pub fn name_width(runs: &[String]) -> usize {
    runs.iter()
        .map(|name| name.chars().count())
        .max()
        .unwrap_or(0)
}

/// One row: the model, the verdict, how long it took and what happened.
///
/// `verdict` is handed in rather than read off the run because the caller has
/// coloured it by then. The model column is padded and nothing else is, so a
/// row with a long summary does not stretch the ones above it, and the line
/// never ends in a space.
pub fn row(run: &Run, verdict: &str, width: usize) -> String {
    let (name, time, summary) = (&run.service_id, took(run.seconds), &run.summary);
    let pad = width.saturating_sub(name.chars().count());
    let time_pad = TIME_WIDTH.saturating_sub(time.chars().count());
    let line = format!(
        "{name}{}{}{verdict}{}{}{time}{}{summary}",
        " ".repeat(pad),
        " ".repeat(NAME_GAP),
        " ".repeat(CELL_GAP),
        " ".repeat(time_pad),
        " ".repeat(CELL_GAP),
    );
    line.trim_end().to_string()
}

/// How long a run took, as the row prints it: `17s`, `2m 9s`.
pub fn took(seconds: u64) -> String {
    crate::jobs::took_text(Duration::from_secs(seconds))
}

/// The line the run adds up to.
///
/// `4 of 5 models pass` while every model was tried; a run with skips in it
/// counts the three buckets instead, because "4 of 5" would be read as one
/// model having failed. Either way it ends on the command that shows why.
pub fn closing(runs: &[Run]) -> String {
    let count = |want: Verdict| runs.iter().filter(|r| r.verdict == want).count();
    let (passed, failed, skipped) = (
        count(Verdict::Pass),
        count(Verdict::Fail),
        count(Verdict::Skip),
    );
    let mut line = if skipped == 0 {
        format!(
            "{passed} of {} {} pass",
            runs.len(),
            plural(runs.len(), "model")
        )
    } else {
        let mut parts = vec![format!("{passed} pass")];
        if failed > 0 {
            parts.push(format!("{failed} fail"));
        }
        parts.push(format!("{skipped} skipped"));
        parts.join(", ")
    };
    if failed > 0 {
        let which = match runs.iter().find(|r| r.verdict == Verdict::Fail) {
            Some(run) if failed == 1 => run.id.clone(),
            _ => "<id>".to_string(),
        };
        line.push_str(&format!(
            "; run `chaps models test {which} -v` for the full output"
        ));
    }
    line
}

/// Whether anything failed, which is the only thing that makes the run
/// non-zero: a skip is a question that could not be asked, not a bad answer.
pub fn any_failed(runs: &[Run]) -> bool {
    runs.iter().any(|r| r.verdict == Verdict::Fail)
}

// ---------------------------------------------------------------------------
// Reading what `chapkit test` printed
// ---------------------------------------------------------------------------

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
/// which is the rule `chaps jobs logs` already uses on a failed job's log.
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

// ---------------------------------------------------------------------------
// Turning chapkit's sample frame into a chap-core dataset
// ---------------------------------------------------------------------------

/// One value of one feature, for one org unit, in one period.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Observation {
    pub period: String,
    #[serde(rename = "orgUnit")]
    pub org_unit: String,
    pub value: Option<f64>,
    #[serde(rename = "featureName")]
    pub feature_name: String,
}

/// The observations and the org units of one `$generate-sample-data` frame.
///
/// chapkit hands back a column-oriented frame - a list of names and a list of
/// rows - and chap-core takes one object per value, so every column but
/// `time_period` and `location` becomes a feature name. A cell that is not a
/// number goes over as `null`, which chap-core accepts as a known-missing
/// observation.
pub fn observations(frame: &serde_json::Value) -> Result<(Vec<Observation>, Vec<String>)> {
    let columns: Vec<String> = frame
        .get("columns")
        .and_then(|c| c.as_array())
        .ok_or_else(|| anyhow::anyhow!("the sample data has no `columns` list"))?
        .iter()
        .map(|name| name.as_str().unwrap_or_default().to_string())
        .collect();
    let rows = frame
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow::anyhow!("the sample data has no `data` rows"))?;

    let time_at = index_of(&columns, TIME_COLUMN)?;
    let location_at = index_of(&columns, LOCATION_COLUMN)?;

    let mut observations = Vec::with_capacity(rows.len() * columns.len());
    let mut locations: Vec<String> = Vec::new();
    for row in rows {
        let cells = row
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("a sample data row is not a list"))?;
        let period = period_code(
            cells
                .get(time_at)
                .and_then(|c| c.as_str())
                .unwrap_or_default(),
        );
        let org_unit = cells
            .get(location_at)
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();
        if !locations.iter().any(|seen| seen == &org_unit) {
            locations.push(org_unit.clone());
        }
        for (at, name) in columns.iter().enumerate() {
            if at == time_at || at == location_at {
                continue;
            }
            observations.push(Observation {
                period: period.clone(),
                org_unit: org_unit.clone(),
                value: cells.get(at).and_then(|c| c.as_f64()),
                feature_name: name.clone(),
            });
        }
    }
    Ok((observations, locations))
}

/// The column named `want`, or the error that says the frame is not one.
fn index_of(columns: &[String], want: &str) -> Result<usize> {
    columns
        .iter()
        .position(|name| name == want)
        .ok_or_else(|| anyhow::anyhow!("the sample data has no `{want}` column"))
}

/// chapkit's period as chap-core spells it: `2020-01` -> `202001`,
/// `2020-W01` -> `2020W01`.
pub fn period_code(text: &str) -> String {
    text.trim().replace('-', "")
}

/// The org-unit geometry for the dataset.
///
/// chapkit's generator puts the location id in `properties.id` and nothing at
/// the top level, and chap-core matches org units on the feature's top-level
/// `id` and drops every one it cannot match - silently, as an import that
/// found nothing - so setting it is what makes the dataset non-empty. A model
/// that needs no geometry gets one feature per location with no geometry at
/// all, which is enough for chap-core to know the org units exist.
pub fn feature_collection(
    geo: Option<&serde_json::Value>,
    locations: &[String],
) -> serde_json::Value {
    let Some(geo) = geo.filter(|value| value.get("features").is_some()) else {
        let features: Vec<serde_json::Value> = locations
            .iter()
            .map(|id| {
                serde_json::json!({
                    "type": "Feature",
                    "id": id,
                    "geometry": serde_json::Value::Null,
                    "properties": {"id": id},
                })
            })
            .collect();
        return serde_json::json!({"type": "FeatureCollection", "features": features});
    };

    let mut collection = geo.clone();
    let features = collection
        .get_mut("features")
        .and_then(|f| f.as_array_mut())
        .expect("checked above");
    for (at, feature) in features.iter_mut().enumerate() {
        let existing = feature
            .get("id")
            .and_then(|id| id.as_str())
            .map(str::to_string);
        let from_properties = feature
            .get("properties")
            .and_then(|p| p.get("id"))
            .and_then(|id| id.as_str())
            .map(str::to_string);
        let id = existing
            .or(from_properties)
            .or_else(|| locations.get(at).cloned());
        if let (Some(id), Some(object)) = (id, feature.as_object_mut()) {
            object.insert("id".to_string(), serde_json::json!(id));
        }
    }
    collection
}

/// `crps 20.0  mae 29.0  rmse 34.9` from chap-core's `aggregateMetrics`.
///
/// Only [`METRICS`], because a row is read at a glance and chap-core reports
/// fourteen scores; `--json` hands back all of them.
pub fn metrics_cell(metrics: &serde_json::Value) -> String {
    let parts: Vec<String> = METRICS
        .iter()
        .filter_map(|name| {
            let value = metrics.get(*name)?.as_f64()?;
            Some(format!("{name} {value:.1}"))
        })
        .collect();
    if parts.is_empty() {
        return "no scores reported".to_string();
    }
    parts.join("  ")
}

// ---------------------------------------------------------------------------
// Which of chap-core's configured models a backtest is of
// ---------------------------------------------------------------------------

/// One row of `GET /v1/crud/configured-models`, cut to what the choice needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredModel {
    /// chap-core's primary key, which is what `create-backtest` is given.
    pub id: i64,
    /// chap-core's name for it: the service id for a service that has just
    /// registered, and `<service id>:<config name>` for one whose configs
    /// chap-core has synced.
    pub name: String,
    /// Whether chap-core has retired it. An archived row still has a name and
    /// is still listed; it is simply not a model anything can be run with.
    pub archived: bool,
}

/// The configured models of one listing, ignoring anything that is not a row.
pub fn configured_models(listed: &serde_json::Value) -> Vec<ConfiguredModel> {
    let Some(rows) = listed.as_array() else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            Some(ConfiguredModel {
                id: row.get("id")?.as_i64()?,
                name: row.get("name")?.as_str()?.to_string(),
                archived: row
                    .get("archived")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
            })
        })
        .collect()
}

/// The configured model a backtest of `service_id` has to name.
///
/// `create-backtest` resolves a string `modelId` against these names, and a
/// service is only named plainly for as long as chap-core has nothing else
/// from it: a service that re-registers with a new version has its own stored
/// configs synced as `<service id>:<config name>` instead, and the bare name
/// stops existing. Sending the service id is then a `ValueError` from inside
/// chap-core rather than a backtest, so the row is picked here and its
/// integer id is what goes out.
///
/// The order is the order of how much the row is the service itself: its bare
/// name, then a config of it, and a `test_config_` last of all, because that
/// is what a previous `chaps models test` or `chapkit test` left behind and
/// not a configuration anybody made. Ties go to the lowest id, so two runs of
/// the same command backtest the same model.
pub fn configured_model_for<'a>(
    models: &'a [ConfiguredModel],
    service_id: &str,
) -> Option<&'a ConfiguredModel> {
    let prefix = format!("{service_id}:");
    let left_behind = format!("{prefix}test_config_");
    models
        .iter()
        .filter(|model| !model.archived)
        .filter_map(|model| {
            let rank = if model.name == service_id {
                0
            } else if !model.name.starts_with(&prefix) {
                return None;
            } else if model.name.starts_with(&left_behind) {
                2
            } else {
                1
            };
            Some((rank, model.id, model))
        })
        .min_by_key(|(rank, id, _)| (*rank, *id))
        .map(|(_, _, model)| model)
}

/// `1 model`, `2 models`.
fn plural(n: usize, word: &str) -> String {
    if n == 1 {
        word.to_string()
    } else {
        format!("{word}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a passing `chapkit test` prints, cut to the block this parser
    /// reads plus the rule above it.
    const PASSED: &str = "\
==================================================
TEST SUMMARY
==================================================
Service URL:           http://127.0.0.1:8000
Elapsed time:          16.85s
Configs created:       1
Trainings completed:   1
Trainings failed:      0
Predictions completed: 1
Predictions failed:    0
Validations run:       2
Validations failed:    0

Result: ALL TESTS PASSED
";

    /// The same for a run whose prediction failed: the `[FAILED]` line goes to
    /// stderr and the counts to stdout, and the caller hands both over at once.
    const FAILED: &str = "\
  [FAILED] Prediction job 01M3A4TSB2YHRPFBEFCT4TMX4A: Job failed: predict script failed with \
exit code 1; diagnostic artifact 01M3A4TSB9WZ8V5XN0A4F4Y1M6 holds the workspace, stdout and \
stderr; stderr tail: Loading required package: Matrix | \
Error in inla.inlaprogram.has.crashed() : The inla program crashed. | Execution halted
==================================================
TEST SUMMARY
==================================================
Service URL:           http://127.0.0.1:8000
Elapsed time:          12.12s
Configs created:       1
Trainings completed:   1
Trainings failed:      0
Predictions completed: 0
Predictions failed:    1
Validations run:       2
Validations failed:    0

Result: 1 FAILURE(S)
";

    fn run(id: &str, service_id: &str, verdict: Verdict, seconds: u64, summary: &str) -> Run {
        let mut run = Run::new(id, service_id, Level::Model);
        run.seconds = seconds;
        run.end(verdict, summary, None)
    }

    #[test]
    fn a_passing_run_is_read_off_the_summary_block() {
        let summary = parse_summary(PASSED).expect("the block is there");
        assert!(summary.passed);
        assert_eq!(summary.elapsed, Some(16.85));
        assert_eq!(summary.configs_created, 1);
        assert_eq!(summary.trainings_completed, 1);
        assert_eq!(summary.predictions_completed, 1);
        assert_eq!(summary.validations_run, 2);
        assert_eq!(summary.validations_failed, 0);
        assert!(summary.failures.is_empty());
        assert_eq!(summary.did(), "1 training, 1 prediction");

        // The counts are read, not guessed: two predictions read as two.
        let two =
            parse_summary(&PASSED.replace("Predictions completed: 1", "Predictions completed: 2"))
                .expect("still a block");
        assert_eq!(two.did(), "1 training, 2 predictions");
    }

    #[test]
    fn a_failing_run_names_the_phase_and_the_reason_from_the_stderr_tail() {
        let summary = parse_summary(FAILED).expect("the block is there");
        assert!(!summary.passed);
        assert_eq!(summary.predictions_failed, 1);
        assert_eq!(summary.trainings_completed, 1);
        assert_eq!(summary.failures.len(), 1);
        assert_eq!(
            summary.why(),
            "predict: Error in inla.inlaprogram.has.crashed() : The inla program crashed."
        );

        // A training failure is the other phase, and the package-loading line
        // that came before the error is not the reason.
        let training = FAILED
            .replace("Prediction job", "Training job")
            .replace("predict script", "train script");
        let summary = parse_summary(&training).expect("a block");
        assert!(
            summary.why().starts_with("train: Error in inla"),
            "{}",
            summary.why()
        );

        // No `[FAILED]` line at all - chapkit counted a failure and said
        // nothing else - still reads as something rather than as nothing.
        let quiet = parse_summary(
            &PASSED
                .replace("Predictions failed:    0", "Predictions failed:    1")
                .replace("Result: ALL TESTS PASSED", "Result: 1 FAILURE(S)"),
        )
        .expect("a block");
        assert!(!quiet.passed);
        assert_eq!(quiet.why(), "1 prediction failed");
    }

    #[test]
    fn a_run_with_no_summary_block_reads_as_nothing_at_all() {
        assert!(parse_summary("").is_none());
        assert!(parse_summary("service unavailable\n").is_none());
        // The heading has to be there; the counts alone are not the block.
        assert!(parse_summary("Trainings completed:   1\n").is_none());
    }

    #[test]
    fn an_image_without_chapkit_is_told_apart_from_a_model_that_failed() {
        // `docker compose exec` with nothing to exec.
        assert!(chapkit_missing(
            1,
            "OCI runtime exec failed: exec failed: unable to start container process: \
             exec: \"chapkit\": executable file not found in $PATH: unknown"
        ));
        // A shell in the container answering for itself.
        assert!(chapkit_missing(127, "sh: 1: chapkit: not found"));
        assert!(chapkit_missing(
            2,
            "Usage: chapkit [OPTIONS] COMMAND\nError: No such command 'test'."
        ));
        // A model whose own error says something was not found is not this.
        assert!(!chapkit_missing(
            1,
            "Error in library(INLA) : there is no package called 'INLA'"
        ));
    }

    #[test]
    fn the_frame_is_transposed_into_one_observation_per_value() {
        let frame = serde_json::json!({
            "columns": ["time_period", "location", "disease_cases", "rainfall"],
            "data": [
                ["2020-01", "location_0", 12.0, 100.5],
                ["2020-01", "location_1", 7.0, 90.0],
                ["2020-02", "location_0", serde_json::Value::Null, 80.25],
            ],
        });
        let (observations, locations) = observations(&frame).expect("a frame");
        assert_eq!(locations, vec!["location_0", "location_1"]);
        // Two feature columns per row, and neither index column becomes one.
        assert_eq!(observations.len(), 6);
        assert_eq!(
            observations[0],
            Observation {
                period: "202001".to_string(),
                org_unit: "location_0".to_string(),
                value: Some(12.0),
                feature_name: "disease_cases".to_string(),
            }
        );
        // A hole in the frame goes over as a known-missing observation.
        assert_eq!(observations[4].value, None);
        assert_eq!(observations[4].feature_name, "disease_cases");
        assert_eq!(observations[4].period, "202002");
        // The wire names are chap-core's, not Rust's.
        let wire = serde_json::to_value(&observations[0]).expect("JSON");
        assert_eq!(wire["orgUnit"], serde_json::json!("location_0"));
        assert_eq!(wire["featureName"], serde_json::json!("disease_cases"));

        // A weekly frame keeps the W chap-core's period code has.
        let weekly = serde_json::json!({
            "columns": ["time_period", "location", "disease_cases"],
            "data": [["2020-W01", "location_0", 1.0], ["2020-W52", "location_0", 2.0]],
        });
        let (weekly, _) = super::observations(&weekly).expect("a frame");
        assert_eq!(weekly[0].period, "2020W01");
        assert_eq!(weekly[1].period, "2020W52");
        assert_eq!(period_code("2020-01"), "202001");
        assert_eq!(period_code(" 2020-W01 "), "2020W01");
    }

    #[test]
    fn a_frame_that_is_not_one_says_which_column_is_missing() {
        let err = observations(&serde_json::json!({"columns": [], "data": []}))
            .expect_err("no time_period");
        assert!(err.to_string().contains("`time_period`"), "{err}");
        let err = observations(&serde_json::json!({"data": []})).expect_err("no columns");
        assert!(err.to_string().contains("`columns`"), "{err}");
        let err = observations(&serde_json::json!({"columns": ["time_period", "location"]}))
            .expect_err("no rows");
        assert!(err.to_string().contains("`data`"), "{err}");
    }

    #[test]
    fn every_feature_carries_the_top_level_id_chap_core_matches_on() {
        // chapkit puts the location in `properties.id` and nothing at the top
        // level, and chap-core drops an org unit whose feature has no `id`.
        let geo = serde_json::json!({
            "type": "FeatureCollection",
            "bbox": [0.0, 0.0, 1.0, 1.0],
            "features": [
                {"type": "Feature", "geometry": {"type": "Point", "coordinates": [0.0, 0.0]},
                 "properties": {"id": "location_0"}},
                {"type": "Feature", "geometry": null, "properties": {"id": "location_1"}},
            ],
        });
        let locations = vec!["location_0".to_string(), "location_1".to_string()];
        let built = feature_collection(Some(&geo), &locations);
        assert_eq!(built["type"], serde_json::json!("FeatureCollection"));
        assert_eq!(built["features"][0]["id"], serde_json::json!("location_0"));
        assert_eq!(built["features"][1]["id"], serde_json::json!("location_1"));
        // The geometry is chapkit's and is passed through untouched.
        assert_eq!(
            built["features"][0]["geometry"]["type"],
            serde_json::json!("Point")
        );
        assert_eq!(built["bbox"], serde_json::json!([0.0, 0.0, 1.0, 1.0]));

        // A feature that already has one keeps it.
        let with_id = serde_json::json!({
            "type": "FeatureCollection",
            "features": [{"type": "Feature", "id": "kept", "properties": {"id": "other"}}],
        });
        let built = feature_collection(Some(&with_id), &locations);
        assert_eq!(built["features"][0]["id"], serde_json::json!("kept"));

        // A model that needs no geometry gets one feature per org unit with
        // none, which is enough for chap-core to know they exist.
        let built = feature_collection(None, &locations);
        assert_eq!(built["features"].as_array().expect("a list").len(), 2);
        assert_eq!(built["features"][0]["id"], serde_json::json!("location_0"));
        assert_eq!(built["features"][0]["geometry"], serde_json::Value::Null);
        assert_eq!(
            built["features"][1]["properties"]["id"],
            serde_json::json!("location_1")
        );
        // A `geo` with no features in it is no geo at all.
        assert_eq!(
            feature_collection(Some(&serde_json::json!({})), &locations)["features"][0]["geometry"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn a_row_pads_the_model_column_and_nothing_else() {
        let names = vec![
            "chapkit-ewars-model".to_string(),
            "chapkit-rwanda-malaria-bym-model".to_string(),
        ];
        let width = name_width(&names);
        assert_eq!(width, 32);

        let passed = run(
            "chapkit_ewars_model",
            "chapkit-ewars-model",
            Verdict::Pass,
            17,
            "1 training, 1 prediction",
        );
        assert_eq!(
            row(&passed, passed.verdict.label(), width),
            "chapkit-ewars-model                 pass   17s   1 training, 1 prediction"
        );

        let failed = run(
            "chapkit_rwanda_malaria_bym_model",
            "chapkit-rwanda-malaria-bym-model",
            Verdict::Fail,
            12,
            "predict: the inla program crashed",
        );
        assert_eq!(
            row(&failed, failed.verdict.label(), width),
            "chapkit-rwanda-malaria-bym-model    FAIL   12s   predict: the inla program crashed"
        );

        // A minute-long backtest reads in two units, and a row with nothing
        // to say in its last cell does not end in a space.
        let mut slow = run("x", "auto-arima-chapkit", Verdict::Pass, 129, "");
        assert_eq!(
            row(&slow, slow.verdict.label(), width),
            format!(
                "auto-arima-chapkit{}pass   2m 9s",
                " ".repeat(14 + NAME_GAP)
            )
        );
        slow.summary = "crps 20.0  mae 29.0  rmse 34.9".to_string();
        assert!(
            row(&slow, slow.verdict.label(), width)
                .ends_with("2m 9s   crps 20.0  mae 29.0  rmse 34.9")
        );

        // A run that took one digit of seconds is right-aligned into the
        // column, so its summary starts where every other row's does.
        let quick = run(
            "x",
            "chapkit-ewars-model",
            Verdict::Pass,
            6,
            "1 training, 1 prediction",
        );
        assert_eq!(
            row(&quick, quick.verdict.label(), width),
            "chapkit-ewars-model                 pass    6s   1 training, 1 prediction"
        );
        // Only a failure shouts.
        assert_eq!(Verdict::Pass.label(), "pass");
        assert_eq!(Verdict::Fail.label(), "FAIL");
        assert_eq!(Verdict::Skip.label(), "skip");
    }

    #[test]
    fn the_header_says_which_level_is_being_run() {
        assert_eq!(
            header(5, Level::Model),
            "testing 5 models (model level; add --backtest to run them through chap-core)"
        );
        assert!(header(1, Level::Model).starts_with("testing 1 model ("));
        assert_eq!(
            header(2, Level::Backtest),
            "testing 2 models (through chap-core: a dataset, a backtest and its scores)"
        );
    }

    #[test]
    fn the_closing_line_counts_what_happened_and_only_a_failure_is_a_failure() {
        let pass = |id: &str| run(id, id, Verdict::Pass, 1, "1 training, 1 prediction");
        let all: Vec<Run> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|id| pass(id))
            .collect();
        assert_eq!(closing(&all), "5 of 5 models pass");
        assert!(!any_failed(&all));

        // One failure names the model, so the command in the line works as
        // typed rather than having to be adapted.
        let mut one_bad = all.clone();
        one_bad[4] = run(
            "ewars",
            "chapkit-ewars-model",
            Verdict::Fail,
            12,
            "predict: boom",
        );
        assert_eq!(
            closing(&one_bad),
            "4 of 5 models pass; run `chaps models test ewars -v` for the full output"
        );
        assert!(any_failed(&one_bad));

        // Two failures cannot, so the line says so.
        let mut two_bad = one_bad.clone();
        two_bad[3] = run("ghr", "chapkit-ghr-model", Verdict::Fail, 9, "train: boom");
        assert!(
            closing(&two_bad).contains("chaps models test <id> -v"),
            "{}",
            closing(&two_bad)
        );

        // With a skip in it the three buckets are counted instead: "4 of 5"
        // would be read as one model having failed.
        let mut skipped = all.clone();
        skipped[3] = run("ghr", "chapkit-ghr-model", Verdict::Fail, 9, "train: boom");
        skipped[4] = run(
            "arima",
            "auto-arima-chapkit",
            Verdict::Skip,
            0,
            "not running",
        );
        assert_eq!(
            closing(&skipped),
            "3 pass, 1 fail, 1 skipped; run `chaps models test ghr -v` for the full output"
        );
        // A skip on its own is not a failure and the run still exits zero.
        let only_skip = vec![run(
            "arima",
            "auto-arima-chapkit",
            Verdict::Skip,
            0,
            "not running",
        )];
        assert_eq!(closing(&only_skip), "0 pass, 1 skipped");
        assert!(!any_failed(&only_skip));
        assert_eq!(closing(&[]), "0 of 0 models pass");
    }

    #[test]
    fn the_json_shape_is_the_documented_one() {
        let mut model = run(
            "chapkit_ewars_model",
            "chapkit-ewars-model",
            Verdict::Pass,
            17,
            "1 training, 1 prediction",
        );
        let value = serde_json::to_value(&model).expect("JSON");
        assert_eq!(value["id"], serde_json::json!("chapkit_ewars_model"));
        assert_eq!(
            value["service_id"],
            serde_json::json!("chapkit-ewars-model")
        );
        assert_eq!(value["level"], serde_json::json!("model"));
        assert_eq!(value["result"], serde_json::json!("pass"));
        assert_eq!(value["seconds"], serde_json::json!(17));
        assert_eq!(
            value["summary"],
            serde_json::json!("1 training, 1 prediction")
        );
        assert_eq!(value["detail"], serde_json::Value::Null);
        // The three that only a backtest has are absent rather than null.
        for absent in ["job_id", "backtest_id", "metrics"] {
            assert!(value.get(absent).is_none(), "{absent} should be left out");
        }

        model.level = Level::Backtest;
        model.job_id = Some("f424cbe3".to_string());
        model.backtest_id = Some(5);
        model.metrics = Some(serde_json::json!({"crps": 4.83, "mae": 6.68}));
        model.detail = Some("run `chaps jobs logs f424cbe3`".to_string());
        let value = serde_json::to_value(&model).expect("JSON");
        assert_eq!(value["level"], serde_json::json!("backtest"));
        assert_eq!(value["job_id"], serde_json::json!("f424cbe3"));
        assert_eq!(value["backtest_id"], serde_json::json!(5));
        assert_eq!(value["metrics"]["crps"], serde_json::json!(4.83));
        assert_eq!(
            value["detail"],
            serde_json::json!("run `chaps jobs logs f424cbe3`")
        );
    }

    #[test]
    fn the_scores_cell_is_three_of_the_fourteen_chap_core_reports() {
        let metrics = serde_json::json!({
            "ratio_above_truth": 0.45,
            "crps": 20.04,
            "crps_log1p": 0.09,
            "mae": 28.96,
            "mape": 12.98,
            "coverage_10_90": 0.6,
            "rmse": 34.88,
        });
        assert_eq!(metrics_cell(&metrics), "crps 20.0  mae 29.0  rmse 34.9");
        // Only the ones that are there, and something honest when none are.
        assert_eq!(metrics_cell(&serde_json::json!({"mae": 1.0})), "mae 1.0");
        assert_eq!(metrics_cell(&serde_json::json!({})), "no scores reported");
    }

    /// `(id, name, archived)` as a row of a configured-model listing.
    fn configured(id: i64, name: &str, archived: bool) -> ConfiguredModel {
        ConfiguredModel {
            id,
            name: name.to_string(),
            archived,
        }
    }

    #[test]
    fn the_configured_model_of_a_service_is_its_own_name_then_one_of_its_configs() {
        let bare = configured(15, "chapkit-ewars-model", false);
        let synced = configured(
            19,
            "chapkit-ewars-model:chapkit-ewars-model_179026711",
            false,
        );
        let left_behind = configured(
            12,
            "chapkit-ewars-model:test_config_01M3A4TSAZTTDYS0SJK62R4S5A",
            false,
        );
        let other = configured(16, "chapkit-ghr-model", false);

        // The bare name is the service itself and wins, whatever else is
        // listed and whatever the ids are.
        let all = vec![
            other.clone(),
            left_behind.clone(),
            synced.clone(),
            bare.clone(),
        ];
        assert_eq!(
            configured_model_for(&all, "chapkit-ewars-model"),
            Some(&bare)
        );

        // Without it - a service that re-registered, which is the state this
        // exists for - a config of the service is the answer, and the one
        // `chapkit test` left behind is the last resort.
        let synced_only = vec![other.clone(), left_behind.clone(), synced.clone()];
        assert_eq!(
            configured_model_for(&synced_only, "chapkit-ewars-model"),
            Some(&synced)
        );
        assert_eq!(
            configured_model_for(&[other.clone(), left_behind.clone()], "chapkit-ewars-model"),
            Some(&left_behind)
        );

        // Two configs of the same service: the lowest id, so two runs of the
        // same command backtest the same model.
        let second = configured(
            9,
            "chapkit-ewars-model:chapkit-ewars-model_179026761",
            false,
        );
        assert_eq!(
            configured_model_for(&[synced.clone(), second.clone()], "chapkit-ewars-model"),
            Some(&second)
        );

        // An archived row is a name chap-core keeps and nothing runs, so the
        // config is chosen over it rather than it over the config.
        let retired = configured(3, "chapkit-ewars-model", true);
        assert_eq!(
            configured_model_for(&[retired.clone(), synced.clone()], "chapkit-ewars-model"),
            Some(&synced)
        );
        assert_eq!(
            configured_model_for(&[retired], "chapkit-ewars-model"),
            None
        );

        // Nothing for this service at all, and a prefix that only looks like
        // one: `chapkit-ewars-model-2` is a different service.
        assert_eq!(configured_model_for(&[other], "chapkit-ewars-model"), None);
        assert_eq!(configured_model_for(&[], "chapkit-ewars-model"), None);
        let neighbour = configured(4, "chapkit-ewars-model-2:config", false);
        assert_eq!(
            configured_model_for(&[neighbour], "chapkit-ewars-model"),
            None
        );
    }

    #[test]
    fn a_configured_model_listing_is_read_down_to_the_three_fields_the_choice_needs() {
        let listed = serde_json::json!([
            {"id": 15, "name": "chapkit-ewars-model", "archived": true, "usesChapkit": true,
             "version": "1.0.0", "sourceDigest": "cafe"},
            {"id": 19, "name": "chapkit-ewars-model:cfg", "archived": false},
            // No `archived` at all reads as a live row, and a row without an
            // id or a name is not one.
            {"id": 20, "name": "auto-arima-chapkit"},
            {"name": "no id"},
            {"id": 21},
            "not a row",
        ]);
        let models = configured_models(&listed);
        assert_eq!(models.len(), 3);
        assert!(models[0].archived);
        assert_eq!(models[1].id, 19);
        assert_eq!(models[1].name, "chapkit-ewars-model:cfg");
        assert!(!models[2].archived);
        // An answer that is not a list at all is no configured model.
        assert!(configured_models(&serde_json::json!({"detail": "Not Found"})).is_empty());

        let chosen = configured_model_for(&models, "chapkit-ewars-model").expect("the config");
        assert_eq!(chosen.id, 19);
    }

    #[test]
    fn a_log_with_no_stderr_section_still_yields_the_line_that_says_why() {
        // What a job that failed inside chap-core leaves: a Python traceback
        // and no model output at all, because the model was never reached.
        let log = "\
2026-09-24 18:00:30,440 [INFO] chap_status: Starting backtest for model '6'
Traceback (most recent call last):
  File \"/app/.venv/lib/python3.13/site-packages/pandas/core/indexes/base.py\", line 6355
    raise KeyError(f\"{not_found} not in index\")
KeyError: \"['mean_relative_humidity'] not in index\"
";
        assert_eq!(
            log_hint(log).as_deref(),
            Some("KeyError: \"['mean_relative_humidity'] not in index\"")
        );
        // A log with nothing in it says nothing rather than something wrong.
        assert_eq!(log_hint(""), None);
        assert_eq!(log_hint("   \n\n").as_deref(), None);
        // The stderr section is still the better answer where there is one,
        // and that is `jobs::stderr_hint`'s job rather than this one's.
        let with_section = format!("{log}--- stderr ---\nError in f() : boom\n");
        assert_eq!(
            crate::jobs::stderr_hint(&with_section).as_deref(),
            Some("Error in f() : boom")
        );
    }

    #[test]
    fn a_reason_is_cut_at_the_first_sentence_and_a_version_number_is_not_one() {
        assert_eq!(
            first_sentence("the inla program crashed. See the log for details."),
            "the inla program crashed"
        );
        // A full stop inside a version is not the end of a sentence.
        assert_eq!(
            first_sentence("chapkit 2.0.0 refused the config"),
            "chapkit 2.0.0 refused the config"
        );
        // Nothing to cut at is the whole line.
        assert_eq!(first_sentence("  boom  "), "boom");
    }
}
