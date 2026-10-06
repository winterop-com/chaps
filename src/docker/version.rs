//! The installed `docker compose` version, and whether it is new enough.

use super::{INCLUDE_COMPOSE_VERSION, MIN_COMPOSE_VERSION, spawn_error, trace_command};
use crate::error::Result;
use std::process::{Command, Stdio};

/// The installed `docker compose` version, from `docker compose version --short`.
pub fn compose_version() -> Result<(u32, u32, u32)> {
    trace_command(&[
        "compose".to_string(),
        "version".to_string(),
        "--short".to_string(),
    ]);
    let out = Command::new("docker")
        .args(["compose", "version", "--short"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| spawn_error(&e))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(anyhow::anyhow!(
            "`docker compose version --short` failed{}",
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {stderr}")
            }
        ));
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

/// Warn when the installed compose predates [`MIN_COMPOSE_VERSION`].
///
/// Returns `Ok(None)` when compose is new enough, and an error when the version
/// cannot be determined at all (docker missing, for instance) so callers can
/// decide whether that is fatal.
pub fn check_compose_version() -> Result<Option<String>> {
    let found = compose_version()?;
    if found >= MIN_COMPOSE_VERSION {
        return Ok(None);
    }
    let (fa, fb, fc) = found;
    let (wa, wb, wc) = MIN_COMPOSE_VERSION;
    let (ia, ib, ic) = INCLUDE_COMPOSE_VERSION;
    Ok(Some(format!(
        "docker compose {fa}.{fb}.{fc} is older than {wa}.{wb}.{wc}; \
         compose.varde.yml uses `!override`, which needs {wa}.{wb}.{wc} or newer, and \
         compose.marketplace.yml uses `include:`, which needs {ia}.{ib}.{ic}"
    )))
}

/// Parse `docker compose version --short` output into `(major, minor, patch)`.
///
/// Accepts `v5.5.1`, `2.24.0`, `2.24.0-desktop.1` and the like: a leading `v`
/// is dropped and everything after the numeric dotted prefix is ignored.
/// Missing components default to zero, so `2.24` parses as `(2, 24, 0)`.
pub fn parse_version(text: &str) -> Result<(u32, u32, u32)> {
    let raw = text.trim();
    let trimmed = raw.strip_prefix('v').unwrap_or(raw).trim_start();
    let numeric: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();

    let mut parts = numeric
        .split('.')
        .filter(|p| !p.is_empty())
        .map(str::parse::<u32>);

    let major = match parts.next() {
        Some(Ok(n)) => n,
        _ => return Err(anyhow::anyhow!("unrecognised compose version `{raw}`")),
    };
    let minor = match parts.next() {
        Some(Ok(n)) => n,
        Some(Err(_)) => return Err(anyhow::anyhow!("unrecognised compose version `{raw}`")),
        None => 0,
    };
    let patch = match parts.next() {
        Some(Ok(n)) => n,
        Some(Err(_)) => return Err(anyhow::anyhow!("unrecognised compose version `{raw}`")),
        None => 0,
    };
    Ok((major, minor, patch))
}
