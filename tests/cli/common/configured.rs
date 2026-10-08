//! The configured-model routes of the stand-in chap-core in the shape of
//! chap-core 2.4 ([`fresh_chap_core_server`]).

use super::*;
use serde_json::Value as Json;

/// The id of the first template [`fresh_chap_core_server`] stores; the
/// others follow in the order of the three stand-in models.
pub(crate) const TEMPLATE_IDS: i64 = 30;

/// The id of the first configured model a client posts there.
pub(crate) const CREATED_IDS: i64 = 100;

/// The template the stand-in stores for `service`, in chap-core's camelCase:
/// the user options of a chapkit config schema, one of each kind.
pub(crate) fn template_json(id: i64, service: &str) -> Json {
    serde_json::json!({
        "id": id,
        "name": service,
        "version": "1.0.1",
        "displayName": "Stand-in model",
        "requiredCovariates": ["population"],
        "allowFreeAdditionalContinuousCovariates": service != FAILING_MODEL,
        "userOptions": {
            "n_lags": {"type": "integer", "default": 3, "description": "Lags of the target."},
            "precision": {"type": "number", "default": 0.5},
            "seasonal": {"type": "boolean", "default": true},
            "method": {"enum": ["fast", "exact"], "type": "string", "default": "fast"},
            "lags": {"type": "array", "items": {"type": "integer"}, "default": [1, 2]},
            "label": {"anyOf": [{"type": "string"}, {"type": "null"}], "default": null}
        }
    })
}

/// The id of the template the stand-in stores for `service`.
fn template_id(service: &str) -> i64 {
    TEMPLATE_IDS
        + [
            PASSING_MODEL,
            FAILING_MODEL,
            OLD_CHAPKIT_MODEL,
            HOST_RUN_MODEL,
        ]
        .iter()
        .position(|known| *known == service)
        .unwrap_or(9) as i64
}

/// `from-service` and the configured models, on a 2.4 stand-in only.
pub(crate) fn configured_route(
    method: &str,
    route: &str,
    body: &str,
    recorded: &mut Recorded,
) -> Option<(u16, &'static str, String)> {
    if !recorded.fresh {
        return None;
    }
    let json = "application/json";
    match (method, route) {
        ("GET", "/v1/crud/configured-models") => Some((
            200,
            json,
            serde_json::to_string(&recorded.created).expect("JSON"),
        )),
        ("POST", "/v1/crud/model-templates/from-service") => {
            let sent: Json = serde_json::from_str(body).unwrap_or(Json::Null);
            let service = sent["service_id"].as_str().unwrap_or_default().to_string();
            if service == OLD_CHAPKIT_MODEL {
                return Some((
                    409,
                    json,
                    r#"{"detail":"the service reports no source revision"}"#.to_string(),
                ));
            }
            if ![PASSING_MODEL, FAILING_MODEL, HOST_RUN_MODEL].contains(&service.as_str()) {
                return Some((
                    404,
                    json,
                    format!(r#"{{"detail":"Service '{service}' not found"}}"#),
                ));
            }
            recorded.templates.push(service.clone());
            Some((
                200,
                json,
                template_json(template_id(&service), &service).to_string(),
            ))
        }
        // The live templates: discovery stores one for every registered
        // service that reports a revision.
        ("GET", "/v1/crud/model-templates") => {
            let rows: Vec<Json> = [PASSING_MODEL, FAILING_MODEL, HOST_RUN_MODEL]
                .iter()
                .map(|service| template_json(template_id(service), service))
                .collect();
            Some((200, json, Json::Array(rows).to_string()))
        }
        // chap-core archives; it never deletes a configured model.
        ("DELETE", _) if route.starts_with("/v1/crud/configured-models/") => {
            let id: i64 = route
                .rsplit('/')
                .next()
                .and_then(|id| id.parse().ok())
                .unwrap_or_default();
            recorded.deleted.push(route.to_string());
            match recorded.created.iter_mut().find(|row| row["id"] == id) {
                Some(row) => {
                    row["archived"] = Json::Bool(true);
                    Some((200, json, r#"{"message":"deleted"}"#.to_string()))
                }
                None => Some((
                    404,
                    json,
                    r#"{"detail":"Configured model not found"}"#.to_string(),
                )),
            }
        }
        ("POST", "/v1/crud/configured-models") => {
            let sent: Json = serde_json::from_str(body).unwrap_or(Json::Null);
            let template = sent["model_template_id"].as_i64().unwrap_or_default();
            let service = [
                PASSING_MODEL,
                FAILING_MODEL,
                OLD_CHAPKIT_MODEL,
                HOST_RUN_MODEL,
            ]
            .get((template - TEMPLATE_IDS) as usize)
            .copied()
            .unwrap_or_default();
            let name = match sent["name"].as_str().unwrap_or_default() {
                "default" => service.to_string(),
                config => format!("{service}:{config}"),
            };
            let id = CREATED_IDS + recorded.created.len() as i64;
            let row = serde_json::json!({
                "id": id,
                "name": name,
                "version": "1.0.1",
                "archived": false,
                "usesChapkit": true,
                "additionalContinuousCovariates": sent["additional_continuous_covariates"],
                "userOptionValues": sent["user_option_values"],
            });
            recorded.created.push(row.clone());
            recorded.configured_posts.push(sent);
            Some((200, json, row.to_string()))
        }
        _ => None,
    }
}
