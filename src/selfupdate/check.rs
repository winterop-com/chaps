//! The once-a-day check: `self-update-check.json`, when it is stale, and the
//! one line the notice prints.

use super::release::{is_commit_sha, same_commit};
use super::{CHECK_FILE, NO_CHECK_ENV};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `self-update-check.json`: when the release feed was last asked, and what it
/// said.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CheckState {
    /// Seconds since the Unix epoch at the time of the check.
    pub checked_at_unix: u64,
    /// The tag the feed reported, empty when the check failed.
    #[serde(default)]
    pub latest: String,
    /// The commit that release was built from, for the `dev` channel, where
    /// the tag never changes and so says nothing on its own. Empty on the
    /// stable channel and on a state written by an older chaps.
    #[serde(default)]
    pub commit: String,
}

/// Where the check state is kept.
pub fn check_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(CHECK_FILE)
}

/// Read the check state, or `None` when there is none to read.
///
/// A missing, truncated or hand-edited file is "no state", never an error:
/// the notice is a courtesy and must not be able to fail a command.
pub fn read_check(cache_dir: &Path) -> Option<CheckState> {
    let body = std::fs::read_to_string(check_path(cache_dir)).ok()?;
    serde_json::from_str(&body).ok()
}

/// Record a check. Failures are swallowed for the same reason.
pub fn write_check(cache_dir: &Path, state: &CheckState) {
    if std::fs::create_dir_all(cache_dir).is_err() {
        return;
    }
    if let Ok(body) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(check_path(cache_dir), format!("{body}\n"));
    }
}

/// Seconds since the Unix epoch, or 0 on a clock that predates it.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Whether the release feed should be asked again.
///
/// No state at all means yes; a timestamp in the future (a clock that moved
/// backwards since, or a copied cache) also means yes rather than never
/// again.
pub fn is_stale(state: Option<&CheckState>, now: u64, interval: Duration) -> bool {
    match state {
        None => true,
        Some(state) => {
            let age = now.saturating_sub(state.checked_at_unix);
            state.checked_at_unix > now || age >= interval.as_secs()
        }
    }
}

/// Whether the environment asked for no update checks at all.
pub fn checks_disabled() -> bool {
    disabled_by(std::env::var(NO_CHECK_ENV).ok().as_deref())
}

/// [`checks_disabled`] for an explicit value.
///
/// Exactly `1` turns the notice off. Unset, empty and anything else leave it
/// on, so `CHAPS_NO_UPDATE_CHECK=0` reads the way it looks.
pub(super) fn disabled_by(value: Option<&str>) -> bool {
    value == Some("1")
}

/// The one line the notice prints, or `None` when there is nothing to say.
pub fn notice_line(latest: &str, current: &str) -> Option<String> {
    crate::chapcore::is_newer(latest, current).then(|| {
        format!("chaps {latest} is available (you have v{current}): run `chaps self update`")
    })
}

/// The same line for a `dev` build, which compares commits rather than
/// versions: the rolling release carries the version of the last tag, so the
/// only thing that moves between two dev builds is the commit.
///
/// Silent unless both commits are known and they differ. An unprompted line
/// that cannot tell whether there is anything new would be noise on every
/// command, which is the opposite trade-off from [`dev_update_available`](super::dev_update_available),
/// where the user asked.
pub fn dev_notice_line(commit: &str, revision: &str) -> Option<String> {
    if !is_commit_sha(commit) || !is_commit_sha(revision) || same_commit(commit, revision) {
        return None;
    }
    let short = &commit[..revision.len().min(commit.len()).max(7)];
    Some(format!(
        "a newer chaps dev build is available (commit {short}, you have {revision}): \
         run `chaps self update`"
    ))
}
