use crate::common::*;
use predicates::prelude::PredicateBooleanExt;
use serde_json::Value as Json;

/// Every line of a `.env` that assigns none of `vars`, commented or not.
///
/// What `varde auth` must leave byte for byte as it found it.
fn env_without<'a>(env: &'a str, vars: &[&str]) -> Vec<&'a str> {
    env.lines()
        .filter(|line| {
            let bare = line.trim_start().trim_start_matches('#').trim_start();
            !vars.iter().any(|var| bare.starts_with(&format!("{var}=")))
        })
        .collect()
}

/// A generated secret: 64 lowercase hex characters.
fn is_generated_secret(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
}

#[test]
fn init_with_an_api_token_writes_both_secrets_and_the_overlay_line() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    let out = sandbox
        .init(&["--models", "chapkit_ewars_model", "--api-token"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(out).expect("utf-8 stdout");

    let env = sandbox.env();
    let token = env_value(&env, "CHAP_API_TOKEN").expect("an active token line");
    let key = env_value(&env, "SERVICEKIT_REGISTRATION_KEY").expect("an active key line");
    assert!(is_generated_secret(token), "{token}");
    assert!(is_generated_secret(key), "{key}");
    assert_ne!(token, key, "two independent secrets");
    // The placeholders are gone, not duplicated below the active lines.
    assert!(!env.contains("# CHAP_API_TOKEN="), "{env}");
    assert!(!env.contains("# SERVICEKIT_REGISTRATION_KEY="), "{env}");

    // The summary shows no part of the token and says where to get it.
    assert!(
        stdout.contains("API token: generated into .env (varde auth show --reveal prints it)"),
        "{stdout}"
    );
    assert!(!stdout.contains(&token[..6]), "{stdout}");
    assert!(
        !stdout.contains(token),
        "the summary leaked the token:\n{stdout}"
    );

    // `.varde/` records booleans and no secret at all.
    let state = state(&dir);
    assert_eq!(state["auth"]["api_token"], true);
    assert_eq!(state["auth"]["registration_key"], true);
    let recorded = read(&dir.join(".varde/project.yaml"));
    assert!(!recorded.contains(token), "the state file holds a secret");
    assert!(!recorded.contains(key), "the state file holds a secret");

    // Every model overlay now hands the key to its service.
    assert!(overlay_sends_the_key(&dir));
    assert!(
        !read(&dir.join("compose.chapkit-ewars-model.yml")).contains(key),
        "the overlay must substitute the value from .env, not inline it"
    );

    // And a sync right after init has nothing left to do.
    chap_in(&sandbox, &dir, &["sync", "--check"])
        .assert()
        .success();
}

#[test]
fn an_explicit_api_token_is_used_verbatim_and_a_short_one_warns() {
    let sandbox = Sandbox::new();
    let given = "mysecrettoken-that-is-long-enough-32ch";
    sandbox
        .init(&["--models", "none", "--api-token", given])
        .assert()
        .success()
        .stderr(predicates::str::contains("chap-core warns below").not());

    let env = sandbox.env();
    assert_eq!(env_value(&env, "CHAP_API_TOKEN"), Some(given));
    // The registration key is generated either way: nobody types two secrets.
    let key = env_value(&env, "SERVICEKIT_REGISTRATION_KEY").expect("a key");
    assert!(is_generated_secret(key), "{key}");

    // A token chap-core would flag as weak is accepted with a warning.
    let short = Sandbox::new();
    short
        .init(&["--models", "none", "--api-token", "sekret"])
        .assert()
        .success()
        .stderr(predicates::str::contains("the API token is 6 characters"))
        .stderr(predicates::str::contains("below 32"));
    assert_eq!(
        env_value(&short.env(), "CHAP_API_TOKEN"),
        Some("sekret"),
        "the value is still used verbatim"
    );
}

#[test]
fn an_api_token_needs_an_env_file_to_live_in() {
    let sandbox = Sandbox::new();
    // clap rejects the pair outright: there would be nowhere to put it.
    sandbox
        .init(&["--models", "none", "--api-token", "--no-env"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot be used with"));

    // And a .env this run keeps is the operator's, so the flag says so rather
    // than silently doing nothing.
    sandbox.init(&["--models", "none"]).assert().success();
    let before = sandbox.env();
    sandbox
        .init(&["--models", "none", "--force", "--api-token"])
        .assert()
        .success()
        .stderr(predicates::str::contains("varde auth enable"));
    assert_eq!(sandbox.env(), before, "a kept .env is never rewritten");
    assert_eq!(state(&sandbox.project())["auth"]["api_token"], false);
}

#[test]
fn auth_show_hides_the_token_until_reveal_asks_for_it() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "none"]).assert().success();

    // Off to begin with.
    sandbox
        .auth(&["show"])
        .assert()
        .success()
        .stdout(predicates::str::contains("API authentication  off"))
        .stdout(predicates::str::contains("nothing protects this API"));

    sandbox.auth(&["enable"]).assert().success();
    let token = env_value(&sandbox.env(), "CHAP_API_TOKEN")
        .expect("a token")
        .to_string();

    let masked = sandbox
        .auth(&["show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let masked = String::from_utf8(masked).expect("utf-8");
    assert!(masked.contains("API authentication  on"), "{masked}");
    assert!(
        masked.contains("API token           set in .env"),
        "{masked}"
    );
    assert!(
        !masked.contains(&token[..6]),
        "part of the token leaked:\n{masked}"
    );
    assert!(!masked.contains(&token), "the token leaked:\n{masked}");

    sandbox
        .auth(&["show", "--reveal"])
        .assert()
        .success()
        .stdout(predicates::str::contains(token.clone()))
        .stdout(predicates::str::contains(
            "the DHIS2 `chap` route carries it once `varde dhis2 connect` has run",
        ));

    // `--json` follows the same rule: the secret only appears with --reveal.
    let out = sandbox
        .auth(&["show", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("auth show --json is JSON");
    assert_eq!(value["api_token"], true);
    assert_eq!(value["registration_key"], true);
    assert_eq!(value["token"], Json::Null);
    assert_eq!(value["token_masked"], Json::Null);
    assert!(!String::from_utf8_lossy(&out).contains(&token));

    let out = sandbox
        .auth(&["show", "--reveal", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: Json = serde_json::from_slice(&out).expect("auth show --json is JSON");
    assert_eq!(value["token"], token);
}

#[test]
fn auth_enable_protects_a_project_that_was_created_without_a_token() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model"])
        .assert()
        .success();
    let before = sandbox.env();
    assert!(!overlay_sends_the_key(&dir));

    sandbox
        .auth(&["enable"])
        .assert()
        .success()
        .stdout(predicates::str::contains("API authentication is on"))
        .stdout(predicates::str::contains(
            "run `varde up` to restart chap-core and the models with authentication",
        ))
        .stdout(predicates::str::contains(
            "the DHIS2 `chap` route carries it once `varde dhis2 connect` has run",
        ))
        .stdout(predicates::str::contains(
            "written  compose.chapkit-ewars-model.yml",
        ));

    let after = sandbox.env();
    let token = env_value(&after, "CHAP_API_TOKEN").expect("a token");
    let key = env_value(&after, "SERVICEKIT_REGISTRATION_KEY").expect("a key");
    assert!(is_generated_secret(token) && is_generated_secret(key));
    // The placeholders were uncommented in place: nothing else moved.
    assert_eq!(
        env_without(&after, &["CHAP_API_TOKEN", "SERVICEKIT_REGISTRATION_KEY"]),
        env_without(&before, &["CHAP_API_TOKEN", "SERVICEKIT_REGISTRATION_KEY"]),
    );

    assert_eq!(state(&dir)["auth"]["api_token"], true);
    assert!(overlay_sends_the_key(&dir));
    chap_in(&sandbox, &dir, &["sync", "--check"])
        .assert()
        .success();

    // Enabling again says so rather than replacing a working secret.
    sandbox
        .auth(&["enable"])
        .assert()
        .success()
        .stdout(predicates::str::contains("already on"))
        .stdout(predicates::str::contains("varde auth rotate"));
    assert_eq!(env_value(&sandbox.env(), "CHAP_API_TOKEN"), Some(token));

    // An explicit token is taken at the point authentication is turned on.
    let explicit = Sandbox::new();
    explicit.init(&["--models", "none"]).assert().success();
    explicit
        .auth(&["enable", "--token", "an-explicit-token-of-ample-length"])
        .assert()
        .success();
    assert_eq!(
        env_value(&explicit.env(), "CHAP_API_TOKEN"),
        Some("an-explicit-token-of-ample-length")
    );
}

#[test]
fn auth_disable_keeps_the_values_so_enable_recovers_them() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model", "--api-token"])
        .assert()
        .success();
    let protected = sandbox.env();
    let token = env_value(&protected, "CHAP_API_TOKEN").unwrap().to_string();
    let key = env_value(&protected, "SERVICEKIT_REGISTRATION_KEY")
        .unwrap()
        .to_string();

    sandbox
        .auth(&["disable"])
        .assert()
        .success()
        .stdout(predicates::str::contains("API authentication is off"))
        .stdout(predicates::str::contains("kept as comments"))
        .stdout(predicates::str::contains("run `varde up`"));

    let off = sandbox.env();
    assert_eq!(env_value(&off, "CHAP_API_TOKEN"), None);
    assert_eq!(env_value(&off, "SERVICEKIT_REGISTRATION_KEY"), None);
    // The values are still there, behind a `#`.
    assert!(off.contains(&format!("# CHAP_API_TOKEN={token}")), "{off}");
    assert!(
        off.contains(&format!("# SERVICEKIT_REGISTRATION_KEY={key}")),
        "{off}"
    );
    assert_eq!(state(&dir)["auth"]["api_token"], false);
    assert!(!overlay_sends_the_key(&dir));

    // Disabling twice changes nothing more.
    sandbox
        .auth(&["disable"])
        .assert()
        .success()
        .stdout(predicates::str::contains("already off"));
    assert_eq!(sandbox.env(), off);

    // And enabling again picks the old secrets back up, so clients that were
    // never reconfigured keep working.
    sandbox.auth(&["enable"]).assert().success();
    let again = sandbox.env();
    assert_eq!(env_value(&again, "CHAP_API_TOKEN"), Some(token.as_str()));
    assert_eq!(
        env_value(&again, "SERVICEKIT_REGISTRATION_KEY"),
        Some(key.as_str())
    );
    assert_eq!(again, protected, "the round trip is byte for byte");
    assert!(overlay_sends_the_key(&dir));
    assert_eq!(state(&dir)["auth"]["registration_key"], true);
}

#[test]
fn auth_rotate_replaces_both_secrets_and_leaves_the_rest_of_env_alone() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "chapkit_ewars_model", "--api-token"])
        .assert()
        .success();
    let before = sandbox.env();
    let old_token = env_value(&before, "CHAP_API_TOKEN").unwrap().to_string();
    let old_key = env_value(&before, "SERVICEKIT_REGISTRATION_KEY")
        .unwrap()
        .to_string();
    let password = password_line(&before).to_string();

    sandbox
        .auth(&["rotate"])
        .assert()
        .success()
        .stdout(predicates::str::contains("API authentication rotated"))
        .stdout(predicates::str::contains(
            "run `varde up` to restart chap-core and the models with authentication",
        ))
        .stdout(predicates::str::contains(
            "every client keeps sending the old token until it is updated",
        ));

    let after = sandbox.env();
    let token = env_value(&after, "CHAP_API_TOKEN").unwrap();
    let key = env_value(&after, "SERVICEKIT_REGISTRATION_KEY").unwrap();
    assert_ne!(token, old_token, "the token did not move");
    assert_ne!(key, old_key, "the registration key did not move");
    assert!(is_generated_secret(token) && is_generated_secret(key));

    // Only those two lines changed: the database password, the pins, the
    // comments and the blank lines are all exactly as they were.
    assert_eq!(
        env_without(&after, &["CHAP_API_TOKEN", "SERVICEKIT_REGISTRATION_KEY"]),
        env_without(&before, &["CHAP_API_TOKEN", "SERVICEKIT_REGISTRATION_KEY"]),
    );
    assert_eq!(password_line(&after), password);
    assert_eq!(after.lines().count(), before.lines().count());
    assert!(
        !after.contains(&old_token),
        "the old token is still in the file"
    );

    // Rotating an unprotected project turns authentication on.
    let fresh = Sandbox::new();
    fresh.init(&["--models", "none"]).assert().success();
    fresh.auth(&["rotate"]).assert().success();
    assert_eq!(state(&fresh.project())["auth"]["api_token"], true);

    // The compose files stay in sync with the new state.
    chap_in(&sandbox, &dir, &["sync", "--check"])
        .assert()
        .success();
}

#[test]
fn auth_outside_a_project_says_so() {
    let sandbox = Sandbox::new();
    for argv in [
        ["auth", "show"].as_slice(),
        ["auth", "enable"].as_slice(),
        ["auth", "disable"].as_slice(),
        ["auth", "rotate"].as_slice(),
    ] {
        chap_in(&sandbox, sandbox.home.path(), argv)
            .assert()
            .failure()
            .stderr(predicates::str::contains("not a varde project"));
    }

    // A project written with --no-env has no file to keep a secret in, and
    // says which flag made it that way.
    sandbox
        .init(&["--models", "none", "--no-env"])
        .assert()
        .success();
    sandbox
        .auth(&["enable"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--no-env"));
}

#[test]
fn no_color_is_accepted_everywhere_and_changes_nothing_off_a_terminal() {
    let sandbox = Sandbox::new();
    let plain = sandbox
        .chap()
        .args(["models", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let asked = sandbox
        .chap()
        .args(["--no-color", "models", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(plain).expect("utf-8"),
        String::from_utf8(asked).expect("utf-8"),
        "piped output is plain either way"
    );

    // It is global, so it also attaches after the subcommand.
    sandbox
        .chap()
        .args(["models", "list", "--no-color"])
        .assert()
        .success();
}

/// The block is about third-party accounts rather than this deployment's own
/// secret, so it is masked whatever `--reveal` asked for.
#[test]
fn auth_show_lists_the_ocs_data_sources_as_set_or_unset() {
    let sandbox = Sandbox::new();
    let dir = sandbox.project();
    sandbox
        .init(&["--models", "none", "--with", "ocs"])
        .assert()
        .success();

    let shown = sandbox.auth(&["show"]).assert().success();
    let text = String::from_utf8_lossy(&shown.get_output().stdout).into_owned();
    assert!(text.contains("OCS data sources"), "{text}");
    assert!(text.contains("ECMWF_DATASTORES_KEY"), "{text}");
    assert!(text.contains("unset"), "{text}");

    // Fill two in, exactly as an operator would.
    let env = dir.join(".env");
    let body = read(&env)
        .replace("# ECMWF_DATASTORES_URL=", "ECMWF_DATASTORES_URL=")
        .replace(
            "# ECMWF_DATASTORES_KEY=",
            "ECMWF_DATASTORES_KEY=0123456789abcdef",
        );
    std::fs::write(&env, body).unwrap();

    for args in [vec!["show"], vec!["show", "--reveal"]] {
        let shown = sandbox.auth(&args).assert().success();
        let text = String::from_utf8_lossy(&shown.get_output().stdout).into_owned();
        assert!(
            text.contains("ECMWF_DATASTORES_KEY  set\n"),
            "{args:?}: {text}"
        );
        assert!(
            !text.contains("0123456789abcdef"),
            "a third-party credential leaked with {args:?}: {text}"
        );
        assert!(text.contains("EDH_API_KEY           unset"), "{text}");
    }

    let value = json_of(&mut sandbox.auth(&["show", "--json"]));
    let sources = value["ocs_data_sources"].as_array().unwrap();
    assert_eq!(sources.len(), 5);
    assert_eq!(
        sources[1]["variable"].as_str(),
        Some("ECMWF_DATASTORES_KEY")
    );
    assert_eq!(sources[1]["set"], serde_json::json!(true));
    assert_eq!(sources[2]["set"], serde_json::json!(false));

    // A deployment without the component has no block at all.
    sandbox.components(&["disable", "ocs"]).assert().success();
    assert!(
        json_of(&mut sandbox.auth(&["show", "--json"]))["ocs_data_sources"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn auth_token_prints_the_token_and_nothing_else() {
    let sandbox = Sandbox::new();
    sandbox.init(&["--models", "none"]).assert().success();

    // Authentication is off to begin with: stdout stays empty, the sentence
    // is on stderr, and the exit code stops a script.
    let assert = chap_in(&sandbox, &sandbox.project(), &["auth", "token"])
        .assert()
        .failure()
        .code(1);
    let out = assert.get_output();
    assert!(out.stdout.is_empty(), "stdout has to stay empty");
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("API authentication is off in this deployment; run `varde auth enable`"),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );

    sandbox.auth(&["enable"]).assert().success();
    let token = env_value(&sandbox.env(), "CHAP_API_TOKEN")
        .expect("a token")
        .to_string();

    // On, and stdout is the token with one newline after it: nothing a
    // `$(...)` would have to strip.
    let out = chap_in(&sandbox, &sandbox.project(), &["auth", "token"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(String::from_utf8(out).expect("text"), format!("{token}\n"));

    // `--json` says the same thing as a document, either way.
    let value = json_of(&mut chap_in(
        &sandbox,
        &sandbox.project(),
        &["--json", "auth", "token"],
    ));
    assert_eq!(value["token"], token);

    sandbox.auth(&["disable"]).assert().success();
    let value = json_of(&mut chap_in(
        &sandbox,
        &sandbox.project(),
        &["--json", "auth", "token"],
    ));
    assert_eq!(value["token"], Json::Null);

    // And outside a deployment it says which one it could not find.
    chap_in(&sandbox, sandbox.home.path(), &["auth", "token"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not a varde project"));
}
