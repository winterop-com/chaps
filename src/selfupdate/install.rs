//! Installing a release: downloading the archive and its checksums,
//! verifying one against the other, and putting the new binary in place of
//! the running one.

use super::{MAX_ARCHIVE_BYTES, SUMS_FILE, USER_AGENT, binary_name};
use crate::chapcore::sha256_hex;
use crate::error::{ChapError, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

// --------------------------------------------------------------------------
// Checksums
// --------------------------------------------------------------------------

/// The recorded digest of `name` in a `SHA256SUMS` body.
///
/// Both coreutils spellings are accepted: `<hex>  <name>` and the binary-mode
/// `<hex> *<name>`, with or without a `./` prefix on the name, which is what
/// `sha256sum` and `shasum -a 256` each write.
pub fn sha256_for(sums: &str, name: &str) -> Option<String> {
    for line in sums.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(2, char::is_whitespace);
        let digest = fields.next()?.trim();
        let listed = fields.next()?.trim().trim_start_matches('*');
        let listed = listed.trim_start_matches("./");
        if listed == name && digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(digest.to_ascii_lowercase());
        }
    }
    None
}

/// Check `bytes` against the digest `sums` records for `name`.
pub fn verify(sums: &str, name: &str, bytes: &[u8]) -> Result<()> {
    let expected = sha256_for(sums, name)
        .ok_or_else(|| anyhow::anyhow!("{SUMS_FILE} does not list {name}"))?;
    let actual = sha256_hex(bytes);
    if actual != expected {
        return Err(anyhow::anyhow!(
            "{name} does not match {SUMS_FILE}: expected {expected}, got {actual}"
        ));
    }
    Ok(())
}

// --------------------------------------------------------------------------
// Replacing the running binary
// --------------------------------------------------------------------------

/// Where the new binary is staged, beside the one it replaces.
///
/// A sibling rather than a temporary directory, because a rename is only
/// atomic within one filesystem and `/tmp` is routinely a different one.
pub fn staged_path(current: &Path) -> PathBuf {
    sibling(current, "new")
}

/// Where the outgoing binary is parked on Windows, which cannot rename over a
/// running executable but can rename the running executable itself.
pub fn backup_path(current: &Path) -> PathBuf {
    sibling(current, "old")
}

fn sibling(current: &Path, extension: &str) -> PathBuf {
    let stem = current
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "varde".to_string());
    current.with_file_name(format!("{stem}.{extension}"))
}

/// Delete a leftover `varde.old` from a previous Windows update.
///
/// Best effort and silent: the file is only still there when the previous
/// process had not exited yet, and the next run will get it. Compiled for
/// Windows alone, because nothing else ever writes a `varde.old` and a file
/// of that name beside a Unix install belongs to whoever put it there.
#[cfg(any(windows, test))]
pub fn clean_backup(current: &Path) {
    let backup = backup_path(current);
    if backup.exists() {
        let _ = std::fs::remove_file(&backup);
    }
}

/// Whether the binary at `path` can be replaced in place.
///
/// The directory has to be writable, since the swap is a rename inside it,
/// not a write through the existing file.
pub fn is_replaceable(path: &Path) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let probe = dir.join(format!(".varde-write-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(file) => {
            // Windows will not delete a file whose handle is still open, so
            // the probe is closed before it is removed.
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Put `contents` at `current`, atomically as far as the platform allows.
///
/// Unix renames the staged file over the running one, which the kernel allows
/// and which leaves no window where `current` does not exist. Windows cannot
/// rename over a running image, so the running one is moved aside first and
/// the move is undone if the second rename fails.
pub fn replace_executable(current: &Path, contents: &[u8]) -> Result<()> {
    let staged = staged_path(current);
    std::fs::write(&staged, contents)
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", staged.display()))?;
    copy_mode(current, &staged);

    if cfg!(windows) {
        let backup = backup_path(current);
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(current, &backup).map_err(|e| {
            let _ = std::fs::remove_file(&staged);
            anyhow::anyhow!("moving {} aside: {e}", current.display())
        })?;
        if let Err(e) = std::fs::rename(&staged, current) {
            // Put the old binary back rather than leave the path empty.
            let _ = std::fs::rename(&backup, current);
            let _ = std::fs::remove_file(&staged);
            return Err(anyhow::anyhow!("replacing {}: {e}", current.display()));
        }
        return Ok(());
    }

    std::fs::rename(&staged, current).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        anyhow::anyhow!("replacing {}: {e}", current.display())
    })
}

/// Give the staged file the mode the binary it replaces has, and make sure it
/// is executable whatever that mode was.
#[cfg(unix)]
fn copy_mode(current: &Path, staged: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(current)
        .map(|meta| meta.permissions().mode() & 0o7777)
        .unwrap_or(0o755);
    let _ = std::fs::set_permissions(staged, std::fs::Permissions::from_mode(mode | 0o700));
}

/// Windows has no mode bits to carry over.
#[cfg(not(unix))]
fn copy_mode(_current: &Path, _staged: &Path) {}

/// Unpack `archive` into `dir` with the system `tar`.
///
/// `tar -xf` reads both formats this project ships: gzipped tar everywhere,
/// and zip on Windows, where `tar.exe` has been bsdtar since Windows 10 1803
/// and handles a zip without a second tool.
pub fn extract(archive: &Path, dir: &Path) -> Result<()> {
    let status = std::process::Command::new("tar")
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(dir)
        .status()
        .map_err(|e| anyhow::anyhow!("running tar to unpack {}: {e}", archive.display()))?;
    if !status.success() {
        return Err(anyhow::anyhow!(
            "tar could not unpack {} (exit {})",
            archive.display(),
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

/// Find the `varde` executable somewhere under `dir`.
///
/// The archives put it in a `varde-<tag>-<target>/` directory, but the search
/// does not depend on that: it walks what was unpacked and takes the first
/// file with the right name.
pub fn find_binary(dir: &Path, target: &str) -> Option<PathBuf> {
    let wanted = binary_name(target);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|name| name == wanted) {
                return Some(path);
            }
        }
    }
    None
}

// --------------------------------------------------------------------------
// HTTP
// --------------------------------------------------------------------------

/// GET `url` as text, with one global deadline covering connect, send and
/// receive.
pub(super) fn get_text(url: &str, timeout: Duration) -> Result<String> {
    let mut response = request(url, timeout)?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("reading the response body from {url}: {e}"))
}

/// GET `url` as bytes, for an archive.
pub fn download(url: &str, timeout: Duration) -> Result<Vec<u8>> {
    let mut response = request(url, timeout)?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_ARCHIVE_BYTES)
        .read_to_vec()
        .map_err(|e| anyhow::anyhow!("downloading {url}: {e}"))
}

/// Download `url` as text, for `SHA256SUMS`.
pub fn download_text(url: &str, timeout: Duration) -> Result<String> {
    get_text(url, timeout)
}

fn request(url: &str, timeout: Duration) -> Result<ureq::http::Response<ureq::Body>> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(USER_AGENT)
            .build(),
    );
    let started = std::time::Instant::now();
    let response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| {
            crate::output::verbose(&format!(
                "GET {url} -> failed in {}ms",
                started.elapsed().as_millis()
            ));
            map_error(url, e)
        })?;
    crate::output::verbose(&format!(
        "GET {url} -> {} in {}ms",
        response.status().as_u16(),
        started.elapsed().as_millis()
    ));
    Ok(response)
}

/// A non-2xx response is a [`ChapError::Http`]; everything else is a transport
/// failure that keeps the URL as context.
pub(super) fn map_error(url: &str, err: ureq::Error) -> anyhow::Error {
    match err {
        ureq::Error::StatusCode(status) => ChapError::Http {
            url: url.to_string(),
            status,
        }
        .into(),
        other => anyhow::anyhow!("fetching {url}: {other}"),
    }
}
