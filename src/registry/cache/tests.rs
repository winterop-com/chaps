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
    let (got_index, got_models, age) = read(&opts).unwrap().expect("the entry was just written");

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
    backdate(&opts, Duration::from_hours(25));

    let (_, _, age) = read(&opts).unwrap().unwrap();
    assert!(age >= Duration::from_hours(25), "age was {age:?}");
    assert!(age < Duration::from_mins(1510));
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
