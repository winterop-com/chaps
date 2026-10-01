//! Step 3: `pg_restore` into a postgres started just for this.

use super::{RestoreReport, compose};
use crate::backup::{self, PgRestore, Stage};
use crate::docker;
use crate::error::Result;
use crate::output;
use crate::project::{ENV_FILE, Project};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

/// How long postgres is given to come up healthy before the restore gives up.
const POSTGRES_TIMEOUT: Duration = Duration::from_secs(180);
/// How often the wait re-reads `docker compose ps`.
const POLL: Duration = Duration::from_secs(2);
/// How many lines of a failed `pg_restore`'s stderr are printed.
const STDERR_TAIL: usize = 10;

/// `pg_restore --clean --if-exists` the dump into a running postgres.
pub(super) fn restore_database(
    project: &Project,
    archive: &Path,
    stage: &Stage,
    running: &BTreeSet<String>,
    report: &mut RestoreReport,
) -> Result<()> {
    let Some(db) = report.plan.manifest.database.clone() else {
        return Ok(());
    };
    // The credentials of the deployment as it is now, which after a files
    // restore is the .env that came out of the archive.
    let env_body = std::fs::read_to_string(project.dir.join(ENV_FILE)).unwrap_or_default();
    let (user, name) = backup::postgres_credentials(&env_body);

    if !running.contains("postgres") {
        compose(
            project,
            &["up".to_string(), "-d".to_string(), "postgres".to_string()],
        )?;
    }
    wait_for_postgres(project)?;
    check_connection(project, &user, &name)?;

    let dump = stage.path("chap_core.dump")?;
    backup::tar_extract_member_to(archive, &db.path, &dump)?;

    let args = exec_args(
        "postgres",
        &[
            "pg_restore",
            "-U",
            &user,
            "-d",
            &name,
            "--clean",
            "--if-exists",
            "--no-owner",
        ],
    );
    let piped = docker::run_compose_piped(project, &args, Some(&dump), None)?;
    match backup::pg_restore_outcome(piped.code, &piped.stderr) {
        PgRestore::Ok => {}
        // Exit 1 with nothing but the complaints `--clean --if-exists` cannot
        // avoid, and the count that says pg_restore carried on regardless:
        // restored. See `backup::pg_restore_outcome`.
        PgRestore::Warnings => {
            report.database_warnings = backup::pg_restore_warnings(&piped.stderr);
            for warning in &report.database_warnings {
                output::warn(&format!("pg_restore: {warning}"));
            }
        }
        PgRestore::Failed => {
            // What pg_restore said is the only useful thing here, and it says
            // it at the end, so the tail goes out before the one-line error.
            for line in backup::tail_lines(&piped.stderr, STDERR_TAIL) {
                output::warn(&line);
            }
            return Err(anyhow::anyhow!(
                "pg_restore failed (exit {}): {}. The database is as pg_restore left it; \
                 nothing else was restored and CHAP was not started",
                piped.code,
                failure_reason(&piped.stderr)
            ));
        }
    }
    report.database = true;
    Ok(())
}

/// Prove the connection before anything is dropped.
///
/// `pg_restore` reports a connection it never made as exit 1, the same code it
/// uses for the object drops `--clean --if-exists` cannot avoid, so a restore
/// that never reached the server is one exit code away from one that finished
/// with warnings. Asking for `select 1` first turns that into the error
/// PostgreSQL actually gave: the wrong role, a database that is not there, a
/// server still starting up.
fn check_connection(project: &Project, user: &str, name: &str) -> Result<()> {
    let args = exec_args(
        "postgres",
        &["psql", "-U", user, "-d", name, "-tAc", "select 1"],
    );
    let (code, _, stderr) = docker::compose_output(project, &args)?;
    if code != 0 {
        return Err(anyhow::anyhow!(
            "the database {name} could not be reached as {user} (psql exited {code}): {}. \
             Nothing was restored",
            backup::first_line(&stderr)
        ));
    }
    Ok(())
}

/// The line of a failed `pg_restore` that says why: the first error that
/// lost something, with the statement it came from, else the last error it
/// reported, else the last thing it printed at all.
pub(super) fn failure_reason(stderr: &str) -> String {
    let errors = backup::pg_restore_error_entries(stderr);
    if let Some(error) = errors.iter().find(|error| !error.is_harmless()) {
        return match &error.command {
            Some(command) => format!("{} (while running `{command}`)", error.message),
            None => error.message.clone(),
        };
    }
    errors
        .last()
        .map(|error| error.message.clone())
        .or_else(|| backup::tail_lines(stderr, 1).pop())
        .unwrap_or_else(|| "no output".to_string())
}

/// `exec -T <service> <cmd..>`, the form that needs no terminal.
pub(super) fn exec_args(service: &str, cmd: &[&str]) -> Vec<String> {
    let mut args = vec!["exec".to_string(), "-T".to_string(), service.to_string()];
    args.extend(cmd.iter().map(|s| (*s).to_string()));
    args
}

/// Poll `docker compose ps` until postgres reports healthy.
fn wait_for_postgres(project: &Project) -> Result<()> {
    let started = Instant::now();
    let args = ["ps".to_string(), "--format".to_string(), "json".to_string()];
    loop {
        if let Ok((0, stdout, _)) = docker::compose_output(project, &args)
            && docker::service_is_healthy(&stdout, "postgres")
        {
            return Ok(());
        }
        if started.elapsed() >= POSTGRES_TIMEOUT {
            return Err(anyhow::anyhow!(
                "postgres did not become healthy within {} seconds; \
                 look at `chaps logs postgres`",
                POSTGRES_TIMEOUT.as_secs()
            ));
        }
        std::thread::sleep(POLL);
    }
}
