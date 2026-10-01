//! The backup archive: its layout, its manifest, and the pure helpers
//! `chaps backup create` and `chaps backup restore` share.
//!
//! An archive is a plain `tar.gz` with five kinds of member, all optional
//! but the first:
//!
//! ```text
//! manifest.yaml            what this archive is and what it holds
//! files/                   .env, .chaps/**, the directory of every component
//!                          that has one, and every compose*.yml
//! db/chap_core.dump        pg_dump -Fc of the chap-core database
//! models/<service_id>.tar  one model data volume, as tar saw it
//! components/<member>.tar  one component data volume, likewise; a component
//!                          with several volumes has a member for each
//! ```
//!
//! Nothing in here is chaps-specific magic: `tar`, `pg_restore` and a busybox
//! container can put a deployment back together without this CLI, which is the
//! point of keeping the layout this flat. The archive is built and read with
//! the system `tar` binary rather than a Rust tar crate, so what an operator
//! sees with `tar -tzf` is exactly what `chaps backup restore` sees.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
/// Scratch directory inside `.chaps/`, never part of a backup.
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
pub const BUSYBOX_IMAGE: &str = "busybox:1.37";

/// Archive member holding one model's data directory.
pub fn model_member(service_id: &str) -> String {
    format!("{MODELS_MEMBER}/{service_id}.tar")
}

/// Archive member holding one component data volume, by the member name
/// [`ComponentVolume::member`] gives it.
pub fn component_member(member: &str) -> String {
    format!("{COMPONENTS_MEMBER}/{member}.tar")
}

/// What one backup is and what it holds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema_version: u32,
    /// e.g. `chaps 0.1.0`.
    pub created_by: String,
    /// UTC, `YYYY-MM-DDTHH:MM:SSZ`.
    pub created_at: String,
    /// Name of the project directory this was taken from.
    pub project: String,
    pub chap_image_tag: String,
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
    /// captured or not. Absent in an archive written before components were
    /// backed up, which is what an empty list says.
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
    /// reachable inside the compose network. An older manifest, written when
    /// every model had a port, holds a number.
    #[serde(default)]
    pub host_port: Option<u16>,
    pub data_dir: String,
    /// `user:group` the service runs as, as recorded in `.chaps/models.yaml`.
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
    /// How the service was held still while its volume was read. See
    /// [`ManifestModel::quiesce`].
    #[serde(default)]
    pub quiesce: Option<String>,
}

/// Parse a manifest, saying which archive it came from when it does not parse.
pub fn parse_manifest(body: &str, archive: &Path) -> Result<Manifest> {
    let manifest: Manifest = serde_yaml_ng::from_str(body).map_err(|e| {
        anyhow::anyhow!(
            "{}: its {MANIFEST_MEMBER} is not a chaps backup manifest: {e}",
            archive.display()
        )
    })?;
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(anyhow::anyhow!(
            "{}: manifest schema version {} is newer than this chaps understands ({SCHEMA_VERSION}); \
             upgrade chaps",
            archive.display(),
            manifest.schema_version
        ));
    }
    Ok(manifest)
}

/// Serialise a manifest the way it is written into an archive.
pub fn render_manifest(manifest: &Manifest) -> Result<String> {
    Ok(format!(
        "# chaps backup manifest. `chaps backup restore` reads this; \
         `tar -xzf <archive> -O {MANIFEST_MEMBER}` prints it.\n{}",
        serde_yaml_ng::to_string(manifest)?
    ))
}

// ---------------------------------------------------------------- naming ---

/// Default archive name: `chaps-backup-<project>-<YYYYMMDD-HHMMSS>.tar.gz`.
///
/// The project name is folded to the characters a file name should hold, so a
/// directory called `CHAP prod (eu)` still yields something scriptable.
pub fn archive_name(project: &str, stamp: &str) -> String {
    let mut safe = String::with_capacity(project.len());
    for ch in project.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' {
            safe.push(ch);
        } else if !safe.ends_with('-') {
            // A run of unusable characters collapses to one separator, so
            // `CHAP prod (eu)` does not become `CHAP-prod--eu-`.
            safe.push('-');
        }
    }
    let safe = safe.trim_matches('-').to_string();
    if safe.is_empty() {
        format!("chaps-backup-{stamp}.tar.gz")
    } else {
        format!("chaps-backup-{safe}-{stamp}.tar.gz")
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

// ------------------------------------------------------------ file lists ---

/// The project files a backup holds, relative to the project directory and in
/// a stable order: `.env`, then `.chaps/**`, then the directory of every
/// component that owns one in [`crate::components::Component::ALL`] order
/// (`ocs/**`, then `dhis2/**`), then the root `compose*.yml`.
///
/// `.chaps/tmp/` is scratch space (this is where the archive is staged) and
/// half-written `.tmp` state files are transient, so neither is included.
/// Compose files are taken from the project root only; a `compose.yml` in a
/// subdirectory belongs to something else.
///
/// The component directories are in here because what they hold is the
/// operator's own, not a rendered artifact: `chaps sync` only ever creates a
/// missing `ocs/climate-service.yaml` or `dhis2/dhis.conf`, so nothing can
/// rebuild the edits made to one, and DHIS2 does not start at all without its.
/// [`crate::components::Component::dir`] is asked rather than any of them being
/// named here, because a name in this function is a name the next component is
/// forgotten from - which is exactly how `dhis2/dhis.conf` came to be missing
/// from every archive.
///
/// A directory is taken whether or not its component is enabled right now: a
/// file that exists is one somebody wrote, `chaps components disable` says the
/// directory is left alone because it is the operator's, and a backup that
/// dropped it the moment the component went off would lose it at the one moment
/// nothing else is looking after it.
pub fn project_files(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if dir.join(crate::project::ENV_FILE).is_file() {
        out.push(crate::project::ENV_FILE.to_string());
    }

    let chaps = dir.join(crate::project::CHAPS_DIR);
    let mut state = Vec::new();
    collect_under(&chaps, crate::project::CHAPS_DIR, &mut state);
    state.sort();
    out.extend(state);

    for owned in crate::components::Component::ALL
        .iter()
        .filter_map(|component| component.dir())
    {
        let mut files = Vec::new();
        collect_under(&dir.join(owned), owned, &mut files);
        files.sort();
        out.extend(files);
    }

    let mut compose = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_compose_file(&name) && entry.path().is_file() {
                compose.push(name);
            }
        }
    }
    compose.sort();
    out.extend(compose);
    out
}

/// Whether a project-root file name is one of the compose files: `compose*.yml`.
pub fn is_compose_file(name: &str) -> bool {
    name.starts_with("compose") && name.ends_with(".yml")
}

/// Every file under `dir`, as `prefix/...` paths with forward slashes.
fn collect_under(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = format!("{prefix}/{name}");
        if entry.path().is_dir() {
            // The staging directory of the backup being taken right now.
            if prefix == crate::project::CHAPS_DIR && name == TMP_DIR {
                continue;
            }
            collect_under(&entry.path(), &path, out);
        } else if !(name.starts_with('.') && name.ends_with(".tmp")) {
            out.push(path);
        }
    }
}

/// Whether an archive-style `a/b/c` path stays inside the directory it is
/// joined to: no empty, `.` or `..` part, no leading `/`, no backslash or
/// drive. A manifest is read from the archive, so its paths are only as
/// trustworthy as the file, and [`join_relative`] would follow a `..` out.
pub fn is_contained_relative(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.contains(['\\', ':', '\0'])
        && rel
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Refuse an archive whose manifest names a project file outside the project
/// directory, before a restore touches anything.
pub fn check_manifest_files(manifest: &Manifest, archive: &Path) -> Result<()> {
    match manifest
        .files
        .iter()
        .find(|rel| !is_contained_relative(rel))
    {
        Some(rel) => Err(anyhow::anyhow!(
            "{} lists the project file `{rel}`, which is not inside a deployment directory; \
             chaps never writes such a path, so this archive was changed after `chaps backup \
             create` made it - restore from another one",
            archive.display()
        )),
        None => Ok(()),
    }
}

/// Copy `rel` from `from` to `to`, creating the parent directories.
pub fn copy_file(from: &Path, to: &Path, rel: &str) -> Result<()> {
    if !is_contained_relative(rel) {
        return Err(anyhow::anyhow!(
            "refusing to copy `{rel}`: it is not a path inside {}",
            to.display()
        ));
    }
    let src = join_relative(from, rel);
    let dst = join_relative(to, rel);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", parent.display()))?;
    }
    std::fs::copy(&src, &dst)
        .map_err(|e| anyhow::anyhow!("copying {} to {}: {e}", src.display(), dst.display()))?;
    Ok(())
}

/// Resolve an archive-style `a/b/c` path against a directory.
pub fn join_relative(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |acc, part| acc.join(part))
}

// --------------------------------------------------------------- staging ---

/// A scratch directory under `.chaps/tmp/`, removed when it goes out of scope.
///
/// It lives inside the project so the staged copy and the finished archive are
/// on the same filesystem, which keeps a multi-gigabyte model volume off
/// `/tmp` (often a small tmpfs) and makes the final rename cheap.
#[derive(Debug)]
pub struct Stage {
    pub dir: PathBuf,
}

impl Stage {
    /// Create `.chaps/tmp/<prefix>-<pid>` under `chaps_dir`.
    pub fn new(chaps_dir: &Path, prefix: &str) -> Result<Stage> {
        let dir = chaps_dir
            .join(TMP_DIR)
            .join(format!("{prefix}-{}", std::process::id()));
        // A crashed earlier run may have left one behind.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .map_err(|e| anyhow::anyhow!("creating {}: {e}", dir.display()))?;
        Ok(Stage { dir })
    }

    /// `self.dir/rel`, with the parent directories created.
    pub fn path(&self, rel: &str) -> Result<PathBuf> {
        let path = join_relative(&self.dir, rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("creating {}: {e}", parent.display()))?;
        }
        Ok(path)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        // And `.chaps/tmp` itself, when this was the last stage in it: an
        // empty directory left behind would show up in the next backup.
        if let Some(parent) = self.dir.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

// ------------------------------------------------------------------- tar ---

/// `tar`, configured the same way for every call.
fn tar_command() -> Command {
    let mut cmd = Command::new("tar");
    // Without this macOS tar stores an AppleDouble `._name` member beside every
    // file that carries extended attributes, which nothing here wants.
    cmd.env("COPYFILE_DISABLE", "1");
    cmd.stdin(Stdio::null());
    cmd
}

/// Pack `members` of `stage` into `archive` as gzipped tar.
pub fn tar_create(archive: &Path, stage: &Path, members: &[String]) -> Result<()> {
    let mut cmd = tar_command();
    cmd.arg("-czf").arg(archive).arg("-C").arg(stage);
    cmd.args(members);
    finish_tar(cmd, &format!("writing {}", archive.display()))?;
    Ok(())
}

/// The member names inside `archive`.
pub fn tar_list(archive: &Path) -> Result<Vec<String>> {
    let mut cmd = tar_command();
    cmd.arg("-tzf").arg(archive);
    let out = finish_tar(cmd, &format!("reading {}", archive.display()))?;
    Ok(out
        .lines()
        .map(|l| l.trim_end_matches('/').to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// One member's bytes as text, without unpacking the archive.
pub fn tar_read_member(archive: &Path, member: &str) -> Result<String> {
    let mut cmd = tar_command();
    cmd.arg("-xzf").arg(archive).arg("-O").arg(member);
    finish_tar(cmd, &format!("reading {member} from {}", archive.display()))
}

/// Unpack one member of `archive` to `dest`, keeping the member's own name out
/// of it: the file lands at `dest` exactly.
pub fn tar_extract_member_to(archive: &Path, member: &str, dest: &Path) -> Result<()> {
    let file =
        File::create(dest).map_err(|e| anyhow::anyhow!("creating {}: {e}", dest.display()))?;
    let mut cmd = tar_command();
    cmd.arg("-xzf")
        .arg(archive)
        .arg("-O")
        .arg(member)
        .stdout(Stdio::from(file))
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| tar_spawn_error(&e))?;
    let stderr = read_all(child.stderr.take());
    let status = child
        .wait()
        .map_err(|e| anyhow::anyhow!("waiting for tar: {e}"))?;
    if !status.success() {
        return Err(anyhow::anyhow!(
            "extracting {member} from {}: {}",
            archive.display(),
            first_line(&stderr)
        ));
    }
    Ok(())
}

/// Unpack `members` of `archive` into `dest`, keeping their paths.
pub fn tar_extract_into(archive: &Path, dest: &Path, members: &[String]) -> Result<()> {
    std::fs::create_dir_all(dest)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", dest.display()))?;
    let mut cmd = tar_command();
    cmd.arg("-xzf").arg(archive).arg("-C").arg(dest);
    cmd.args(members);
    finish_tar(cmd, &format!("unpacking {}", archive.display()))?;
    Ok(())
}

fn finish_tar(mut cmd: Command, what: &str) -> Result<String> {
    let out = cmd.output().map_err(|e| tar_spawn_error(&e))?;
    if !out.status.success() {
        return Err(anyhow::anyhow!(
            "{what}: tar failed: {}",
            first_line(&String::from_utf8_lossy(&out.stderr))
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn tar_spawn_error(err: &std::io::Error) -> anyhow::Error {
    if err.kind() == std::io::ErrorKind::NotFound {
        anyhow::anyhow!(
            "`tar` was not found on PATH; chaps builds and reads backup archives with it"
        )
    } else {
        anyhow::anyhow!("could not run `tar`: {err}")
    }
}

// ---------------------------------------------------- component volumes ---

/// One named volume a component keeps state in. A component with several
/// volumes has one entry here per volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentVolume {
    /// Component name, as `components.yaml` and `--with` spell it, and as
    /// [`crate::components::Component::name`] returns it. Several entries share
    /// it when one component keeps several volumes.
    pub name: &'static str,
    /// What this volume's archive member is called inside [`COMPONENTS_MEMBER`],
    /// as [`component_member`] spells it out.
    ///
    /// The component name for a component that keeps one volume, so `ocs` is
    /// `components/ocs.tar` and `s3` is `components/s3.tar`, as they have been
    /// in every archive ever written; `<name>-<what it holds>` for each volume
    /// of a component that keeps several, because two members under one name
    /// would be two entries of the same path in the tar and only the last of
    /// them would come back out. So this field is the member and `name` is the
    /// component: the manifest goes on reporting the component an operator asked
    /// for while the archive keeps one member per volume.
    pub member: &'static str,
    /// Compose service the volume belongs to, and the one to hold still while
    /// it is read.
    pub service: &'static str,
    /// Named volume, without the compose project prefix.
    pub volume: &'static str,
    /// Where the service mounts it. Recorded for the operator who restores by
    /// hand; the tar itself is taken relative to the volume root.
    pub data_dir: &'static str,
}

/// Every volume a component keeps state in, in [`crate::components::Component`]
/// order and, within a component, in the order
/// [`crate::components::Component::volumes`] lists them.
///
/// chap-core's state is the database and the model volumes, each captured in
/// its own right. `ocs` keeps a data directory and `s3` its object store, and
/// the `data_dir`s here are the `target:` of the volume mount in
/// `compose.ocs.yml`, `compose.s3.yml` and `compose.dhis2.yml`.
///
/// This is the list an archive is made of, which is not quite the list a
/// component *keeps*: `dhis2_dump` is declared by `compose.dhis2.yml` and named
/// by [`crate::components::Component::volumes`], and it is deliberately not here.
/// It is a download cache - the one-shot fetches the dump again when it is
/// missing - so archiving it would add the whole dump to every backup of that
/// deployment for nothing.
/// `the_table_and_the_component_agree_on_every_volume` checks the two tables
/// against each other in the direction that always holds, and pins that one
/// omission with its reason.
pub const COMPONENT_VOLUMES: &[ComponentVolume] = &[
    ComponentVolume {
        name: "ocs",
        member: "ocs",
        service: crate::compose::OCS_SERVICE,
        volume: crate::compose::render::OCS_VOLUME,
        data_dir: "/app/data",
    },
    ComponentVolume {
        name: "s3",
        member: "s3",
        service: crate::compose::S3_SERVICE,
        volume: crate::compose::render::S3_VOLUME,
        data_dir: "/data",
    },
    ComponentVolume {
        name: "dhis2",
        member: "dhis2-home",
        service: crate::compose::DHIS2_SERVICE,
        volume: crate::compose::render::DHIS2_HOME_VOLUME,
        data_dir: "/opt/dhis2",
    },
    ComponentVolume {
        name: "dhis2",
        member: "dhis2-db",
        service: "dhis2-db",
        volume: crate::compose::render::DHIS2_DB_VOLUME,
        data_dir: "/var/lib/postgresql/data",
    },
];

/// The [`COMPONENT_VOLUMES`] this deployment has enabled, one entry per volume.
pub fn component_volumes(
    components: &crate::components::Components,
) -> Vec<&'static ComponentVolume> {
    COMPONENT_VOLUMES
        .iter()
        .filter(|part| {
            crate::components::Component::from_name(part.name)
                .is_ok_and(|component| components.is_enabled(component))
        })
        .collect()
}

// ------------------------------------------------------ volume contents ---

/// `docker run` arguments that read a named volume as a tar on stdout.
///
/// The model services carry a `<service>-init` container that mounts their
/// volume, so compose can read those without help; nothing but the component
/// itself mounts a volume in [`COMPONENT_VOLUMES`], and a container of their
/// own is the only way in. The volume is mounted at `/v` and nothing else is,
/// so an operator can run the same line by hand.
pub fn volume_read_args(volume: &str) -> Vec<String> {
    let mut args = docker_run_args(volume, false);
    args.extend(
        ["tar", "-C", "/v", "-cf", "-", "."]
            .iter()
            .map(|s| s.to_string()),
    );
    args
}

/// `docker run` arguments that empty a named volume and refill it from a tar
/// on stdin.
///
/// The three globs are what it takes to clear a directory with `sh`: `*`
/// misses dotfiles, `.[!.]*` catches them, and `..?*` catches the `..foo`
/// names neither of the others match, while none of them can ever match `.`
/// or `..` themselves. `rm` complains about the patterns that match nothing,
/// which is why only its stderr is dropped; tar's is the exit code that
/// counts.
pub fn volume_write_args(volume: &str) -> Vec<String> {
    let mut args = docker_run_args(volume, true);
    args.extend(["sh".to_string(), "-c".to_string(), refill_script("/v")]);
    args
}

/// The `sh` script that empties `dir` - dotfiles included, see
/// [`volume_write_args`] - and refills it from a tar on stdin.
///
/// One script for component volumes and model data directories alike: a
/// model restore that cleared `dir/*` alone left every hidden file written
/// since the backup in the restored volume.
pub fn refill_script(dir: &str) -> String {
    format!("rm -rf {dir}/* {dir}/.[!.]* {dir}/..?* 2>/dev/null; tar -C {dir} -xf -")
}

fn docker_run_args(volume: &str, stdin: bool) -> Vec<String> {
    let mut args = vec!["run".to_string(), "--rm".to_string()];
    if stdin {
        args.push("-i".to_string());
    }
    args.push("-v".to_string());
    args.push(format!("{volume}:/v"));
    args.push(BUSYBOX_IMAGE.to_string());
    args
}

/// Read the named volume `volume` into the file `dest`.
pub fn read_volume(volume: &str, dest: &Path) -> Result<crate::docker::Piped> {
    run_docker(&volume_read_args(volume), None, Some(dest))
}

/// Empty the named volume `volume` and unpack the tar `src` into it.
pub fn write_volume(volume: &str, src: &Path) -> Result<crate::docker::Piped> {
    run_docker(&volume_write_args(volume), Some(src), None)
}

/// Run `docker <args>` with its payload streams wired to files.
///
/// The same shape as [`crate::docker::run_compose_piped`] and for the same
/// reason: a volume tar is arbitrarily large, so it may not travel through a
/// pipe this process would have to drain. Only stderr is a pipe.
fn run_docker(
    args: &[String],
    stdin: Option<&Path>,
    stdout: Option<&Path>,
) -> Result<crate::docker::Piped> {
    crate::output::verbose(&format!("$ docker {}", args.join(" ")));
    let mut cmd = Command::new("docker");
    cmd.args(args);
    match stdin {
        Some(path) => {
            let file =
                File::open(path).map_err(|e| anyhow::anyhow!("opening {}: {e}", path.display()))?;
            cmd.stdin(Stdio::from(file));
        }
        None => {
            cmd.stdin(Stdio::null());
        }
    }
    match stdout {
        Some(path) => {
            let file = File::create(path)
                .map_err(|e| anyhow::anyhow!("creating {}: {e}", path.display()))?;
            cmd.stdout(Stdio::from(file));
        }
        None => {
            cmd.stdout(Stdio::null());
        }
    }
    cmd.stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!("`docker` was not found on PATH")
        } else {
            anyhow::anyhow!("could not run `docker`: {e}")
        }
    })?;
    let stderr = read_all(child.stderr.take());
    let status = child
        .wait()
        .map_err(|e| anyhow::anyhow!("waiting for docker: {e}"))?;
    Ok(crate::docker::Piped {
        code: status.code().unwrap_or(1),
        stderr,
    })
}

fn read_all(stream: Option<impl Read>) -> String {
    let mut text = String::new();
    if let Some(mut stream) = stream {
        let _ = stream.read_to_string(&mut text);
    }
    text
}

/// The first non-empty line of a child's stderr, for a one-line error.
pub fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no output")
        .to_string()
}

// ------------------------------------------------------------ pg_restore ---

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

// -------------------------------------------------------------- identity ---

/// The compose project name a restore leaves the deployment with.
///
/// The destination keeps its own: the compose project name is what every
/// container and named volume of a deployment is prefixed with, so taking the
/// archive's would point this deployment at the volumes of the one the backup
/// came from - and leave its own behind, running, under no name anything still
/// refers to. Restoring into a second deployment is a normal thing to do (a
/// staging copy of production, a rebuild beside the original), and it must not
/// be a takeover.
///
/// `adopt` is `--adopt-identity`: the archive's name is taken over, which is
/// what a deployment restoring itself onto a new machine wants. An empty
/// destination name is one `project.yaml` never recorded; it stays empty, so
/// compose keeps deriving it from the directory the way it always has.
pub fn restored_compose_project(destination: &str, archived: &str, adopt: bool) -> String {
    if adopt {
        archived.trim().to_string()
    } else {
        destination.trim().to_string()
    }
}

/// The `compose_project` an archived `.chaps/project.yaml` records, if any.
///
/// Read as plain YAML rather than through `ProjectState`, because this has to
/// work on a `project.yaml` from any version of chaps, including one this
/// binary would refuse to deserialise.
pub fn archived_compose_project(body: &str) -> Option<String> {
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(body).ok()?;
    let name = value.get("compose_project")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

// ------------------------------------------------------------------ plan ---

/// One model a restore will overwrite.
#[derive(Debug, Clone, Serialize)]
pub struct PlannedModel {
    pub service_id: String,
    pub data_dir: String,
    pub volume: String,
}

/// One component data volume a restore will overwrite.
#[derive(Debug, Clone, Serialize)]
pub struct PlannedComponent {
    pub name: String,
    pub service: String,
    pub data_dir: String,
    pub volume: String,
}

/// What `chaps backup restore` is about to do, printed before it asks.
#[derive(Debug, Clone, Serialize)]
pub struct RestorePlan {
    pub archive: PathBuf,
    pub project_dir: PathBuf,
    pub manifest: Manifest,
    /// Project files that will be written, relative to the project directory.
    pub files: Vec<String>,
    /// Whether the database will be restored over the running one.
    pub database: bool,
    /// Model data volumes that will be emptied and refilled.
    pub models: Vec<PlannedModel>,
    /// Component data volumes that will be emptied and refilled.
    pub components: Vec<PlannedComponent>,
    /// Running services that will be stopped first.
    pub stop: Vec<String>,
    /// Whether CHAP is brought back up at the end.
    pub start: bool,
    /// The compose project name the deployment is left with. See
    /// [`restored_compose_project`].
    pub compose_project: String,
    /// The compose project name the archive was taken under, when it recorded
    /// one.
    pub archived_compose_project: Option<String>,
    /// Whether `--adopt-identity` was passed.
    pub adopt_identity: bool,
}

impl RestorePlan {
    /// Whether the plan would change anything at all.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
            && !self.database
            && self.models.is_empty()
            && self.components.is_empty()
    }
}

/// What the plan says about the compose project name, when there is anything
/// to say: nothing when the archive records none, and nothing when it records
/// the one this deployment already has.
///
/// Worth a line of its own whenever the two differ, because it is the one
/// thing a restore deliberately does not take from the archive. See
/// [`restored_compose_project`].
pub fn identity_line(plan: &RestorePlan) -> String {
    let Some(archived) = plan.archived_compose_project.as_deref() else {
        return String::new();
    };
    let kept = if plan.compose_project.is_empty() {
        "the name compose derives from this directory"
    } else {
        &plan.compose_project
    };
    if plan.adopt_identity {
        return format!(
            "  identity  compose project {archived}, taken over from the archive \
             (--adopt-identity)\n"
        );
    }
    if archived == plan.compose_project {
        return String::new();
    }
    format!(
        "  identity  {kept} is kept; the archive's own ({archived}) is not adopted, so this \
         deployment keeps its containers and volumes\n"
    )
}

/// The plan as an operator reads it before answering the confirmation.
pub fn plan_text(plan: &RestorePlan) -> String {
    let manifest = &plan.manifest;
    let mut text = String::new();
    text.push_str(&format!("restore {}\n", plan.archive.display()));
    text.push_str(&crate::output::fields(
        2,
        &[
            (
                "taken",
                format!("{} by {}", manifest.created_at, manifest.created_by),
            ),
            (
                "from",
                format!(
                    "project {} (chap-core {})",
                    manifest.project, manifest.chap_image_tag
                ),
            ),
            ("into", plan.project_dir.display().to_string()),
        ],
    ));

    text.push_str("\nthis overwrites\n");
    if plan.files.is_empty() {
        text.push_str("  files     nothing\n");
    } else {
        text.push_str(&format!(
            "  files     {} file(s) in the project directory: {}\n",
            plan.files.len(),
            plan.files.join(", ")
        ));
    }
    match (&plan.database, &manifest.database) {
        (true, Some(db)) => text.push_str(&format!(
            "  database  {} on postgres, dropped and reloaded (pg_restore --clean --if-exists)\n",
            db.name
        )),
        _ => text.push_str("  database  nothing\n"),
    }
    if plan.models.is_empty() {
        text.push_str("  models    nothing\n");
    } else {
        for (i, model) in plan.models.iter().enumerate() {
            let label = if i == 0 { "  models  " } else { "          " };
            text.push_str(&format!(
                "{label}  {} {} emptied and refilled (volume {})\n",
                model.service_id, model.data_dir, model.volume
            ));
        }
    }
    if plan.components.is_empty() {
        text.push_str("  parts     nothing\n");
    } else {
        for (i, part) in plan.components.iter().enumerate() {
            let label = if i == 0 { "  parts   " } else { "          " };
            text.push_str(&format!(
                "{label}  {} {} emptied and refilled (volume {})\n",
                part.service, part.data_dir, part.volume
            ));
        }
    }
    text.push_str(&identity_line(plan));

    text.push('\n');
    if plan.stop.is_empty() {
        text.push_str("nothing is running, so nothing is stopped first\n");
    } else {
        text.push_str(&format!("stops first  {}\n", plan.stop.join(", ")));
    }
    if plan.start {
        text.push_str("then runs    docker compose up -d\n");
    } else {
        text.push_str("then leaves  CHAP as it is (--no-start)\n");
    }
    text
}

#[cfg(test)]
mod tests;
