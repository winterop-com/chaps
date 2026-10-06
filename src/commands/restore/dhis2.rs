//! The DHIS2 database from its dump: a new database, `pg_restore` with
//! several jobs, then the mark of a complete restore. See `backup::dhis2`.

use super::RestoreReport;
use super::database::exec_args;
use crate::backup::{self, PgRestore};
use crate::docker;
use crate::error::Result;
use crate::output;
use crate::project::Project;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long `dhis2-db` is given to accept connections.
const READY_TIMEOUT: Duration = Duration::from_secs(180);
/// How many lines of a failed `pg_restore` are printed.
const STDERR_TAIL: usize = 10;

/// Load the dump into a new DHIS2 database.
///
/// Only `dhis2-db` is started, without the seed one-shot: the database is
/// made new here, so a seed has nothing to add.
pub(super) fn restore_dhis2_db(
    project: &Project,
    dump: &Path,
    report: &mut RestoreReport,
) -> Result<()> {
    compose(project, &["up", "-d", "--no-deps", "dhis2-db"])?;
    wait_until_ready(project)?;

    let target = format!("dhis2-db:{}", backup::DHIS2_RESTORE_PATH);
    let source = dump.display().to_string();
    compose(project, &["cp", &source, &target])?;

    let command = backup::dhis2_restore_command();
    let args = exec_args("dhis2-db", &["sh", "-c", &command]);
    let piped = docker::run_compose_piped(project, &args, None, None)?;
    let _ = docker::run_compose_piped(
        project,
        &exec_args("dhis2-db", &["rm", "-f", backup::DHIS2_RESTORE_PATH]),
        None,
        None,
    );
    match backup::pg_restore_outcome(piped.code, &piped.stderr) {
        PgRestore::Ok => {}
        PgRestore::Warnings => {
            for warning in backup::pg_restore_warnings(&piped.stderr) {
                output::warn(&format!("pg_restore (dhis2-db): {warning}"));
                report.database_warnings.push(warning);
            }
        }
        PgRestore::Failed => {
            for line in backup::tail_lines(&piped.stderr, STDERR_TAIL) {
                output::warn(&line);
            }
            return Err(anyhow::anyhow!(
                "pg_restore of the DHIS2 database failed (exit {}); the lines above say why, \
                 and `chaps logs dhis2-db` has the rest",
                piped.code
            ));
        }
    }

    let command = backup::dhis2_mark_command();
    let piped = docker::run_compose_piped(
        project,
        &exec_args("dhis2-db", &["sh", "-c", &command]),
        None,
        None,
    )?;
    if piped.code != 0 {
        return Err(anyhow::anyhow!(
            "the DHIS2 database was restored, but its mark could not be set (exit {}): {}",
            piped.code,
            backup::first_line(&piped.stderr)
        ));
    }
    Ok(())
}

/// `docker compose <args>`, as an error when it fails.
fn compose(project: &Project, args: &[&str]) -> Result<()> {
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    let (code, _, stderr) = docker::compose_output(project, &args)?;
    if code != 0 {
        return Err(anyhow::anyhow!(
            "`docker compose {}` failed (exit {code}): {}",
            args.join(" "),
            backup::first_line(&stderr)
        ));
    }
    Ok(())
}

/// Wait until `dhis2-db` accepts connections over TCP. Not its health check:
/// a seeded database without the mark is unhealthy until this restore sets it.
fn wait_until_ready(project: &Project) -> Result<()> {
    let started = Instant::now();
    let args = exec_args(
        "dhis2-db",
        &["sh", "-c", "pg_isready -h 127.0.0.1 -U \"$POSTGRES_USER\""],
    );
    loop {
        if let Ok((0, _, _)) = docker::compose_output(project, &args) {
            return Ok(());
        }
        if started.elapsed() >= READY_TIMEOUT {
            return Err(anyhow::anyhow!(
                "dhis2-db did not accept connections within {} seconds; look at \
                 `chaps logs dhis2-db`",
                READY_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}
