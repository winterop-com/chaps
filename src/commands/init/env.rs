//! `DIR/.env`: whether `init` writes it, and the secrets that go in it.

use crate::auth;
use crate::error::Result;
use crate::project::{API_PORT_ENV_VAR, AuthState};
use serde::Serialize;
use std::path::Path;

/// The port an active (uncommented) `CHAP_API_PORT=` line of a `.env` sets.
///
/// A commented placeholder is not a setting, and a value that is not a port
/// number is the operator's problem to see for themselves - neither yields a
/// warning here.
pub(super) fn env_api_port(body: &str) -> Option<u16> {
    body.lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix(&format!("{API_PORT_ENV_VAR}=")))
        .find_map(|value| value.trim().parse::<u16>().ok())
}

/// What `init` did with `DIR/.env`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum EnvAction {
    /// A freshly rendered file, with a new database password.
    Written,
    /// The file that was already there, byte for byte.
    Kept,
    /// No `.env` at all (`--no-env`).
    Skipped,
}

/// Decide what to do with `DIR/.env`.
///
/// `--force` is deliberately not an input: it re-renders the compose files and
/// the state, but the credentials in `.env` outlive it. Only `--fresh-env`
/// replaces a file that is already there.
pub(super) fn env_action(exists: bool, fresh_env: bool, no_env: bool) -> EnvAction {
    match (no_env, exists, fresh_env) {
        (true, _, _) => EnvAction::Skipped,
        (false, true, false) => EnvAction::Kept,
        (false, _, _) => EnvAction::Written,
    }
}

/// 32 lowercase hex characters, so the generated password is URL-safe inside
/// the postgres URL chap-core composes from POSTGRES_*.
pub(super) fn random_password() -> Result<String> {
    auth::random_hex(16)
}

/// The pair of secrets `--api-token` writes into `.env`.
#[derive(Debug, Clone)]
pub(super) struct Secrets {
    pub(super) api_token: String,
    pub(super) registration_key: String,
}

/// The secrets `--api-token` asks for: both of them, or neither.
///
/// A bare flag generates a token; an explicit value is used verbatim, with a
/// warning when chap-core would call it weak. The registration key is always
/// generated and can never be given on the command line: chap-core rejects an
/// unauthenticated registration once the API is protected, so a deployment with
/// a token needs a key as well, and there is no reason to make anyone type a
/// second secret.
pub(super) fn resolve_secrets(requested: Option<&Option<String>>) -> Result<Option<Secrets>> {
    let Some(value) = requested else {
        return Ok(None);
    };
    let api_token = match value.as_deref().map(str::trim) {
        Some(given) if !given.is_empty() => {
            if let Some(problem) = crate::dotenv::literal_problem(given) {
                return Err(crate::error::ChapError::Usage(format!(
                    "--api-token contains {problem}; choose another, or pass --api-token with no \
                     value and one is generated"
                ))
                .into());
            }
            let length = given.chars().count();
            if length < auth::MIN_TOKEN_LENGTH {
                crate::output::warn(&auth::weak_token_warning(length));
            }
            given.to_string()
        }
        Some(_) => {
            crate::output::warn("--api-token was given an empty value; generated one instead");
            auth::random_secret()?
        }
        None => auth::random_secret()?,
    };
    Ok(Some(Secrets {
        api_token,
        registration_key: auth::random_secret()?,
    }))
}

/// The auth state `DIR/.env` describes, once `init` has settled its fate.
///
/// The file is the single source of truth, so reading it back covers all three
/// cases at once: a file this run rendered with `--api-token`, one that was
/// kept and already carries secrets from an earlier run or `chaps auth enable`,
/// and `--no-env`, which has no file and therefore no secrets.
pub(super) fn env_auth(env: EnvAction, path: &Path) -> AuthState {
    if env == EnvAction::Skipped {
        return AuthState::default();
    }
    std::fs::read_to_string(path)
        .map(|body| auth::state_of(&body))
        .unwrap_or_default()
}
