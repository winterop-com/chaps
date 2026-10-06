use super::*;

/// A `.env` shaped like the generated one: comments, an active line, and
/// commented placeholders for both secrets.
const ENV: &str = "\
# Written once by varde init; varde never rewrites it. See the docs for each setting.
POSTGRES_PASSWORD=0123456789abcdef
CHAP_API_PORT=8000

# API authentication. Commented out means none.
# CHAP_API_TOKEN=

# Shared secret for chapkit service registration.
# SERVICEKIT_REGISTRATION_KEY=

# CHAPKIT_EWARS_MODEL_IMAGE_TAG=sha-fa880a1
";

#[test]
fn a_generated_secret_is_sixty_four_lowercase_hex_characters() {
    let secret = random_secret().unwrap();
    assert_eq!(secret.len(), 64);
    assert!(
        secret
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
        "{secret}"
    );
    assert_ne!(secret, random_secret().unwrap());
    // Long enough that chap-core will not call it weak.
    assert!(secret.len() >= MIN_TOKEN_LENGTH);
}

#[test]
fn random_hex_is_two_characters_per_byte() {
    assert_eq!(random_hex(16).unwrap().len(), 32);
    assert_eq!(random_hex(1).unwrap().len(), 2);
    assert_eq!(random_hex(0).unwrap(), "");
}

#[test]
fn the_weak_token_warning_names_both_lengths() {
    let text = weak_token_warning(12);
    assert!(text.contains("12 characters"), "{text}");
    assert!(text.contains("below 32"), "{text}");
}

#[test]
fn an_active_value_is_read_and_a_commented_or_empty_one_is_not() {
    assert_eq!(
        active_value("CHAP_API_TOKEN=abc123\n", API_TOKEN_ENV_VAR).as_deref(),
        Some("abc123")
    );
    // Whitespace around the value belongs to the file, not the secret.
    assert_eq!(
        active_value("  CHAP_API_TOKEN=  abc123  \n", API_TOKEN_ENV_VAR).as_deref(),
        Some("abc123")
    );
    // A commented placeholder and an empty assignment both mean "unset",
    // the way chap-core reads them.
    assert_eq!(active_value(ENV, API_TOKEN_ENV_VAR), None);
    assert_eq!(active_value("CHAP_API_TOKEN=\n", API_TOKEN_ENV_VAR), None);
    assert_eq!(active_value("", API_TOKEN_ENV_VAR), None);
    // A variable whose name merely ends in ours is a different variable.
    assert_eq!(
        active_value("X_CHAP_API_TOKEN=abc\n", API_TOKEN_ENV_VAR),
        None
    );
    // The last usable value wins, the way compose reads the file.
    assert_eq!(
        active_value(
            "CHAP_API_TOKEN=one\nCHAP_API_TOKEN=two\n",
            API_TOKEN_ENV_VAR
        )
        .as_deref(),
        Some("two")
    );
    // Including the shapes compose accepts and this file used not to.
    assert_eq!(
        active_value("export CHAP_API_TOKEN=\"abc\"\n", API_TOKEN_ENV_VAR).as_deref(),
        Some("abc")
    );
}

/// The rotation bug: `rotate` reported success while compose went on
/// reading a second, untouched `CHAP_API_TOKEN=` line further down.
#[test]
fn rotating_over_a_duplicated_line_leaves_one_active_assignment() {
    let body = "CHAP_API_TOKEN=old\n# a comment\nCHAP_API_TOKEN=also-old\nPOSTGRES_DB=chap_core\n";
    let out = write_secrets(body, &[(API_TOKEN_ENV_VAR, "new")]);
    assert_eq!(
        out,
        "CHAP_API_TOKEN=new\n# a comment\nPOSTGRES_DB=chap_core\n"
    );
    assert_eq!(
        active_value(&out, API_TOKEN_ENV_VAR).as_deref(),
        Some("new")
    );
    assert_eq!(
        out.lines()
            .filter(|line| line.starts_with("CHAP_API_TOKEN="))
            .count(),
        1
    );
    // And nothing of the old token is left anywhere in the file.
    assert!(!out.contains("old"), "{out}");
}

#[test]
fn the_state_follows_the_active_lines() {
    assert_eq!(state_of(ENV), AuthState::default());
    assert!(!state_of(ENV).is_on());

    let with_both = write_secrets(
        ENV,
        &[(API_TOKEN_ENV_VAR, "t"), (REGISTRATION_KEY_ENV_VAR, "k")],
    );
    assert_eq!(
        state_of(&with_both),
        AuthState {
            api_token: true,
            registration_key: true
        }
    );
    assert!(state_of(&with_both).is_on());

    // Either one on its own is on.
    let key_only = write_secrets(ENV, &[(REGISTRATION_KEY_ENV_VAR, "k")]);
    assert_eq!(
        state_of(&key_only),
        AuthState {
            api_token: false,
            registration_key: true
        }
    );
}

#[test]
fn writing_secrets_uncomments_the_placeholders_and_touches_nothing_else() {
    let out = write_secrets(
        ENV,
        &[
            (API_TOKEN_ENV_VAR, "a".repeat(64).as_str()),
            (REGISTRATION_KEY_ENV_VAR, "b".repeat(64).as_str()),
        ],
    );
    assert!(out.contains(&format!("\nCHAP_API_TOKEN={}\n", "a".repeat(64))));
    assert!(out.contains(&format!(
        "\nSERVICEKIT_REGISTRATION_KEY={}\n",
        "b".repeat(64)
    )));
    assert!(!out.contains("# CHAP_API_TOKEN="));
    assert!(!out.contains("# SERVICEKIT_REGISTRATION_KEY="));
    assert!(
        !out.contains(APPENDED_HEADING),
        "the lines were already there"
    );

    // Every other line is where it was, byte for byte.
    let before: Vec<&str> = ENV.lines().collect();
    let after: Vec<&str> = out.lines().collect();
    assert_eq!(before.len(), after.len(), "no line was added or removed");
    for (i, (was, is)) in before.iter().zip(&after).enumerate() {
        if was.contains("CHAP_API_TOKEN") || was.contains("SERVICEKIT_REGISTRATION_KEY") {
            continue;
        }
        assert_eq!(was, is, "line {i} changed");
    }
}

#[test]
fn writing_secrets_replaces_an_active_line_in_place() {
    let body = "CHAP_API_TOKEN=old\n# a comment\nPOSTGRES_DB=chap_core\n";
    assert_eq!(
        write_secrets(body, &[(API_TOKEN_ENV_VAR, "new")]),
        "CHAP_API_TOKEN=new\n# a comment\nPOSTGRES_DB=chap_core\n"
    );
}

#[test]
fn a_secret_with_no_line_at_all_is_appended_under_one_heading() {
    let body = "POSTGRES_DB=chap_core\n";
    let out = write_secrets(
        body,
        &[(API_TOKEN_ENV_VAR, "t"), (REGISTRATION_KEY_ENV_VAR, "k")],
    );
    assert_eq!(
        out,
        format!(
            "POSTGRES_DB=chap_core\n\n{APPENDED_HEADING}\n\
                 CHAP_API_TOKEN=t\nSERVICEKIT_REGISTRATION_KEY=k\n"
        )
    );
    assert_eq!(out.matches(APPENDED_HEADING).count(), 1);

    // A file already ending in a blank line gains no second one.
    let out = write_secrets("POSTGRES_DB=chap_core\n\n", &[(API_TOKEN_ENV_VAR, "t")]);
    assert_eq!(
        out,
        format!("POSTGRES_DB=chap_core\n\n{APPENDED_HEADING}\nCHAP_API_TOKEN=t\n")
    );

    // And an empty file is still a valid one afterwards.
    assert_eq!(
        write_secrets("", &[(API_TOKEN_ENV_VAR, "t")]),
        format!("{APPENDED_HEADING}\nCHAP_API_TOKEN=t\n")
    );
}

#[test]
fn commenting_out_keeps_the_value_and_the_rest_of_the_file() {
    let body = write_secrets(ENV, &[(API_TOKEN_ENV_VAR, "sekret")]);
    let out = comment_out(&body, API_TOKEN_ENV_VAR);
    assert!(out.contains("\n# CHAP_API_TOKEN=sekret\n"), "{out}");
    assert_eq!(active_value(&out, API_TOKEN_ENV_VAR), None);
    // The value survives, so enabling again recovers it.
    assert_eq!(
        active_value(
            &write_secrets(&out, &[(API_TOKEN_ENV_VAR, "sekret")]),
            API_TOKEN_ENV_VAR
        )
        .as_deref(),
        Some("sekret")
    );

    // Nothing to comment out: the body is unchanged.
    assert_eq!(comment_out(ENV, API_TOKEN_ENV_VAR), ENV);
    // An already commented line is not commented twice.
    assert!(!comment_out(&out, API_TOKEN_ENV_VAR).contains("# # CHAP_API_TOKEN"));
}

#[test]
fn a_commented_value_is_recoverable_and_the_empty_placeholder_is_not() {
    // What the generated file ships: a placeholder with no value.
    assert_eq!(commented_value(ENV, API_TOKEN_ENV_VAR), None);
    // What `disable` leaves behind.
    let off = comment_out(
        &write_secrets(ENV, &[(API_TOKEN_ENV_VAR, "sekret")]),
        API_TOKEN_ENV_VAR,
    );
    assert_eq!(
        commented_value(&off, API_TOKEN_ENV_VAR).as_deref(),
        Some("sekret")
    );
    // An active line is not a commented one.
    assert_eq!(
        commented_value("CHAP_API_TOKEN=sekret\n", API_TOKEN_ENV_VAR),
        None
    );
    // Turning it back on is a round trip, byte for byte.
    assert_eq!(
        write_secrets(&off, &[(API_TOKEN_ENV_VAR, "sekret")]),
        write_secrets(ENV, &[(API_TOKEN_ENV_VAR, "sekret")])
    );
}

#[test]
fn the_token_is_read_out_of_a_projects_env_file() {
    let dir = tempfile::tempdir().unwrap();
    // No file at all.
    assert_eq!(token_in(dir.path()), None);

    std::fs::write(dir.path().join(ENV_FILE), ENV).unwrap();
    assert_eq!(token_in(dir.path()), None, "only a commented placeholder");

    std::fs::write(
        dir.path().join(ENV_FILE),
        write_secrets(ENV, &[(API_TOKEN_ENV_VAR, "sekret")]),
    )
    .unwrap();
    assert_eq!(token_in(dir.path()).as_deref(), Some("sekret"));
}

#[test]
fn the_open_paths_are_the_ones_chap_core_leaves_alone() {
    assert_eq!(OPEN_PATHS, &["/health", "/health/ready", "/system/info"]);
    // The service registry is not open: a client reading it needs the
    // token like any other caller.
    assert!(!OPEN_PATHS.contains(&crate::status::SERVICES_PATH));
}
