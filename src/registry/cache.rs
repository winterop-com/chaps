//! On-disk cache of the fetched marketplace snapshot.
//!
//! Layout under [`RegistryOptions::cache_dir`](crate::registry::RegistryOptions):
//!
//! ```text
//! registries/<key>/registry.yaml
//! registries/<key>/models/<id>.yaml
//! registries/<key>/meta.json
//! ```
//!
//! The model files keep the same relative paths the index lists, so
//! `registries/<key>/` is interchangeable with the vendored snapshot. `<key>`
//! is derived from the registry URL ([`cache_key`]), so a private marketplace
//! and the public one never share an entry, and `meta.json` records the URL
//! and the fetch time the age is measured from.
//!
//! Every read failure is non-fatal: a missing, truncated or hand-edited cache
//! is reported as "no cache" plus a warning, never as an error, because the
//! caller always has the network and the embedded snapshot to fall back on.

use crate::error::Result;
use crate::registry::{RegistryIndex, RegistryOptions};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Directory under the cache root holding one entry per registry URL.
const REGISTRIES_DIR: &str = "registries";
/// The cached index, under the entry directory.
const INDEX_FILE: &str = "registry.yaml";
/// Bookkeeping for the entry, under the entry directory.
const META_FILE: &str = "meta.json";

/// A cached snapshot: `(index_yaml, model_files, age of the cache entry)`.
///
/// The alias only exists to keep clippy's type-complexity lint quiet; the
/// shape is exactly the tuple it names.
pub type CachedSnapshot = (String, Vec<(String, String)>, Duration);

/// `meta.json`: which URL the entry came from, and when.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Meta {
    url: String,
    /// Seconds since the Unix epoch at the time of the fetch.
    fetched_at_unix: u64,
}

/// Directory holding the cache entry for `opts.url`.
pub fn dir_for(opts: &RegistryOptions) -> PathBuf {
    opts.cache_dir
        .join(REGISTRIES_DIR)
        .join(cache_key(&opts.url))
}

/// A filesystem-safe directory name for a registry URL.
///
/// The readable part is the URL with every non-alphanumeric run collapsed to a
/// single `_` and truncated, so a cache directory can be understood at a
/// glance; the trailing hash is what actually makes it unique, since
/// truncation and collapsing both lose information.
pub fn cache_key(url: &str) -> String {
    let mut slug = String::with_capacity(url.len());
    for ch in url.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_');
    // Byte truncation is safe: every character above is ASCII.
    let slug = &slug[..slug.len().min(64)];
    format!("{slug}-{:016x}", fnv1a(url.as_bytes()))
}

/// FNV-1a, 64-bit. A non-cryptographic hash is enough here: it only has to
/// separate two URLs that slugify the same, and pulling in a hash crate for
/// that would not pay for itself.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Read the cached snapshot for `opts.url`.
///
/// Returns `Ok(None)` when there is no usable cache, including when the entry
/// exists but is unreadable, corrupt or incomplete; those cases warn on
/// stderr so a permanently broken cache is not silent.
pub fn read(opts: &RegistryOptions) -> Result<Option<CachedSnapshot>> {
    let dir = dir_for(opts);

    let meta_body = match std::fs::read_to_string(dir.join(META_FILE)) {
        Ok(body) => body,
        // A missing entry is the normal cold-start case, not a problem.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Ok(unusable(&dir, format!("{META_FILE} is unreadable: {e}"))),
    };
    let meta: Meta = match serde_json::from_str(&meta_body) {
        Ok(meta) => meta,
        Err(e) => return Ok(unusable(&dir, format!("{META_FILE} is corrupt: {e}"))),
    };
    if meta.url != opts.url {
        return Ok(unusable(
            &dir,
            format!("it holds {} but {} was asked for", meta.url, opts.url),
        ));
    }

    let index_yaml = match std::fs::read_to_string(dir.join(INDEX_FILE)) {
        Ok(body) => body,
        Err(e) => return Ok(unusable(&dir, format!("{INDEX_FILE} is unreadable: {e}"))),
    };
    let index: RegistryIndex = match serde_yaml_ng::from_str(&index_yaml) {
        Ok(index) => index,
        Err(e) => return Ok(unusable(&dir, format!("{INDEX_FILE} is corrupt: {e}"))),
    };

    let mut model_files = Vec::with_capacity(index.models.len());
    for rel in &index.models {
        let path = match entry_path(&dir, rel) {
            Ok(path) => path,
            Err(reason) => return Ok(unusable(&dir, reason)),
        };
        match std::fs::read_to_string(&path) {
            Ok(body) => model_files.push((rel.clone(), body)),
            Err(e) => return Ok(unusable(&dir, format!("{rel} is unreadable: {e}"))),
        }
    }

    Ok(Some((
        index_yaml,
        model_files,
        age_of(meta.fetched_at_unix),
    )))
}

/// Write a snapshot to the cache entry for `opts.url`, creating it if needed.
///
/// Each file is written to a sibling temporary file and renamed, so a crash
/// or a full disk leaves the previous entry intact rather than a half-written
/// one. `meta.json` is written last, which is what makes the entry visible to
/// [`read`]: until it lands the entry reads as absent.
pub fn write(
    opts: &RegistryOptions,
    index_yaml: &str,
    model_files: &[(String, String)],
) -> Result<()> {
    let dir = dir_for(opts);
    std::fs::create_dir_all(&dir)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", dir.display()))?;

    write_atomic(&dir.join(INDEX_FILE), index_yaml)?;
    for (rel, body) in model_files {
        let path = entry_path(&dir, rel).map_err(|reason| anyhow::anyhow!("{reason}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("creating {}: {e}", parent.display()))?;
        }
        write_atomic(&path, body)?;
    }

    let meta = Meta {
        url: opts.url.clone(),
        fetched_at_unix: now_unix(),
    };
    let mut body = serde_json::to_string_pretty(&meta)?;
    body.push('\n');
    write_atomic(&dir.join(META_FILE), &body)
}

/// Resolve a path the index listed against the cache entry directory.
///
/// The index is fetched over the network, so its paths are untrusted input:
/// anything absolute or containing `..` could write outside the cache, and is
/// rejected instead.
fn entry_path(dir: &Path, rel: &str) -> std::result::Result<PathBuf, String> {
    let rejected = rel.is_empty()
        || rel.starts_with('/')
        || rel.starts_with('\\')
        || rel
            .split(['/', '\\'])
            .any(|part| part == ".." || part.is_empty());
    if rejected {
        return Err(format!("{rel} is not a valid path inside the registry"));
    }
    Ok(dir.join(rel))
}

fn write_atomic(path: &Path, body: &str) -> Result<()> {
    // A hidden sibling, so the temporary file can never collide with a real
    // entry file and is obviously not part of the snapshot if one is left.
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, body).map_err(|e| anyhow::anyhow!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .map_err(|e| anyhow::anyhow!("renaming {} to {}: {e}", tmp.display(), path.display()))
}

/// Report an unusable entry and treat it as absent.
fn unusable(dir: &Path, reason: String) -> Option<CachedSnapshot> {
    crate::output::warn(&format!(
        "ignoring the registry cache in {}: {reason}",
        dir.display()
    ));
    None
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Age of an entry fetched at `fetched_at_unix`.
///
/// A timestamp in the future (a clock that moved backwards) reads as age zero
/// rather than as an enormous number, which would make a good cache look
/// stale.
fn age_of(fetched_at_unix: u64) -> Duration {
    Duration::from_secs(now_unix().saturating_sub(fetched_at_unix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::embedded;

    fn opts(dir: &Path) -> RegistryOptions {
        RegistryOptions {
            url: "https://example.test/marketplace/registry.yaml".to_string(),
            cache_dir: dir.to_path_buf(),
            ..RegistryOptions::default()
        }
    }

    fn snapshot() -> (String, Vec<(String, String)>) {
        (embedded::index_yaml().to_string(), embedded::model_files())
    }

    /// Backdate an existing entry so age-dependent behaviour is testable.
    fn backdate(opts: &RegistryOptions, age: Duration) {
        let path = dir_for(opts).join(META_FILE);
        let meta = Meta {
            url: opts.url.clone(),
            fetched_at_unix: now_unix() - age.as_secs(),
        };
        std::fs::write(path, serde_json::to_string(&meta).unwrap()).unwrap();
    }

    #[test]
    fn write_then_read_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = opts(tmp.path());
        let (index, models) = snapshot();

        write(&opts, &index, &models).unwrap();
        let (got_index, got_models, age) =
            read(&opts).unwrap().expect("the entry was just written");

        assert_eq!(got_index, index);
        assert_eq!(got_models, models);
        assert!(age < Duration::from_secs(60), "a fresh entry has no age");

        // The layout is the documented one, and interchangeable with vendor/.
        let dir = dir_for(&opts);
        assert!(dir.starts_with(tmp.path().join(REGISTRIES_DIR)));
        assert!(dir.join("registry.yaml").is_file());
        assert!(dir.join("models/chapkit_ewars_model.yaml").is_file());
        assert!(dir.join("meta.json").is_file());
    }

    #[test]
    fn write_leaves_no_temporary_files_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = opts(tmp.path());
        let (index, models) = snapshot();
        write(&opts, &index, &models).unwrap();

        let dir = dir_for(&opts);
        for entry in std::fs::read_dir(dir.join("models")).unwrap() {
            let name = entry.unwrap().file_name();
            assert!(
                !name.to_string_lossy().ends_with(".tmp"),
                "{name:?} was not renamed"
            );
        }
    }

    #[test]
    fn read_reports_the_recorded_age() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = opts(tmp.path());
        let (index, models) = snapshot();
        write(&opts, &index, &models).unwrap();
        backdate(&opts, Duration::from_secs(90_000));

        let (_, _, age) = read(&opts).unwrap().unwrap();
        assert!(age >= Duration::from_secs(90_000), "age was {age:?}");
        assert!(age < Duration::from_secs(90_600));
    }

    #[test]
    fn a_missing_entry_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read(&opts(tmp.path())).unwrap().is_none());
    }

    #[test]
    fn corrupt_meta_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = opts(tmp.path());
        let (index, models) = snapshot();
        write(&opts, &index, &models).unwrap();
        std::fs::write(dir_for(&opts).join(META_FILE), "{ not json").unwrap();

        assert!(read(&opts).unwrap().is_none());
    }

    #[test]
    fn a_corrupt_index_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = opts(tmp.path());
        let (index, models) = snapshot();
        write(&opts, &index, &models).unwrap();
        std::fs::write(dir_for(&opts).join(INDEX_FILE), "just: a: mapping: no").unwrap();

        assert!(read(&opts).unwrap().is_none());
    }

    #[test]
    fn a_missing_model_file_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = opts(tmp.path());
        let (index, models) = snapshot();
        write(&opts, &index, &models).unwrap();
        std::fs::remove_file(dir_for(&opts).join("models/chapkit_ewars_model.yaml")).unwrap();

        assert!(read(&opts).unwrap().is_none());
    }

    #[test]
    fn an_entry_written_for_another_url_is_not_reused() {
        let tmp = tempfile::tempdir().unwrap();
        let mine = opts(tmp.path());
        let (index, models) = snapshot();
        write(&mine, &index, &models).unwrap();

        // Same directory, different URL: the key differs, so nothing is found.
        let theirs = RegistryOptions {
            url: "https://other.test/registry.yaml".to_string(),
            ..mine.clone()
        };
        assert!(read(&theirs).unwrap().is_none());
        assert!(read(&mine).unwrap().is_some());
    }

    #[test]
    fn cache_key_is_filesystem_safe_and_url_specific() {
        let key = cache_key("https://raw.githubusercontent.com/dhis2-chap/x/main/registry.yaml");
        assert!(
            key.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "{key}"
        );
        assert!(!key.starts_with('_') && !key.starts_with('-'), "{key}");
        assert!(key.len() <= 81, "{key} is {} chars", key.len());
        assert!(key.starts_with("https_raw_githubusercontent_com"), "{key}");

        assert_eq!(
            key,
            cache_key("https://raw.githubusercontent.com/dhis2-chap/x/main/registry.yaml")
        );
        assert_ne!(
            key,
            cache_key("https://raw.githubusercontent.com/dhis2-chap/y/main/registry.yaml")
        );
        // Two URLs that slugify identically still get separate entries.
        assert_ne!(
            cache_key("https://a.test/r?x=1"),
            cache_key("https://a.test/r?x/1")
        );
    }

    #[test]
    fn entry_paths_cannot_escape_the_cache_directory() {
        let dir = Path::new("/cache/entry");
        assert_eq!(
            entry_path(dir, "models/a.yaml").unwrap(),
            dir.join("models/a.yaml")
        );
        for bad in ["../evil.yaml", "models/../../evil.yaml", "/etc/passwd", ""] {
            assert!(entry_path(dir, bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn write_rejects_a_traversing_model_path() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = opts(tmp.path());
        let err = write(
            &opts,
            embedded::index_yaml(),
            &[("../escape.yaml".to_string(), "x".to_string())],
        )
        .expect_err("traversal is rejected");
        assert!(err.to_string().contains("escape.yaml"));
        assert!(!tmp.path().join("escape.yaml").exists());
    }
}
