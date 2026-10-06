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
        return Err(anyhow::anyhow!(failure_message(
            reason,
            &crate::output::human_age(began.elapsed())
        )));
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

/// The message for an analytics run that failed, from the reason DHIS2 gave.
///
/// A failed SQL statement comes back whole, with the statement in front of
/// the cause, and a statement of DHIS2 analytics is several screens long. So
/// the cause after `ERROR:` is kept, a long run of one digit is cut, and the
/// memory note is there only when the cause is about memory.
pub(super) fn failure_message(reason: &str, after: &str) -> String {
    let cause = match reason.rfind("ERROR:") {
        Some(at) => reason[at + "ERROR:".len()..].trim(),
        None => reason.trim(),
    };
    let cause = shorten_numbers(cause);
    let lower = cause.to_lowercase();
    let next = if lower.contains("out of range for type double precision") {
        "a data value is a number too large for DHIS2; find it in the `datavalue` table, then \
         correct it or delete it in DHIS2"
    } else if ["memory", "heap", "killed", "terminat"]
        .iter()
        .any(|word| lower.contains(word))
    {
        "DHIS2 wants about 4 to 5 GB for the populate phase; see `DHIS2_JAVA_TOOL_OPTIONS` in `.env`"
    } else {
        "`chaps logs dhis2` has the rest"
    };
    format!("the analytics run failed after {after}: {cause}; {next}")
}

/// Cut each run of more than 20 digits to its first 12, so that a number with
/// thousands of digits reads as one: `999999999999... (2081 digits)`.
fn shorten_numbers(text: &str) -> String {
    let mut out = String::new();
    let mut digits = String::new();
    let flush = |out: &mut String, digits: &mut String| {
        match digits.len() > 20 {
            true => out.push_str(&format!("{}... ({} digits)", &digits[..12], digits.len())),
            false => out.push_str(digits),
        }
        digits.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            flush(&mut out, &mut digits);
            out.push(c);
        }
    }
    flush(&mut out, &mut digits);
    out
}
