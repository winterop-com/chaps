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
///
/// The Modeling App never sees it: DHIS2's `chap` route adds it to every
/// request it proxies, and `chaps dhis2 connect` is what writes it there.
pub const MODELING_APP_HINT: &str =
    "the DHIS2 `chap` route carries it once `chaps dhis2 connect` has run";

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

/// The warning for a token chap-core will flag as weak at startup.
pub fn weak_token_warning(length: usize) -> String {
    format!(
        "the API token is {length} characters; chap-core warns below {MIN_TOKEN_LENGTH} because \
         its API has no rate limiting, so a short token is guessable by anyone who can reach \
         the port"
    )
}

/// The value of the last active (uncommented) `var=` line of a `.env` body.
///
/// The last, because that is the one compose reads: see [`crate::dotenv`],
/// which is where the rules live and which every other reader of `.env` goes
/// through too.
///
/// An empty assignment is not a secret: chap-core reads the variable as
/// `os.getenv(...) or None`, so `CHAP_API_TOKEN=` disables authentication
/// exactly like a commented line does.
pub fn active_value(body: &str, var: &str) -> Option<String> {
    crate::dotenv::non_empty(body, var)
}

/// The value of the last commented `# var=` line that still carries one.
///
/// [`comment_out`] keeps the value behind the `#`, so this is how
/// `chaps auth enable` hands a deployment back the very token its clients are
/// already configured with, rather than a new one nobody has yet. The empty
/// placeholder the generated `.env` ships carries no value and is skipped.
pub fn commented_value(body: &str, var: &str) -> Option<String> {
    crate::dotenv::commented_value(body, var)
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

/// Write each `(var, value)` pair into a `.env` body as the one active line
/// that assigns it.
///
/// [`crate::dotenv::set`] does the writing, so the line left behind is exactly
/// the line [`active_value`] reads back: the first active `VAR=` line is
/// rewritten and every later duplicate of it removed, else the commented
/// placeholder the generated `.env` ships is uncommented in place, else the
/// assignment is appended under [`APPENDED_HEADING`]. Comments, blank lines,
/// ordering and every other variable survive byte for byte.
pub fn write_secrets(body: &str, secrets: &[(&str, &str)]) -> String {
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    let mut appended = Vec::new();
    for (var, value) in secrets {
        if crate::dotenv::set(&mut lines, var, value) == crate::dotenv::Wrote::Absent {
            appended.push(format!("{var}={}", crate::dotenv::encode(value)));
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
    crate::dotenv::join(&lines)
}

/// Comment out every active `var=` line of a `.env` body, keeping the value
/// behind the `#`.
///
/// The value is kept so `chaps auth enable` can recover it, and so an operator
/// who turned authentication off by mistake still has the token their clients
/// were configured with.
pub fn comment_out(body: &str, var: &str) -> String {
    crate::dotenv::comment_out(body, var)
}

#[cfg(test)]
mod tests;
