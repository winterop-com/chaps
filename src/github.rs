//! The one door to GitHub's REST API.
//!
//! Three parts of this CLI ask GitHub questions - [`crate::chapcore`] for
//! chap-core's releases, [`crate::manual::github`] for a model repository's
//! branch and its commits, and [`crate::selfupdate`] for `varde`' own
//! releases - and each of them used to build its own request. They all come
//! through here now, so that every `api.github.com` call carries the same
//! headers, the same token where this run has one, and the same reading of a
//! rate limit that has run out.
//!
//! `raw.githubusercontent.com` is deliberately not part of this. It serves
//! files, counts against no limit and needs no credential, so the compose
//! file and the marketplace index are fetched where they always were.
//!
//! Unauthenticated GitHub allows 60 requests an hour per address, which one
//! busy operator - or one CI runner shared with everybody else on its egress
//! address - uses up in a morning. A token raises that to 5000. `varde` never
//! stores one and never prints one: a `-v` trace says the header went out and
//! nothing more, exactly as [`crate::api`] does with chap-core's token.

use crate::error::{ChapError, Result};
use std::time::Duration;

/// The variable the REST base URL is moved with, for the tests.
pub const API_VAR: &str = "VARDE_GITHUB_API";

/// Public GitHub, unless [`API_VAR`] points somewhere else.
pub const DEFAULT_API: &str = "https://api.github.com";

/// The variables a token is read from, in order: the first one with a
/// non-empty value wins. `GITHUB_TOKEN` is what GitHub Actions sets by
/// itself; `GH_TOKEN` is what the `gh` CLI and its `gh auth token` use.
pub const TOKEN_VARS: [&str; 2] = ["GITHUB_TOKEN", "GH_TOKEN"];

/// The media type the REST API answers best in.
pub const ACCEPT: &str = "application/vnd.github+json";

/// The REST API version every request pins itself to, so a future default
/// cannot change an answer under us.
pub const API_VERSION: &str = "2022-11-28";

/// The header [`API_VERSION`] is sent in.
pub const API_VERSION_HEADER: &str = "X-GitHub-Api-Version";

/// `User-Agent` sent with every request, matching the registry fetch.
const USER_AGENT: &str = concat!("varde/", env!("CARGO_PKG_VERSION"));

/// Requests an hour GitHub gives one unauthenticated address.
pub const ANON_LIMIT: u64 = 60;

/// Requests an hour GitHub gives a token.
pub const TOKEN_LIMIT: u64 = 5000;

/// What a 403 with a token that GitHub still refused reads as.
///
/// A token that is expired, revoked or scoped to nothing reaches GitHub and
/// is turned away like any other; so does a token whose own 5000 are gone.
/// The three are one sentence here because the way out of all of them is to
/// look at the token.
pub const REFUSED_WITH_TOKEN: &str = "GitHub refused the request (HTTP 403 with a token; \
     check GITHUB_TOKEN's scopes or the rate limit)";

/// Base of the GitHub REST API this run talks to.
pub fn api_base() -> String {
    base(API_VAR, DEFAULT_API)
}

/// A base URL from `var`, falling back to `default`.
pub fn base(var: &str, default: &str) -> String {
    base_of(std::env::var(var).ok(), default)
}

/// [`base`] with the variable already read, so the rule is testable without
/// touching the environment of a process running tests in parallel.
pub fn base_of(value: Option<String>, default: &str) -> String {
    value
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// The token this run sends, or `None` when neither variable names one.
pub fn token() -> Option<String> {
    let read: Vec<Option<String>> = TOKEN_VARS
        .iter()
        .map(|var| std::env::var(var).ok())
        .collect();
    token_of(&read.iter().map(Option::as_deref).collect::<Vec<_>>())
}

/// [`token`] with the variables already read, in [`TOKEN_VARS`] order.
///
/// An empty or blank value is no token at all: `GITHUB_TOKEN=` in a CI job
/// that could not resolve a secret must fall through to `GH_TOKEN`, not send
/// an `Authorization: Bearer` header with nothing behind it.
pub fn token_of(values: &[Option<&str>]) -> Option<String> {
    values
        .iter()
        .flatten()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

/// The headers one GitHub API request carries, in the order they are sent.
///
/// `User-Agent` is not here: it is set on the agent, the way every other
/// fetch in this CLI sets it.
pub fn headers(token: Option<&str>) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("Accept", ACCEPT.to_string()),
        (API_VERSION_HEADER, API_VERSION.to_string()),
    ];
    if let Some(token) = token {
        headers.push(("Authorization", format!("Bearer {token}")));
    }
    headers
}

/// What the `x-ratelimit-*` headers of one answer said.
///
/// Every field is optional because every one of them is absent from an answer
/// that did not come from GitHub itself - a proxy, a captive portal, or the
/// local stand-in the tests run against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RateLimit {
    /// Requests this address is allowed in the window.
    pub limit: Option<u64>,
    /// Requests left in it.
    pub remaining: Option<u64>,
    /// Unix seconds the window resets at.
    pub reset: Option<u64>,
}

/// One answer from the REST API: any status, with what it said about the
/// rate limit and whether this request carried a token.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    pub body: String,
    pub rate: RateLimit,
    /// Whether an `Authorization` header went out with the request.
    pub token: bool,
}

impl Answer {
    /// Whether GitHub answered the question rather than refusing it.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The error a non-2xx answer is: the sentence a refusal deserves, or the
    /// bare status where the number is all there is to say.
    pub fn error(&self, url: &str) -> anyhow::Error {
        match refusal(
            self.status,
            self.rate.remaining,
            self.rate.reset,
            self.token,
        ) {
            Some(text) => anyhow::anyhow!(text),
            None => ChapError::Http {
                url: url.to_string(),
                status: self.status,
            }
            .into(),
        }
    }
}

/// What a GitHub refusal reads as, or `None` when its status number is the
/// whole of what happened.
///
/// Two cases are worth a sentence of their own. A 403 or 429 with no requests
/// left is the rate limit, and the way out of it is a token or the clock. A
/// 403 that arrives *with* a token is not the anonymous limit at all, so the
/// advice to set one would be wrong; what is worth checking then is the token
/// itself.
pub fn refusal(
    status: u16,
    remaining: Option<u64>,
    reset: Option<u64>,
    token: bool,
) -> Option<String> {
    match (status, token) {
        (403, true) => Some(REFUSED_WITH_TOKEN.to_string()),
        (403 | 429, false) if remaining == Some(0) => Some(format!(
            "GitHub's rate limit is used up (unauthenticated, {ANON_LIMIT} requests an hour per \
             address); set GITHUB_TOKEN for {TOKEN_LIMIT}, or wait until {}",
            reset_clock(reset)
        )),
        _ => None,
    }
}

/// The reset of a rate-limit window as a clock a reader can compare with
/// theirs, or `-` when the answer carried no reset at all.
pub fn reset_clock(reset: Option<u64>) -> String {
    match reset {
        Some(reset) => crate::output::local_clock(reset),
        None => "-".to_string(),
    }
}

/// GET a GitHub REST URL with the standard headers and, where this run has
/// one, the token.
///
/// Any status is an answer, not a failure: the rate-limit headers live on the
/// 403 that reports the limit, and a 404 is how "there is no such release" is
/// spelled. Only a transport failure is an `Err`.
pub fn get(url: &str, timeout: Duration) -> Result<Answer> {
    let token = token();
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(USER_AGENT)
            // A status code is an answer: dropping the 403 would drop the
            // headers that say why it is one.
            .http_status_as_error(false)
            .build(),
    );
    let mut request = agent.get(url);
    for (name, value) in headers(token.as_deref()) {
        request = request.header(name, value);
    }

    let started = std::time::Instant::now();
    let mut response = request.call().map_err(|e| {
        crate::output::verbose(&format!(
            "GET {url} -> failed in {}ms",
            started.elapsed().as_millis()
        ));
        anyhow::anyhow!("fetching {url}: {e}")
    })?;
    let status = response.status().as_u16();
    crate::output::verbose(&format!(
        "GET {url} -> {status} in {}ms",
        started.elapsed().as_millis()
    ));
    if token.is_some() {
        // Never the value: a trace line ends up in bug reports.
        crate::output::verbose("  Authorization: Bearer <token>");
    }
    let rate = {
        let number = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
        };
        RateLimit {
            limit: number("x-ratelimit-limit"),
            remaining: number("x-ratelimit-remaining"),
            reset: number("x-ratelimit-reset"),
        }
    };
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("reading the response body from {url}: {e}"))?;
    crate::output::debug(&format!("  {}", crate::output::trace_body(&body)));
    Ok(Answer {
        status,
        body,
        rate,
        token: token.is_some(),
    })
}

/// [`get`], failing on anything but a 2xx.
pub fn get_ok(url: &str, timeout: Duration) -> Result<String> {
    let answer = get(url, timeout)?;
    match answer.ok() {
        true => Ok(answer.body),
        false => Err(answer.error(url)),
    }
}

// ---------------------------------------------------------------------------
// The quota itself
// ---------------------------------------------------------------------------

/// What this address has left of its hour, as `GET /rate_limit` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quota {
    /// Requests allowed in the window.
    pub limit: u64,
    /// Requests left in it.
    pub remaining: u64,
    /// Unix seconds the window resets at, where the answer named one.
    pub reset: Option<u64>,
    /// Whether the request that asked carried a token, which is what decides
    /// which of the two limits this is.
    pub token: bool,
}

/// The endpoint that reports the quota. It is documented as not counting
/// against it, which is what makes it safe for a checklist.
pub fn rate_limit_url() -> String {
    format!("{}/rate_limit", api_base())
}

/// Ask GitHub what is left of this address's hour.
pub fn quota(timeout: Duration) -> Result<Quota> {
    let url = rate_limit_url();
    let answer = get(&url, timeout)?;
    if !answer.ok() {
        return Err(answer.error(&url));
    }
    let (limit, remaining, reset) = parse_quota(&answer.body)
        .ok_or_else(|| anyhow::anyhow!("{url} did not answer with a rate limit"))?;
    Ok(Quota {
        limit,
        remaining,
        reset,
        token: answer.token,
    })
}

/// `(limit, remaining, reset)` out of a `/rate_limit` payload.
///
/// `resources.core` is the bucket every call this CLI makes is counted in;
/// `rate` is the same numbers under the older name, read as a fallback so an
/// enterprise instance that omits one of them still answers.
pub fn parse_quota(body: &str) -> Option<(u64, u64, Option<u64>)> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let bucket = value
        .get("resources")
        .and_then(|resources| resources.get("core"))
        .or_else(|| value.get("rate"))?;
    let number = |name: &str| bucket.get(name).and_then(serde_json::Value::as_u64);
    Some((number("limit")?, number("remaining")?, number("reset")))
}

#[cfg(test)]
mod tests;
