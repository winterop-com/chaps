//! What a wrapper says once docker has finished, and when there was nothing
//! for docker to do.

use super::down::{DownVolumes, down_next, down_summary, forget_dhis2_connect, removed_volumes};
use super::note;
use crate::cli::DockerCmd;
use crate::commands::Ctx;
use crate::components::{Components, dhis2_connect_hint};
use crate::docker;
use crate::output::Out;
use crate::project::Project;

/// What `logs` and `docker ps` say for a project that has no containers at
/// all. Both would otherwise print nothing whatsoever.
pub(super) const NOTHING_RUNNING: &str =
    "nothing is running for this project; start CHAP with `chaps up`";

/// What `restart` says when there is nothing to recreate. Recreating is not
/// starting: a deployment that is down is `chaps up`'s to bring up, the same
/// answer `chaps status` gives.
const NOT_RUNNING: &str = "CHAP is not running; start it with `chaps up`";

/// The hint that closes a detached `up`.
const AFTER_UP: &str = "run `chaps status` to check that everything answers";

/// Say what the wrapper did, now that docker has finished.
///
/// `volumes` are the ones docker held before a `down --volumes`, and empty
/// for every other wrapper.
pub(super) fn report_what_changed(
    ctx: &Ctx,
    project: &mut Project,
    cmd: &DockerCmd,
    before: &[docker::Container],
    volumes: &[String],
) {
    match cmd {
        // An attached `up` has just streamed the logs and been interrupted;
        // there is nothing left running to summarise.
        DockerCmd::Up(args) if !args.attach => {
            let after = docker::running_containers(project).unwrap_or_default();
            note(
                ctx,
                &up_summary(&ctx.out, before, &after, &project.state.components),
            );
        }
        DockerCmd::Restart(args) => {
            let after = docker::running_containers(project).unwrap_or_default();
            note(
                ctx,
                &restart_summary(&ctx.out, before, &after, &args.services),
            );
        }
        DockerCmd::Down(args) => {
            let removed = args.volumes.then(|| removed_volumes(project, volumes));
            note(
                ctx,
                &down_summary(
                    &ctx.out,
                    &docker::service_names(before),
                    match &removed {
                        Some(names) => DownVolumes::Removed(names),
                        None => DownVolumes::Kept,
                    },
                    project.compose_project_name().as_deref(),
                ),
            );
            // Said after the line that names what went, because it is a
            // consequence of it.
            if let Some(names) = &removed
                && let Some(line) = forget_dhis2_connect(project, names)
            {
                note(ctx, &ctx.out.backticks(&line));
            }
            note(ctx, &ctx.out.backticks(down_next(removed.is_some())));
        }
        DockerCmd::Pull(_) => note(
            ctx,
            &ctx.out.ok(&pull_summary(docker::image_count(project))),
        ),
        _ => {}
    }
}

/// What a detached `up` changed, from the containers before and after.
///
/// Compose prints one line per service as it goes, in no particular order and
/// in the language of its own steps ("Created", "Running"); this is the one
/// line that says which services are new to this run.
///
/// A deployment with a DHIS2 nothing has connected gets one more line under
/// that, because this is the run the reader is about to wait minutes for and
/// the line that named `chaps dhis2 connect` scrolled past when the component
/// was added. It comes off `.chaps/components.yaml` alone - `up` asks DHIS2
/// nothing, and there would be nothing to ask yet - so it says what chaps has
/// recorded rather than what DHIS2 is. See [`dhis2_connect_hint`].
///
/// Not on the run that started nothing at all: there is no deployment up to
/// connect, and the line above already says to go and read the logs.
pub fn up_summary(
    out: &Out,
    before: &[docker::Container],
    after: &[docker::Container],
    components: &Components,
) -> String {
    let (started, unchanged) = docker::diff_containers(before, after);
    let started_cell = |names: &[String]| {
        format!(
            "{} {}",
            out.ok("started/recreated:"),
            out.value(&names.join(", "))
        )
    };
    let unchanged_cell = |names: &[String]| out.dim(&format!("unchanged: {}", names.join(", ")));
    let summary = match (started.is_empty(), unchanged.is_empty()) {
        (true, true) => {
            return out.backticks("nothing is running after `up`; run `chaps logs` to see why");
        }
        (false, true) => started_cell(&started),
        (true, false) => unchanged_cell(&unchanged),
        (false, false) => format!("{}; {}", started_cell(&started), unchanged_cell(&unchanged)),
    };
    let mut text = format!("{summary}\n{}", out.backticks(AFTER_UP));
    if components.dhis2_needs_connecting() {
        text.push('\n');
        text.push_str(&out.backticks(&dhis2_connect_hint(false)));
    }
    text
}

/// What `restart` recreated, from the containers before and after.
///
/// Compose recreates only the containers that no longer match the files, and
/// says nothing at all about the ones it skipped; this is the line that says
/// which half each service fell in. A run that recreated nothing is the
/// answer "nothing had moved on under you", not a failure.
///
/// `named` are the services the command was given, which is what the way to
/// force it spells out; none named is the whole project.
pub fn restart_summary(
    out: &Out,
    before: &[docker::Container],
    after: &[docker::Container],
    named: &[String],
) -> String {
    let (recreated, unchanged) = docker::diff_containers(before, after);
    if recreated.is_empty() {
        let force = match named {
            [] => "`chaps restart --all` recreates every one anyway".to_string(),
            [one] => format!("`chaps restart --all {one}` recreates it anyway"),
            many => format!(
                "`chaps restart --all {}` recreates them anyway",
                many.join(" ")
            ),
        };
        return out.backticks(&format!(
            "nothing needed a restart: every container matches its files; {force}"
        ));
    }
    let head = format!(
        "{} {}",
        out.ok("recreated:"),
        out.value(&recreated.join(", "))
    );
    let head = match unchanged.is_empty() {
        true => head,
        false => format!(
            "{head}; {}",
            out.dim(&format!("unchanged: {}", unchanged.join(", ")))
        ),
    };
    format!("{head}\n{}", out.backticks(AFTER_UP))
}

/// The "there is nothing here" answer `logs` and `ps` give a project whose
/// containers have never been created.
pub(super) fn nothing_running(out: &Out) -> String {
    out.backticks(NOTHING_RUNNING)
}

/// The "there is nothing to recreate" answer `restart` gives a deployment
/// that is not running.
pub(super) fn not_running(out: &Out) -> String {
    out.backticks(NOT_RUNNING)
}

/// What `docker pull` fetched. Compose's own output is progress, not a result.
pub fn pull_summary(images: Option<usize>) -> String {
    match images {
        Some(1) => "pulled 1 image".to_string(),
        Some(count) => format!("pulled {count} images"),
        // `config --images` is the only thing that could have failed here, and
        // the pull itself succeeded, so the count is all that is missing.
        None => "pulled the images this project pins".to_string(),
    }
}

/// What `logs SERVICE` says about a name the project does not have.
pub fn unknown_service_message(unknown: &[String], services: &[String]) -> String {
    let named: Vec<String> = unknown.iter().map(|s| format!("`{s}`")).collect();
    format!(
        "no service {} in this project; the services are {}",
        named.join(", "),
        services.join(", ")
    )
}
