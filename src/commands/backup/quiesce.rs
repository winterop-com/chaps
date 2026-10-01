//! Holding a running service still while its data volume is read, and
//! asking Docker what is running at most once per run.

use crate::backup;
use crate::docker;
use crate::output;
use crate::project::Project;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// `docker compose ps`, asked at most once per run and only when something
/// needs to know.
///
/// Three parts of a backup do: the database dump refuses without a postgres,
/// and both volume captures pause whatever is running while they read. A run
/// that captures none of them asks Docker nothing at all, which is what makes
/// `--no-db --no-models --no-components` work on a machine with no Docker.
pub(super) struct Running<'a> {
    project: &'a Project,
    asked: Option<BTreeSet<String>>,
}

impl<'a> Running<'a> {
    pub(super) fn new(project: &'a Project) -> Running<'a> {
        Running {
            project,
            asked: None,
        }
    }

    fn services(&mut self) -> &BTreeSet<String> {
        let project = self.project;
        self.asked
            .get_or_insert_with(|| docker::running_services(project))
    }

    pub(super) fn has(&mut self, service: &str) -> bool {
        self.services().contains(service)
    }
}

/// How a service was held still while its data volume was read.
#[derive(Debug, Clone, Copy)]
enum Held {
    /// `docker compose pause`: the processes are frozen, the container stays.
    Paused,
    /// `docker compose stop`, for a Docker with no pause.
    Stopped,
}

impl Held {
    fn verb(self) -> &'static str {
        match self {
            Held::Paused => "paused",
            Held::Stopped => "stopped",
        }
    }

    /// The compose command that lets the service go again.
    fn release(self) -> &'static str {
        match self {
            Held::Paused => "unpause",
            Held::Stopped => "start",
        }
    }
}

/// A running service held still for as long as its volume takes to read.
///
/// `docker compose pause` sends the container's processes `SIGSTOP`, so
/// nothing in it can write while tar reads - which is the whole point, because
/// a model keeps a live SQLite database in its data directory and tar has no
/// idea it is being written to. The service is always let go again, including
/// when the read fails: [`Drop`] releases whatever [`Quiesce::release`] has
/// not.
pub(super) struct Quiesce<'a> {
    project: &'a Project,
    service: String,
    held: Option<Held>,
    since: Instant,
}

impl<'a> Quiesce<'a> {
    /// Hold `service` still, when it is running.
    ///
    /// A service that is not running cannot write, so nothing is done and
    /// nothing is reported. Where `pause` is unsupported - Windows containers,
    /// some rootless setups - stopping the service is slower but just as
    /// still; where neither works the read goes ahead with a warning, because
    /// a backup of a possibly-torn volume beats no backup at all.
    pub(super) fn hold(project: &'a Project, service: &str, running: bool) -> Quiesce<'a> {
        let mut held = None;
        if running {
            held = match compose_step(project, &["pause", service]) {
                Ok(()) => Some(Held::Paused),
                Err(pause) => match compose_step(project, &["stop", service]) {
                    Ok(()) => Some(Held::Stopped),
                    Err(stop) => {
                        output::warn(&format!(
                            "{service} could not be held still, so its data is read while the \
                             service may be writing to it: {pause}; {stop}"
                        ));
                        None
                    }
                },
            };
        }
        Quiesce {
            project,
            service: service.to_string(),
            held,
            since: Instant::now(),
        }
    }

    /// Let the service go, and say how long it was held: `paused for 1.4 s`.
    pub(super) fn release(&mut self) -> Option<String> {
        let held = self.held.take()?;
        let note = quiesce_note(held.verb(), self.since.elapsed());
        if let Err(why) = compose_step(self.project, &[held.release(), &self.service]) {
            output::warn(&format!(
                "{} was {} for the backup and could not be started again: {why}; \
                 run `chaps docker run -- {} {}`",
                self.service,
                held.verb(),
                held.release(),
                self.service
            ));
        }
        Some(note)
    }
}

impl Drop for Quiesce<'_> {
    fn drop(&mut self) {
        // Nothing to do when release() already ran; everything to do when the
        // read failed and the `?` went straight past it.
        let _ = self.release();
    }
}

/// How long a service was held still, for the manifest and the report.
pub(super) fn quiesce_note(verb: &str, held: Duration) -> String {
    format!("{verb} for {:.1} s", held.as_secs_f64())
}

/// Run a short `docker compose` command, saying what it said when it failed.
fn compose_step(project: &Project, args: &[&str]) -> std::result::Result<(), String> {
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    match docker::compose_output(project, &args) {
        Ok((0, _, _)) => Ok(()),
        Ok((code, _, stderr)) => Err(format!(
            "`docker compose {}` exited {code}: {}",
            args.join(" "),
            backup::first_line(&stderr)
        )),
        Err(e) => Err(format!("`docker compose {}`: {e}", args.join(" "))),
    }
}
