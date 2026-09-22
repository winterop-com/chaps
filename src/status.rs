//! `chap status`: chap-core health plus the services it has registered.
//!
//! Owned by agent C.

use crate::project::Project;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// What `chap status` reports.
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
///
/// Owned by agent C.
pub fn status(project: &Project, api_url: &str, _timeout: Duration) -> StatusReport {
    let expected: Vec<String> = project
        .state
        .models
        .values()
        .map(|m| m.service_id.clone())
        .collect();
    StatusReport {
        api_url: api_url.to_string(),
        api: ApiHealth::Down {
            error: "status probing is not implemented yet".to_string(),
        },
        registered: Vec::new(),
        missing: expected.clone(),
        expected,
    }
}
