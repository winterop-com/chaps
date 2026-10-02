//! `chaps dhis2 analytics`: the analytics tables, generated and waited for.

use super::*;

/// `chaps dhis2 analytics` — generate the analytics tables.
pub fn analytics(ctx: &Ctx, args: &Dhis2AnalyticsArgs) -> Result<()> {
    let session = open_session(ctx, &args.common)?;
    let analytics = run_analytics(ctx, &session, args.timeout, args.no_wait)?;
    let report = Dhis2Report {
        instance: session.instance(),
        next: match (analytics.finished, analytics.job.is_empty()) {
            (true, _) => "run `chaps dhis2 show` to see what is still missing".to_string(),
            // Watching again finds the job while it runs; without its id a
            // second run could just as well start another generation.
            (false, false) => "run `chaps dhis2 analytics` again to watch the same run".to_string(),
            (false, true) => "DHIS2 did not say which job it started; `chaps dhis2 show` says \
                              when the analytics tables are ready"
                .to_string(),
        },
        route: None,
        apps: None,
        analytics: Some(analytics),
        skipped: Vec::new(),
        record: None,
    };
    ctx.out.emit(&report, || human_report(&report, &ctx.out))
}

/// Start an analytics run, or adopt the one that is already going, and wait.
pub(super) fn run_analytics(
    ctx: &Ctx,
    session: &Session,
    timeout: u64,
    no_wait: bool,
) -> Result<AnalyticsReport> {
    let client = &session.dhis2;
    // DHIS2 runs one analytics job at a time. A second POST queues behind the
    // first and its notifier stays empty, so a wait on it would report nothing
    // for as long as the first one takes - which is why the one already going
    // is watched instead of a new one being asked for.
    let running = dhis2::running_job(&client.get_json(&dhis2::jobs_path())?);
    let (job, started) = match running {
        Some(job) => {
            ctx.out
                .verbose(&format!("analytics job {job} is already running"));
            (job, false)
        }
        None => {
            // No body: the endpoint takes its two answers in the query string,
            // and this is the request that was measured against a live DHIS2.
            let answer = client.send("POST", dhis2::ANALYTICS_PATH, None)?;
            if !answer.is_success() {
                return Err(client.status_error(dhis2::ANALYTICS_PATH, &answer));
            }
            let response = answer.json().unwrap_or(serde_json::Value::Null);
            // An id that could not be read is not a failure: DHIS2 has started
            // the run, and the run it is running is the one to watch.
            // Asked twice, a moment apart: DHIS2 may not list the job it has
            // just accepted yet.
            let running = || dhis2::running_job(&client.get_json(&dhis2::jobs_path()).ok()?);
            let job = dhis2::job_id_of(&response)
                .or_else(running)
                .or_else(|| {
                    std::thread::sleep(Duration::from_secs(2));
                    running()
                })
                .unwrap_or_default();
            (job, true)
        }
    };

    if no_wait || job.is_empty() {
        return Ok(AnalyticsReport {
            job,
            started,
            finished: false,
            seconds: 0,
            message: String::new(),
            last_success: session.ready.last_analytics.clone(),
        });
    }

    let began = Instant::now();
    // Not `now() - interval`: a run that finishes on the first poll would print
    // its one step above the report it is part of, which reads as noise.
    let mut last = Instant::now();
    let progress = client.wait_for_job(
        &job,
        Duration::from_secs(timeout),
        dhis2::POLL_INTERVAL,
        // A run that takes an hour must not be an hour of silence. Every step
        // under `-v`, and otherwise one line every half minute, on stderr.
        &mut |message| {
            if ctx.out.is_verbose() {
                ctx.out.verbose(&format!("analytics: {message}"));
            } else if last.elapsed() >= Duration::from_secs(30) {
                last = Instant::now();
                crate::output::notice(&format!("analytics: {message}"));
            }
        },
    )?;
    let seconds = began.elapsed().as_secs();
    if let Progress::Failed(reason) = &progress {
        return Err(anyhow::anyhow!(
            "the analytics run failed after {}: {reason}; `chaps logs dhis2` has the rest, and \
             DHIS2 wants about 4 to 5 GB for the populate phase",
            crate::output::human_age(began.elapsed())
        ));
    }
    Ok(AnalyticsReport {
        job,
        started,
        finished: true,
        seconds,
        message: progress.message().to_string(),
        // Re-read, because this is the fact that says the run landed: DHIS2
        // records the time of its last successful analytics generation.
        last_success: client
            .get_json(dhis2::SYSTEM_INFO_PATH)
            .ok()
            .as_ref()
            .and_then(|info| info.get("lastAnalyticsTableSuccess"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}
