//! Filesystem locations that are not part of a generated project.

use std::path::PathBuf;

/// Directory holding the cached registry snapshot.
///
/// Resolution order:
/// 1. `$CHAP_CLI_CACHE_DIR`
/// 2. `$XDG_CACHE_HOME/chap-cli`
/// 3. `$HOME/.cache/chap-cli`
/// 4. `./.chap-cache` (last resort, so the CLI still works in a bare container)
///
/// Empty environment variables are treated as unset. The directory is not
/// created here; writers create it on demand.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = non_empty_env("CHAP_CLI_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = non_empty_env("XDG_CACHE_HOME") {
        return PathBuf::from(dir).join("chap-cli");
    }
    if let Some(home) = non_empty_env("HOME") {
        return PathBuf::from(home).join(".cache").join("chap-cli");
    }
    PathBuf::from(".chap-cache")
}

fn non_empty_env(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_is_always_named() {
        // The env-dependent branches are exercised by the CLI tests, which
        // control the whole environment; here we only pin the invariant that
        // every branch yields a named directory.
        assert!(cache_dir().file_name().is_some());
    }
}
