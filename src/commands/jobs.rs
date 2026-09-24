//! `chaps jobs` — what chap-core has been asked to compute, and how it went.
//!
//! chap-core runs everything slow on a Celery worker: building a dataset,
//! backtesting a model, making a prediction. The Modeling App starts them and
//! watches them, and until now a deployment with no browser in front of it had
//! no way to ask what it was doing. These five verbs are that way.
//!
//! The one thing the API makes awkward, and that this module exists to hide:
//! a job description carries a status and nothing else, so a `FAILURE` says
//! only that something went wrong. The message is in
//! `GET /v1/jobs/{id}/logs`, mixed into the model's own output, which is why
//! `chaps jobs logs` is the command the failure line points at and why it
//! digs the last error line out of the `--- stderr ---` section for a job that
//! failed.
//!
//! Everything printed here goes through [`crate::jobs`], which holds no
//! requests at all; this module holds the requests and nothing else.

use crate::api::Api;
use crate::cli::{JobsCancelArgs, JobsDeleteArgs, JobsListArgs, JobsLogsArgs, JobsShowArgs};
use crate::commands::Ctx;
use crate::error::Result;
use crate::jobs::{self, Job, Matched};
use crate::output::Out;
use crate::project::Project;
use std::time::Duration;

/// `chaps jobs list`, and the bare `chaps jobs`.
pub fn list(ctx: &Ctx, args: &JobsListArgs) -> Result<()> {
    let (_project, api) = connect(ctx)?;
    let mut query: Vec<String> = args
        .status
        .iter()
        .map(|status| crate::api::query_pair("status", status.trim()))
        .collect();
    if let Some(kind) = args.kind.as_deref() {
        query.push(crate::api::query_pair("type", kind.trim()));
    }
    let (jobs, raw) = fetch(&api, &crate::api::with_query(jobs::JOBS_PATH, &query))?;

    // The order is the table's; the limit is applied after it, so `--limit 5`
    // is the five newest and not five arbitrary ones.
    let keep = args.limit.unwrap_or(jobs.len()).min(jobs.len());
    let jobs: Vec<Job> = jobs.into_iter().take(keep).collect();
    let raw: Vec<serde_json::Value> = raw.into_iter().take(keep).collect();

    let now = crate::backup::now();
    ctx.out.emit(&raw, || human_list(&jobs, now, &ctx.out))
}

/// The table, the line it adds up to, and what to do about a failure.
fn human_list(jobs: &[Job], now: u64, out: &Out) -> String {
    if jobs.is_empty() {
        return format!("{}\n", out.backticks(jobs::NO_JOBS));
    }
    let mut text = out.table(jobs::HEADERS, &jobs::rows(jobs, now));
    text.push('\n');
    text.push_str(&out.cmd(&jobs::summary(jobs)));
    text.push('\n');
    if let Some(hint) = jobs::failure_hint(jobs) {
        text.push_str(&format!("  {}\n", out.backticks(&hint)));
    }
    text
}

/// `chaps jobs show ID`.
pub fn show(ctx: &Ctx, args: &JobsShowArgs) -> Result<()> {
    let (_project, api) = connect(ctx)?;
    let (id, jobs) = find(ctx, &api, &args.id)?;
    let job = jobs
        .into_iter()
        .find(|j| j.id == id)
        .unwrap_or_else(|| Job {
            id: id.clone(),
            ..Job::default()
        });

    // The list is a snapshot and the job may have moved on since; the status
    // endpoint is the current answer, and it is one request.
    let status = current_status(&api, &id)?.unwrap_or_else(|| job.status.clone());
    // Where the result landed, which is the number every `/v1/crud/` path
    // wants next. Only a finished job has one, and asking about a running job
    // is a 400 rather than an answer.
    let database_result = database_result(&api, &id, &status);

    let value = serde_json::json!({
        "id": job.id,
        "type": job.kind,
        "name": job.name,
        "status": status,
        "start_time": job.start_time,
        "end_time": job.end_time,
        "duration_seconds": job.duration().map(|d| d.as_secs()),
        "result": job.result,
        "prediction_setup_id": job.prediction_setup_id,
        "database_result_id": database_result,
    });
    let now = crate::backup::now();
    ctx.out.emit(&value, || {
        human_show(&job, &status, database_result, now, &ctx.out)
    })
}

/// One job as a block of fields, then the next step its status implies.
fn human_show(
    job: &Job,
    status: &str,
    database_result: Option<i64>,
    now: u64,
    out: &Out,
) -> String {
    let started = match (job.start_time.as_deref(), job.started_at()) {
        (Some(stamp), Some(at)) => format!(
            "{stamp} ({})",
            crate::output::ago(Duration::from_secs(now.saturating_sub(at)))
        ),
        (Some(stamp), None) => stamp.to_string(),
        (None, _) => String::new(),
    };
    let rows = vec![
        ("Job", out.value(&job.id)),
        ("Type", job.kind.clone()),
        ("Name", job.name.clone()),
        ("Status", status_cell(out, status)),
        ("Started", started),
        ("Ended", job.end_time.clone().unwrap_or_default()),
        (
            "Duration",
            job.duration().map(jobs::took_text).unwrap_or_default(),
        ),
        ("Result", job.result.clone().unwrap_or_default()),
        (
            "Prediction setup",
            job.prediction_setup_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
        ),
        (
            "Database result",
            database_result.map(|id| id.to_string()).unwrap_or_default(),
        ),
    ];
    let mut text = crate::output::fields_with(0, &rows, &|label| out.key(label));
    text.push('\n');
    text.push_str(&out.backticks(&next_step(job, status, database_result)));
    text.push('\n');
    text
}

/// What to do with this job now, which is never nothing.
fn next_step(job: &Job, status: &str, database_result: Option<i64>) -> String {
    match jobs::Outcome::of(status) {
        jobs::Outcome::Failed => format!("run `chaps jobs logs {}` to see why", job.id),
        jobs::Outcome::Running => format!(
            "still running; `chaps jobs` says when it finishes and \
             `chaps jobs cancel {}` stops it",
            job.id
        ),
        jobs::Outcome::Cancelled => format!(
            "this job was cancelled; `chaps jobs logs {}` shows how far it got",
            job.id
        ),
        jobs::Outcome::Done => match database_result {
            // The collection the row is in follows the job type, so the line
            // is a command that works rather than one to adapt.
            Some(id) => match collection_of(&job.kind) {
                Some(collection) => format!(
                    "the result is row {id} in chap-core's database; \
                     `chaps api GET /v1/crud/{collection}/{id}` reads it"
                ),
                None => format!("the result is row {id} in chap-core's database"),
            },
            None => format!("run `chaps jobs logs {}` to see what it did", job.id),
        },
    }
}

/// The `/v1/crud/` collection a job of this type writes its result into.
///
/// `None` for a job type this CLI has not heard of: chap-core is free to add
/// one, and guessing at a path would be worse than saying only the row.
fn collection_of(kind: &str) -> Option<&'static str> {
    match kind.trim() {
        "create_dataset" => Some("datasets"),
        "create_backtest" => Some("backtests"),
        "create_prediction" => Some("predictions"),
        _ => None,
    }
}

/// The STATUS field, coloured the way the rest of the CLI colours a verdict.
fn status_cell(out: &Out, status: &str) -> String {
    match jobs::Outcome::of(status) {
        jobs::Outcome::Done => out.ok(status),
        jobs::Outcome::Failed => out.bad(status),
        jobs::Outcome::Cancelled => out.warn(status),
        jobs::Outcome::Running => out.value(status),
    }
}

/// `chaps jobs logs ID`.
///
/// stdout is the log and nothing else, so it can be piped into `grep` or
/// `less`; which job it is, and the one line that says why it failed, go to
/// stderr.
pub fn logs(ctx: &Ctx, args: &JobsLogsArgs) -> Result<()> {
    let (_project, api) = connect(ctx)?;
    let (id, jobs) = find(ctx, &api, &args.id)?;
    let job = jobs
        .into_iter()
        .find(|j| j.id == id)
        .unwrap_or_else(|| Job {
            id: id.clone(),
            ..Job::default()
        });

    let path = format!("{}/{}/logs", jobs::JOBS_PATH, crate::api::encode(&id));
    let answer = api.send("GET", &path, None)?;
    if !answer.is_success() {
        return Err(api.status_error(&path, &answer));
    }
    // The endpoint answers with a JSON string holding the whole log; anything
    // else is taken as the log itself, so a chap-core that stops quoting it
    // still prints.
    let text = match answer.json() {
        Some(serde_json::Value::String(text)) => text,
        _ => answer.text().into_owned(),
    };
    let shown = match args.tail {
        Some(n) => jobs::tail(&text, n),
        None => text.clone(),
    };

    if !ctx.out.json {
        eprintln!("{}", job.headline());
    }
    let value = serde_json::json!({
        "id": job.id,
        "type": job.kind,
        "name": job.name,
        "status": job.status,
        "logs": shown,
    });
    ctx.out.emit(&value, || shown.clone())?;

    // Only for a job that failed, and only from the section a model's own
    // error stream is in: on a job that worked the last stderr line is
    // usually a package loading itself.
    if !ctx.out.json && job.outcome() == jobs::Outcome::Failed {
        match jobs::stderr_hint(&text) {
            Some(hint) => crate::output::notice(&format!("stderr: {hint}")),
            None => crate::output::notice(
                "this job failed; the traceback above is chap-core's own record of it",
            ),
        }
    }
    Ok(())
}

/// `chaps jobs cancel ID`.
pub fn cancel(ctx: &Ctx, args: &JobsCancelArgs) -> Result<()> {
    let (_project, api) = connect(ctx)?;
    let (id, _) = find(ctx, &api, &args.id)?;
    let path = format!("{}/{}/cancel", jobs::JOBS_PATH, crate::api::encode(&id));
    let answer = api.send("POST", &path, None)?;
    if !answer.is_success() {
        return Err(api.status_error(&path, &answer));
    }
    let message = message_of(&answer, "cancelled");
    let value = serde_json::json!({ "id": id, "cancelled": true, "message": message });
    ctx.out.emit(&value, || {
        format!(
            "{message}\n{}\n",
            ctx.out
                .backticks("run `chaps jobs` to see whether it has stopped")
        )
    })
}

/// `chaps jobs delete ID`.
pub fn delete(ctx: &Ctx, args: &JobsDeleteArgs) -> Result<()> {
    let (_project, api) = connect(ctx)?;
    let (id, _) = find(ctx, &api, &args.id)?;
    let path = format!("{}/{}", jobs::JOBS_PATH, crate::api::encode(&id));
    let answer = api.send("DELETE", &path, None)?;
    // chap-core refuses to forget a job it is still working on, and says so
    // with a 400. The verb that does apply is `cancel`, so name it.
    if answer.status == 400 {
        return Err(anyhow::anyhow!(
            "job {id} is still running; cancel it first (`chaps jobs cancel {id}`)"
        ));
    }
    if !answer.is_success() {
        return Err(api.status_error(&path, &answer));
    }
    let message = message_of(&answer, "deleted");
    let value = serde_json::json!({ "id": id, "deleted": true, "message": message });
    ctx.out.emit(&value, || {
        format!(
            "{message}\n{}\n",
            ctx.out.backticks("run `chaps jobs` for what is left")
        )
    })
}

/// The `message` chap-core answered with, or a sentence of our own when it
/// answered with something else.
fn message_of(answer: &crate::api::Answer, verb: &str) -> String {
    answer
        .json()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("job {verb}"))
}

/// This deployment and a client for its API.
fn connect(ctx: &Ctx) -> Result<(Project, Api)> {
    let project = ctx.project()?;
    let token = crate::auth::token_in(&project.dir);
    let api = Api::new(&project.api_url(), token, crate::api::DEFAULT_TIMEOUT);
    ctx.out
        .verbose(&format!("asking chap-core at {}", api.base()));
    Ok((project, api))
}

/// The job list, parsed and in table order, with the wire documents beside it.
fn fetch(api: &Api, path: &str) -> Result<(Vec<Job>, Vec<serde_json::Value>)> {
    let value = api.get_json(path)?;
    let list = value.as_array().ok_or_else(|| {
        anyhow::anyhow!(
            "{} answered with {} rather than a list of jobs",
            api.url(path),
            kind_of(&value)
        )
    })?;
    let jobs: Vec<Job> = list
        .iter()
        .map(|entry| serde_json::from_value(entry.clone()))
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| anyhow::anyhow!("reading the job list from {}: {e}", api.url(path)))?;
    let order = jobs::order(&jobs);
    Ok((
        order.iter().map(|i| jobs[*i].clone()).collect(),
        order.iter().map(|i| list[*i].clone()).collect(),
    ))
}

/// What a JSON value is, for an error that has to say what came back instead.
fn kind_of(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "a list",
        serde_json::Value::Object(_) => "an object",
    }
}

/// The full id `given` names, and the list it was found in.
///
/// An id is matched exactly first and then as a prefix, so the eight
/// characters the table prints are enough to type. A prefix that came out of a
/// table is announced under `-v`: which job a command acted on is the thing to
/// check when it acted on the wrong one.
fn find(ctx: &Ctx, api: &Api, given: &str) -> Result<(String, Vec<Job>)> {
    let (jobs, _) = fetch(api, jobs::JOBS_PATH)?;
    let matched = jobs::resolve(&jobs, given);
    if let Matched::One(id) = &matched {
        if !id.eq_ignore_ascii_case(given.trim()) {
            ctx.out.verbose(&format!("matched {id}"));
        }
        return Ok((id.clone(), jobs));
    }
    // A job chap-core knows and does not list is not a case the API has, but
    // asking costs one request and is the difference between "not found" and
    // a wrong answer if it ever gains one.
    let given = given.trim().to_string();
    if matches!(matched, Matched::None) && current_status(api, &given)?.is_some() {
        return Ok((given, jobs));
    }
    Err(jobs::no_such_job(&given, &matched))
}

/// `GET /v1/jobs/{id}`, which answers with a bare status string.
///
/// `None` for a 404, which is how chap-core says it has no such job.
fn current_status(api: &Api, id: &str) -> Result<Option<String>> {
    let path = format!("{}/{}", jobs::JOBS_PATH, crate::api::encode(id));
    let answer = api.send("GET", &path, None)?;
    if answer.status == 404 {
        return Ok(None);
    }
    if !answer.is_success() {
        return Err(api.status_error(&path, &answer));
    }
    Ok(Some(match answer.json() {
        Some(serde_json::Value::String(text)) => text,
        _ => answer.text().trim().trim_matches('"').to_string(),
    }))
}

/// The row a finished job wrote, from `GET /v1/jobs/{id}/database_result`.
///
/// Best effort: the endpoint answers 400 while the job runs and 400 again for
/// one that failed, and neither is something to stop `chaps jobs show` over.
fn database_result(api: &Api, id: &str, status: &str) -> Option<i64> {
    if jobs::Outcome::of(status) != jobs::Outcome::Done {
        return None;
    }
    let path = format!(
        "{}/{}/database_result",
        jobs::JOBS_PATH,
        crate::api::encode(id)
    );
    let answer = api.send("GET", &path, None).ok()?;
    if !answer.is_success() {
        return None;
    }
    answer.json()?.get("id")?.as_i64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out() -> Out {
        Out::default()
    }

    fn job(id: &str, status: &str) -> Job {
        Job {
            id: id.to_string(),
            kind: "create_backtest".to_string(),
            name: "eval".to_string(),
            status: status.to_string(),
            start_time: Some("2026-09-24T16:00:00".to_string()),
            end_time: Some("2026-09-24T16:00:30".to_string()),
            result: Some("4".to_string()),
            prediction_setup_id: None,
        }
    }

    /// Epoch seconds for `2026-09-24T16:01:00Z`.
    const NOW: u64 = 1_790_265_660;

    #[test]
    fn an_empty_list_says_where_a_job_would_come_from() {
        let text = human_list(&[], NOW, &out());
        assert!(text.starts_with("no jobs yet;"), "{text}");
        assert!(text.contains("Modeling App"), "{text}");
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn the_table_is_followed_by_the_line_it_adds_up_to() {
        let jobs = vec![
            job("aaaaaaaa-1111", "SUCCESS"),
            job("bbbbbbbb-2222", "FAILURE"),
        ];
        let text = human_list(&jobs, NOW, &out());
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("ID  "), "{text}");
        assert!(lines[0].contains("DURATION"), "{text}");
        assert!(lines[1].starts_with("aaaaaaaa..."), "{text}");
        assert!(text.contains("2 jobs: 1 done, 1 failed"), "{text}");
        assert!(
            text.contains("run `chaps jobs logs bbbbbbbb-2222` to see why"),
            "{text}"
        );
    }

    #[test]
    fn a_clean_list_offers_no_failure_hint() {
        let text = human_list(&[job("aaaaaaaa-1111", "SUCCESS")], NOW, &out());
        assert!(text.contains("1 job: 1 done"), "{text}");
        assert!(!text.contains("logs"), "{text}");
    }

    #[test]
    fn show_prints_every_field_and_ends_on_the_next_step() {
        let job = job("aaaaaaaa-1111", "SUCCESS");
        let text = human_show(&job, "SUCCESS", Some(4), NOW, &out());
        for label in [
            "Job",
            "Type",
            "Name",
            "Status",
            "Started",
            "Ended",
            "Duration",
            "Result",
            "Database result",
        ] {
            assert!(text.contains(label), "{label} is missing:\n{text}");
        }
        assert!(text.contains("1m ago"), "{text}");
        assert!(text.contains("30s"), "{text}");
        assert!(text.contains("row 4 in chap-core's database"), "{text}");

        // An empty field is left out rather than printed as a blank.
        let mut bare = job.clone();
        bare.end_time = None;
        bare.result = None;
        let text = human_show(&bare, "STARTED", None, NOW, &out());
        assert!(!text.contains("Ended"), "{text}");
        assert!(!text.contains("Result"), "{text}");
        assert!(text.contains("still running"), "{text}");
    }

    #[test]
    fn the_next_step_follows_the_status() {
        let job = job("abc", "SUCCESS");
        assert!(next_step(&job, "FAILURE", None).starts_with("run `chaps jobs logs abc`"));
        assert!(next_step(&job, "STARTED", None).contains("chaps jobs cancel abc"));
        assert!(next_step(&job, "REVOKED", None).contains("was cancelled"));
        assert!(next_step(&job, "SUCCESS", Some(9)).contains("/v1/crud/backtests/9"));
        assert!(next_step(&job, "SUCCESS", None).contains("to see what it did"));

        // The collection follows the job type rather than always being the
        // backtests one.
        let mut dataset = job.clone();
        dataset.kind = "create_dataset".to_string();
        assert!(next_step(&dataset, "SUCCESS", Some(1)).contains("/v1/crud/datasets/1"));
        let mut prediction = job.clone();
        prediction.kind = "create_prediction".to_string();
        assert!(next_step(&prediction, "SUCCESS", Some(2)).contains("/v1/crud/predictions/2"));
        // A job type this CLI has never seen names the row and no path.
        let mut unknown = job.clone();
        unknown.kind = "create_something_new".to_string();
        let text = next_step(&unknown, "SUCCESS", Some(3));
        assert!(text.contains("row 3 in chap-core's database"), "{text}");
        assert!(!text.contains("/v1/crud/"), "{text}");
    }

    #[test]
    fn the_message_is_chap_cores_own_when_it_sent_one() {
        let answer = |body: &str| crate::api::Answer {
            status: 200,
            reason: "OK".to_string(),
            content_type: crate::api::JSON.to_string(),
            body: body.as_bytes().to_vec(),
        };
        assert_eq!(
            message_of(&answer(r#"{"message":"Job cancelled"}"#), "cancelled"),
            "Job cancelled"
        );
        assert_eq!(message_of(&answer("{}"), "cancelled"), "job cancelled");
        assert_eq!(message_of(&answer("not json"), "deleted"), "job deleted");
    }

    #[test]
    fn a_body_that_is_not_a_list_is_named_in_the_error() {
        assert_eq!(kind_of(&serde_json::json!({})), "an object");
        assert_eq!(kind_of(&serde_json::json!("x")), "a string");
        assert_eq!(kind_of(&serde_json::json!(null)), "null");
        assert_eq!(kind_of(&serde_json::json!([])), "a list");
    }
}
