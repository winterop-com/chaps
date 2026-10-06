use super::*;

#[test]
fn every_vendored_model_is_in_the_table() {
    let registry = crate::registry::load_embedded().unwrap();
    for m in &registry.models {
        assert!(
            known_override(&m.id).is_some(),
            "{} has no entry; check its Dockerfile before enabling it",
            m.id
        );
    }
    assert_eq!(KNOWN.len(), registry.models.len());
}

/// The table is the last resort, so it has to land where reading the
/// image config would have. Each pair below was read off the published
/// image the `stable` channel pins, anonymously from ghcr: a pull token,
/// the OCI index, the amd64 manifest inside it and the config blob it
/// names - `config.User` and `config.WorkingDir` verbatim. `docker image
/// inspect` of the same tags agrees, except for the simple multistep
/// model, whose local copy on the development machine carries an empty
/// config; ghcr is what is quoted here.
#[test]
fn the_table_matches_what_the_marketplace_images_declare() {
    // (id, WorkingDir, User) as ghcr serves them today.
    let declared = [
        ("chapkit_ewars_model", "/app", "chapkit"),
        ("chapkit_simple_multistep_model", "/app", "chap"),
        ("chapkit_rwanda_malaria_bym_model", "/work", "chapkit"),
        ("auto_arima_chapkit", "/work", "root"),
        ("chapkit_ghr_model", "/work", "app"),
        ("chapkit_minimalist_example_py", "/work", "root"),
        ("chapkit_minimalist_example_r", "/work", "root"),
    ];
    for (id, working_dir, user) in declared {
        let found = known_override(id).unwrap_or_else(|| panic!("no entry for {id}"));
        assert_eq!(
            found.data_dir,
            format!("{working_dir}/data"),
            "{id} writes under its WorkingDir"
        );
        assert_eq!(found.user, user, "{id} user");
    }
}

/// The three images that end on `USER root` get no `user:` line; the four
/// that drop to an account of their own do. All seven get an init
/// container.
#[test]
fn the_table_says_which_images_run_as_root() {
    let root: Vec<&str> = KNOWN
        .iter()
        .filter(|(_, o)| is_root(o.user))
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(
        root,
        [
            "auto_arima_chapkit",
            "chapkit_minimalist_example_py",
            "chapkit_minimalist_example_r",
        ]
    );
}

#[test]
fn an_unknown_id_falls_back_to_the_defaults() {
    assert_eq!(known_override("something_else"), None);
    assert_eq!(DEFAULT_DATA_DIR, "/work/data");
    assert_eq!(DEFAULT_USER, "chapkit:chapkit");
}

#[test]
fn numeric_user_resolves_the_names_the_images_create() {
    // busybox has no chapkit account, so the init container needs numbers.
    assert_eq!(
        numeric_user("chapkit:chapkit").as_deref(),
        Some("1000:1000")
    );
    // `chap` is 1001, not 1000: useradd skipped the uid chapkit holds.
    assert_eq!(numeric_user("chap:chap").as_deref(), Some("1001:1001"));
    assert_eq!(numeric_user("root:root").as_deref(), Some("0:0"));
    assert_eq!(numeric_user("chapkit").as_deref(), Some("1000"));
    assert_eq!(numeric_user("chapkit:chap").as_deref(), Some("1000:1001"));
}

#[test]
fn numeric_user_passes_numbers_through() {
    assert_eq!(numeric_user("1000:1000").as_deref(), Some("1000:1000"));
    assert_eq!(numeric_user("1001:2002").as_deref(), Some("1001:2002"));
    assert_eq!(numeric_user("0:0").as_deref(), Some("0:0"));
    assert_eq!(numeric_user("1000").as_deref(), Some("1000"));
    // A mixed pair resolves either side.
    assert_eq!(numeric_user("1000:chapkit").as_deref(), Some("1000:1000"));
}

#[test]
fn numeric_user_gives_up_on_a_name_it_does_not_know() {
    for user in ["nobody", "chapkit:nobody", "nobody:1000", "", "a:b:c", "-1"] {
        assert_eq!(numeric_user(user), None, "{user:?}");
    }
    assert_eq!(FALLBACK_UID_GID, "1000:1000");
}

/// The init container chowns from busybox, so a table user that cannot be
/// turned into numbers is a `chown` that fails. GHRmodel is the one
/// deliberate exception: its `app` is nobody's convention, so it stays out
/// of `KNOWN_IDS` and is asked of the image instead. Enabling it reaches
/// the registry or the local daemon and records `10001:10001`; only a run
/// that can reach neither lands on this row, and `varde sync` warns there.
#[test]
fn every_user_in_the_table_is_resolvable() {
    const PROBED: &[&str] = &["chapkit_ghr_model"];
    for (id, o) in KNOWN {
        if PROBED.contains(id) {
            assert_eq!(
                numeric_pair(o.user),
                None,
                "{id} would no longer be probed for its uid"
            );
            continue;
        }
        assert!(
            numeric_pair(o.user).is_some(),
            "{id} runs as {} which the init container cannot chown to",
            o.user
        );
    }
}

#[test]
fn numeric_pair_gives_a_bare_account_name_its_group() {
    // What an image config says - `USER chapkit` - has to render the same
    // pair as a hand-written `chapkit:chapkit`.
    assert_eq!(numeric_pair("chapkit").as_deref(), Some("1000:1000"));
    assert_eq!(
        numeric_pair("chapkit:chapkit").as_deref(),
        Some("1000:1000")
    );
    assert_eq!(numeric_pair("chap").as_deref(), Some("1001:1001"));
    assert_eq!(numeric_pair("root").as_deref(), Some("0:0"));
    // A chown always gets a pair for root, whatever spelling it arrived
    // in, because root's group is not a guess.
    for spelling in ["root", "root:root", "0", "0:0", ""] {
        assert_eq!(chown_pair(spelling), "0:0", "{spelling:?}");
    }
    assert_eq!(chown_pair("chapkit"), "1000:1000");
    assert_eq!(chown_pair("1500"), "1500", "nothing knows uid 1500's group");
    assert_eq!(chown_pair("1500:1600"), "1500:1600");
    assert_eq!(chown_pair("nobody-in-particular"), FALLBACK_UID_GID);
    // A bare number names no group this CLI can look up, so it is left
    // exactly as it was given.
    assert_eq!(numeric_pair("1500").as_deref(), Some("1500"));
    assert_eq!(numeric_pair("1500:1600").as_deref(), Some("1500:1600"));
    assert_eq!(numeric_pair("nobody"), None);
    assert_eq!(numeric_pair("chapkit:nobody"), None);
}

#[test]
fn root_is_recognised_however_it_is_written() {
    for user in ["root", "root:root", "0", "0:0", "0:root", " root ", ""] {
        assert!(is_root(user), "{user:?}");
    }
    for user in ["chapkit", "chapkit:chapkit", "1000:1000", "nobody"] {
        assert!(!is_root(user), "{user:?}");
    }
}
