//! Network fetch of the marketplace index and the model files it lists.
//!
//! Owned by agent A.
//!
//! The marketplace has no JSON API, so the catalogue is plain YAML served from
//! `raw.githubusercontent.com`: one request for the index, then one per model
//! file it lists. Requests are sequential on purpose — six small files over a
//! kept-alive connection is fast enough, and a failure is easier to attribute.

use crate::error::{ChapError, Result};
use crate::registry::{RegistryIndex, RegistryOptions, model_url};
use std::time::Duration;

/// `User-Agent` sent with every registry request, so the marketplace can tell
/// CLI traffic apart from browsers.
const USER_AGENT: &str = concat!("chaps-cli/", env!("CARGO_PKG_VERSION"));

/// Fetch `registry.yaml` and every model file it lists over HTTP.
///
/// Returns `(index_yaml, model_files)` where each model file is
/// `(relative path as listed in the index, contents)`.
///
/// Contract: uses `ureq` with `opts.timeout`, resolves model URLs through
/// [`crate::registry::model_url`], and surfaces non-2xx responses as
/// [`crate::error::ChapError::Http`].
pub fn fetch_registry(opts: &RegistryOptions) -> Result<(String, Vec<(String, String)>)> {
    let agent = agent(opts.timeout);

    let index_yaml = get(&agent, &opts.url)?;
    // Parsed here as well as in `Registry::parse` because the list of model
    // files is what says which URLs to fetch next.
    let index: RegistryIndex = serde_yaml_ng::from_str(&index_yaml)
        .map_err(|e| anyhow::anyhow!("{}: invalid registry index: {e}", opts.url))?;

    let mut files = Vec::with_capacity(index.models.len());
    for rel in &index.models {
        let url = model_url(&opts.url, rel);
        files.push((rel.clone(), get(&agent, &url)?));
    }
    Ok((index_yaml, files))
}

/// A blocking agent whose global timeout covers connect, send and receive, so
/// a stalled read cannot hang the CLI past `timeout`.
fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(USER_AGENT)
            .build(),
    )
}

/// GET `url` and read the body as UTF-8 text.
fn get(agent: &ureq::Agent, url: &str) -> Result<String> {
    let started = std::time::Instant::now();
    let mut response = agent.get(url).call().map_err(|e| {
        crate::output::verbose(&format!(
            "GET {url} -> failed in {}ms",
            started.elapsed().as_millis()
        ));
        map_error(url, e)
    })?;
    crate::output::verbose(&format!(
        "GET {url} -> {} in {}ms",
        response.status().as_u16(),
        started.elapsed().as_millis()
    ));
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("reading the response body from {url}: {e}"))?;
    crate::output::debug(&format!("  {}", crate::output::trace_body(&body)));
    Ok(body)
}

/// Turn a ureq failure into a `chaps` error.
///
/// ureq 3 reports a non-2xx response as [`ureq::Error::StatusCode`] rather
/// than as a successful response, so that is where [`ChapError::Http`] comes
/// from; everything else is a transport problem and keeps the URL as context.
fn map_error(url: &str, err: ureq::Error) -> anyhow::Error {
    match err {
        ureq::Error::StatusCode(status) => ChapError::Http {
            url: url.to_string(),
            status,
        }
        .into(),
        other => anyhow::anyhow!("fetching {url}: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port 9 is the discard service and is not listening on a developer
    /// machine, so this exercises the transport-error path without a network.
    const UNREACHABLE: &str = "http://127.0.0.1:9/registry.yaml";

    #[test]
    fn a_status_code_becomes_a_typed_http_error() {
        let err = map_error(
            "https://example.test/registry.yaml",
            ureq::Error::StatusCode(404),
        );
        match err.downcast_ref::<ChapError>() {
            Some(ChapError::Http { url, status }) => {
                assert_eq!(url, "https://example.test/registry.yaml");
                assert_eq!(*status, 404);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn a_transport_failure_keeps_the_url_as_context() {
        let err = map_error(
            "https://example.test/registry.yaml",
            ureq::Error::HostNotFound,
        );
        assert!(err.downcast_ref::<ChapError>().is_none());
        assert!(
            err.to_string()
                .contains("https://example.test/registry.yaml")
        );
    }

    #[test]
    fn an_unreachable_registry_is_an_error_not_a_hang() {
        let opts = RegistryOptions {
            url: UNREACHABLE.to_string(),
            timeout: Duration::from_secs(2),
            ..RegistryOptions::default()
        };
        let err = fetch_registry(&opts).expect_err("nothing is listening on port 9");
        assert!(err.to_string().contains(UNREACHABLE), "{err}");
    }
}
