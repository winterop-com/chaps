//! The backtest level: through chap-core, the way the Modeling App goes.

use super::{first_line, period_type, service_info};
use crate::api::Api;
use crate::cli::ModelsTestArgs;
use crate::commands::Ctx;
use crate::error::Result;
use crate::jobs;
use crate::modeltest::{self, Level, Run, Verdict};
use crate::project::EnabledModel;
use std::time::{Duration, Instant};

/// How often the dataset job is asked whether it has finished.
const DATASET_POLL: Duration = Duration::from_secs(3);

/// How often the backtest job is, which is the slower of the two.
const BACKTEST_POLL: Duration = Duration::from_secs(5);

/// Build a dataset from the model's own sample data, backtest over it, and
/// report the scores.
pub(super) fn backtest_level(
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

    // A registration is the model calling chap-core; a backtest is chap-core
    // calling the model. When that way back is broken, every later step fails
    // with a message about something else, so it is asked first.
    let proxied = crate::status::proxied_health_path(&enabled.service_id);
    let unreachable = match api.send("GET", &proxied, None) {
        Ok(answer) if answer.status >= 500 => Some(answer.status_line()),
        Ok(_) => None,
        Err(err) => Some(err.to_string()),
    };
    if let Some(answer) = unreachable {
        return run.end(
            Verdict::Skip,
            format!("chap-core cannot reach it ({answer} from {proxied})"),
            Some("run `chaps status`, which names the address and the fix".to_string()),
        );
    }

    // Before anything is built: a service chap-core has nothing configured
    // for cannot be backtested, and finding that out after a dataset has been
    // imported would be a dataset created and deleted for nothing.
    let Some((model_id, covariates)) = configured_model(ctx, api, &enabled.service_id) else {
        return run.end(
            Verdict::Skip,
            format!(
                "chap-core has no configured model for {}",
                enabled.service_id
            ),
            Some(format!(
                "it is registered but nothing runs it, run `chaps restart --all {}` and try \
                 again",
                enabled.service_id
            )),
        );
    };

    let sample = match sample_data(
        api,
        &enabled.service_id,
        &period,
        covariates.len(),
        args.seed,
    ) {
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

    let mut sample = sample;
    let frame = match sample.get_mut("data") {
        Some(frame) => frame,
        None => {
            return run.end(
                Verdict::Skip,
                "its sample data carries no frame",
                Some("run `chaps models test --backtest -v` to see the answer".to_string()),
            );
        }
    };
    for (from, to) in modeltest::fill_covariates(frame, &covariates) {
        ctx.out.verbose(&format!(
            "{}: the sample has no `{to}`, which the configured model asks for; `{from}` stands in",
            enabled.service_id
        ));
    }
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
        "modelId": model_id,
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

/// What `create-backtest` is given as its `modelId` for this service.
///
/// chap-core takes either the integer key of a configured model or a string it
/// resolves against their names, and the string only works while the service
/// has a configured model named plainly after it. That is true of a service
/// chap-core has just met and stops being true the moment it re-registers with
/// a new version: chap-core then syncs the configs the service itself holds as
/// `<service id>:<config name>` and the bare name is gone, so the old spelling
/// comes back as `ValueError: Configured model with name ... not found` from
/// inside the job. The row is therefore chosen here, by
/// [`modeltest::configured_model_for`], and its id is what goes out, with the
/// covariates the backtest will hand the model.
///
/// `None` is a chap-core that listed its configured models and had none for
/// this service, which is a skip. A chap-core that could not be asked at all
/// is not: the service id is the spelling that worked before this, and a
/// listing that failed is no reason to refuse to backtest.
fn configured_model(
    ctx: &Ctx,
    api: &Api,
    service_id: &str,
) -> Option<(serde_json::Value, Vec<String>)> {
    const PATH: &str = "/v1/crud/configured-models";
    let listed = match api.send("GET", PATH, None) {
        Ok(answer) if answer.is_success() => answer.json(),
        Ok(answer) => {
            ctx.out.verbose(&format!(
                "{service_id}: {} answered {}",
                api.url(PATH),
                answer.status_line()
            ));
            None
        }
        Err(err) => {
            ctx.out
                .verbose(&format!("{service_id}: {}", first_line(&err.to_string())));
            None
        }
    };
    let Some(listed) = listed else {
        ctx.out.verbose(&format!(
            "{service_id}: could not read chap-core's configured models, sending the service id"
        ));
        return Some((serde_json::json!(service_id), Vec::new()));
    };
    let models = modeltest::configured_models(&listed);
    let chosen = modeltest::configured_model_for(&models, service_id)?;
    ctx.out.verbose(&format!(
        "{service_id}: configured model {} {}",
        chosen.id, chosen.name
    ));
    Some((serde_json::json!(chosen.id), chosen.covariates.clone()))
}

/// The chapkit release the service says it was built with.
fn chapkit_version(info: Option<&serde_json::Value>) -> String {
    info.and_then(|info| info.get("chapkit_version"))
        .and_then(|value| value.as_str())
        .unwrap_or("nothing")
        .to_string()
}

/// `GET .../$generate-sample-data` through chap-core's proxy.
///
/// `Ok(None)` for a 404, which is how a model built before chapkit
/// [`modeltest::SAMPLE_DATA_CHAPKIT`] says it has no such route.
///
/// `covariates` is how many the configured model asks for: at least that many
/// `feature_N` columns are generated, so each one the sample lacks has a spare
/// to be renamed from (see [`modeltest::fill_covariates`]).
fn sample_data(
    api: &Api,
    service_id: &str,
    period: &str,
    covariates: usize,
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
        "/v2/services/{}/run/api/v1/ml/%24generate-sample-data?kind=train&num_locations={}&num_periods={}&period_type={period}&include_geo=true&num_features={}",
        crate::api::encode(service_id),
        modeltest::SAMPLE_LOCATIONS,
        modeltest::SAMPLE_PERIODS,
        covariates.max(modeltest::SAMPLE_FEATURES),
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
