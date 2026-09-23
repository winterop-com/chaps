//! The two shared secrets a deployment may use, and the `.env` lines they
//! live on.
//!
//! chap-core has two independent opt-in secrets, each disabled when its
//! variable is unset:
//!
//! - `CHAP_API_TOKEN` gates the whole API. Clients send it as
//!   `Authorization: Bearer <token>`; chap-core also accepts it in
//!   `X-Service-Key`, because servicekit can send no other header.
//!   [`OPEN_PATHS`] stay reachable without it.
//! - `SERVICEKIT_REGISTRATION_KEY` gates chapkit service registration alone.
//!   A model presents it as `X-Service-Key` on `/v2/services/$register` and
//!   `$ping`, and chap-core accepts it only under `/v2/services`, so a
//!   registration key is no general-purpose API credential.
//!
//! Both values belong in `.env` and nowhere else: `.chaps/project.yaml`
//! records only whether each one is in use, so the state directory can be read,
//! copied and committed without leaking a credential.

use crate::error::Result;
use crate::project::{AuthState, ENV_FILE};
use std::path::Path;

/// The `.env` variable chap-core reads the API token from.
pub const API_TOKEN_ENV_VAR: &str = "CHAP_API_TOKEN";
/// The `.env` variable chap-core and every model overlay read the service
/// registration key from.
pub const REGISTRATION_KEY_ENV_VAR: &str = "SERVICEKIT_REGISTRATION_KEY";

/// Bytes of entropy in a generated secret: 32, so the hex spelling is the
/// 64 characters `openssl rand -hex 32` produces.
const SECRET_BYTES: usize = 32;

/// Shortest token chap-core accepts without warning at startup. The API has no
/// rate limiting, so a short token is guessable by anyone who can reach the
/// port.
pub const MIN_TOKEN_LENGTH: usize = 32;

/// Characters of a token shown in place of the whole thing.
const MASK_PREFIX: usize = 6;

/// Paths chap-core answers without a token even when `CHAP_API_TOKEN` is set:
/// the container healthcheck calls the first two with no headers at all, and
/// the third is how a client discovers that a token is required.
///
/// Everything else needs one, `/v2/services` and `/docs` included.
pub const OPEN_PATHS: &[&str] = &["/health", "/health/ready", "/system/info"];

/// Comment `chaps auth enable` writes above secrets that had no line of their
/// own in `.env`.
pub const APPENDED_HEADING: &str = "# API authentication, written by `chaps auth`.";

/// What to do with the token once it exists.
pub const MODELING_APP_HINT: &str = "paste this token in the Modeling App's CHAP settings";

/// A freshly generated secret: [`SECRET_BYTES`] random bytes as hex.
pub fn random_secret() -> Result<String> {
    random_hex(SECRET_BYTES)
}

/// `bytes` random bytes as lowercase hex, the shape of `openssl rand -hex`.
///
/// Hex rather than base64 so the value is safe in a `.env` line, in a URL and
/// in a shell argument without any quoting.
pub fn random_hex(bytes: usize) -> Result<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| anyhow::anyhow!("generating a random secret: {e}"))?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// A secret as it is safe to print: the first few characters and an ellipsis.
///
/// Enough to tell two tokens apart in a terminal or a bug report, and not
/// enough to use. A secret too short to abbreviate is hidden entirely.
pub fn mask(secret: &str) -> String {
    if secret.chars().count() <= MASK_PREFIX {
        return "...".to_string();
    }
    let prefix: String = secret.chars().take(MASK_PREFIX).collect();
    format!("{prefix}...")
}

/// The warning for a token chap-core will flag as weak at startup.
pub fn weak_token_warning(length: usize) -> String {
    format!(
        "the API token is {length} characters; chap-core warns below {MIN_TOKEN_LENGTH} because \
         its API has no rate limiting, so a short token is guessable by anyone who can reach \
         the port"
    )
}

/// The value of the first active (uncommented) `var=` line of a `.env` body.
///
/// An empty assignment is not a secret: chap-core reads the variable as
/// `os.getenv(...) or None`, so `CHAP_API_TOKEN=` disables authentication
/// exactly like a commented line does.
pub fn active_value(body: &str, var: &str) -> Option<String> {
    body.lines()
        .filter_map(|line| assignment(line, var))
        .map(str::to_string)
        .find(|value| !value.is_empty())
}

/// The value of the first commented `# var=` line that still carries one.
///
/// [`comment_out`] keeps the value behind the `#`, so this is how
/// `chaps auth enable` hands a deployment back the very token its clients are
/// already configured with, rather than a new one nobody has yet. The empty
/// placeholder the generated `.env` ships carries no value and is skipped.
pub fn commented_value(body: &str, var: &str) -> Option<String> {
    body.lines()
        .filter(|line| is_commented(line, var))
        .filter_map(|line| {
            line.trim_start()
                .trim_start_matches('#')
                .trim_start()
                .strip_prefix(&format!("{var}="))
        })
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

/// Which of the two secrets a `.env` body actually sets.
pub fn state_of(body: &str) -> AuthState {
    AuthState {
        api_token: active_value(body, API_TOKEN_ENV_VAR).is_some(),
        registration_key: active_value(body, REGISTRATION_KEY_ENV_VAR).is_some(),
    }
}

/// The API token this deployment's `.env` sets, if any.
///
/// Best-effort by design: a project written with `init --no-env` has no file to
/// read, and a deployment without authentication has no line in it, both of
/// which mean "send no token".
pub fn token_in(dir: &Path) -> Option<String> {
    let body = std::fs::read_to_string(dir.join(ENV_FILE)).ok()?;
    active_value(&body, API_TOKEN_ENV_VAR)
}

/// Write each `(var, value)` pair into a `.env` body as an active line.
///
/// One line per variable changes and nothing else. In order: the first active
/// `VAR=` line is rewritten, else the commented placeholder the generated
/// `.env` ships is uncommented in place, else the assignment is appended under
/// [`APPENDED_HEADING`]. Comments, blank lines, ordering and every other
/// variable survive byte for byte.
pub fn write_secrets(body: &str, secrets: &[(&str, &str)]) -> String {
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    let mut appended = Vec::new();
    for (var, value) in secrets {
        let wanted = format!("{var}={value}");
        if let Some(at) = lines
            .iter()
            .position(|line| assignment(line, var).is_some())
        {
            lines[at] = wanted;
        } else if let Some(at) = lines.iter().position(|line| is_commented(line, var)) {
            lines[at] = wanted;
        } else {
            appended.push(wanted);
        }
    }
    if !appended.is_empty() {
        // One blank line before the new block, unless the file already ends in
        // one.
        if lines.last().is_some_and(|line| !line.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(APPENDED_HEADING.to_string());
        lines.extend(appended);
    }
    join(&lines)
}

/// Comment out every active `var=` line of a `.env` body, keeping the value
/// behind the `#`.
///
/// The value is kept so `chaps auth enable` can recover it, and so an operator
/// who turned authentication off by mistake still has the token their clients
/// were configured with.
pub fn comment_out(body: &str, var: &str) -> String {
    let lines: Vec<String> = body
        .lines()
        .map(|line| match assignment(line, var) {
            Some(_) => format!("# {line}"),
            None => line.to_string(),
        })
        .collect();
    join(&lines)
}

/// The value an active `var=` line assigns, trimmed; `None` for any other
/// line, a comment included.
fn assignment<'a>(line: &'a str, var: &str) -> Option<&'a str> {
    line.trim_start()
        .strip_prefix(&format!("{var}="))
        .map(str::trim)
}

/// `# VAR=...`, with any amount of whitespace around the `#`.
fn is_commented(line: &str, var: &str) -> bool {
    let rest = line.trim_start().strip_prefix('#');
    rest.is_some_and(|rest| rest.trim_start().starts_with(&format!("{var}=")))
}

/// Lines back into a file body, one newline each.
fn join(lines: &[String]) -> String {
    let mut out = String::new();
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.env` shaped like the generated one: comments, an active line, and
    /// commented placeholders for both secrets.
    const ENV: &str = "\
# Generated by chap init (0.1.0).
POSTGRES_PASSWORD=0123456789abcdef
CHAP_API_PORT=8000

# API authentication (optional).
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
    fn masking_shows_six_characters_and_hides_a_short_secret_entirely() {
        assert_eq!(mask("0123456789abcdef"), "012345...");
        assert_eq!(mask("abcdefg"), "abcdef...");
        // Nothing to abbreviate: showing it would be showing the secret.
        assert_eq!(mask("abcdef"), "...");
        assert_eq!(mask("ab"), "...");
        assert_eq!(mask(""), "...");
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
        // The first usable value wins, the way compose reads the file.
        assert_eq!(
            active_value(
                "CHAP_API_TOKEN=one\nCHAP_API_TOKEN=two\n",
                API_TOKEN_ENV_VAR
            )
            .as_deref(),
            Some("one")
        );
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
}
