//! Filesystem locations that are not part of a generated project.

use std::path::PathBuf;

/// Directory holding the cached registry snapshot.
///
/// Resolution order:
/// 1. `$CHAPS_CACHE_DIR`
/// 2. `$XDG_CACHE_HOME/chaps`
/// 3. `%LOCALAPPDATA%\chaps\cache` (Windows only)
/// 4. `$HOME/.cache/chaps`
/// 5. `./.chap-cache` (last resort, so the CLI still works in a bare container)
///
/// Empty environment variables are treated as unset. The directory is not
/// created here; writers create it on demand.
pub fn cache_dir() -> PathBuf {
    cache_dir_from(&non_empty_env, cfg!(windows))
}

/// Directory for what chaps keeps for this user beyond the cache: the
/// deployment `chaps run` uses when it is not run inside one.
///
/// Resolution order:
/// 1. `$CHAPS_DATA_DIR`
/// 2. `$XDG_DATA_HOME/chaps`
/// 3. `%LOCALAPPDATA%\chaps\data` (Windows only)
/// 4. `$HOME/.local/share/chaps`
/// 5. `./.chaps-data` (last resort, as for the cache)
pub fn data_dir() -> PathBuf {
    data_dir_from(&non_empty_env, cfg!(windows))
}

/// Directory holding the `chaps run` groups, one deployment each.
pub fn run_groups_dir() -> PathBuf {
    data_dir().join("run")
}

type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

fn cache_dir_from(env: Env, windows: bool) -> PathBuf {
    if let Some(dir) = env("CHAPS_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = env("XDG_CACHE_HOME") {
        return PathBuf::from(dir).join("chaps");
    }
    if let Some(dir) = windows.then(|| env("LOCALAPPDATA")).flatten() {
        return PathBuf::from(dir).join("chaps").join("cache");
    }
    if let Some(home) = env("HOME") {
        return PathBuf::from(home).join(".cache").join("chaps");
    }
    PathBuf::from(".chap-cache")
}

fn data_dir_from(env: Env, windows: bool) -> PathBuf {
    if let Some(dir) = env("CHAPS_DATA_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = env("XDG_DATA_HOME") {
        return PathBuf::from(dir).join("chaps");
    }
    if let Some(dir) = windows.then(|| env("LOCALAPPDATA")).flatten() {
        return PathBuf::from(dir).join("chaps").join("data");
    }
    if let Some(home) = env("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("chaps");
    }
    PathBuf::from(".chaps-data")
}

fn non_empty_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
