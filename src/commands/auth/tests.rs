use super::*;

/// What `varde auth show` prints: the tables, then the lines with their
/// level in front.
fn show_human(
    out: &Out,
    effective: &AuthState,
    recorded: AuthState,
    token: Option<&str>,
    reveal: bool,
    data_sources: &[(&str, Option<String>)],
) -> String {
    let mut lines = Report::default();
    show_summary(effective, recorded, data_sources, &mut lines);
    format!(
        "{}{}",
        show_tables(out, effective, token, reveal, data_sources),
        lines.text()
    )
}

fn on() -> AuthState {
    AuthState {
        api_token: true,
        registration_key: true,
    }
}

#[test]
fn show_says_off_and_what_to_do_about_it() {
    let text = show_human(
        &Out::default(),
        &AuthState::default(),
        AuthState::default(),
        None,
        false,
        &[],
    );
    assert!(text.contains("API authentication  off"), "{text}");
    assert!(text.contains("Registration key    off"), "{text}");
    assert!(text.contains("nothing protects this API"), "{text}");
    assert!(text.contains("hint: `varde auth enable`"), "{text}");
    assert!(!text.contains("API token"), "there is none: {text}");
}

#[test]
fn show_hides_the_token_until_reveal_asks_for_it() {
    let token = "0123456789abcdef0123456789abcdef";
    let masked = show_human(&Out::default(), &on(), on(), Some(token), false, &[]);
    assert!(
        masked.contains("API token           set in .env"),
        "{masked}"
    );
    assert!(
        !masked.contains("012345"),
        "part of the secret leaked: {masked}"
    );
    assert!(masked.contains("--reveal prints it"), "{masked}");
    assert!(masked.contains(MODELING_APP_HINT), "{masked}");
    assert!(masked.contains("X-Service-Key"), "{masked}");

    let revealed = show_human(&Out::default(), &on(), on(), Some(token), true, &[]);
    assert!(
        revealed.contains(&format!("API token           {token}")),
        "{revealed}"
    );
    assert!(!revealed.contains("--reveal"), "{revealed}");
}

#[test]
fn show_flags_a_state_file_that_disagrees_with_the_env() {
    // `.env` has both, `.varde/` remembers neither: a hand edit.
    let text = show_human(
        &Out::default(),
        &on(),
        AuthState::default(),
        Some("sekret-token-value"),
        false,
        &[],
    );
    assert!(
        text.contains("warning: `.varde/project.yaml` records"),
        "{text}"
    );
    assert!(text.contains("api_token: false"), "{text}");

    // Agreement says nothing at all.
    assert!(
        !show_human(
            &Out::default(),
            &on(),
            on(),
            Some("sekret-token-value"),
            false,
            &[]
        )
        .contains("warning:")
    );
}

#[test]
fn show_reports_one_secret_on_its_own() {
    let key_only = AuthState {
        api_token: false,
        registration_key: true,
    };
    let text = show_human(&Out::default(), &key_only, key_only, None, false, &[]);
    assert!(text.contains("API authentication  off"), "{text}");
    assert!(text.contains("Registration key    on"), "{text}");
    assert!(text.contains("X-Service-Key"), "{text}");
    assert!(!text.contains("nothing protects"), "{text}");
}

/// The OCS block is there whether or not the API is protected, because the
/// two have nothing to do with each other: an unprotected deployment can
/// still have a Copernicus key, and a protected one can still be missing
/// it.
#[test]
fn show_reports_whether_the_ocs_data_sources_are_set_and_never_their_values() {
    let key = "0123456789abcdef";
    let sources = vec![
        (
            "ECMWF_DATASTORES_URL",
            Some("https://cds.example/api".into()),
        ),
        ("ECMWF_DATASTORES_KEY", Some(key.to_string())),
        ("EDH_API_KEY", None),
        ("CDSE_S3_ACCESS_KEY", None),
        ("CDSE_S3_SECRET_KEY", None),
    ];
    for reveal in [false, true] {
        let text = show_human(&Out::default(), &on(), on(), Some("t"), reveal, &sources);
        assert!(text.contains("OCS data sources"), "{text}");
        assert!(text.contains("ECMWF_DATASTORES_KEY  set\n"), "{text}");
        assert!(
            !text.contains(key),
            "a third-party credential leaked at reveal={reveal}: {text}"
        );
        assert!(text.contains("EDH_API_KEY           unset"), "{text}");
        // The advice under the rows says which account a dataset needs, and
        // the two ERA5-Land accounts are not alternatives: a daily dataset
        // reads both. Picking one is not something to tell an operator.
        assert!(
            text.contains(
                "hint: ERA5-Land needs one or both of ECMWF_DATASTORES_* and EDH_API_KEY, \
                     per dataset; WorldPop and CHIRPS3 need none;"
            ),
            "{text}"
        );
        assert!(!text.contains("needs one of"), "{text}");
    }

    // Off does not skip it: the block is the answer to a different
    // question, and the off path returns early.
    let off = show_human(
        &Out::default(),
        &AuthState::default(),
        AuthState::default(),
        None,
        false,
        &sources,
    );
    assert!(off.contains("OCS data sources"), "{off}");

    // A deployment without the component has no block at all.
    let none = show_human(&Out::default(), &on(), on(), Some("t"), false, &[]);
    assert!(!none.contains("OCS data sources"), "{none}");
}

#[test]
fn the_written_files_are_hints() {
    let dir = Path::new("/p");
    let mut lines = Report::default();
    written_lines(dir, &crate::compose::SyncReport::default(), &mut lines);
    assert_eq!(lines.text(), "");
    let report = crate::compose::SyncReport {
        written: vec![std::path::PathBuf::from("/p/compose.x.yml")],
        ..Default::default()
    };
    written_lines(dir, &report, &mut lines);
    assert_eq!(lines.text(), "hint: wrote compose.x.yml\n");
}

#[test]
fn the_restart_hint_names_the_command_that_applies_it() {
    assert!(RESTART_HINT.contains("varde up"));
}
