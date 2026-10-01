//! Asking chap-core over HTTP, and reading its answers strictly.

use super::{ApiHealth, ApiVersion, INFO_PATHS, RegisteredService, SERVICES_PATH};
use serde::Deserialize;
use std::time::Duration;

/// Parse a `/health` body, strictly.
///
/// chap-core answers `{"status":"success","message":"healthy"}`. Anything
/// that is not JSON with a `status` field is somebody else holding the port -
/// a dev server, a proxy, an old deployment - and reporting that as `up`
/// would be worse than reporting nothing at all.
pub fn parse_health(api_url: &str, content_type: &str, body: &str) -> ApiHealth {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => {
            return ApiHealth::Down {
                error: is_not_chap_core(api_url, &body_description(content_type, body)),
            };
        }
    };
    let Some(status) = value.get("status").map(json_text) else {
        return ApiHealth::Down {
            error: is_not_chap_core(
                api_url,
                &format!(
                    "{} without a `status` field",
                    body_description(content_type, body)
                ),
            ),
        };
    };
    ApiHealth::Up {
        status,
        message: value.get("message").map(json_text).unwrap_or_default(),
    }
}

/// `port 8000 answers but it is not chap-core (got text/html)`.
pub fn is_not_chap_core(api_url: &str, what: &str) -> String {
    format!(
        "{} answers but it is not chap-core (got {what})",
        endpoint_phrase(api_url)
    )
}

/// What an HTTP 401 means: the token, never the identity of the server.
///
/// `sent` says whether `.env` had a token to send. Both halves matter: a
/// deployment whose `.env` is out of step with its running chap-core, and a
/// chap-core that was given a token while `.env` was not. The path is named
/// because the two ends of it differ - [`crate::auth::OPEN_PATHS`] answer
/// without a token at all, so a 401 from one of those means something in front
/// of chap-core is asking as well.
pub fn token_rejected(api_url: &str, path: &str, sent: bool) -> String {
    let endpoint = endpoint_phrase(api_url);
    let mut text = if sent {
        format!(
            "{endpoint} answers {path} with HTTP 401: the API token in .env is not accepted; \
             `chaps auth show --reveal` prints it, and `chaps up` hands a rotated one to \
             chap-core"
        )
    } else {
        format!(
            "{endpoint} answers {path} with HTTP 401: it requires an API token and .env sets \
             none; `chaps auth enable` writes one, or add CHAP_API_TOKEN to .env to match the \
             chap-core that is running"
        )
    };
    if crate::auth::OPEN_PATHS.contains(&path) {
        text.push_str(&format!(
            " ({path} is open even when CHAP_API_TOKEN is set, so something in front of \
             chap-core may be asking for credentials too)"
        ));
    }
    text
}

/// The same verdict, reached through the service registry instead.
pub fn services_are_not_chap_core(api_url: &str, what: &str) -> String {
    format!(
        "{} answers but it is not chap-core: {SERVICES_PATH} returned {what}, not {{count, services}}",
        endpoint_phrase(api_url)
    )
}

/// How to name the thing that answered: its port, which is what the operator
/// has to free, or the whole URL when there is no port in it.
fn endpoint_phrase(api_url: &str) -> String {
    match port_of(api_url) {
        Some(port) => format!("port {port}"),
        None => api_url.to_string(),
    }
}

/// The port of `http://host:PORT/path`, when it has one.
pub(super) fn port_of(url: &str) -> Option<u16> {
    let authority = url.rsplit("://").next()?.split('/').next()?;
    authority.rsplit_once(':')?.1.parse().ok()
}

/// What to call a body that is not chap-core's JSON: the content type the
/// server sent, or the first bytes when it sent none.
pub fn body_description(content_type: &str, body: &str) -> String {
    // `text/html; charset=utf-8` says nothing more than `text/html` here.
    let kind = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if !kind.is_empty() {
        return kind;
    }
    let head: String = body
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(FIRST_BYTES)
        .collect();
    if head.is_empty() {
        return "an empty body".to_string();
    }
    let ellipsis = if body.trim().chars().count() > FIRST_BYTES {
        "..."
    } else {
        ""
    };
    format!("`{head}{ellipsis}`")
}

/// How much of an unrecognised body to quote back.
const FIRST_BYTES: usize = 40;

/// Parse a `/v2/services` body into the flattened report entries.
///
/// Strict about the envelope: chap-core's registry is `{count, services}`, so
/// a body without both is not chap-core's, however well-formed it is.
pub fn parse_services(body: &str) -> Result<Vec<RegisteredService>, serde_json::Error> {
    let wire: ServicesBody = serde_json::from_str(body)?;
    Ok(wire.services.into_iter().map(flatten).collect())
}

/// chap-core's own version, from whichever info endpoint answers.
///
/// Best-effort: neither path is required, and a chap-core that publishes
/// neither still gets a version in the report - the tag the project pins,
/// marked as such so nobody reads it as the running build.
pub(super) fn version_of(
    agent: &ureq::Agent,
    base: &str,
    api: &ApiHealth,
    pinned: &str,
    token: Option<&str>,
) -> ApiVersion {
    if matches!(api, ApiHealth::Up { .. }) {
        for path in INFO_PATHS {
            if let Ok(answer) = get(agent, base, path, token)
                && let Some(version) = parse_version(&answer.body)
            {
                return ApiVersion {
                    value: version,
                    pinned: false,
                    revision: parse_revision(&answer.body),
                };
            }
        }
    }
    ApiVersion {
        value: pinned.to_string(),
        pinned: true,
        revision: None,
    }
}

/// The commit an info body says the build came from.
///
/// chap-core fills `revision` from `GIT_REVISION`, which is empty in a build
/// that was not given one; empty is no answer, not an answer of "".
pub fn parse_revision(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    const KEYS: &[&str] = &["revision", "git_revision", "commit"];
    for key in KEYS {
        if let Some(found) = string_at(&value, key) {
            return Some(found);
        }
    }
    None
}

/// A version out of an info body, wherever it keeps it.
///
/// chap-core has moved this field around between releases, so the obvious
/// spellings are all accepted and an unknown shape is simply no answer.
pub fn parse_version(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    const KEYS: &[&str] = &["version", "chap_core_version", "app_version"];
    const NESTED: &[&str] = &["info", "system", "chap_core"];
    for key in KEYS {
        if let Some(found) = string_at(&value, key) {
            return Some(found);
        }
    }
    for outer in NESTED {
        let inner = value.get(outer)?;
        for key in KEYS {
            if let Some(found) = string_at(inner, key) {
                return Some(found);
            }
        }
    }
    None
}

fn string_at(value: &serde_json::Value, key: &str) -> Option<String> {
    let text = value.get(key)?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// A JSON value as the text to print: a string as itself, anything else as
/// its JSON spelling, so a numeric `status` is still reported rather than
/// dropped.
fn json_text(value: &serde_json::Value) -> String {
    match value.as_str() {
        Some(text) => text.to_string(),
        None => value.to_string(),
    }
}
fn flatten(w: WireService) -> RegisteredService {
    let id = if w.id.is_empty() {
        w.info.id.clone()
    } else {
        w.id
    };
    let display_name = if w.info.display_name.is_empty() {
        id.clone()
    } else {
        w.info.display_name
    };
    RegisteredService {
        id,
        url: w.url,
        display_name,
        version: w.info.version,
        last_ping_at: w.last_ping_at,
        expires_at: w.expires_at,
    }
}

/// One answer from the API: the body plus the content type, which is how an
/// unrecognised body is named in the report.
pub(super) struct Answer {
    pub(super) content_type: String,
    pub(super) body: String,
}

/// Why a request yielded no usable answer.
pub(super) enum Failure {
    /// HTTP 401. Reported separately because it is the one status code that
    /// says something about the deployment's own configuration rather than
    /// about whatever is on the port.
    Unauthorized,
    /// Anything else, already described for a human.
    Other(String),
}

/// `GET base+path`, with the API token when there is one.
pub(super) fn get(
    agent: &ureq::Agent,
    base: &str,
    path: &str,
    token: Option<&str>,
) -> Result<Answer, Failure> {
    let mut request = agent.get(format!("{base}{path}"));
    if let Some(token) = token {
        // The header chap-core documents in its OpenAPI spec and enforces in
        // its middleware. It also accepts the token in `X-Service-Key`, but
        // that spelling exists for servicekit, which can send no other.
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let url = format!("{base}{path}");
    let started = std::time::Instant::now();
    let mut response = request.call().map_err(|e| {
        crate::output::verbose(&format!(
            "GET {url} -> failed in {}ms",
            started.elapsed().as_millis()
        ));
        Failure::Other(e.to_string())
    })?;
    let status = response.status();
    crate::output::verbose(&format!(
        "GET {url} -> {} in {}ms",
        status.as_u16(),
        started.elapsed().as_millis()
    ));
    if status.as_u16() == 401 {
        return Err(Failure::Unauthorized);
    }
    if !status.is_success() {
        return Err(Failure::Other(format!("HTTP {}", status.as_u16())));
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| Failure::Other(e.to_string()))?;
    crate::output::debug(&format!("  {}", crate::output::trace_body(&body)));
    Ok(Answer { content_type, body })
}

pub(super) fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        // Status codes are reported by the caller, not raised as errors.
        .http_status_as_error(false)
        .user_agent(concat!("chaps/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

/// `GET /v2/services`. The envelope is required (see [`parse_services`]);
/// fields the report does not use - `registered_at`, everything chap-core
/// adds later - are ignored.
#[derive(Debug, Deserialize)]
struct ServicesBody {
    #[expect(dead_code, reason = "required in the envelope, never read")]
    count: u64,
    services: Vec<WireService>,
}

#[derive(Debug, Default, Deserialize)]
struct WireService {
    #[serde(default)]
    id: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    info: WireInfo,
    #[serde(default)]
    last_ping_at: String,
    #[serde(default)]
    expires_at: String,
}

#[derive(Debug, Default, Deserialize)]
struct WireInfo {
    #[serde(default)]
    id: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    version: String,
}
