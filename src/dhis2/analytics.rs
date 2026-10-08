//! Analytics: starting a run, following its job, and what varde knows about
//! the tables it made.

use super::client::text_at;
use super::{ANALYTICS_JOB_TYPE, Dhis2, JOB_CONFIGURATIONS_PATH, TASKS_PATH};
use crate::error::Result;
use serde::Serialize;
use std::time::{Duration, Instant};

/// Where one job's notifications are read.
pub fn job_path(job: &str) -> String {
    format!(
        "{TASKS_PATH}/{ANALYTICS_JOB_TYPE}/{}",
        crate::api::encode(job)
    )
}

/// Where every analytics job's notifications are read, newest run included.
pub fn jobs_path() -> String {
    format!("{TASKS_PATH}/{ANALYTICS_JOB_TYPE}")
}

/// How far a job has got, as its notifications say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// Still going. The message is the latest step it announced.
    Running(String),
    /// Finished, and nothing said it failed.
    Done(String),
    /// A notification came in at `ERROR` level.
    Failed(String),
}

impl Progress {
    /// The step or the reason, whichever this is.
    pub fn message(&self) -> &str {
        match self {
            Progress::Running(text) | Progress::Done(text) | Progress::Failed(text) => text,
        }
    }
}

/// What a notification array says about the job it belongs to.
///
/// `None` for an empty array, which is what a job that has been queued behind
/// another one looks like: DHIS2 runs one analytics job at a time, and the
/// second one's notifier stays empty until the first finishes.
///
/// The entries are ordered by their own `time` rather than by their position,
/// because the two ends of the array have swapped between DHIS2 versions and
/// the timestamps are ISO-8601, which sorts as text. An `ERROR` outranks a
/// `completed`, since a failed run announces both.
pub fn progress_of(notifications: &serde_json::Value) -> Option<Progress> {
    let entries = notifications.as_array()?;
    if entries.is_empty() {
        return None;
    }
    let message_of = |entry: &serde_json::Value| text_at(entry, "message");
    let latest = entries
        .iter()
        .max_by_key(|entry| text_at(entry, "time"))
        .map(message_of)
        .unwrap_or_default();
    if let Some(failure) = entries
        .iter()
        .filter(|entry| text_at(entry, "level").eq_ignore_ascii_case("ERROR"))
        .max_by_key(|entry| text_at(entry, "time"))
    {
        return Some(Progress::Failed(message_of(failure)));
    }
    if entries
        .iter()
        .any(|entry| entry.get("completed").and_then(serde_json::Value::as_bool) == Some(true))
    {
        return Some(Progress::Done(latest));
    }
    Some(Progress::Running(latest))
}

/// The analytics job that is already running, out of `GET /api/system/tasks/
/// ANALYTICS_TABLE`.
///
/// Worth asking before starting one: DHIS2 runs a single analytics job at a
/// time, so a second `POST` queues behind the first and its notifier stays
/// empty - a wait on it would report nothing for as long as the first one takes.
pub fn running_job(tasks: &serde_json::Value) -> Option<String> {
    let mut running: Vec<(String, String)> = tasks
        .as_object()?
        .iter()
        .filter(|(_, notifications)| {
            matches!(progress_of(notifications), Some(Progress::Running(_)))
        })
        .map(|(job, notifications)| (latest_time(notifications), job.clone()))
        .collect();
    running.sort();
    running.pop().map(|(_, job)| job)
}

/// The query for the analytics jobs DHIS2 has, with the fields that say
/// whether one waits to run.
pub fn queued_jobs_query() -> String {
    format!(
        "{JOB_CONFIGURATIONS_PATH}?fields=id,jobStatus,schedulingType,enabled,lastFinished\
         &filter=jobType:eq:{ANALYTICS_JOB_TYPE}&paging=false"
    )
}

/// The analytics job that DHIS2 accepted and has not finished, out of
/// [`queued_jobs_query`].
///
/// The notifier lists a job only when it starts, about five seconds after
/// the `POST` that asked for it. The job configuration is there at once: a
/// run that was asked for is a `ONCE_ASAP` job, `SCHEDULED` until it starts
/// and `RUNNING` after, and DHIS2 disables it and records `lastFinished`
/// when it ends. So a second `varde dhis2 analytics` a moment after the
/// first finds that job here and does not start another.
pub fn queued_job(listing: &serde_json::Value) -> Option<String> {
    let jobs = listing.get("jobConfigurations")?.as_array()?;
    let waiting = |job: &&serde_json::Value| {
        text_at(job, "schedulingType") == "ONCE_ASAP"
            && matches!(text_at(job, "jobStatus").as_str(), "SCHEDULED" | "RUNNING")
            && job.get("enabled").and_then(serde_json::Value::as_bool) != Some(false)
            && text_at(job, "lastFinished").is_empty()
    };
    // A job that runs comes first: the others wait behind it.
    jobs.iter()
        .filter(waiting)
        .max_by_key(|job| text_at(job, "jobStatus") == "RUNNING")
        .map(|job| text_at(job, "id"))
        .filter(|id| !id.trim().is_empty())
}

/// The newest `time` in a notification array, for ordering two of them.
fn latest_time(notifications: &serde_json::Value) -> String {
    notifications
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| text_at(entry, "time"))
        .max()
        .unwrap_or_default()
}

/// Whether an analytics run has finished on this DHIS2 since it started.
///
/// `GET /api/system/tasks/ANALYTICS_TABLE` is DHIS2's notifier, and the
/// notifier lives in the running process rather than in the database. That is
/// exactly what makes it useful here: a deployment seeded from a dump starts
/// with it empty however much analytics the instance the dump was taken from
/// ever ran, so a completed run in it is a run that happened *here*.
///
/// It is evidence one way only. A restart empties it, so a `false` means varde
/// has not seen a run rather than that there was none, and every sentence built
/// on it has to say so.
pub fn finished_here(tasks: &serde_json::Value) -> bool {
    tasks
        .as_object()
        .into_iter()
        .flatten()
        .any(|(_, notifications)| matches!(progress_of(notifications), Some(Progress::Done(_))))
}

/// What varde actually knows about a deployment's analytics tables.
///
/// **`lastAnalyticsTableSuccess` is not a fact about this deployment.** It is a
/// row of DHIS2's own settings, so a seeded deployment inherits it from the
/// dump: measured on the Laos climate demo, a freshly seeded instance reported
/// a last success of `2026-06-16T07:51:00.093` while `analytics_2024` did not
/// exist at all - the table was absent, not empty. That is the failure this
/// command exists to catch, and the timestamp on its own hides it.
///
/// varde talks HTTP and cannot look at the tables, so it reports what it
/// checked and no more. Two questions settle which of these four it is: what
/// DHIS2 records, and whether a run has finished here, which [`finished_here`]
/// answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnalyticsEvidence {
    /// DHIS2 records no successful run at all, neither here nor in a dump.
    Never,
    /// A run finished on this DHIS2 since it started. varde saw it happen.
    RanHere,
    /// DHIS2 records a success and no dump could have brought it: this
    /// deployment's database was migrated from empty, so the record was made
    /// against it.
    Recorded,
    /// DHIS2 records a success, the database was restored from a seed dump
    /// that carries that timestamp, and no run has finished here since DHIS2
    /// started. The timestamp is not evidence either way.
    Unconfirmed,
}

/// Which of the four this deployment is in.
///
/// `ran_here` outranks everything, because it is the only first-hand answer.
/// `seeded` is read from `.varde/components.yaml` rather than from DHIS2: varde
/// knows whether it restored a dump into this database, and DHIS2 does not know
/// where its rows came from.
pub fn analytics_evidence(last_success: &str, ran_here: bool, seeded: bool) -> AnalyticsEvidence {
    match (ran_here, recorded_success(last_success).is_empty(), seeded) {
        (true, _, _) => AnalyticsEvidence::RanHere,
        (false, true, _) => AnalyticsEvidence::Never,
        (false, false, true) => AnalyticsEvidence::Unconfirmed,
        (false, false, false) => AnalyticsEvidence::Recorded,
    }
}

/// `lastAnalyticsTableSuccess` as a record of a run, or empty for none.
///
/// A DHIS2 that has never run analytics does not always leave the setting
/// out: 2.42.6 on an empty database sends `1970-01-01T00:00:00.000`, the
/// epoch, which is the default of the setting and not a run. So a date in
/// 1970 counts as no run, the same as an empty value.
pub fn recorded_success(raw: &str) -> String {
    let raw = raw.trim();
    match raw.is_empty() || raw.starts_with("1970-01-01") {
        true => String::new(),
        false => raw.to_string(),
    }
}

/// The job id a `POST /api/resourceTables/analytics` answered with.
///
/// Three spellings, because the shape of that `WebMessage` has moved: the id
/// under `response`, the notifier endpoint it is the last segment of, and a
/// bare `id` at the top. A run whose id cannot be read is not a failure - the
/// caller falls back to whatever job is running, which is this one.
pub fn job_id_of(response: &serde_json::Value) -> Option<String> {
    let inner = response.get("response");
    let id = inner
        .and_then(|inner| inner.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            inner
                .and_then(|inner| inner.get("relativeNotifierEndpoint"))
                .and_then(serde_json::Value::as_str)
                .and_then(|endpoint| endpoint.rsplit('/').next())
                .map(str::to_string)
        })
        .or_else(|| {
            response
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })?;
    Some(id).filter(|id| !id.trim().is_empty())
}

/// How many polls of an analytics job may fail in a row before the wait
/// gives up.
const MAX_POLL_FAILURES: u32 = 3;

impl Dhis2 {
    /// Poll one analytics job until it is no longer running.
    ///
    /// `step` is called with each message the job announces, so a wait that
    /// takes an hour is not a silent one. The deadline is an error rather than
    /// a verdict: DHIS2 is still doing the work, and stopping it is nobody's
    /// business but the operator's, so the message names the job.
    pub fn wait_for_job(
        &self,
        job: &str,
        timeout: Duration,
        interval: Duration,
        step: &mut impl FnMut(&str),
    ) -> Result<Progress> {
        let until = Instant::now() + timeout;
        let path = job_path(job);
        let mut last = String::new();
        // A poll that fails while DHIS2 is busy generating the tables - a
        // timeout in the populate phase - says nothing about the run; only
        // several in a row end the wait.
        let mut failures = 0;
        loop {
            let progress = match self.get_json(&path) {
                Ok(answer) => {
                    failures = 0;
                    progress_of(&answer)
                }
                Err(err) if failures < MAX_POLL_FAILURES => {
                    failures += 1;
                    crate::output::verbose(&format!(
                        "polling analytics job {job} failed ({failures} of {MAX_POLL_FAILURES}): \
                         {err:#}"
                    ));
                    std::thread::sleep(interval);
                    continue;
                }
                Err(err) => return Err(err),
            };
            if let Some(progress) = &progress
                && progress.message() != last
            {
                last = progress.message().to_string();
                step(&last);
            }
            match progress {
                Some(Progress::Running(_)) | None => {}
                Some(done) => return Ok(done),
            }
            if Instant::now() + interval >= until {
                return Err(anyhow::anyhow!(
                    "analytics job {job} was still running after {}; DHIS2 is still generating the \
                     tables, so `varde dhis2 analytics` watches the same job again, and \
                     `--timeout SECONDS` waits longer",
                    crate::output::human_age(timeout)
                ));
            }
            std::thread::sleep(interval);
        }
    }
}
