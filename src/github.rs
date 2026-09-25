//! The one door to GitHub's REST API.
//!
//! Three parts of this CLI ask GitHub questions - [`crate::chapcore`] for
//! chap-core's releases, [`crate::manual::github`] for a model repository's
//! branch and its commits, and [`crate::selfupdate`] for `chaps`' own
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
//! address - uses up in a morning. A token raises that to 5000. `chaps` never
//! stores one and never prints one: a `-v` trace says the header went out and
//! nothing more, exactly as [`crate::api`] does with chap-core's token.

use crate::error::{ChapError, Result};
use std::time::Duration;

/// The variable the REST base URL is moved with, for the tests.
pub const API_VAR: &str = "CHAPS_GITHUB_API";

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
const USER_AGENT: &str = concat!("chaps-cli/", env!("CARGO_PKG_VERSION"));

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
mod tests {
    use super::*;

    #[test]
    fn the_token_is_the_first_variable_with_something_in_it() {
        assert_eq!(
            token_of(&[Some("ghp_one"), Some("ghp_two")]).as_deref(),
            Some("ghp_one")
        );
        // GITHUB_TOKEN unset, GH_TOKEN set: the `gh` CLI's variable.
        assert_eq!(
            token_of(&[None, Some("ghp_two")]).as_deref(),
            Some("ghp_two")
        );
        // Set to nothing is not set: a CI secret that did not resolve must
        // not send an empty Bearer.
        assert_eq!(
            token_of(&[Some(""), Some("ghp_two")]).as_deref(),
            Some("ghp_two")
        );
        assert_eq!(token_of(&[Some("   "), None]), None);
        assert_eq!(token_of(&[None, None]), None);
        assert_eq!(token_of(&[]), None);
        // Whitespace around a value pasted out of a terminal is not part of it.
        assert_eq!(token_of(&[Some(" ghp_one\n")]).as_deref(), Some("ghp_one"));
    }

    #[test]
    fn every_request_carries_the_media_type_and_the_api_version() {
        let anonymous = headers(None);
        assert_eq!(
            anonymous,
            vec![
                ("Accept", ACCEPT.to_string()),
                ("X-GitHub-Api-Version", "2022-11-28".to_string()),
            ]
        );
        assert!(
            !anonymous.iter().any(|(name, _)| *name == "Authorization"),
            "no token, no Authorization header"
        );

        let authenticated = headers(Some("ghp_secret"));
        assert_eq!(
            authenticated.last(),
            Some(&("Authorization", "Bearer ghp_secret".to_string()))
        );
        // The other two are still there, unchanged.
        assert_eq!(&authenticated[..2], &anonymous[..]);
    }

    #[test]
    fn the_base_url_falls_back_to_public_github() {
        assert_eq!(
            base_of(Some("http://127.0.0.1:18099/".into()), DEFAULT_API),
            "http://127.0.0.1:18099"
        );
        assert_eq!(base_of(None, DEFAULT_API), DEFAULT_API);
        assert_eq!(base_of(Some(String::new()), DEFAULT_API), DEFAULT_API);
        assert_eq!(base_of(Some("  ".into()), DEFAULT_API), DEFAULT_API);
        assert!(rate_limit_url().ends_with("/rate_limit"));
    }

    /// The whole point of the module: a 403 that is the anonymous limit says
    /// so, and says what to do about it.
    #[test]
    fn a_used_up_anonymous_limit_says_what_it_is_and_what_to_do() {
        // 2026-09-25T13:04:00Z, whatever this machine calls that hour.
        let reset = 1_758_805_440;
        let text = refusal(403, Some(0), Some(reset), false).expect("a rate limit reads as one");
        assert!(
            text.starts_with(
                "GitHub's rate limit is used up (unauthenticated, 60 requests an hour per \
                 address); set GITHUB_TOKEN for 5000, or wait until "
            ),
            "{text}"
        );
        assert!(text.ends_with(&crate::output::local_clock(reset)), "{text}");

        // A secondary limit arrives as a 429 and is the same sentence.
        assert_eq!(
            refusal(429, Some(0), Some(reset), false),
            refusal(403, Some(0), Some(reset), false)
        );

        // No reset header: the sentence still stands, with nothing to wait for.
        let text = refusal(403, Some(0), None, false).expect("still the limit");
        assert!(text.ends_with("or wait until -"), "{text}");
    }

    #[test]
    fn a_403_with_a_token_points_at_the_token() {
        let text = refusal(403, Some(0), Some(1_758_805_440), true).expect("a refusal");
        assert_eq!(text, REFUSED_WITH_TOKEN);
        assert!(text.contains("GITHUB_TOKEN's scopes"), "{text}");
        // Not the anonymous advice, which would be wrong here.
        assert!(!text.contains("set GITHUB_TOKEN for"), "{text}");
        // The quota headers make no difference once a token was sent.
        assert_eq!(
            refusal(403, None, None, true).as_deref(),
            Some(REFUSED_WITH_TOKEN)
        );
    }

    /// Everything else keeps the wording it always had, which is the bare
    /// `HTTP <status> from <url>` the caller builds.
    #[test]
    fn every_other_status_says_only_its_number() {
        assert_eq!(refusal(404, Some(59), None, false), None);
        assert_eq!(refusal(500, None, None, true), None);
        // A 403 that is not the limit - requests left, or no header at all -
        // is not the rate limit and must not claim to be.
        assert_eq!(refusal(403, Some(42), None, false), None);
        assert_eq!(refusal(403, None, None, false), None);
        assert_eq!(refusal(429, Some(7), None, false), None);
    }

    /// The error a caller sees: the sentence where there is one, the typed
    /// [`ChapError::Http`] otherwise, because `release_exists` matches on it.
    #[test]
    fn the_answer_turns_into_the_error_its_status_deserves() {
        let url = "https://api.github.com/repos/dhis2-chap/chap-core/releases/latest";
        let limited = Answer {
            status: 403,
            body: r#"{"message":"API rate limit exceeded"}"#.to_string(),
            rate: RateLimit {
                limit: Some(60),
                remaining: Some(0),
                reset: Some(1_758_805_440),
            },
            token: false,
        };
        let err = limited.error(url);
        assert!(err.to_string().contains("rate limit is used up"), "{err}");
        assert!(err.downcast_ref::<ChapError>().is_none(), "{err}");

        let missing = Answer {
            status: 404,
            body: String::new(),
            rate: RateLimit::default(),
            token: false,
        };
        match missing.error(url).downcast_ref::<ChapError>() {
            Some(ChapError::Http { status, .. }) => assert_eq!(*status, 404),
            other => panic!("a 404 has to stay a typed HTTP error: {other:?}"),
        }
        assert!(!missing.ok());
        assert!(
            Answer {
                status: 200,
                body: String::new(),
                rate: RateLimit::default(),
                token: true,
            }
            .ok()
        );
    }

    #[test]
    fn the_quota_payload_yields_the_core_bucket() {
        const BODY: &str = r#"{
          "resources": {
            "core": {"limit": 5000, "remaining": 4990, "reset": 1758805440, "used": 10},
            "search": {"limit": 30, "remaining": 30, "reset": 1758805440}
          },
          "rate": {"limit": 5000, "remaining": 4990, "reset": 1758805440}
        }"#;
        assert_eq!(parse_quota(BODY), Some((5000, 4990, Some(1_758_805_440))));

        // Only the older `rate` field: still an answer.
        assert_eq!(
            parse_quota(r#"{"rate":{"limit":60,"remaining":43,"reset":1758805440}}"#),
            Some((60, 43, Some(1_758_805_440)))
        );
        // Nothing to read is no quota rather than a wrong one.
        assert_eq!(parse_quota(r#"{"message":"Not Found"}"#), None);
        assert_eq!(parse_quota("<html>"), None);
        assert_eq!(parse_quota(r#"{"rate":{"limit":60}}"#), None);
    }

    /// Port 9 is the discard service, so this covers the transport-error path
    /// without a network.
    #[test]
    fn an_unreachable_host_is_an_error_not_a_hang() {
        let err = get_ok("http://127.0.0.1:9/rate_limit", Duration::from_secs(2))
            .expect_err("nothing is listening on port 9");
        assert!(err.to_string().contains("127.0.0.1:9"), "{err}");
    }
}
