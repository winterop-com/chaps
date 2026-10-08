//! The backup archive: its layout, its manifest, and the pure helpers
//! `varde backup create` and `varde backup restore` share.
//!
//! An archive is a plain `tar.gz` with five kinds of member, all optional
//! but the first:
//!
//! ```text
//! manifest.yaml            what this archive is and what it holds
//! files/                   .env, .varde/**, the directory of every component
//!                          that has one, and every compose*.yml
//! db/chap_core.dump        pg_dump -Fc of the chap-core database
//! models/<service_id>.tar  one model data volume, as tar saw it
//! components/<member>.tar  one component data volume, likewise; a component
//!                          with several volumes has a member for each
//! ```
//!
//! Nothing in here is varde-specific magic: `tar`, `pg_restore` and a busybox
//! container can put a deployment back together without this CLI, which is the
//! point of keeping the layout this flat. The archive is built and read with
//! the system `tar` binary rather than a Rust tar crate, so what an operator
//! sees with `tar -tzf` is exactly what `varde backup restore` sees.

mod dhis2;
mod files;
mod pg_restore;
mod plan;
mod tar;
mod volumes;

pub use dhis2::*;
pub use files::{Stage, check_manifest_files, copy_file, join_relative, project_files};
pub use pg_restore::{
    PgRestore, pg_restore_error_entries, pg_restore_outcome, pg_restore_warnings, tail_lines,
};
pub use plan::{
    PlannedComponent, PlannedModel, RestorePlan, archived_compose_project, identity_line,
    plan_text, restored_compose_project,
};
pub use tar::{
    first_line, tar_create, tar_extract_into, tar_extract_member_to, tar_list, tar_read_member,
};
pub use volumes::{
    COMPONENT_VOLUMES, component_volumes, ensure_volume, read_volume, refill_script, write_volume,
};

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Manifest schema version written by this CLI.
pub const SCHEMA_VERSION: u32 = 1;
/// Archive member describing the archive.
pub const MANIFEST_MEMBER: &str = "manifest.yaml";
/// Archive member holding the project's files.
pub const FILES_MEMBER: &str = "files";
/// Archive member holding the database dump.
pub const DB_MEMBER: &str = "db/chap_core.dump";
/// Archive member holding the model data tars.
pub const MODELS_MEMBER: &str = "models";
/// Archive member holding the component data tars.
pub const COMPONENTS_MEMBER: &str = "components";
/// Scratch directory inside `.varde/`, never part of a backup.
pub const TMP_DIR: &str = "tmp";
/// Where `backup restore` parks the `.env` it is about to replace.
pub const ENV_BACKUP_FILE: &str = ".env.before-restore";

/// Image the component volumes are read and written through.
///
/// The model services carry a one-shot `<service>-init` container that mounts
/// the same volume, but no component has one that mounts a volume in
/// [`COMPONENT_VOLUMES`], so those are reached by mounting them into a
/// throwaway container of their own. Pinned rather than `latest`: what reads a
/// backup a year from now should be what wrote it.
pub const BUSYBOX_IMAGE: &str = "busybox:1.38";

/// Archive member holding one model's data directory.
pub fn model_member(service_id: &str) -> String {
    format!("{MODELS_MEMBER}/{service_id}.tar")
}

/// Archive member holding one component data volume, by the member name
/// [`ComponentVolume::member`](volumes::ComponentVolume::member) gives it.
pub fn component_member(member: &str) -> String {
    format!("{COMPONENTS_MEMBER}/{member}.tar")
}

/// What one backup is and what it holds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema_version: u32,
    /// e.g. `varde 0.1.0`.
    pub created_by: String,
    /// UTC, `YYYY-MM-DDTHH:MM:SSZ`.
    pub created_at: String,
    /// Name of the project directory this was taken from.
    pub project: String,
    /// The chap-core tag, or none for a deployment without chap-core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chap_image_tag: Option<String>,
    /// Project files in the archive, relative to the project directory.
    #[serde(default)]
    pub files: Vec<String>,
    /// The database dump, when one was taken.
    #[serde(default)]
    pub database: Option<ManifestDatabase>,
    /// Every model the project had enabled, captured or not.
    #[serde(default)]
    pub models: Vec<ManifestModel>,
    /// Every component with persistent state the project had enabled,
    /// captured or not.
    #[serde(default)]
    pub components: Vec<ManifestComponent>,
}

impl Manifest {
    /// Models whose data directory is actually in the archive.
    pub fn captured_models(&self) -> impl Iterator<Item = &ManifestModel> {
        self.models.iter().filter(|m| m.path.is_some())
    }

    /// Components whose data volume is actually in the archive.
    pub fn captured_components(&self) -> impl Iterator<Item = &ManifestComponent> {
        self.components.iter().filter(|c| c.path.is_some())
    }

    /// Bytes the archive holds before compression.
    pub fn content_bytes(&self) -> u64 {
        self.database.as_ref().map_or(0, |d| d.size_bytes)
            + self.models.iter().map(|m| m.size_bytes).sum::<u64>()
            + self.components.iter().map(|c| c.size_bytes).sum::<u64>()
    }
}

/// The database dump in an archive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestDatabase {
    /// Archive member, always [`DB_MEMBER`].
    pub path: String,
    /// `POSTGRES_USER` the dump was taken as.
    pub user: String,
    /// `POSTGRES_DB` that was dumped.
    pub name: String,
    /// What the server reported for `show server_version`.
    #[serde(default)]
    pub server_version: Option<String>,
    pub size_bytes: u64,
}

/// One enabled model, as the backup found it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestModel {
    /// Marketplace id.
    pub id: String,
    pub service_id: String,
    pub version: String,
    pub image_tag: String,
    /// Host port the service published, or `None` for one that was only
    /// reachable inside the compose network.
    #[serde(default)]
    pub host_port: Option<u16>,
    pub data_dir: String,
    /// `user:group` the service runs as, as recorded in `.varde/models.yaml`.
    pub user: String,
    /// Named volume the data directory lives in.
    pub volume: String,
    /// Archive member holding the data, or `None` when it was not captured.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub size_bytes: u64,
    /// Why the data was not captured, when it was not.
    #[serde(default)]
    pub skipped: Option<String>,
    /// Whether that was a read that failed, rather than data there was no
    /// reason to read: what makes `backup create` exit non-zero.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed: bool,
    /// How the service was held still while its volume was read, and for how
    /// long: `paused for 1.4 s`. `None` when it was not running, so there was
    /// nothing that could write while tar read.
    #[serde(default)]
    pub quiesce: Option<String>,
}

/// One component data volume, as the backup found it.
///
/// A component keeps its state in a named volume like a model does, but no
/// component one-shot mounts a volume an archive holds, so it is read through a
/// throwaway [`BUSYBOX_IMAGE`] container instead. One entry per volume, so a
/// component that keeps several has several entries here, all under its name;
/// `volume` is what tells them apart, and it is the key a restore looks an
/// entry up by.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestComponent {
    /// Component name, as [`crate::components::Component::name`] gives it.
    /// Every entry of a component that keeps several volumes carries the same
    /// one.
    pub name: String,
    /// Compose service the volume belongs to.
    pub service: String,
    /// Named volume, without the compose project prefix. Unique within an
    /// archive, since no two volumes of a deployment share a name.
    pub volume: String,
    /// Where the service mounts it.
    pub data_dir: String,
    /// Archive member holding the data, or `None` when it was not captured.
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub size_bytes: u64,
    /// Why the data was not captured, when it was not.
    #[serde(default)]
    pub skipped: Option<String>,
    /// Whether that was a read that failed, rather than data there was no
    /// reason to read: what makes `backup create` exit non-zero.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub failed: bool,
    /// How the service was held still while its volume was read. See
    /// [`ManifestModel::quiesce`].
    #[serde(default)]
    pub quiesce: Option<String>,
}

/// Parse a manifest, saying which archive it came from when it does not parse.
pub fn parse_manifest(body: &str, archive: &Path) -> Result<Manifest> {
    let manifest: Manifest = serde_yaml_ng::from_str(body).map_err(|e| {
        anyhow::anyhow!(
            "{}: its {MANIFEST_MEMBER} is not a varde backup manifest: {e}",
            archive.display()
        )
    })?;
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(anyhow::anyhow!(
            "{}: manifest schema version {} is newer than this varde understands ({SCHEMA_VERSION}); \
             upgrade varde",
            archive.display(),
            manifest.schema_version
        ));
    }
    Ok(manifest)
}

/// Serialise a manifest the way it is written into an archive.
pub fn render_manifest(manifest: &Manifest) -> Result<String> {
    Ok(format!(
        "# varde backup manifest. `varde backup restore` reads this; \
         `tar -xzf <archive> -O {MANIFEST_MEMBER}` prints it.\n{}",
        serde_yaml_ng::to_string(manifest)?
    ))
}

// ---------------------------------------------------------------- naming ---

/// Default archive name: `varde-backup-<project>-<YYYYMMDD-HHMMSS>.tar.gz`.
///
/// The project name is folded to the characters a file name should hold, so a
/// directory called `Chap prod (eu)` still yields something scriptable.
pub fn archive_name(project: &str, stamp: &str) -> String {
    let mut safe = String::with_capacity(project.len());
    for ch in project.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' {
            safe.push(ch);
        } else if !safe.ends_with('-') {
            // A run of unusable characters collapses to one separator, so
            // `Chap prod (eu)` does not become `Chap-prod--eu-`.
            safe.push('-');
        }
    }
    let safe = safe.trim_matches('-').to_string();
    if safe.is_empty() {
        format!("varde-backup-{stamp}.tar.gz")
    } else {
        format!("varde-backup-{safe}-{stamp}.tar.gz")
    }
}

/// Where `--out` points: the flag as given, the default name inside it when it
/// names a directory, or the default name in `cwd` when it is absent.
///
/// `out_is_dir` is the caller's `Path::is_dir`; kept out of here so the
/// decision stays a pure function.
pub fn resolve_out_path(
    out: Option<&Path>,
    out_is_dir: bool,
    cwd: &Path,
    default_name: &str,
) -> PathBuf {
    match out {
        None => cwd.join(default_name),
        Some(path) if out_is_dir => path.join(default_name),
        // A path written with a trailing separator means a directory even
        // before it exists; tar would fail on it much less clearly.
        Some(path) if ends_with_separator(path) => path.join(default_name),
        Some(path) => path.to_path_buf(),
    }
}

fn ends_with_separator(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.ends_with('/') || (cfg!(windows) && text.ends_with('\\'))
}

/// Where an archive is written before it is renamed into place:
/// `.<name>.tmp`, beside the destination.
///
/// tar writes straight into the file it is given, so writing to the
/// destination would truncate whatever is there the moment it starts - and a
/// failure halfway (a full disk, a model volume that cannot be read) would
/// leave last night's backup destroyed and today's half written. A sibling
/// keeps the rename cheap: same directory, therefore same filesystem.
pub fn temp_archive_path(out: &Path) -> PathBuf {
    let name = out
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    out.with_file_name(format!(".{name}.tmp"))
}

// ------------------------------------------------------------------ time ---

/// UTC `(year, month, day, hour, minute, second)` for a Unix timestamp.
pub fn utc_parts(unix: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (unix / 86_400) as i64;
    let rest = unix % 86_400;
    let (y, m, d) = civil_from_days(days);
    (
        y,
        m,
        d,
        (rest / 3600) as u32,
        ((rest % 3600) / 60) as u32,
        (rest % 60) as u32,
    )
}

/// Days since the Unix epoch as a civil `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days`, which is exact for every date this will
/// ever see and needs no calendar crate.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYYMMDD-HHMMSS` in UTC, the stamp in an archive name.
pub fn stamp(unix: u64) -> String {
    let (y, m, d, hh, mm, ss) = utc_parts(unix);
    format!("{y:04}{m:02}{d:02}-{hh:02}{mm:02}{ss:02}")
}

/// `YYYY-MM-DDTHH:MM:SSZ`, the manifest's `created_at`.
pub fn timestamp(unix: u64) -> String {
    let (y, m, d, hh, mm, ss) = utc_parts(unix);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Seconds since the Unix epoch, or 0 on a clock set before 1970.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ----------------------------------------------------------------- sizes ---

/// A size for a human: `912 B`, `4.3 KB`, `1.2 GB`.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// Size of a file, or 0 when it cannot be stat'ed.
pub fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

// ------------------------------------------------------------------- env ---

/// The value of `key` in a `.env` body, or `None` when it is absent or
/// commented out.
///
/// [`crate::dotenv::value`], so a restore reads the file by compose's rules -
/// inline comments and quoting included - like every other reader here.
pub fn env_value(body: &str, key: &str) -> Option<String> {
    crate::dotenv::value(body, key)
}

/// The `.env` variables whose value is fixed inside chap-core's `postgres`
/// volume: the role, its password and the database were set when the volume
/// was created, and a `pg_restore` into it changes none of them.
pub const CHAP_DB_CREDENTIALS: &[&str] = &[
    "POSTGRES_USER",
    "POSTGRES_PASSWORD",
    "POSTGRES_DB",
    "CHAP_DATABASE_URL",
];

/// The same for the `dhis2_db` volume: the database password, and the key
/// DHIS2 encrypted values in that database with.
pub const DHIS2_DB_CREDENTIALS: &[&str] = &["DHIS2_DB_PASSWORD", "DHIS2_ENCRYPTION_PASSWORD"];

/// Put `vars` in a restored `.env` back to what `current` - the `.env` of the
/// deployment being restored into - set them to, and name the ones that moved.
///
/// For a database volume the restore leaves in place: its credentials live in
/// the volume, so the archive's would be a `.env` that no longer opens its own
/// database. A variable `current` does not set is commented out, so compose's
/// default - the one that volume was created with - applies again.
pub fn keep_credentials(restored: &str, current: &str, vars: &[&str]) -> (String, Vec<String>) {
    let mut body = restored.to_string();
    let mut moved = Vec::new();
    for var in vars {
        let theirs = crate::dotenv::value(&body, var);
        let ours = crate::dotenv::value(current, var);
        if theirs == ours {
            continue;
        }
        body = match &ours {
            Some(value) => crate::auth::write_secrets(&body, &[(var, value)]),
            None => crate::dotenv::comment_out(&body, var),
        };
        moved.push((*var).to_string());
    }
    (body, moved)
}

/// The PostgreSQL user and database a project's `.env` names, with chap-core's
/// own defaults when it does not.
pub fn postgres_credentials(env_body: &str) -> (String, String) {
    (
        env_value(env_body, "POSTGRES_USER").unwrap_or_else(|| "chap".to_string()),
        env_value(env_body, "POSTGRES_DB").unwrap_or_else(|| "chap_core".to_string()),
    )
}

/// Whether the `.env` about to be overwritten is worth keeping as
/// [`ENV_BACKUP_FILE`].
///
/// Only when both exist and differ: an identical copy is noise, and there is
/// nothing to preserve when the project has no `.env` or the archive brings
/// none (in which case the project's own is left alone).
pub fn keep_env_copy(current: Option<&[u8]>, incoming: Option<&[u8]>) -> bool {
    matches!((current, incoming), (Some(cur), Some(inc)) if cur != inc)
}

#[cfg(test)]
mod tests;
