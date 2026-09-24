//! A thin authenticated HTTP client for chap-core's API.
//!
//! [`crate::status`] asks chap-core two fixed questions and turns the answers
//! into a report. This is the other half: any method, any path, the body handed
//! back as it arrived. `chaps api` is the command that exposes it, and
//! `chaps jobs` is the first caller that uses it for something shaped.
//!
//! Three rules it keeps, so every caller reports the same things the same way:
//!
//! - A transport failure is [`ChapError::Unreachable`], which is the sentence
//!   `chaps status` uses plus the command that explains it. An HTTP status is
//!   not a failure here - the body of a 404 is often the only place chap-core
//!   says what it did not find - so a non-2xx answer comes back as an
//!   [`Answer`] and the caller decides.
//! - The API token goes out as `Authorization: Bearer`, never in a trace line.
//! - `-v` narrates the request line and the headers, `-d` adds the body.

use crate::error::{ChapError, Result};
use std::borrow::Cow;
use std::time::Duration;

/// How long a request may take, all of connect, send and receive.
///
/// Longer than the [`crate::status`] probe: that one is a health check that
/// must not hold a terminal, while a request here may be a query chap-core
/// has to compute an answer to.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// `User-Agent` sent with every request, matching [`crate::chapcore`].
const USER_AGENT: &str = concat!("chaps-cli/", env!("CARGO_PKG_VERSION"));

/// The content type a JSON body is sent and read as.
pub const JSON: &str = "application/json";

/// The environment variable a token is read from outside a deployment.
///
/// The same name chap-core itself reads, and the same one `.env` sets, so a
/// shell that already exports it for `curl` needs nothing new for `chaps api`.
pub const TOKEN_ENV_VAR: &str = crate::auth::API_TOKEN_ENV_VAR;

/// The methods `chaps api` accepts, in the order the help lists them.
pub const METHODS: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE"];

/// One chap-core API, ready to be asked.
#[derive(Debug, Clone)]
pub struct Api {
    base: String,
    token: Option<String>,
    timeout: Duration,
}

/// One answer: the status, the body, and enough about both to print them.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    /// The reason phrase, e.g. `Not Found`. Empty for a status code that has
    /// no canonical one.
    pub reason: String,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Answer {
    /// Whether the status is a 2xx.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as text, with anything that is not UTF-8 replaced.
    pub fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }

    /// The body as JSON, or `None` when it is not JSON at all.
    pub fn json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.body).ok()
    }

    /// `HTTP 404 Not Found`, the line a failed request reports on stderr.
    pub fn status_line(&self) -> String {
        if self.reason.is_empty() {
            format!("HTTP {}", self.status)
        } else {
            format!("HTTP {} {}", self.status, self.reason)
        }
    }

    /// What came back, for an error that has to say what it was instead.
    ///
    /// The content type when the server sent one, because `text/html` is the
    /// whole diagnosis when a proxy or a dev server holds chap-core's port,
    /// and how many bytes it was otherwise.
    pub fn body_description(&self) -> String {
        let kind = self
            .content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim();
        if kind.is_empty() {
            format!("{} bytes", self.body.len())
        } else {
            format!("{kind} ({} bytes)", self.body.len())
        }
    }

    /// What chap-core said went wrong: its `detail` field, or the body.
    ///
    /// FastAPI answers every error with `{"detail": ...}`, so that is the one
    /// sentence worth putting in an error message; anything else is handed
    /// back trimmed and cut, because a stack trace in an error line helps
    /// nobody.
    pub fn detail(&self) -> String {
        if let Some(value) = self.json()
            && let Some(detail) = value.get("detail")
        {
            return match detail.as_str() {
                Some(text) => text.to_string(),
                None => detail.to_string(),
            };
        }
        let text = self.text();
        let text = text.trim();
        if text.chars().count() > 200 {
            let cut: String = text.chars().take(200).collect();
            format!("{cut}...")
        } else {
            text.to_string()
        }
    }
}

impl Api {
    /// A client for `base`, sending `token` when there is one.
    pub fn new(base: &str, token: Option<String>, timeout: Duration) -> Api {
        Api {
            base: base.trim_end_matches('/').to_string(),
            token,
            timeout,
        }
    }

    /// The base URL, without its trailing slash.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Whether a token will be sent.
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    /// The full URL `path` is asked at.
    pub fn url(&self, path: &str) -> String {
        if path.starts_with('/') {
            format!("{}{path}", self.base)
        } else {
            format!("{}/{path}", self.base)
        }
    }

    /// `GET path` as JSON.
    ///
    /// The convenience shape for code that knows what it asked for: a non-2xx
    /// or a body that is not JSON is an error, so the caller gets a value or a
    /// sentence. Use [`Api::send`] where the status code is part of the
    /// answer, as it is for a 404 that means "no such job".
    pub fn get_json(&self, path: &str) -> Result<serde_json::Value> {
        let answer = self.send("GET", path, None)?;
        if !answer.is_success() {
            return Err(self.status_error(path, &answer));
        }
        answer.json().ok_or_else(|| {
            anyhow::anyhow!(
                "{} answered {} with {}, which is not JSON",
                self.url(path),
                answer.status_line(),
                answer.body_description()
            )
        })
    }

    /// `method path` with an optional JSON body.
    ///
    /// The body is sent verbatim with `Content-Type: application/json`; it is
    /// the caller's text, not a value re-serialised here, so a document read
    /// from a file reaches chap-core byte for byte.
    pub fn send_json(&self, method: &str, path: &str, body: Option<&str>) -> Result<Answer> {
        self.send(method, path, body.map(str::as_bytes))
    }

    /// The one request path: `method path`, with the token and the body.
    pub fn send(&self, method: &str, path: &str, body: Option<&[u8]>) -> Result<Answer> {
        let url = self.url(path);
        let agent = ureq::Agent::new_with_config(
            ureq::Agent::config_builder()
                .timeout_global(Some(self.timeout))
                .user_agent(USER_AGENT)
                // A status code is an answer, not a transport failure: the
                // body of a 4xx is where chap-core says what was wrong with
                // the request, and dropping it would be dropping the message.
                .http_status_as_error(false)
                .build(),
        );

        let mut request = ureq::http::Request::builder().method(method).uri(&url);
        crate::output::verbose(&format!("{method} {url}"));
        if self.token.is_some() {
            request = request.header("Authorization", format!("Bearer {}", self.bearer()));
            // Never the value: a trace line ends up in bug reports.
            crate::output::verbose("  Authorization: Bearer <token>");
        }
        if body.is_some() {
            request = request.header("Content-Type", JSON);
            crate::output::verbose(&format!("  Content-Type: {JSON}"));
        }
        if let Some(body) = body {
            crate::output::debug(&format!(
                "  {}",
                crate::output::trace_body(&String::from_utf8_lossy(body))
            ));
        }

        let request = request
            .body(body.unwrap_or_default().to_vec())
            .map_err(|e| ChapError::Usage(format!("{method} {url} is not a valid request: {e}")))?;

        let started = std::time::Instant::now();
        let mut response = agent.run(request).map_err(|e| {
            crate::output::verbose(&format!(
                "{method} {url} -> failed in {}ms",
                started.elapsed().as_millis()
            ));
            ChapError::Unreachable {
                url: self.base.clone(),
                reason: e.to_string(),
            }
        })?;
        let status = response.status();
        crate::output::verbose(&format!(
            "{method} {url} -> {} in {}ms",
            status.as_u16(),
            started.elapsed().as_millis()
        ));
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let body = response
            .body_mut()
            .read_to_vec()
            .map_err(|e| anyhow::anyhow!("reading the response body from {url}: {e}"))?;
        crate::output::debug(&format!(
            "  {}",
            crate::output::trace_body(&String::from_utf8_lossy(&body))
        ));
        Ok(Answer {
            status: status.as_u16(),
            reason: status.canonical_reason().unwrap_or_default().to_string(),
            content_type,
            body,
        })
    }

    /// The error for a non-2xx answer to a request that had to work.
    pub fn status_error(&self, path: &str, answer: &Answer) -> anyhow::Error {
        let detail = answer.detail();
        let where_ = self.url(path);
        if detail.is_empty() {
            anyhow::anyhow!("{where_} answered {}", answer.status_line())
        } else {
            anyhow::anyhow!("{where_} answered {}: {detail}", answer.status_line())
        }
    }

    fn bearer(&self) -> &str {
        self.token.as_deref().unwrap_or_default()
    }
}

/// The token to send: this deployment's, else the environment's.
///
/// `.env` is the deployment's own answer and wins. Outside a deployment - a
/// `chaps api --url` aimed at a server elsewhere - there is no `.env` to read,
/// and `CHAP_API_TOKEN` in the environment is the only place a token can come
/// from. Reading it as a fallback rather than an override means a project
/// directory always talks with its own credentials.
pub fn token_for(project_dir: Option<&std::path::Path>) -> Option<String> {
    let from_project = project_dir.and_then(crate::auth::token_in);
    let from_env = std::env::var(TOKEN_ENV_VAR).ok();
    pick_token(from_project, from_env)
}

/// [`token_for`]'s rule, with both sources handed in.
///
/// An empty or blank value is no token at all: `CHAP_API_TOKEN=` in a shell
/// means the same thing there as it does in `.env`, which is "unset".
fn pick_token(from_project: Option<String>, from_env: Option<String>) -> Option<String> {
    from_project
        .into_iter()
        .chain(from_env)
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

/// `method` in the spelling chap-core expects, or the usage error.
pub fn method_of(text: &str) -> Result<String> {
    let upper = text.trim().to_ascii_uppercase();
    if METHODS.contains(&upper.as_str()) {
        return Ok(upper);
    }
    Err(ChapError::Usage(format!(
        "`{text}` is not an HTTP method chaps sends; the methods are {}",
        METHODS.join(", ")
    ))
    .into())
}

/// A request path, checked before anything is sent.
///
/// A path has to start with `/`: `chaps api GET v1/jobs` would otherwise be
/// joined to the base URL as if it had, and a typo that silently works is a
/// typo that stays.
pub fn path_of(text: &str) -> Result<String> {
    let text = text.trim();
    if text.starts_with('/') {
        return Ok(text.to_string());
    }
    Err(ChapError::Usage(format!(
        "`{text}` is not a request path; a path starts with `/`, as in `/v1/jobs`"
    ))
    .into())
}

/// `name=value` with the characters a query string cannot carry escaped.
///
/// Written out rather than pulled in: the values are job statuses and job
/// types, and the six release targets keep the dependency set they have.
pub fn query_pair(name: &str, value: &str) -> String {
    format!("{}={}", encode(name), encode(value))
}

/// Percent-encode everything that is not unreserved, for one path segment or
/// one query value.
///
/// A job id is a UUID and needs none of this, but an id typed by hand is
/// whatever was typed, and a `/` in it would otherwise reach chap-core as a
/// path of its own.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Append `pairs` to `path` as a query string, keeping one it already carries.
pub fn with_query(path: &str, pairs: &[String]) -> String {
    if pairs.is_empty() {
        return path.to_string();
    }
    let separator = if path.contains('?') { '&' } else { '?' };
    format!("{path}{separator}{}", pairs.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(status: u16, body: &str) -> Answer {
        Answer {
            status,
            reason: "Not Found".to_string(),
            content_type: JSON.to_string(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn the_url_is_the_base_and_the_path_with_one_slash_between_them() {
        let api = Api::new("http://localhost:8140/", None, DEFAULT_TIMEOUT);
        assert_eq!(api.base(), "http://localhost:8140");
        assert_eq!(api.url("/v1/jobs"), "http://localhost:8140/v1/jobs");
        assert_eq!(api.url("v1/jobs"), "http://localhost:8140/v1/jobs");
        assert!(!api.has_token());
        assert!(Api::new("http://x", Some("t".into()), DEFAULT_TIMEOUT).has_token());
    }

    #[test]
    fn the_methods_are_the_five_and_case_does_not_matter() {
        for spelling in ["get", "GET", " Get "] {
            assert_eq!(method_of(spelling).unwrap(), "GET");
        }
        assert_eq!(method_of("delete").unwrap(), "DELETE");
        let err = method_of("BREW").expect_err("not a method chaps sends");
        assert!(err.to_string().contains("GET, POST, PUT, PATCH, DELETE"));
    }

    #[test]
    fn a_path_has_to_start_with_a_slash() {
        assert_eq!(path_of("/v1/jobs").unwrap(), "/v1/jobs");
        assert_eq!(path_of(" /v1/jobs?limit=2 ").unwrap(), "/v1/jobs?limit=2");
        let err = path_of("v1/jobs").expect_err("no leading slash");
        assert!(err.to_string().contains("starts with `/`"), "{err}");
    }

    #[test]
    fn a_status_line_names_the_code_and_the_reason() {
        assert_eq!(answer(404, "{}").status_line(), "HTTP 404 Not Found");
        let mut bare = answer(499, "{}");
        bare.reason = String::new();
        assert_eq!(bare.status_line(), "HTTP 499");
        assert!(!answer(404, "{}").is_success());
        let mut ok = answer(204, "");
        ok.status = 204;
        assert!(ok.is_success());
    }

    #[test]
    fn the_detail_field_is_what_chap_core_said() {
        assert_eq!(
            answer(404, r#"{"detail":"Job not found"}"#).detail(),
            "Job not found"
        );
        // A 422 answers with a list of problems rather than a sentence.
        assert_eq!(
            answer(422, r#"{"detail":[{"loc":["body"]}]}"#).detail(),
            r#"[{"loc":["body"]}]"#
        );
        // No `detail` at all: the body, trimmed.
        assert_eq!(answer(500, "  boom \n").detail(), "boom");
        // And something enormous is cut rather than printed into an error.
        let long = "x".repeat(500);
        let cut = answer(500, &long).detail();
        assert_eq!(cut.chars().count(), 203);
        assert!(cut.ends_with("..."));
    }

    #[test]
    fn a_json_body_reads_back_as_json_and_anything_else_does_not() {
        assert_eq!(
            answer(200, r#"{"id":"abc"}"#).json().unwrap()["id"],
            serde_json::json!("abc")
        );
        assert!(answer(200, "<html></html>").json().is_none());
        assert_eq!(answer(200, "  text ").text(), "  text ");
    }

    #[test]
    fn query_values_are_escaped_and_appended() {
        assert_eq!(query_pair("status", "SUCCESS"), "status=SUCCESS");
        assert_eq!(
            query_pair("type", "create backtest/1"),
            "type=create%20backtest%2F1"
        );
        assert_eq!(with_query("/v1/jobs", &[]), "/v1/jobs");
        assert_eq!(
            with_query("/v1/jobs", &[query_pair("status", "SUCCESS")]),
            "/v1/jobs?status=SUCCESS"
        );
        assert_eq!(
            with_query(
                "/v1/jobs?type=x",
                &[query_pair("status", "A"), query_pair("status", "B")]
            ),
            "/v1/jobs?type=x&status=A&status=B"
        );
    }

    /// Port 9 is the discard service, so this covers the transport-error path
    /// without a network.
    #[test]
    fn an_unreachable_api_is_the_error_status_uses() {
        let api = Api::new("http://127.0.0.1:9", None, Duration::from_secs(2));
        let err = api
            .send("GET", "/v1/jobs", None)
            .expect_err("nothing is listening on port 9");
        assert!(
            matches!(
                err.downcast_ref::<ChapError>(),
                Some(ChapError::Unreachable { .. })
            ),
            "{err}"
        );
        let text = err.to_string();
        assert!(text.contains("http://127.0.0.1:9"), "{text}");
        assert!(text.contains("is not responding"), "{text}");
        assert!(text.contains("chaps status"), "{text}");
    }

    #[test]
    fn the_deployments_token_wins_and_the_environment_is_the_fallback() {
        let project = || Some("from-env-file".to_string());
        let shell = || Some("from-shell".to_string());
        assert_eq!(
            pick_token(project(), shell()).as_deref(),
            Some("from-env-file")
        );
        assert_eq!(pick_token(None, shell()).as_deref(), Some("from-shell"));
        assert_eq!(
            pick_token(project(), None).as_deref(),
            Some("from-env-file")
        );
        assert_eq!(pick_token(None, None), None);
        // An empty value is no token, from either side.
        assert_eq!(
            pick_token(Some("  ".into()), shell()).as_deref(),
            Some("from-shell")
        );
        assert_eq!(pick_token(None, Some(String::new())), None);
        assert_eq!(
            pick_token(None, Some(" spaced ".into())).as_deref(),
            Some("spaced")
        );
    }
}
