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

/// The fewest `feature_N` columns the generated sample data carries, which is
/// chapkit's own default.
pub const SAMPLE_FEATURES: usize = 3;

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

/// The same for a backtest, which runs into minutes: `1m 38s`. The rows are
/// printed as each model finishes, so the column cannot be sized from the
/// slowest one; it is sized for the longest a backtest normally takes.
const BACKTEST_TIME_WIDTH: usize = 6;

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
            format!("{what} (model level; add --backtest to go through chap-core)")
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
    let time_width = match run.level {
        Level::Model => TIME_WIDTH,
        Level::Backtest => BACKTEST_TIME_WIDTH,
    };
    let time_pad = time_width.saturating_sub(time.chars().count());
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
            "{passed} of {} {} {}",
            runs.len(),
            plural(runs.len(), "model"),
            if passed == 1 { "passes" } else { "pass" }
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

/// Give the frame every covariate in `wanted`, renaming chapkit's spare
/// `feature_N` columns into the ones it lacks, and return the renames.
///
/// `$generate-sample-data` generates the service's required covariates and a
/// fixed pair of climate ones, but not a configuration's
/// `additional_continuous_covariates`: a model whose configuration defaults to
/// `mean_relative_humidity` gets a frame without it, and the backtest then
/// fails inside the model with a `KeyError` that says nothing about the model.
/// The `feature_N` columns are the same kind of synthetic seasonal series, so
/// one of them stands in. A covariate with no spare column left stays missing,
/// and the model says so the way it would with real data.
pub fn fill_covariates(frame: &mut serde_json::Value, wanted: &[String]) -> Vec<(String, String)> {
    let Some(columns) = frame.get_mut("columns").and_then(|c| c.as_array_mut()) else {
        return Vec::new();
    };
    let has = |columns: &[serde_json::Value], name: &str| {
        columns.iter().any(|column| column.as_str() == Some(name))
    };
    let mut renamed = Vec::new();
    for name in wanted {
        if has(columns, name) {
            continue;
        }
        let spare = columns.iter().position(|column| {
            column.as_str().is_some_and(|text| {
                text.strip_prefix("feature_")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                    && !wanted.iter().any(|want| want == text)
            })
        });
        let Some(at) = spare else { continue };
        let from = columns[at].as_str().unwrap_or_default().to_string();
        columns[at] = serde_json::json!(name);
        renamed.push((from, name.clone()));
    }
    renamed
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
    /// Its `additionalContinuousCovariates`: the columns a backtest of it
    /// hands the model, which the dataset therefore has to carry.
    pub covariates: Vec<String>,
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
                covariates: row
                    .get("additionalContinuousCovariates")
                    .and_then(|value| value.as_array())
                    .map(|names| {
                        names
                            .iter()
                            .filter_map(|name| name.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
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
mod tests;
