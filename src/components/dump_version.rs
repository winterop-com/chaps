//! The DHIS2 version of a seed dump, read from its Flyway table.
//!
//! Each migration DHIS2 ran is a row of `flyway_schema_history` (very old
//! DHIS2: `schema_version`), and its version starts with the minor version
//! of DHIS2: `2.42.54` is migration 54 of the 2.42 line. The newest row is the
//! version of the database. The last number is not the DHIS2 patch release.

use super::*;
use std::io::BufRead;
use std::path::Path;

/// The oldest DHIS2 minor version varde supports: the Modeling App and the
/// route to Chap need it.
pub const DHIS2_OLDEST_MINOR: &str = "2.41";

/// The newest Flyway migration in the SQL text of a dump, such as `2.42.54`.
/// Reading stops after the Flyway table, so a large dump is not read whole.
/// `None` for a dump with no Flyway table, which is not a DHIS2 database.
pub fn dump_migration_of(mut reader: impl BufRead) -> Option<String> {
    // Bytes in one buffer that is used again, not a `String` for each line:
    // a large dump has tens of millions of lines before the Flyway table.
    let mut buf = Vec::with_capacity(1 << 16);
    let mut column = None;
    let mut newest: Option<String> = None;
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = buf.strip_suffix(b"\n").unwrap_or(&buf);
        match column {
            None => {
                if line.starts_with(b"COPY public.") {
                    column = flyway_version_column(&String::from_utf8_lossy(line));
                }
            }
            Some(_) if line == b"\\." => break,
            Some(at) => {
                let text = String::from_utf8_lossy(line);
                let Some(version) = text.split('\t').nth(at) else {
                    continue;
                };
                if version.starts_with(|c: char| c.is_ascii_digit())
                    && newest.as_deref().is_none_or(|seen| {
                        crate::dhis2::version_parts(version) > crate::dhis2::version_parts(seen)
                    })
                {
                    newest = Some(version.to_string());
                }
            }
        }
    }
    newest
}

/// The index of the `version` column when `line` opens the data of a Flyway
/// table: `COPY public.flyway_schema_history (installed_rank, version, ...)`.
fn flyway_version_column(line: &str) -> Option<usize> {
    let rest = line
        .strip_prefix("COPY public.flyway_schema_history (")
        .or_else(|| line.strip_prefix("COPY public.schema_version ("))?;
    let columns = rest.split(')').next()?;
    columns
        .split(',')
        .position(|name| name.trim().trim_matches('"') == "version")
}

/// [`dump_migration_of`] for a gzipped dump on disk.
pub fn dump_migration(path: &Path) -> Result<Option<String>> {
    let file = std::fs::File::open(path)
        .map_err(|e| anyhow::anyhow!("could not read the dump {}: {e}", path.display()))?;
    let reader =
        std::io::BufReader::with_capacity(1 << 20, flate2::read::MultiGzDecoder::new(file));
    Ok(dump_migration_of(reader))
}

/// `2.42` from `2.42.54`, and `None` for a version with no minor part.
pub fn migration_minor(migration: &str) -> Option<String> {
    match crate::dhis2::version_parts(migration).as_slice() {
        [major, minor, ..] => Some(format!("{major}.{minor}")),
        _ => None,
    }
}

/// What a dump of `dump_minor` means for the tag of this deployment.
///
/// `tag` is the tag that `--dhis2-tag` gave, or `None` for the default. A dump
/// older than 2.41 is refused; a tag older than the dump is refused, because
/// DHIS2 does not run a database of a newer version. With no tag given, the
/// tag is the minor version of the dump.
pub fn tag_for_dump(dump_minor: &str, tag: Option<&str>) -> Result<String> {
    let dump = crate::dhis2::version_parts(dump_minor);
    if dump < crate::dhis2::version_parts(DHIS2_OLDEST_MINOR) {
        return Err(anyhow::anyhow!(
            "the dump is from DHIS2 {dump_minor} (its newest Flyway migration), and varde \
             supports DHIS2 {DHIS2_OLDEST_MINOR} and newer; upgrade the database in DHIS2 first"
        ));
    }
    let Some(tag) = tag else {
        return Ok(dump_minor.to_string());
    };
    // A tag that is not a version, such as `master`, is the newest build.
    let given = crate::dhis2::version_parts(dhis2_minor(tag));
    if !given.is_empty() && given < dump {
        return Err(anyhow::anyhow!(
            "--dhis2-tag {tag} is older than the dump, which is from DHIS2 {dump_minor}, and \
             DHIS2 does not run a newer database; use `--dhis2-tag {dump_minor}` or newer, or \
             leave the option out"
        ));
    }
    Ok(tag.to_string())
}
