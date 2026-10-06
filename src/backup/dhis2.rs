//! The DHIS2 database in a backup: a `pg_dump` without the tables that DHIS2
//! makes again, rather than a copy of its volume.
//!
//! The volume of a DHIS2 database that has run analytics is mostly analytics
//! tables: 62 GB for a database of 22 GB, measured on a national dump. A copy
//! of the volume also pauses the database for as long as the copy takes. The
//! exclusions are those of dhis2-server-tools
//! (`deploy/roles/backups/templates/backup-script`).

/// The archive member that holds the dump.
pub const DHIS2_DB_DUMP_MEMBER: &str = "components/dhis2-db.dump";

/// What the manifest says about where the data came from, in place of the
/// data directory of a volume.
pub const DHIS2_DB_DUMP_SOURCE: &str =
    "pg_dump without analytics_*, aggregated_*, completeness_*, _* and the data of audit";

/// The tables a backup leaves out: DHIS2 makes them again with the next
/// analytics run. With the underscore: `analytics*` would also take
/// `analyticstablehook`, which is metadata.
pub const DHIS2_EXCLUDED_TABLES: &[&str] = &["analytics_*", "aggregated_*", "completeness_*", "_*"];

/// The tables whose rows a backup leaves out, and whose definition it keeps.
pub const DHIS2_EXCLUDED_DATA: &[&str] = &["audit"];

/// Where the restore puts the dump inside `dhis2-db`, so that `pg_restore`
/// can read it with several jobs: a dump on stdin allows one job only.
pub const DHIS2_RESTORE_PATH: &str = "/tmp/varde-dhis2-db.dump";

/// How many jobs `pg_restore` runs at once.
pub const DHIS2_RESTORE_JOBS: u32 = 4;

/// The shell command that writes the dump to stdout, run in `dhis2-db`.
/// `-Z 1`: fast compression, because the dump is large and the speed matters
/// more than the last tenth of the size.
pub fn dhis2_dump_command() -> String {
    let mut command = String::from("pg_dump -U \"$POSTGRES_USER\" -d \"$POSTGRES_DB\" -Fc -Z 1");
    for table in DHIS2_EXCLUDED_TABLES {
        command.push_str(&format!(" -T '{table}'"));
    }
    for table in DHIS2_EXCLUDED_DATA {
        command.push_str(&format!(" --exclude-table-data='{table}'"));
    }
    command
}

/// The shell command that makes the database new and loads the dump into it,
/// run in `dhis2-db`. The mark of a complete restore goes on last, so that the
/// health check of a seeded deployment passes.
pub fn dhis2_restore_command() -> String {
    format!(
        "dropdb -U \"$POSTGRES_USER\" --if-exists --force \"$POSTGRES_DB\" && \
         createdb -U \"$POSTGRES_USER\" -O \"$POSTGRES_USER\" \"$POSTGRES_DB\" && \
         pg_restore -U \"$POSTGRES_USER\" -d \"$POSTGRES_DB\" -j {DHIS2_RESTORE_JOBS} \
         --no-owner --no-privileges {DHIS2_RESTORE_PATH}"
    )
}

/// The command that sets the mark of a complete restore on the database.
pub fn dhis2_mark_command() -> String {
    format!(
        "psql -U \"$POSTGRES_USER\" -d \"$POSTGRES_DB\" -v ON_ERROR_STOP=1 -qc \
         \"COMMENT ON DATABASE \\\"$POSTGRES_DB\\\" IS '{}'\"",
        crate::compose::render::DHIS2_SEED_MARK
    )
}

/// Whether an archive member is a DHIS2 database dump rather than a volume.
pub fn is_dhis2_db_dump(member: &str) -> bool {
    member == DHIS2_DB_DUMP_MEMBER
}
