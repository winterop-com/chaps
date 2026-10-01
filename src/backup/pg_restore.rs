//! Reading what a `pg_restore` run said.

/// How a `pg_restore` run should be read: its exit code together with what it
/// said.
///
/// The exit code alone is not enough. `pg_restore` exits 1 both when it
/// finished and only complained about objects `--clean --if-exists` could not
/// avoid complaining about, and when it restored nothing at all - a refused
/// connection is exit 1 too. Reading every 1 as "restored, with warnings" is
/// how a failed restore comes to be reported as a success, so exit 1 is only
/// read that way when `pg_restore` also said it ignored those errors and
/// carried on, and when every error it printed is one of
/// [`PG_RESTORE_IGNORABLE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgRestore {
    /// Clean run.
    Ok,
    /// Restored, with warnings worth printing.
    Warnings,
    /// Did not restore.
    Failed,
}

/// The `pg_restore: error:` lines a `--clean --if-exists --no-owner` restore
/// prints while losing nothing.
///
/// `--clean` drops what the dump is about to create, `--if-exists` makes the
/// drops conditional at the SQL level but PostgreSQL still reports the objects
/// inside them that were never there (`does not exist`); a dump reloaded into
/// a database that kept an extension or a schema reports `already exists`; and
/// `--no-owner` leaves objects to the restoring role, which may not own what
/// it is asked to re-own (`must be owner of`). Everything else - a refused
/// connection, a missing role, a disk that filled up - is a failure, and so is
/// any of these while loading rows (see [`PgRestoreError::loads_data`]): a
/// `COPY` into a table that `does not exist` is the data lost, not a drop that
/// had nothing to drop.
pub const PG_RESTORE_IGNORABLE: &[&str] = &["already exists", "does not exist", "must be owner of"];

/// Read a `pg_restore` run: the exit code, and the stderr it explained itself
/// on. See [`PgRestore`].
pub fn pg_restore_outcome(code: i32, stderr: &str) -> PgRestore {
    if code == 0 {
        return PgRestore::Ok;
    }
    if code != 1 {
        return PgRestore::Failed;
    }
    // Exit 1 without the closing count is not "finished with complaints": it
    // is pg_restore stopping, and the reason is on stderr.
    if pg_restore_ignored_count(stderr).is_none() {
        return PgRestore::Failed;
    }
    if pg_restore_error_entries(stderr)
        .iter()
        .any(|error| !error.is_harmless())
    {
        return PgRestore::Failed;
    }
    PgRestore::Warnings
}

/// One `pg_restore: error:` line, with the `Command was:` line PostgreSQL
/// printed under it when the error came from a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgRestoreError {
    /// The error, without the `pg_restore: error:` prefix.
    pub message: String,
    /// The statement that failed, without the `Command was:` prefix.
    pub command: Option<String>,
}

impl PgRestoreError {
    /// Whether this error happened while loading rows: a `COPY` statement, or
    /// pg_restore's own `COPY failed for table`. Measured on PostgreSQL 17, a
    /// table the dump's data has nowhere to go into reports
    /// `relation "public.jobs" does not exist` with `Command was: COPY
    /// public.jobs (id, name) FROM stdin;` - the same words as a harmless
    /// `--clean` drop.
    pub fn loads_data(&self) -> bool {
        let starts_with_copy = |text: &str| {
            text.trim_start()
                .get(..5)
                .is_some_and(|head| head.eq_ignore_ascii_case("copy "))
        };
        self.command.as_deref().is_some_and(starts_with_copy)
            || self.message.to_ascii_lowercase().contains("copy failed")
    }

    /// Whether the restore lost nothing to it: one of
    /// [`PG_RESTORE_IGNORABLE`], and not while loading rows.
    pub fn is_harmless(&self) -> bool {
        !self.loads_data() && pg_restore_error_is_ignorable(&self.message)
    }
}

/// Every `pg_restore: error:` of a run, each with the `Command was:` line
/// that follows it, if any.
pub fn pg_restore_error_entries(stderr: &str) -> Vec<PgRestoreError> {
    let mut entries: Vec<PgRestoreError> = Vec::new();
    for line in stderr.lines().map(str::trim) {
        if let Some(message) = line.strip_prefix("pg_restore: error:") {
            entries.push(PgRestoreError {
                message: message.trim().to_string(),
                command: None,
            });
        } else if let Some(command) = line.strip_prefix("Command was:")
            && let Some(last) = entries.last_mut()
            && last.command.is_none()
        {
            last.command = Some(command.trim().to_string());
        }
    }
    entries
}

/// The `N` of `pg_restore`'s closing `errors ignored on restore: N`, which it
/// prints when it finished in spite of them. `None` when it never got there.
pub fn pg_restore_ignored_count(stderr: &str) -> Option<u64> {
    const MARKER: &str = "errors ignored on restore:";
    stderr.lines().rev().find_map(|line| {
        let (_, count) = line.trim().split_once(MARKER)?;
        count.trim().parse::<u64>().ok()
    })
}

/// Whether one `pg_restore` error line is one of [`PG_RESTORE_IGNORABLE`].
pub fn pg_restore_error_is_ignorable(line: &str) -> bool {
    let line = line.to_ascii_lowercase();
    PG_RESTORE_IGNORABLE
        .iter()
        .any(|ignorable| line.contains(ignorable))
}

/// The warning lines of a `pg_restore` run, without the noise.
///
/// Only `pg_restore:` diagnostics are kept, and the `error:`/`warning:`
/// prefixes it repeats on every line are left in place so the operator sees
/// what PostgreSQL called them.
pub fn pg_restore_warnings(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.trim_start_matches("pg_restore: ").to_string())
        .collect()
}

/// The last `count` non-empty lines of what a child printed, trimmed.
///
/// What a failed `pg_restore` has to say is at the end of its stderr, and the
/// first line of it is usually `connection to server ... failed` with the
/// reason after it, so an error that quotes one line quotes the wrong one.
pub fn tail_lines(text: &str, count: usize) -> Vec<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines[lines.len().saturating_sub(count)..]
        .iter()
        .map(|l| (*l).to_string())
        .collect()
}
