//! Filesystem locations that are not part of a generated project.

use std::path::PathBuf;

/// Directory holding the cached registry snapshot.
///
/// Resolution order:
/// 1. `$VARDE_CACHE_DIR`
/// 2. `$XDG_CACHE_HOME/varde`
/// 3. `%LOCALAPPDATA%\varde\cache` (Windows only)
/// 4. `$HOME/.cache/varde`
/// 5. `./.chap-cache` (last resort, so the CLI still works in a bare container)
///
/// Empty environment variables are treated as unset. The directory is not
/// created here; writers create it on demand.
pub fn cache_dir() -> PathBuf {
    cache_dir_from(&non_empty_env, cfg!(windows))
}

/// Directory for what varde keeps for this user beyond the cache: the
/// deployment `varde run` uses when it is not run inside one.
///
/// Resolution order:
/// 1. `$VARDE_DATA_DIR`
/// 2. `$XDG_DATA_HOME/varde`
/// 3. `%LOCALAPPDATA%\varde\data` (Windows only)
/// 4. `$HOME/.local/share/varde`
/// 5. `./.varde-data` (last resort, as for the cache)
pub fn data_dir() -> PathBuf {
    data_dir_from(&non_empty_env, cfg!(windows))
}

/// Directory holding the `varde run` groups, one deployment each.
pub fn run_groups_dir() -> PathBuf {
    data_dir().join("run")
}

type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

fn cache_dir_from(env: Env, windows: bool) -> PathBuf {
    if let Some(dir) = env("VARDE_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = env("XDG_CACHE_HOME") {
        return PathBuf::from(dir).join("varde");
    }
    if let Some(dir) = windows.then(|| env("LOCALAPPDATA")).flatten() {
        return PathBuf::from(dir).join("varde").join("cache");
    }
    if let Some(home) = env("HOME") {
        return PathBuf::from(home).join(".cache").join("varde");
    }
    PathBuf::from(".chap-cache")
}

fn data_dir_from(env: Env, windows: bool) -> PathBuf {
    if let Some(dir) = env("VARDE_DATA_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = env("XDG_DATA_HOME") {
        return PathBuf::from(dir).join("varde");
    }
    if let Some(dir) = windows.then(|| env("LOCALAPPDATA")).flatten() {
        return PathBuf::from(dir).join("varde").join("data");
    }
    if let Some(home) = env("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("varde");
    }
    PathBuf::from(".varde-data")
}

fn non_empty_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
