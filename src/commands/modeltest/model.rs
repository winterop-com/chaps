//! The model level: `chapkit test` in the model's own container.

use super::cleanup::{clean, held};
use super::{first_line, period_type, service_info, service_url};
use crate::api::Api;
use crate::cli::ModelsTestArgs;
use crate::commands::Ctx;
use crate::docker;
use crate::modeltest::{self, Level, Run, Verdict};
use crate::project::{EnabledModel, Project};
use std::collections::BTreeSet;
use std::time::Duration;

/// Run `chapkit test` inside one model's container and read what it printed.
#[expect(
    clippy::too_many_arguments,
    reason = "the run's context and the one model it tests"
)]
pub(super) fn model_level(
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
    if enabled.compose_file.is_empty() {
        return run.end(
            Verdict::Skip,
            "varde does not run it, so there is no container to test it in",
            Some(format!(
                "run `varde models test {} --backtest` to test it through chap-core",
                enabled.service_id
            )),
        );
    }
    if !running.contains(&enabled.service_id) {
        return run.end(
            Verdict::Skip,
            "its container is not running",
            Some("run `varde up`".to_string()),
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
    let url = service_url(project, &enabled.service_id);
    exec.extend(
        ["chapkit", "test", "--url", &url, "--timeout"]
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
                "raise the limit with `varde models test {id} --timeout {}`",
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
                    "update the model with `varde update`, or go through chap-core \
                     with `varde models test {id} --backtest`"
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
                "run `varde models test {id} -v` for the full output"
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
                "run `varde models test {id} -v` for the full output, \
                 and `varde logs {}` for the service's own",
                enabled.service_id
            )),
        )
    }
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
