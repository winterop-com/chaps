//! The backup archive: its layout, its manifest, and the pure helpers
//! `chaps backup create` and `chaps backup restore` share.
//!
//! An archive is a plain `tar.gz` with four members, all optional but the
//! first:
//!
//! ```text
//! manifest.yaml            what this archive is and what it holds
//! files/                   .env, .chaps/** and every compose*.yml
//! db/chap_core.dump        pg_dump -Fc of the chap-core database
//! models/<service_id>.tar  one model data volume, as tar saw it
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
/// Scratch directory inside `.chaps/`, never part of a backup.
pub const TMP_DIR: &str = "tmp";
/// Where `backup restore` parks the `.env` it is about to replace.
pub const ENV_BACKUP_FILE: &str = ".env.before-restore";

/// Archive member holding one model's data directory.
pub fn model_member(service_id: &str) -> String {
    format!("{MODELS_MEMBER}/{service_id}.tar")
}

/// What one backup is and what it holds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema_version: u32,
    /// e.g. `chaps-cli 0.1.0`.
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
}

impl Manifest {
    /// Models whose data directory is actually in the archive.
    pub fn captured_models(&self) -> impl Iterator<Item = &ManifestModel> {
        self.models.iter().filter(|m| m.path.is_some())
    }

    /// Bytes the archive holds before compression.
    pub fn content_bytes(&self) -> u64 {
        self.database.as_ref().map_or(0, |d| d.size_bytes)
            + self.models.iter().map(|m| m.size_bytes).sum::<u64>()
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
    pub host_port: u16,
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
/// Only what docker compose itself reads: `KEY=VALUE`, an optional `export`
/// prefix, and quotes stripped when they wrap the whole value. A later line
/// wins, the way compose resolves duplicates.
pub fn env_value(body: &str, key: &str) -> Option<String> {
    let mut found = None;
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        found = Some(value.to_string());
    }
    found
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
/// a stable order: `.env`, then `.chaps/**`, then the root `compose*.yml`.
///
/// `.chaps/tmp/` is scratch space (this is where the archive is staged) and
/// half-written `.tmp` state files are transient, so neither is included.
/// Compose files are taken from the project root only; a `compose.yml` in a
/// subdirectory belongs to something else.
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

/// Copy `rel` from `from` to `to`, creating the parent directories.
pub fn copy_file(from: &Path, to: &Path, rel: &str) -> Result<()> {
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

/// How a `pg_restore` exit code should be read.
///
/// `pg_restore` exits 1 when it finished but something it did not need
/// succeeded - dropping an object `--if-exists` never found, most of all - so
/// treating 1 as failure would make every `--clean` restore look broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgRestore {
    /// Clean run.
    Ok,
    /// Restored, with warnings worth printing.
    Warnings,
    /// Did not restore.
    Failed,
}

/// Read a `pg_restore` exit code.
pub fn pg_restore_outcome(code: i32) -> PgRestore {
    match code {
        0 => PgRestore::Ok,
        1 => PgRestore::Warnings,
        _ => PgRestore::Failed,
    }
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

// ------------------------------------------------------------------ plan ---

/// One model a restore will overwrite.
#[derive(Debug, Clone, Serialize)]
pub struct PlannedModel {
    pub service_id: String,
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
    /// Running services that will be stopped first.
    pub stop: Vec<String>,
    /// Whether the stack is brought back up at the end.
    pub start: bool,
}

impl RestorePlan {
    /// Whether the plan would change anything at all.
    pub fn is_empty(&self) -> bool {
        self.files.is_empty() && !self.database && self.models.is_empty()
    }
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

    text.push('\n');
    if plan.stop.is_empty() {
        text.push_str("nothing is running, so nothing is stopped first\n");
    } else {
        text.push_str(&format!("stops first  {}\n", plan.stop.join(", ")));
    }
    if plan.start {
        text.push_str("then runs    docker compose up -d\n");
    } else {
        text.push_str("then leaves  the stack as it is (--no-start)\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            schema_version: SCHEMA_VERSION,
            created_by: "chaps-cli 0.1.0".into(),
            created_at: "2026-09-23T07:10:00Z".into(),
            project: "e2e".into(),
            chap_image_tag: "latest".into(),
            files: vec![".env".into(), ".chaps/models.yaml".into()],
            database: Some(ManifestDatabase {
                path: DB_MEMBER.into(),
                user: "chap".into(),
                name: "chap_core".into(),
                server_version: Some("17.6".into()),
                size_bytes: 2048,
            }),
            models: vec![ManifestModel {
                id: "chapkit_ewars_model".into(),
                service_id: "chapkit-ewars-model".into(),
                version: "1.0.0".into(),
                image_tag: "sha-fa880a1".into(),
                host_port: 5001,
                data_dir: "/app/data".into(),
                user: "chapkit:chapkit".into(),
                volume: "ck_chapkit_ewars_model_data".into(),
                path: Some(model_member("chapkit-ewars-model")),
                size_bytes: 40960,
                skipped: None,
            }],
        }
    }

    #[test]
    fn a_manifest_round_trips_through_yaml() {
        let body = render_manifest(&manifest()).unwrap();
        assert!(body.starts_with("# chaps backup manifest"));
        assert!(body.contains("schema_version: 1"));
        assert!(body.contains("service_id: chapkit-ewars-model"));
        let back = parse_manifest(&body, Path::new("/tmp/a.tar.gz")).unwrap();
        assert_eq!(back, manifest());
        assert_eq!(back.captured_models().count(), 1);
        assert_eq!(back.content_bytes(), 2048 + 40960);
    }

    #[test]
    fn a_manifest_without_the_optional_parts_still_parses() {
        let body = "schema_version: 1\n\
                    created_by: chaps-cli 0.1.0\n\
                    created_at: 2026-09-23T07:10:00Z\n\
                    project: e2e\n\
                    chap_image_tag: latest\n";
        let parsed = parse_manifest(body, Path::new("/tmp/a.tar.gz")).unwrap();
        assert!(parsed.files.is_empty());
        assert!(parsed.database.is_none());
        assert!(parsed.models.is_empty());
        assert_eq!(parsed.content_bytes(), 0);
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_half_understood() {
        let body = "schema_version: 99\n\
                    created_by: chaps-cli 9.9.9\n\
                    created_at: 2026-09-23T07:10:00Z\n\
                    project: e2e\n\
                    chap_image_tag: latest\n";
        let err = parse_manifest(body, Path::new("/tmp/a.tar.gz")).unwrap_err();
        assert!(err.to_string().contains("newer than this chaps"));

        let err = parse_manifest("not: [a manifest", Path::new("/tmp/a.tar.gz")).unwrap_err();
        assert!(err.to_string().contains("not a chaps backup manifest"));
    }

    #[test]
    fn archive_names_carry_the_project_and_the_stamp() {
        assert_eq!(
            archive_name("e2e", "20260923-071000"),
            "chaps-backup-e2e-20260923-071000.tar.gz"
        );
        assert_eq!(
            archive_name("CHAP prod (eu)", "20260923-071000"),
            "chaps-backup-CHAP-prod-eu-20260923-071000.tar.gz"
        );
        assert_eq!(
            archive_name("", "20260923-071000"),
            "chaps-backup-20260923-071000.tar.gz"
        );
    }

    #[test]
    fn out_is_a_file_a_directory_or_the_working_directory() {
        let cwd = Path::new("/home/me");
        let name = "chaps-backup-e2e-20260923-071000.tar.gz";

        assert_eq!(
            resolve_out_path(None, false, cwd, name),
            PathBuf::from("/home/me").join(name)
        );
        assert_eq!(
            resolve_out_path(Some(Path::new("/backups")), true, cwd, name),
            PathBuf::from("/backups").join(name)
        );
        assert_eq!(
            resolve_out_path(Some(Path::new("/backups/today.tar.gz")), false, cwd, name),
            PathBuf::from("/backups/today.tar.gz")
        );
        // A trailing separator says "directory" even for one that does not exist.
        assert_eq!(
            resolve_out_path(Some(Path::new("nightly/")), false, cwd, name),
            PathBuf::from("nightly").join(name)
        );
    }

    #[test]
    fn utc_conversion_matches_known_instants() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(stamp(0), "19700101-000000");
        // 2026-09-23T07:10:00Z
        assert_eq!(timestamp(1_790_147_400), "2026-09-23T07:10:00Z");
        assert_eq!(stamp(1_790_147_400), "20260923-071000");
        // A leap day, and the last second of a year.
        assert_eq!(timestamp(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(timestamp(1_735_689_599), "2024-12-31T23:59:59Z");
        assert!(now() > 1_700_000_000, "the clock is past 2023");
    }

    #[test]
    fn sizes_read_like_du() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(912), "912 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(4400), "4.3 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn env_values_come_from_active_lines_only() {
        let body = "# POSTGRES_USER=commented\n\
                    POSTGRES_USER=chapuser\n\
                    export POSTGRES_DB=\"chapdb\"\n\
                    POSTGRES_PASSWORD='p a s s'\n\
                    JUNK\n";
        assert_eq!(
            env_value(body, "POSTGRES_USER").as_deref(),
            Some("chapuser")
        );
        assert_eq!(env_value(body, "POSTGRES_DB").as_deref(), Some("chapdb"));
        assert_eq!(
            env_value(body, "POSTGRES_PASSWORD").as_deref(),
            Some("p a s s")
        );
        assert_eq!(env_value(body, "NOPE"), None);
        assert_eq!(
            postgres_credentials(body),
            ("chapuser".to_string(), "chapdb".to_string())
        );
        // A later line wins, the way compose resolves duplicates.
        assert_eq!(
            env_value("POSTGRES_DB=a\nPOSTGRES_DB=b\n", "POSTGRES_DB").as_deref(),
            Some("b")
        );
        // Nothing to read: chap-core's own defaults.
        assert_eq!(
            postgres_credentials(""),
            ("chap".to_string(), "chap_core".to_string())
        );
    }

    #[test]
    fn the_old_env_is_kept_only_when_it_differs() {
        assert!(keep_env_copy(Some(b"a=1"), Some(b"a=2")));
        assert!(!keep_env_copy(Some(b"a=1"), Some(b"a=1")));
        assert!(!keep_env_copy(Some(b"a=1"), None));
        assert!(!keep_env_copy(None, Some(b"a=1")));
        assert!(!keep_env_copy(None, None));
    }

    #[test]
    fn the_file_list_takes_env_chaps_and_the_root_compose_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let write = |rel: &str, body: &str| {
            let path = join_relative(root, rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        };
        write(".env", "POSTGRES_USER=chap\n");
        write(".chaps/project.yaml", "schema_version: 1\n");
        write(".chaps/models.yaml", "{}\n");
        write(".chaps/compose.chap-core.v2.3.1.yml", "services: {}\n");
        write(".chaps/.models.yaml.tmp", "half written");
        write(".chaps/tmp/backup-1/manifest.yaml", "staged");
        write("compose.yml", "services: {}\n");
        write("compose.marketplace.yml", "include: []\n");
        write("compose.chapkit-ewars-model.yml", "services: {}\n");
        write("compose.override.yml", "services: {}\n");
        write("notes.md", "not a compose file");
        write("compose.yaml.bak", "not a compose file either");
        write("sub/compose.yml", "someone else's stack");

        assert_eq!(
            project_files(root),
            vec![
                ".env",
                ".chaps/compose.chap-core.v2.3.1.yml",
                ".chaps/models.yaml",
                ".chaps/project.yaml",
                "compose.chapkit-ewars-model.yml",
                "compose.marketplace.yml",
                "compose.override.yml",
                "compose.yml",
            ]
        );
    }

    #[test]
    fn the_file_list_of_an_empty_directory_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(project_files(dir.path()).is_empty());
        assert!(is_compose_file("compose.yml"));
        assert!(is_compose_file("compose.marketplace.yml"));
        assert!(!is_compose_file("compose.yaml"));
        assert!(!is_compose_file("docker-compose.yml"));
    }

    #[test]
    fn pg_restore_exit_one_is_warnings_not_failure() {
        assert_eq!(pg_restore_outcome(0), PgRestore::Ok);
        assert_eq!(pg_restore_outcome(1), PgRestore::Warnings);
        assert_eq!(pg_restore_outcome(2), PgRestore::Failed);
        assert_eq!(pg_restore_outcome(127), PgRestore::Failed);

        let warnings = pg_restore_warnings(
            "pg_restore: warning: errors ignored on restore: 2\n\
             \n\
             pg_restore: error: could not execute query: DROP SCHEMA public\n",
        );
        assert_eq!(
            warnings,
            vec![
                "warning: errors ignored on restore: 2",
                "error: could not execute query: DROP SCHEMA public",
            ]
        );
        assert!(pg_restore_warnings("   \n\n").is_empty());
    }

    fn plan(start: bool, stop: Vec<&str>) -> RestorePlan {
        RestorePlan {
            archive: PathBuf::from("/backups/chaps-backup-e2e-20260923-071000.tar.gz"),
            project_dir: PathBuf::from("/srv/e2e"),
            manifest: manifest(),
            files: vec![".env".into(), ".chaps/models.yaml".into()],
            database: true,
            models: vec![PlannedModel {
                service_id: "chapkit-ewars-model".into(),
                data_dir: "/app/data".into(),
                volume: "ck_chapkit_ewars_model_data".into(),
            }],
            stop: stop.into_iter().map(str::to_string).collect(),
            start,
        }
    }

    #[test]
    fn the_plan_says_what_it_overwrites_and_what_it_stops() {
        let text = plan_text(&plan(true, vec!["chap", "worker"]));
        assert!(text.starts_with("restore /backups/chaps-backup-e2e-20260923-071000.tar.gz\n"));
        assert!(text.contains("taken  2026-09-23T07:10:00Z by chaps-cli 0.1.0"));
        assert!(text.contains("into   /srv/e2e"));
        assert!(
            text.contains("files     2 file(s) in the project directory: .env, .chaps/models.yaml")
        );
        assert!(text.contains("database  chap_core on postgres, dropped and reloaded"));
        assert!(text.contains("chapkit-ewars-model /app/data emptied and refilled"));
        assert!(text.contains("stops first  chap, worker"));
        assert!(text.contains("then runs    docker compose up -d"));
        assert!(!plan(true, vec![]).is_empty());
    }

    #[test]
    fn the_plan_of_a_stopped_stack_says_there_is_nothing_to_stop() {
        let text = plan_text(&plan(false, vec![]));
        assert!(text.contains("nothing is running, so nothing is stopped first"));
        assert!(text.contains("then leaves  the stack as it is (--no-start)"));
    }

    #[test]
    fn a_plan_that_restores_nothing_says_so() {
        let mut plan = plan(true, vec![]);
        plan.files.clear();
        plan.database = false;
        plan.models.clear();
        assert!(plan.is_empty());
        let text = plan_text(&plan);
        assert!(text.contains("files     nothing"));
        assert!(text.contains("database  nothing"));
        assert!(text.contains("models    nothing"));
    }

    #[test]
    fn the_stage_is_inside_chaps_and_cleans_up_after_itself() {
        let dir = tempfile::tempdir().unwrap();
        let chaps = dir.path().join(".chaps");
        let path = {
            let stage = Stage::new(&chaps, "backup").unwrap();
            assert!(stage.dir.starts_with(chaps.join(TMP_DIR)));
            let nested = stage.path("files/.chaps/models.yaml").unwrap();
            std::fs::write(&nested, "x").unwrap();
            assert!(nested.is_file());
            stage.dir.clone()
        };
        assert!(!path.exists(), "the stage is removed when it is dropped");
        assert!(
            !chaps.join(TMP_DIR).exists(),
            "and so is the tmp directory it lived in"
        );
    }

    #[test]
    fn archives_are_written_listed_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let stage = dir.path().join("stage");
        std::fs::create_dir_all(stage.join("files/.chaps")).unwrap();
        std::fs::write(stage.join("manifest.yaml"), "schema_version: 1\n").unwrap();
        std::fs::write(stage.join("files/.env"), "POSTGRES_USER=chap\n").unwrap();
        std::fs::write(stage.join("files/.chaps/models.yaml"), "{}\n").unwrap();

        let archive = dir.path().join("out.tar.gz");
        tar_create(
            &archive,
            &stage,
            &[MANIFEST_MEMBER.to_string(), FILES_MEMBER.to_string()],
        )
        .unwrap();
        assert!(archive.is_file());

        let members = tar_list(&archive).unwrap();
        assert!(members.contains(&MANIFEST_MEMBER.to_string()));
        assert!(members.iter().any(|m| m == "files/.env"));

        assert_eq!(
            tar_read_member(&archive, MANIFEST_MEMBER).unwrap(),
            "schema_version: 1\n"
        );

        let one = dir.path().join("one.txt");
        tar_extract_member_to(&archive, "files/.env", &one).unwrap();
        assert_eq!(
            std::fs::read_to_string(&one).unwrap(),
            "POSTGRES_USER=chap\n"
        );

        let out = dir.path().join("unpacked");
        tar_extract_into(&archive, &out, &[FILES_MEMBER.to_string()]).unwrap();
        assert!(out.join("files/.chaps/models.yaml").is_file());

        let err = tar_read_member(&archive, "db/chap_core.dump").unwrap_err();
        assert!(err.to_string().contains("reading db/chap_core.dump"));
    }

    #[test]
    fn copy_file_creates_the_directories_it_needs() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("from");
        let to = dir.path().join("to");
        std::fs::create_dir_all(from.join(".chaps")).unwrap();
        std::fs::write(from.join(".chaps/models.yaml"), "{}\n").unwrap();
        copy_file(&from, &to, ".chaps/models.yaml").unwrap();
        assert_eq!(
            std::fs::read_to_string(to.join(".chaps/models.yaml")).unwrap(),
            "{}\n"
        );
    }
}
