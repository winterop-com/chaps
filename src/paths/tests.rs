use super::*;

#[test]
fn cache_dir_is_always_named() {
    // The env-dependent branches are exercised by the CLI tests, which
    // control the whole environment; here we only pin the invariant that
    // every branch yields a named directory.
    assert!(cache_dir().file_name().is_some());
}

fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |key| {
        pairs
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
    }
}

#[test]
fn windows_uses_local_app_data_before_home() {
    let env = env_of(&[("LOCALAPPDATA", "appdata"), ("HOME", "home")]);
    assert_eq!(
        cache_dir_from(&env, true),
        PathBuf::from("appdata").join("chaps").join("cache")
    );
    assert_eq!(
        data_dir_from(&env, true),
        PathBuf::from("appdata").join("chaps").join("data")
    );
}

#[test]
fn local_app_data_is_ignored_off_windows() {
    let env = env_of(&[("LOCALAPPDATA", "appdata"), ("HOME", "home")]);
    assert_eq!(
        cache_dir_from(&env, false),
        PathBuf::from("home").join(".cache").join("chaps")
    );
    assert_eq!(
        data_dir_from(&env, false),
        PathBuf::from("home")
            .join(".local")
            .join("share")
            .join("chaps")
    );
}

#[test]
fn xdg_and_the_chaps_overrides_win_on_windows_too() {
    let env = env_of(&[("XDG_DATA_HOME", "xdg"), ("LOCALAPPDATA", "appdata")]);
    assert_eq!(
        data_dir_from(&env, true),
        PathBuf::from("xdg").join("chaps")
    );
    let env = env_of(&[("CHAPS_CACHE_DIR", "mine"), ("LOCALAPPDATA", "appdata")]);
    assert_eq!(cache_dir_from(&env, true), PathBuf::from("mine"));
}
