//! `chaps status`: chap-core health plus the services it has registered.
//!
//! Owned by agent C.
//!
//! Everything here is deliberately lenient: chap-core may add fields, rename
//! optional ones or answer with an empty body, and none of that should turn
//! `chaps status` into a crash. An unreachable API is data, not an error.

use crate::project::Project;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Path of the chap-core health endpoint.
pub const HEALTH_PATH: &str = "/health";
/// Path of the service registry chapkit models register themselves with.
pub const SERVICES_PATH: &str = "/v2/services";

/// What `chaps status` reports.
#[derive(Debug, Serialize)]
pub struct StatusReport {
    pub api_url: String,
    pub api: ApiHealth,
    /// Services chap-core currently knows about, from `/v2/services`.
    pub registered: Vec<RegisteredService>,
    /// Service ids the project expects to be registered.
    pub expected: Vec<String>,
    /// Expected ids that are not registered.
    pub missing: Vec<String>,
}

impl StatusReport {
    /// Whether the API answered its health check.
    #[cfg(test)]
    pub fn is_up(&self) -> bool {
        matches!(self.api, ApiHealth::Up { .. })
    }

    /// Whether everything the project enabled has registered.
    #[cfg(test)]
    pub fn is_complete(&self) -> bool {
        self.is_up() && self.missing.is_empty()
    }
}

/// Result of `GET /health`.
#[derive(Debug, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum ApiHealth {
    Up { status: String, message: String },
    Down { error: String },
}

/// One entry of `GET /v2/services`.
#[derive(Debug, Serialize, Deserialize)]
pub struct RegisteredService {
    pub id: String,
    pub url: String,
    pub display_name: String,
    pub version: String,
    pub last_ping_at: String,
    pub expires_at: String,
}

/// Probe chap-core and diff the registered services against the project.
///
/// Never fails: an unreachable API is reported as [`ApiHealth::Down`].
pub fn status(project: &Project, api_url: &str, timeout: Duration) -> StatusReport {
    let base = api_url.trim_end_matches('/').to_string();
    let mut expected: Vec<String> = project
        .state
        .models
        .values()
        .map(|m| m.service_id.clone())
        .collect();
    expected.sort();
    expected.dedup();

    let agent = agent(timeout);
    let api = match get_text(&agent, &format!("{base}{HEALTH_PATH}")) {
        Ok(body) => parse_health(&body),
        Err(error) => ApiHealth::Down { error },
    };

    // Only ask for the service list when the API answered at all: otherwise we
    // would wait out a second connection timeout to learn the same thing.
    let registered = if matches!(api, ApiHealth::Up { .. }) {
        match get_text(&agent, &format!("{base}{SERVICES_PATH}")) {
            Ok(body) => parse_services(&body).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let missing = missing_ids(&expected, &registered);
    StatusReport {
        api_url: base,
        api,
        registered,
        expected,
        missing,
    }
}

/// Expected service ids that nothing has registered.
pub fn missing_ids(expected: &[String], registered: &[RegisteredService]) -> Vec<String> {
    expected
        .iter()
        .filter(|id| !registered.iter().any(|s| &&s.id == id))
        .cloned()
        .collect()
}

/// Parse a `/health` body. An unparseable body still counts as up: the service
/// answered, which is what the caller asked about.
pub fn parse_health(body: &str) -> ApiHealth {
    let wire: HealthBody = serde_json::from_str(body).unwrap_or_default();
    ApiHealth::Up {
        status: if wire.status.is_empty() {
            "ok".to_string()
        } else {
            wire.status
        },
        message: wire.message,
    }
}

/// Parse a `/v2/services` body into the flattened report entries.
pub fn parse_services(body: &str) -> Result<Vec<RegisteredService>, serde_json::Error> {
    let wire: ServicesBody = serde_json::from_str(body)?;
    Ok(wire.services.into_iter().map(flatten).collect())
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

/// `GET url`, returning the body text or a human-readable failure.
fn get_text(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {} from {url}", status.as_u16()));
    }
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        // Status codes are reported by the caller, not raised as errors.
        .http_status_as_error(false)
        .user_agent(concat!("chaps-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

#[derive(Debug, Default, Deserialize)]
struct HealthBody {
    #[serde(default)]
    status: String,
    #[serde(default)]
    message: String,
}

/// `GET /v2/services`. Fields the report does not use (`count`,
/// `registered_at`, everything chap-core adds later) are ignored.
#[derive(Debug, Default, Deserialize)]
struct ServicesBody {
    #[serde(default)]
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

#[cfg(test)]
mod tests {
    use super::*;

    const SERVICES: &str = r#"{
      "count": 2,
      "services": [
        {
          "id": "chapkit-ewars-model",
          "url": "http://chapkit-ewars-model:8000",
          "info": {
            "id": "chapkit-ewars-model",
            "display_name": "CHAP-EWARS",
            "version": "1.0.0",
            "author": "someone",
            "unknown_future_field": 42
          },
          "registered_at": "2026-09-22T09:00:00Z",
          "last_ping_at": "2026-09-22T09:04:30Z",
          "expires_at": "2026-09-22T09:09:30Z"
        },
        {
          "id": "auto-arima-chapkit",
          "url": "http://auto-arima-chapkit:8000",
          "info": { "id": "auto-arima-chapkit", "display_name": "Auto ARIMA", "version": "1.2.0" },
          "registered_at": "2026-09-22T09:01:00Z",
          "last_ping_at": "2026-09-22T09:04:31Z",
          "expires_at": "2026-09-22T09:09:31Z"
        }
      ]
    }"#;

    #[test]
    fn services_payload_is_flattened() {
        let services = parse_services(SERVICES).expect("sample payload parses");
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].id, "chapkit-ewars-model");
        assert_eq!(services[0].display_name, "CHAP-EWARS");
        assert_eq!(services[0].version, "1.0.0");
        assert_eq!(services[0].url, "http://chapkit-ewars-model:8000");
        assert_eq!(services[0].last_ping_at, "2026-09-22T09:04:30Z");
        assert_eq!(services[0].expires_at, "2026-09-22T09:09:30Z");
        assert_eq!(services[1].version, "1.2.0");
    }

    #[test]
    fn services_payload_tolerates_missing_fields() {
        let services = parse_services(r#"{"services":[{"id":"x"}]}"#).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].id, "x");
        // Without an info block the id doubles as the display name.
        assert_eq!(services[0].display_name, "x");
        assert!(services[0].version.is_empty());
        assert!(services[0].url.is_empty());
    }

    #[test]
    fn services_payload_falls_back_to_the_info_id() {
        let services = parse_services(r#"{"count":1,"services":[{"info":{"id":"y"}}]}"#).unwrap();
        assert_eq!(services[0].id, "y");
        assert_eq!(services[0].display_name, "y");
    }

    #[test]
    fn an_empty_registry_parses_to_nothing() {
        assert!(
            parse_services(r#"{"count":0,"services":[]}"#)
                .unwrap()
                .is_empty()
        );
        assert!(parse_services("{}").unwrap().is_empty());
    }

    #[test]
    fn a_non_json_services_body_is_an_error_not_a_panic() {
        assert!(parse_services("<html>nope</html>").is_err());
    }

    #[test]
    fn health_payload_is_read_leniently() {
        let ApiHealth::Up { status, message } =
            parse_health(r#"{"status":"ok","message":"CHAP is running"}"#)
        else {
            panic!("expected up");
        };
        assert_eq!(status, "ok");
        assert_eq!(message, "CHAP is running");

        let ApiHealth::Up { status, message } = parse_health("{}") else {
            panic!("expected up");
        };
        assert_eq!(status, "ok", "an empty body still means the API answered");
        assert!(message.is_empty());

        let ApiHealth::Up { status, .. } = parse_health("not json at all") else {
            panic!("expected up");
        };
        assert_eq!(status, "ok");
    }

    #[test]
    fn missing_is_expected_minus_registered() {
        let registered = parse_services(SERVICES).unwrap();
        let expected = vec![
            "auto-arima-chapkit".to_string(),
            "chapkit-ewars-model".to_string(),
            "chapkit-simple-multistep-model".to_string(),
        ];
        assert_eq!(
            missing_ids(&expected, &registered),
            vec!["chapkit-simple-multistep-model".to_string()]
        );
        assert!(missing_ids(&[], &registered).is_empty());
        assert_eq!(missing_ids(&expected, &[]), expected);
    }

    #[test]
    fn report_helpers_describe_the_state() {
        let report = StatusReport {
            api_url: "http://localhost:8000".into(),
            api: ApiHealth::Down {
                error: "connection refused".into(),
            },
            registered: Vec::new(),
            expected: vec!["a".into()],
            missing: vec!["a".into()],
        };
        assert!(!report.is_up());
        assert!(!report.is_complete());

        let report = StatusReport {
            api: ApiHealth::Up {
                status: "ok".into(),
                message: String::new(),
            },
            missing: Vec::new(),
            ..report
        };
        assert!(report.is_up());
        assert!(report.is_complete());
    }

    #[test]
    fn a_down_api_serialises_with_its_state_tag() {
        let value = serde_json::to_value(ApiHealth::Down {
            error: "connection refused".into(),
        })
        .unwrap();
        assert_eq!(value["state"], "down");
        assert_eq!(value["error"], "connection refused");
    }
}
