//! What `varde models test` proves, and how it reads on a terminal.
//!
//! Registration is a heartbeat. A model that answers its healthcheck and pings
//! chap-core every thirty seconds can still be unable to produce a single
//! prediction - the user it runs as cannot write, a library is missing from
//! the image, the covariates it declares are not the ones it reads - and
//! neither `varde status` nor `varde doctor` can tell. The only way to know is
//! to make the model do the work.
//!
//! Everything here is pure: the parsing of what `chapkit test` printed, the
//! transposition of a sample frame into the observations chap-core's
//! `make-dataset` takes, and the rendering of a row. The `docker compose exec`
//! and the requests live in [`crate::commands::modeltest`], so every rule in
//! this module can be tested without docker and without a server.

mod configured;
mod dataset;
mod summary;

pub use configured::{
    ConfiguredModel, configured_model_for, configured_models, configured_variant,
};
pub use dataset::{feature_collection, fill_covariates, metrics_cell, observations};
pub use summary::{SUMMARY_MARKER, chapkit_missing, log_hint, parse_summary};

use serde::Serialize;
use std::time::Duration;

/// Seconds one model gets at the model level before varde gives up on it.
///
/// Passed to `chapkit test --timeout` as well, so chapkit abandons a single
/// job no later than varde abandons the whole run. Five minutes is roughly
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

/// The backtest varde asks for: three periods ahead, two splits, stride one.
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
    /// The chap-core job the backtest level ended on, for `varde jobs logs`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backtest_id: Option<i64>,
    /// chap-core's `aggregateMetrics`, whole, for a backtest that finished.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<serde_json::Value>,
    /// The configured model the backtest is of, so that the scores never
    /// come from a configuration that the reader does not see.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configured_model: Option<UsedModel>,
    /// What the cleanup kept or could not remove. The closing lines carry
    /// them, so `--json` has them in `messages`.
    #[serde(skip)]
    pub cleanup: Vec<crate::output::Message>,
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
            configured_model: None,
            cleanup: Vec::new(),
        }
    }

    /// The same run, with the lines its cleanup reported.
    pub fn with_cleanup(mut self, cleanup: Vec<crate::output::Message>) -> Run {
        self.cleanup.extend(cleanup);
        self
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

/// The configured model a backtest names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsedModel {
    /// chap-core's key for it. `None` when chap-core could not list its
    /// configured models, and the backtest names the service id instead.
    pub id: Option<i64>,
    /// chap-core's name for it: `<service id>:<variant>`, or the service id.
    pub name: String,
    /// The variant name, as `varde models configs` shows it.
    pub variant: String,
}

impl UsedModel {
    /// The line under the row.
    pub fn line(&self) -> String {
        match self.id {
            Some(id) => format!("configured model: {} (id {id})", self.variant),
            None => format!(
                "configured model: {} (by name, as chap-core could not list them)",
                self.variant
            ),
        }
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
            "; run `varde models test {which} -vv` for the full output"
        ));
    }
    line
}

/// Whether anything failed, which is the only thing that makes the run
/// non-zero: a skip is a question that could not be asked, not a bad answer.
pub fn any_failed(runs: &[Run]) -> bool {
    runs.iter().any(|r| r.verdict == Verdict::Fail)
}

/// Whether the run tested no model at all: every one was skipped.
pub fn none_tested(runs: &[Run]) -> bool {
    !runs.is_empty() && runs.iter().all(|r| r.verdict == Verdict::Skip)
}

/// The warning for a run in which every model was skipped.
pub const NONE_TESTED: &str =
    "no model was tested, because every one was skipped; the line under each row says why";

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
