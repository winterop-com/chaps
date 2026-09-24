//! `chaps models test` — make a model do the work, and say whether it could.
//!
//! `chaps status` and `chaps doctor` can both be fully green while a model
//! cannot produce a single prediction: registration is a heartbeat, and a
//! heartbeat says the service is alive, not that its runtime works. This
//! command is the check neither of them can make, at two levels.
//!
//! The model level runs `chapkit test` inside the model's own container.
//! Every chapkit-built image ships it: it reads the service's own config
//! schema, generates data that matches the covariates, period type and
//! geometry the service declares, then validates, trains and predicts. It
//! needs no chap-core at all, which is what makes it the level to reach for
//! when chap-core is the thing that is broken.
//!
//! The backtest level goes the whole way round: chapkit's sample data comes
//! out through chap-core's proxy, goes back in as a dataset, and a two-split
//! backtest is run over it. That is the path the Modeling App takes, so it is
//! the one that proves a model is usable rather than merely runnable.
//!
//! Both levels clean up after themselves, and `--keep` says what was kept.
//! The rules for parsing, transposing and rendering are in
//! [`crate::modeltest`]; this module holds the requests and the `exec`.

use crate::api::Api;
use crate::cli::ModelsTestArgs;
use crate::commands::Ctx;
use crate::docker;
use crate::error::{ChapError, Result};
use crate::jobs;
use crate::modeltest::{self, Level, Run, Verdict};
use crate::output::Out;
use crate::project::{EnabledModel, Project};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// A model service's own address inside its container.
///
/// Not the published host port and not the compose DNS name: `chapkit test`
/// and the `curl` that cleans up both run inside the container, and this is
/// the one address that is right whether or not the model publishes a port.
const SERVICE_URL: &str = "http://127.0.0.1:8000";

/// How long one request to chap-core may take.
///
/// Longer than [`crate::api::DEFAULT_TIMEOUT`]: `make-dataset` is a few
/// thousand observations going over the wire, and `$generate-sample-data`
/// makes the model service compute a frame before it answers.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// How long the `curl` that lists or deletes inside a container may take.
const SERVICE_TIMEOUT: Duration = Duration::from_secs(30);

/// How often the dataset job is asked whether it has finished.
const DATASET_POLL: Duration = Duration::from_secs(3);

/// How often the backtest job is, which is the slower of the two.
const BACKTEST_POLL: Duration = Duration::from_secs(5);

/// What the process exits with when a model failed.
///
/// Everything there was to say is on the screen already, so there is no
/// `error:` line on top of it - the same reasoning as `chaps doctor`.
const EXIT_FAILED: i32 = 1;

/// `chaps models test [ID..] [--all]`.
pub fn run(ctx: &Ctx, args: &ModelsTestArgs) -> Result<()> {
    let project = ctx.project()?;
    let targets = targets(ctx, &project, args)?;
    let level = if args.backtest {
        Level::Backtest
    } else {
        Level::Model
    };
    let timeout = Duration::from_secs(args.timeout.unwrap_or(match level {
        Level::Model => modeltest::MODEL_TIMEOUT,
        Level::Backtest => modeltest::BACKTEST_TIMEOUT,
    }));

    let token = crate::auth::token_in(&project.dir);
    let api = Api::new(&project.api_url(), token, REQUEST_TIMEOUT);
    ctx.out
        .verbose(&format!("asking chap-core at {}", api.base()));

    let names: Vec<String> = targets
        .iter()
        .map(|(_, enabled)| enabled.service_id.clone())
        .collect();
    let width = modeltest::name_width(&names);
    if !ctx.out.json {
        println!("{}", ctx.out.dim(&modeltest::header(targets.len(), level)));
    }

    // One `docker compose ps` for the whole run: a container that was not up
    // when the command started is not one this run can test.
    let running = if level == Level::Model {
        docker::running_services(&project)
    } else {
        BTreeSet::new()
    };

    let mut runs: Vec<Run> = Vec::with_capacity(targets.len());
    for (id, enabled) in &targets {
        let started = Instant::now();
        let mut run = match level {
            Level::Model => model_level(ctx, &project, &api, id, enabled, args, timeout, &running),
            Level::Backtest => backtest_level(ctx, &api, id, enabled, args, timeout),
        };
        run.seconds = started.elapsed().as_secs();
        if !ctx.out.json {
            print_run(&ctx.out, &run, width);
        }
        runs.push(run);
    }

    if ctx.out.json {
        ctx.out.emit(&runs, String::new)?;
    } else {
        println!();
        println!("{}", ctx.out.cmd(&modeltest::closing(&runs)));
    }
    if modeltest::any_failed(&runs) {
        std::process::exit(EXIT_FAILED);
    }
    Ok(())
}

/// One row, and the way out under it when there is one.
fn print_run(out: &Out, run: &Run, width: usize) {
    let verdict = match run.verdict {
        Verdict::Pass => out.ok(run.verdict.label()),
        Verdict::Fail => out.bad(run.verdict.label()),
        Verdict::Skip => out.warn(run.verdict.label()),
    };
    println!("{}", modeltest::row(run, &verdict, width));
    if let Some(detail) = &run.detail {
        println!("  {}", out.backticks(detail));
    }
}

/// The models this run is about, in the order they will be tested.
///
/// Ids are resolved the way `models enable` and `models info` resolve them -
/// a marketplace id or a service id, a manual definition included - and a
/// model that is not enabled here is a mistake rather than a skip: there is
/// no container to test and no registration to go through.
fn targets(
    ctx: &Ctx,
    project: &Project,
    args: &ModelsTestArgs,
) -> Result<Vec<(String, EnabledModel)>> {
    if args.all {
        let all: Vec<(String, EnabledModel)> = project
            .state
            .models
            .iter()
            .map(|(id, enabled)| (id.clone(), enabled.clone()))
            .collect();
        if all.is_empty() {
            return Err(anyhow::anyhow!(
                "this deployment enables no models; enable one with `chaps models enable ID`"
            ));
        }
        return Ok(all);
    }
    if args.ids.is_empty() {
        return Err(ChapError::Usage(
            "name a model to test, or pass --all for every model this deployment enables"
                .to_string(),
        )
        .into());
    }

    let registry = super::registry_for(ctx, Some(project))?;
    let mut picked: Vec<(String, EnabledModel)> = Vec::new();
    for given in &args.ids {
        let model = registry
            .get(given)
            .ok_or_else(|| ChapError::UnknownModel(given.clone()))?;
        let enabled = project.state.models.get(&model.id).ok_or_else(|| {
            anyhow::anyhow!(
                "{} is not enabled in this project; enable it with `chaps models enable {}`",
                model.id,
                model.id
            )
        })?;
        if picked.iter().any(|(id, _)| id == &model.id) {
            continue;
        }
        picked.push((model.id.clone(), enabled.clone()));
    }
    Ok(picked)
}

// ---------------------------------------------------------------------------
// The model level: `chapkit test` in the model's own container
// ---------------------------------------------------------------------------

/// Run `chapkit test` inside one model's container and read what it printed.
#[allow(clippy::too_many_arguments)]
fn model_level(
    ctx: &Ctx,
    project: &Project,
    api: &Api,
    id: &str,
    enabled: &EnabledModel,
    args: &ModelsTestArgs,
    timeout: Duration,
    running: &BTreeSet<String>,
) -> Run {
    let run = Run::new(id, &enabled.service_id, Level::Model);
    if !running.contains(&enabled.service_id) {
        return run.end(
            Verdict::Skip,
            "its container is not running",
            Some("run `chaps up`".to_string()),
        );
    }

    // Best effort, and only used to sharpen two sentences: which chapkit the
    // service reports when the image has no `chapkit test`, and whether the
    // generated data has to be weekly. A chap-core that is down costs both
    // and nothing else, which is the point of this level.
    let info = service_info(ctx, api, &enabled.service_id);

    let mut exec: Vec<String> = ["exec", "-T", &enabled.service_id]
        .iter()
        .map(|part| part.to_string())
        .collect();
    exec.extend(
        ["chapkit", "test", "--url", SERVICE_URL, "--timeout"]
            .iter()
            .map(|part| part.to_string()),
    );
    exec.push(timeout.as_secs().to_string());
    if let Some(seed) = args.seed {
        exec.push("--seed".to_string());
        exec.push(seed.to_string());
    }
    // `chapkit test` generates monthly data unless told otherwise, and a
    // service that only accepts weekly periods would refuse every row of it.
    if period_type(info.as_ref()) == "weekly" {
        exec.push("--period-type".to_string());
        exec.push("weekly".to_string());
    }

    let before = held(ctx, project, api, &enabled.service_id);
    let outcome = match docker::compose_captured(project, &exec, ctx.out.is_verbose(), timeout) {
        Ok(outcome) => outcome,
        Err(err) => {
            return run.end(
                Verdict::Skip,
                "docker could not run it",
                Some(first_line(&err.to_string())),
            );
        }
    };
    let text = outcome.text();

    if outcome.timed_out {
        clean(ctx, project, api, &enabled.service_id, &before, args.keep);
        return run.end(
            Verdict::Skip,
            format!("no answer in {}", modeltest::took(timeout.as_secs())),
            Some(format!(
                "raise the limit with `chaps models test {id} --timeout {}`",
                timeout.as_secs() * 2
            )),
        );
    }

    let Some(summary) = modeltest::parse_summary(&text) else {
        if modeltest::chapkit_missing(outcome.code, &text) {
            return run.end(
                Verdict::Skip,
                format!("the image has no `chapkit test`{}", reported(info.as_ref())),
                Some(format!(
                    "update the model with `chaps update`, or go through chap-core \
                     with `chaps models test {id} --backtest`"
                )),
            );
        }
        clean(ctx, project, api, &enabled.service_id, &before, args.keep);
        return run.end(
            Verdict::Fail,
            format!(
                "chapkit test printed no {} block",
                modeltest::SUMMARY_MARKER
            ),
            Some(format!(
                "run `chaps models test {id} -v` for the full output"
            )),
        );
    };

    clean(ctx, project, api, &enabled.service_id, &before, args.keep);
    if summary.passed {
        run.end(Verdict::Pass, summary.did(), None)
    } else {
        run.end(
            Verdict::Fail,
            summary.why(),
            Some(format!(
                "run `chaps models test {id} -v` for the full output, \
                 and `chaps logs {}` for the service's own",
                enabled.service_id
            )),
        )
    }
}

// ---------------------------------------------------------------------------
// The backtest level: through chap-core, the way the Modeling App goes
// ---------------------------------------------------------------------------

/// Build a dataset from the model's own sample data, backtest over it, and
/// report the scores.
fn backtest_level(
    ctx: &Ctx,
    api: &Api,
    id: &str,
    enabled: &EnabledModel,
    args: &ModelsTestArgs,
    timeout: Duration,
) -> Run {
    let run = Run::new(id, &enabled.service_id, Level::Backtest);
    let until = Instant::now() + timeout;

    let Some(info) = service_info(ctx, api, &enabled.service_id) else {
        return run.end(
            Verdict::Skip,
            "chap-core has no registration for it",
            Some("run `chaps status` to see why".to_string()),
        );
    };
    let period = period_type(Some(&info));

    let sample = match sample_data(api, &enabled.service_id, &period, args.seed) {
        Ok(Some(sample)) => sample,
        Ok(None) => {
            return run.end(
                Verdict::Skip,
                format!(
                    "the image's chapkit predates the sample-data route \
                     (needs chapkit {}, this one reports {})",
                    modeltest::SAMPLE_DATA_CHAPKIT,
                    chapkit_version(Some(&info))
                ),
                Some(format!(
                    "update the model with `chaps update`, or test it on its own \
                     with `chaps models test {id}`"
                )),
            );
        }
        Err(err) => {
            return run.end(
                Verdict::Skip,
                "its sample data could not be fetched",
                Some(first_line(&err.to_string())),
            );
        }
    };

    let frame = match sample.get("data") {
        Some(frame) => frame,
        None => {
            return run.end(
                Verdict::Skip,
                "its sample data carries no frame",
                Some("run `chaps models test --backtest -v` to see the answer".to_string()),
            );
        }
    };
    let (observations, locations) = match modeltest::observations(frame) {
        Ok(parts) => parts,
        Err(err) => {
            return run.end(
                Verdict::Skip,
                "its sample data is not a frame chaps understands",
                Some(first_line(&err.to_string())),
            );
        }
    };
    let geojson = modeltest::feature_collection(sample.get("geo"), &locations);
    let name = format!(
        "chaps-test-{}-{}",
        enabled.service_id,
        crate::backup::stamp(crate::backup::now())
    );
    ctx.out.verbose(&format!(
        "{}: {} observations over {} org units as `{name}`",
        enabled.service_id,
        observations.len(),
        locations.len()
    ));

    // --- the dataset ---
    let body = serde_json::json!({
        "name": name,
        "geojson": geojson,
        "providedData": observations,
        "dataToBeFetched": [],
    });
    let made = match post(api, "/v1/analytics/make-dataset", &body) {
        Ok(made) => made,
        Err(err) => {
            return run.end(
                Verdict::Skip,
                "chap-core would not take the dataset",
                Some(first_line(&err.to_string())),
            );
        }
    };
    if made.get("importedCount").and_then(|c| c.as_i64()) == Some(0) {
        return run.end(
            Verdict::Fail,
            "chap-core imported no org units from the sample data",
            Some("run `chaps models test --backtest -v` to see what was sent".to_string()),
        );
    }
    let Some(dataset_job) = made.get("id").and_then(|id| id.as_str()) else {
        return run.end(
            Verdict::Skip,
            "chap-core answered make-dataset without a job id",
            Some("run `chaps models test --backtest -v` to see the answer".to_string()),
        );
    };
    let mut run = run;
    run.job_id = Some(dataset_job.to_string());
    match wait(ctx, api, dataset_job, DATASET_POLL, until) {
        Ok(Some(status)) if jobs::Outcome::of(&status) == jobs::Outcome::Done => {}
        Ok(Some(_)) => {
            let (why, next) = job_failure(api, dataset_job);
            return run.end(Verdict::Fail, format!("dataset: {why}"), Some(next));
        }
        Ok(None) => {
            return run.end(
                Verdict::Skip,
                format!(
                    "its dataset was still building after {}",
                    modeltest::took(timeout.as_secs())
                ),
                Some(format!(
                    "run `chaps jobs show {dataset_job}` for where it got to"
                )),
            );
        }
        Err(err) => {
            return run.end(
                Verdict::Skip,
                "chap-core stopped answering",
                Some(first_line(&err.to_string())),
            );
        }
    }
    let Some(dataset) = database_result(api, dataset_job) else {
        return run.end(
            Verdict::Skip,
            "chap-core built the dataset without saying which row it is",
            Some(format!("run `chaps jobs show {dataset_job}`")),
        );
    };
    ctx.out
        .verbose(&format!("{}: dataset {dataset}", enabled.service_id));

    // --- the backtest ---
    let body = serde_json::json!({
        "name": name,
        "modelId": enabled.service_id,
        "datasetId": dataset,
        "nPeriods": modeltest::BACKTEST_PERIODS,
        "nSplits": modeltest::BACKTEST_SPLITS,
        "stride": modeltest::BACKTEST_STRIDE,
    });
    let started = match post(api, "/v1/analytics/create-backtest", &body) {
        Ok(started) => started,
        Err(err) => {
            drop_rows(ctx, api, None, Some(dataset), args.keep);
            return run.end(
                Verdict::Skip,
                "chap-core would not start the backtest",
                Some(first_line(&err.to_string())),
            );
        }
    };
    let Some(backtest_job) = started
        .get("id")
        .and_then(|id| id.as_str())
        .map(str::to_string)
    else {
        drop_rows(ctx, api, None, Some(dataset), args.keep);
        return run.end(
            Verdict::Skip,
            "chap-core answered create-backtest without a job id",
            Some("run `chaps models test --backtest -v` to see the answer".to_string()),
        );
    };
    run.job_id = Some(backtest_job.clone());

    match wait(ctx, api, &backtest_job, BACKTEST_POLL, until) {
        Ok(Some(status)) if jobs::Outcome::of(&status) == jobs::Outcome::Done => {}
        Ok(Some(_)) => {
            let (why, next) = job_failure(api, &backtest_job);
            drop_rows(ctx, api, None, Some(dataset), args.keep);
            return run.end(Verdict::Fail, why, Some(next));
        }
        Ok(None) => {
            // Nothing is deleted: the job is still running, and its dataset
            // is what it is reading.
            return run.end(
                Verdict::Skip,
                format!(
                    "it was still backtesting after {}",
                    modeltest::took(timeout.as_secs())
                ),
                Some(format!(
                    "run `chaps jobs show {backtest_job}` for where it got to; \
                     dataset {dataset} was left behind"
                )),
            );
        }
        Err(err) => {
            return run.end(
                Verdict::Skip,
                "chap-core stopped answering",
                Some(first_line(&err.to_string())),
            );
        }
    }

    let backtest = database_result(api, &backtest_job);
    run.backtest_id = backtest;
    let metrics = backtest.and_then(|row| {
        api.get_json(&format!("/v1/crud/backtests/{row}"))
            .ok()?
            .get("aggregateMetrics")
            .cloned()
    });
    let summary = match &metrics {
        Some(metrics) => modeltest::metrics_cell(metrics),
        None => "it finished, but chap-core reported no scores".to_string(),
    };
    run.metrics = metrics;
    drop_rows(ctx, api, backtest, Some(dataset), args.keep);
    run.end(Verdict::Pass, summary, None)
}

/// `GET /v2/services/{id}` — the `info` block the service registered with.
///
/// `None` both for a service chap-core has never seen and for a chap-core
/// that cannot be reached: neither is worth an error of its own here, and the
/// caller says what it cost.
fn service_info(ctx: &Ctx, api: &Api, service_id: &str) -> Option<serde_json::Value> {
    let path = format!("/v2/services/{}", crate::api::encode(service_id));
    match api.send("GET", &path, None) {
        Ok(answer) if answer.is_success() => answer.json()?.get("info").cloned(),
        Ok(answer) => {
            ctx.out.verbose(&format!(
                "{service_id}: {} answered {}",
                api.url(&path),
                answer.status_line()
            ));
            None
        }
        Err(err) => {
            ctx.out
                .verbose(&format!("{service_id}: {}", first_line(&err.to_string())));
            None
        }
    }
}

/// The period type the generated data has to be in.
///
/// A service that declares `any` takes both, and monthly is what chapkit's
/// generator and chap-core's dataset both default to.
fn period_type(info: Option<&serde_json::Value>) -> String {
    match info
        .and_then(|info| info.get("period_type"))
        .and_then(|value| value.as_str())
        .unwrap_or("monthly")
    {
        "weekly" => "weekly".to_string(),
        _ => "monthly".to_string(),
    }
}

/// The chapkit release the service says it was built with.
fn chapkit_version(info: Option<&serde_json::Value>) -> String {
    info.and_then(|info| info.get("chapkit_version"))
        .and_then(|value| value.as_str())
        .unwrap_or("nothing")
        .to_string()
}

/// `; the service reports chapkit 2.0.0`, or nothing when it did not answer.
fn reported(info: Option<&serde_json::Value>) -> String {
    match info.and_then(|info| info.get("chapkit_version")) {
        Some(value) if value.is_string() => format!(
            "; the service reports chapkit {}",
            value.as_str().unwrap_or_default()
        ),
        _ => String::new(),
    }
}

/// `GET .../$generate-sample-data` through chap-core's proxy.
///
/// `Ok(None)` for a 404, which is how a model built before chapkit
/// [`modeltest::SAMPLE_DATA_CHAPKIT`] says it has no such route.
fn sample_data(
    api: &Api,
    service_id: &str,
    period: &str,
    seed: Option<i64>,
) -> Result<Option<serde_json::Value>> {
    // `%24` rather than a bare `$`: the proxy forwards the raw sub-path, and
    // the encoded form is the spelling every hop agrees on.
    //
    // `include_geo=true` whatever the service declares. chap-core's dataset
    // always carries a geometry, so a model that says it needs none would
    // otherwise be handed features with no geometry in them - and a model
    // that builds a neighbour graph fails on an empty polygon where it would
    // have been perfectly happy with none at all. Real polygons are never
    // worse: a model that ignores geometry ignores these too.
    let mut path = format!(
        "/v2/services/{}/run/api/v1/ml/%24generate-sample-data?kind=train&num_locations={}&num_periods={}&period_type={period}&include_geo=true",
        crate::api::encode(service_id),
        modeltest::SAMPLE_LOCATIONS,
        modeltest::SAMPLE_PERIODS,
    );
    if let Some(seed) = seed {
        path.push_str(&format!("&seed={seed}"));
    }
    let answer = api.send("GET", &path, None)?;
    if answer.status == 404 {
        return Ok(None);
    }
    if !answer.is_success() {
        return Err(api.status_error(&path, &answer));
    }
    answer
        .json()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("{} did not answer with JSON", api.url(&path)))
}

/// `POST path` with a JSON body, where a non-2xx is an error.
fn post(api: &Api, path: &str, body: &serde_json::Value) -> Result<serde_json::Value> {
    let text = serde_json::to_string(body)?;
    let answer = api.send_json("POST", path, Some(&text))?;
    if !answer.is_success() {
        return Err(api.status_error(path, &answer));
    }
    answer
        .json()
        .ok_or_else(|| anyhow::anyhow!("{} did not answer with JSON", api.url(path)))
}

/// Poll one job until it is no longer running.
///
/// `Ok(None)` when the deadline passed first, which leaves the job running:
/// chap-core is doing the work and stopping it is the operator's call, so the
/// caller names the job rather than cancelling it.
fn wait(
    ctx: &Ctx,
    api: &Api,
    job: &str,
    every: Duration,
    until: Instant,
) -> Result<Option<String>> {
    loop {
        let status = job_status(api, job)?;
        if !jobs::Outcome::of(&status).is_running() {
            return Ok(Some(status));
        }
        if Instant::now() + every >= until {
            return Ok(None);
        }
        ctx.out.verbose(&format!("job {job} {status}"));
        std::thread::sleep(every);
    }
}

/// `GET /v1/jobs/{id}`, which answers with a bare status string.
fn job_status(api: &Api, job: &str) -> Result<String> {
    let path = format!("{}/{}", jobs::JOBS_PATH, crate::api::encode(job));
    let answer = api.send("GET", &path, None)?;
    if !answer.is_success() {
        return Err(api.status_error(&path, &answer));
    }
    Ok(match answer.json() {
        Some(serde_json::Value::String(text)) => text,
        _ => answer.text().trim().trim_matches('"').to_string(),
    })
}

/// The row a finished job wrote, from `GET /v1/jobs/{id}/database_result`.
fn database_result(api: &Api, job: &str) -> Option<i64> {
    let path = format!(
        "{}/{}/database_result",
        jobs::JOBS_PATH,
        crate::api::encode(job)
    );
    let answer = api.send("GET", &path, None).ok()?;
    if !answer.is_success() {
        return None;
    }
    answer.json()?.get("id")?.as_i64()
}

/// Why a failed job failed, and what to run about it.
///
/// The job description carries a status and nothing else, so the reason is in
/// the log; the rule for picking the line out of it is the one
/// `chaps jobs logs` already uses.
fn job_failure(api: &Api, job: &str) -> (String, String) {
    let next = format!("run `chaps jobs logs {job}`");
    let path = format!("{}/{}/logs", jobs::JOBS_PATH, crate::api::encode(job));
    let text = match api.send("GET", &path, None) {
        Ok(answer) if answer.is_success() => match answer.json() {
            Some(serde_json::Value::String(text)) => text,
            _ => answer.text().into_owned(),
        },
        _ => String::new(),
    };
    // The model's own error stream first, then chap-core's record of it: a
    // job that failed before the model was reached has no stderr section at
    // all, and the traceback is then the whole of the answer.
    match jobs::stderr_hint(&text).or_else(|| modeltest::log_hint(&text)) {
        Some(hint) => (hint, next),
        None => (
            "the job failed and its log says nothing more".to_string(),
            next,
        ),
    }
}

/// Delete the rows this run created, and nothing else.
fn drop_rows(ctx: &Ctx, api: &Api, backtest: Option<i64>, dataset: Option<i64>, keep: bool) {
    if keep {
        let kept: Vec<(&str, i64)> = [("backtests", backtest), ("datasets", dataset)]
            .into_iter()
            .filter_map(|(collection, row)| row.map(|row| (collection, row)))
            .collect();
        if !kept.is_empty() {
            let named: Vec<String> = kept
                .iter()
                .map(|(collection, row)| format!("{} {row}", collection.trim_end_matches('s')))
                .collect();
            let commands: Vec<String> = kept
                .iter()
                .map(|(collection, row)| format!("`chaps api DELETE /v1/crud/{collection}/{row}`"))
                .collect();
            // The backtest first, because chap-core will not forget a dataset
            // something still points at.
            crate::output::notice(&format!(
                "kept {}; remove them with {}",
                named.join(" and "),
                commands.join(" then ")
            ));
        }
        return;
    }
    // The backtest first: it points at the dataset, and chap-core refuses to
    // forget a dataset something still references.
    for (collection, row) in [("backtests", backtest), ("datasets", dataset)] {
        let Some(row) = row else { continue };
        let path = format!("/v1/crud/{collection}/{row}");
        match api.send("DELETE", &path, None) {
            Ok(answer) if answer.is_success() => ctx.out.verbose(&format!("deleted {path}")),
            Ok(answer) => crate::output::notice(&format!(
                "could not delete {path} ({}); remove it with `chaps api DELETE {path}`",
                answer.status_line()
            )),
            Err(err) => crate::output::notice(&format!(
                "could not delete {path} ({}); remove it with `chaps api DELETE {path}`",
                first_line(&err.to_string())
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// What the model level leaves in the service's own database
// ---------------------------------------------------------------------------

/// The configs and artifacts one model service holds.
///
/// `chapkit test` writes one config named `test_config_<ulid>` plus a handful
/// of artifacts, and both stay in the service's own database. Diffing the two
/// listings is what lets the cleanup delete exactly what this run created and
/// leave a config somebody is using alone.
#[derive(Debug, Clone, Default)]
struct Held {
    /// `(id, name)`, because the note about what was left names the config.
    configs: Vec<(String, String)>,
    artifacts: Vec<String>,
}

/// What the service holds now, or `None` when it could not be asked.
fn held(ctx: &Ctx, project: &Project, api: &Api, service_id: &str) -> Option<Held> {
    let configs = list(ctx, project, api, service_id, "configs")?;
    let artifacts = list(ctx, project, api, service_id, "artifacts")?;
    Some(Held {
        configs: configs
            .iter()
            .map(|entry| (entry.0.clone(), entry.1.clone()))
            .collect(),
        artifacts: artifacts.into_iter().map(|entry| entry.0).collect(),
    })
}

/// `GET /api/v1/{collection}` as `(id, name)` pairs.
///
/// Through chap-core's proxy first, which needs nothing of the container, and
/// otherwise with the `curl` every chapkit image carries for its healthcheck.
fn list(
    ctx: &Ctx,
    project: &Project,
    api: &Api,
    service_id: &str,
    collection: &str,
) -> Option<Vec<(String, String)>> {
    let path = format!(
        "/v2/services/{}/run/api/v1/{collection}",
        crate::api::encode(service_id)
    );
    if let Ok(answer) = api.send("GET", &path, None)
        && answer.is_success()
        && let Some(value) = answer.json()
        && let Some(entries) = entries_of(&value)
    {
        return Some(entries);
    }
    let url = format!("{SERVICE_URL}/api/v1/{collection}");
    let exec: Vec<String> = ["exec", "-T", service_id, "curl", "-fsS", &url]
        .iter()
        .map(|part| part.to_string())
        .collect();
    let outcome = docker::compose_captured(project, &exec, false, SERVICE_TIMEOUT).ok()?;
    if outcome.code != 0 {
        ctx.out.verbose(&format!(
            "{service_id}: could not list {collection}: {}",
            first_line(outcome.stderr.trim())
        ));
        return None;
    }
    entries_of(&serde_json::from_str(&outcome.stdout).ok()?)
}

/// The `(id, name)` pairs of a chapkit list answer.
///
/// A bare array is what chapkit sends; an object with the list under `items`
/// or `data` is accepted too, so a chapkit that grows a page wrapper still
/// gets cleaned up after.
fn entries_of(value: &serde_json::Value) -> Option<Vec<(String, String)>> {
    let list = value
        .as_array()
        .or_else(|| value.get("items")?.as_array())
        .or_else(|| value.get("data")?.as_array())?;
    Some(
        list.iter()
            .filter_map(|entry| {
                let id = entry.get("id")?.as_str()?.to_string();
                let name = entry
                    .get("name")
                    .and_then(|name| name.as_str())
                    .unwrap_or_default()
                    .to_string();
                Some((id, name))
            })
            .collect(),
    )
}

/// Delete whatever appeared in the service's database while the test ran.
///
/// Configs first: chapkit cascades a config delete to the artifact trees
/// linked to it, so the artifacts a training and a prediction left behind
/// usually go with it and the second pass has nothing to do.
fn clean(
    ctx: &Ctx,
    project: &Project,
    api: &Api,
    service_id: &str,
    before: &Option<Held>,
    keep: bool,
) {
    let Some(before) = before else {
        crate::output::notice(&format!(
            "{service_id}: could not list its configs, so nothing was cleaned up; \
             `chaps api GET /v2/services/{service_id}/run/api/v1/configs` shows what it holds"
        ));
        return;
    };
    let Some(after) = held(ctx, project, api, service_id) else {
        crate::output::notice(&format!(
            "{service_id}: could not list its configs afterwards, so nothing was cleaned up"
        ));
        return;
    };

    let new_configs: Vec<(String, String)> = after
        .configs
        .iter()
        .filter(|(id, _)| !before.configs.iter().any(|(was, _)| was == id))
        .cloned()
        .collect();
    let new_artifacts: Vec<String> = after
        .artifacts
        .iter()
        .filter(|id| !before.artifacts.contains(id))
        .cloned()
        .collect();
    if new_configs.is_empty() && new_artifacts.is_empty() {
        return;
    }

    if keep {
        let named: Vec<String> = new_configs
            .iter()
            .map(|(id, name)| {
                if name.is_empty() {
                    id.clone()
                } else {
                    name.clone()
                }
            })
            .collect();
        crate::output::notice(&format!(
            "{service_id}: kept {} and {} artifact{} in its database",
            if named.is_empty() {
                "no config".to_string()
            } else {
                named.join(", ")
            },
            new_artifacts.len(),
            if new_artifacts.len() == 1 { "" } else { "s" }
        ));
        return;
    }

    let mut left: Vec<String> = Vec::new();
    for (id, name) in &new_configs {
        if !delete(ctx, project, service_id, &format!("/api/v1/configs/{id}")) {
            left.push(if name.is_empty() {
                id.clone()
            } else {
                name.clone()
            });
        }
    }
    // The config delete cascades, so this is the remainder rather than the
    // list: asking again costs one request and deletes nothing twice.
    if let Some(now) = held(ctx, project, api, service_id) {
        for id in new_artifacts.iter().filter(|id| now.artifacts.contains(id)) {
            if !delete(ctx, project, service_id, &format!("/api/v1/artifacts/{id}")) {
                left.push(format!("artifact {id}"));
            }
        }
    }
    if !left.is_empty() {
        crate::output::notice(&format!(
            "{service_id}: could not clean up {}; \
             delete it from the service's own API or restart the model",
            left.join(", ")
        ));
    }
}

/// `curl -X DELETE` inside the container.
///
/// chap-core's proxy is read-only by design, so a delete cannot go through
/// it; the service's own API is one `exec` away, and every chapkit image
/// carries the `curl` its healthcheck uses.
fn delete(ctx: &Ctx, project: &Project, service_id: &str, path: &str) -> bool {
    let url = format!("{SERVICE_URL}{path}");
    let exec: Vec<String> = [
        "exec", "-T", service_id, "curl", "-fsS", "-X", "DELETE", &url,
    ]
    .iter()
    .map(|part| part.to_string())
    .collect();
    match docker::compose_captured(project, &exec, false, SERVICE_TIMEOUT) {
        Ok(outcome) if outcome.code == 0 => {
            ctx.out.verbose(&format!("{service_id}: deleted {path}"));
            true
        }
        Ok(outcome) => {
            ctx.out.verbose(&format!(
                "{service_id}: DELETE {path} exited {}: {}",
                outcome.code,
                first_line(outcome.stderr.trim())
            ));
            false
        }
        Err(_) => false,
    }
}

/// The first line of a message, for a cell that has room for one.
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().trim().to_string()
}
