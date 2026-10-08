//! The configured-model routes of the stand-in chap-core in the shape of
//! chap-core 2.4 ([`fresh_chap_core_server`]).

use super::*;
use serde_json::Value as Json;

/// The id of the first template [`fresh_chap_core_server`] stores; the
/// others follow in the order of the three stand-in models.
pub(crate) const TEMPLATE_IDS: i64 = 30;

/// The id of the first configured model a client posts there.
pub(crate) const CREATED_IDS: i64 = 100;

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
            recorded.templates.push(service.clone());
            let id = TEMPLATE_IDS
                + [PASSING_MODEL, FAILING_MODEL, OLD_CHAPKIT_MODEL]
                    .iter()
                    .position(|known| *known == service)
                    .unwrap_or(9) as i64;
            Some((
                200,
                json,
                serde_json::json!({"id": id, "name": service, "version": "1.0.1"}).to_string(),
            ))
        }
        ("POST", "/v1/crud/configured-models") => {
            let sent: Json = serde_json::from_str(body).unwrap_or(Json::Null);
            let template = sent["model_template_id"].as_i64().unwrap_or_default();
            let service = [PASSING_MODEL, FAILING_MODEL, OLD_CHAPKIT_MODEL]
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
            });
            recorded.created.push(row.clone());
            recorded.configured_posts.push(sent);
            Some((200, json, row.to_string()))
        }
        _ => None,
    }
}
