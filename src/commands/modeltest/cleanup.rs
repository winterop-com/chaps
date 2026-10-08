//! What the model level leaves in the service's own database, and taking it
//! away again.

use super::{first_line, service_url};
use crate::api::Api;
use crate::commands::Ctx;
use crate::docker;
use crate::output::Message;
use crate::project::Project;
use std::time::Duration;

/// How long the `curl` that lists or deletes inside a container may take.
const SERVICE_TIMEOUT: Duration = Duration::from_secs(30);

/// The configs and artifacts one model service holds.
///
/// `chapkit test` writes one config named `test_config_<ulid>` plus a handful
/// of artifacts, and both stay in the service's own database. Diffing the two
/// listings is what lets the cleanup delete exactly what this run created and
/// leave a config somebody is using alone.
#[derive(Debug, Clone, Default)]
pub(super) struct Held {
    /// `(id, name)`, because the note about what was left names the config.
    configs: Vec<(String, String)>,
    artifacts: Vec<String>,
}

/// What the service holds now, or `None` when it could not be asked.
pub(super) fn held(ctx: &Ctx, project: &Project, api: &Api, service_id: &str) -> Option<Held> {
    let configs = list(ctx, project, api, service_id, "configs")?;
    let artifacts = list(ctx, project, api, service_id, "artifacts")?;
    Some(Held {
        configs: configs
            .iter()
            .map(|entry| (entry.0.clone(), entry.1.clone()))
            .collect(),
        artifacts: artifacts.into_iter().map(|entry| entry.0).collect(),
    })
}

/// `GET /api/v1/{collection}` as `(id, name)` pairs.
///
/// Through chap-core's proxy first, which needs nothing of the container, and
/// otherwise with the `curl` every chapkit image carries for its healthcheck.
fn list(
    ctx: &Ctx,
    project: &Project,
    api: &Api,
    service_id: &str,
    collection: &str,
) -> Option<Vec<(String, String)>> {
    let path = format!(
        "/v2/services/{}/run/api/v1/{collection}",
        crate::api::encode(service_id)
    );
    if let Ok(answer) = api.send("GET", &path, None)
        && answer.is_success()
        && let Some(value) = answer.json()
        && let Some(entries) = entries_of(&value)
    {
        return Some(entries);
    }
    let url = format!("{}/api/v1/{collection}", service_url(project, service_id));
    let exec: Vec<String> = ["exec", "-T", service_id, "curl", "-fsS", &url]
        .iter()
        .map(|part| part.to_string())
        .collect();
    let outcome = docker::compose_captured(project, &exec, false, SERVICE_TIMEOUT).ok()?;
    if outcome.code != 0 {
        ctx.out.verbose(&format!(
            "{service_id}: could not list {collection}: {}",
            first_line(outcome.stderr.trim())
        ));
        return None;
    }
    entries_of(&serde_json::from_str(&outcome.stdout).ok()?)
}

/// The `(id, name)` pairs of a chapkit list answer.
///
/// A bare array is what chapkit sends; an object with the list under `items`
/// or `data` is accepted too, so a chapkit that grows a page wrapper still
/// gets cleaned up after.
fn entries_of(value: &serde_json::Value) -> Option<Vec<(String, String)>> {
    let list = value
        .as_array()
        .or_else(|| value.get("items")?.as_array())
        .or_else(|| value.get("data")?.as_array())?;
    Some(
        list.iter()
            .filter_map(|entry| {
                let id = entry.get("id")?.as_str()?.to_string();
                let name = entry
                    .get("name")
                    .and_then(|name| name.as_str())
                    .unwrap_or_default()
                    .to_string();
                Some((id, name))
            })
            .collect(),
    )
}

/// Delete whatever appeared in the service's database while the test ran,
/// and return what was kept or left behind, for the closing lines.
///
/// Configs first: chapkit cascades a config delete to the artifact trees
/// linked to it, so the artifacts a training and a prediction left behind
/// usually go with it and the second pass has nothing to do.
pub(super) fn clean(
    ctx: &Ctx,
    project: &Project,
    api: &Api,
    service_id: &str,
    before: &Option<Held>,
    keep: bool,
) -> Vec<Message> {
    let shows =
        format!("`varde api GET /v2/services/{service_id}/run/api/v1/configs` shows what it holds");
    let Some(before) = before else {
        return vec![Message::warning(format!(
            "{service_id}: could not list its configs, so nothing was cleaned up; {shows}"
        ))];
    };
    let Some(after) = held(ctx, project, api, service_id) else {
        return vec![Message::warning(format!(
            "{service_id}: could not list its configs afterwards, so nothing was cleaned up; \
             {shows}"
        ))];
    };

    let Held {
        configs: new_configs,
        artifacts: new_artifacts,
    } = created(before, &after);
    if new_configs.is_empty() && new_artifacts.is_empty() {
        return Vec::new();
    }

    if keep {
        let named: Vec<String> = new_configs
            .iter()
            .map(|(id, name)| {
                if name.is_empty() {
                    id.clone()
                } else {
                    name.clone()
                }
            })
            .collect();
        return vec![Message::info(kept_line(
            service_id,
            &service_url(project, service_id),
            &named,
            &new_configs,
            &new_artifacts,
        ))];
    }

    let mut left: Vec<String> = Vec::new();
    for (id, name) in &new_configs {
        if !delete(ctx, project, service_id, &format!("/api/v1/configs/{id}")) {
            left.push(if name.is_empty() {
                id.clone()
            } else {
                name.clone()
            });
        }
    }
    // The config delete cascades, so this is the remainder rather than the
    // list: asking again costs one request and deletes nothing twice.
    if let Some(now) = held(ctx, project, api, service_id) {
        for id in new_artifacts.iter().filter(|id| now.artifacts.contains(id)) {
            if !delete(ctx, project, service_id, &format!("/api/v1/artifacts/{id}")) {
                left.push(format!("artifact {id}"));
            }
        }
    }
    if left.is_empty() {
        return Vec::new();
    }
    vec![Message::warning(format!(
        "{service_id}: could not clean up {}; \
         delete it from the service's own API or restart the model",
        left.join(", ")
    ))]
}

/// What `--keep` left in the service's database, and the commands that
/// remove it. A config delete takes its artifacts with it, so only a run
/// that kept artifacts and no config names the artifacts one by one.
pub(super) fn kept_line(
    service_id: &str,
    url: &str,
    named: &[String],
    configs: &[(String, String)],
    artifacts: &[String],
) -> String {
    let what = format!(
        "{service_id}: kept {} and {} artifact{} in its database",
        if named.is_empty() {
            "no config".to_string()
        } else {
            named.join(", ")
        },
        artifacts.len(),
        if artifacts.len() == 1 { "" } else { "s" }
    );
    let remove =
        |path: String| format!("`varde docker exec {service_id} curl -fsS -X DELETE {url}{path}`");
    let commands: Vec<String> = match configs.is_empty() {
        false => configs
            .iter()
            .map(|(id, _)| remove(format!("/api/v1/configs/{id}")))
            .collect(),
        true => artifacts
            .iter()
            .map(|id| remove(format!("/api/v1/artifacts/{id}")))
            .collect(),
    };
    match commands.is_empty() {
        true => what,
        false => format!("{what}; remove it with {}", commands.join(" then ")),
    }
}

/// What `after` holds that `before` did not, by id: the configs and the
/// artifacts the test run made, and nothing that was there before it.
fn created(before: &Held, after: &Held) -> Held {
    Held {
        configs: after
            .configs
            .iter()
            .filter(|(id, _)| !before.configs.iter().any(|(was, _)| was == id))
            .cloned()
            .collect(),
        artifacts: after
            .artifacts
            .iter()
            .filter(|id| !before.artifacts.contains(id))
            .cloned()
            .collect(),
    }
}

/// `curl -X DELETE` inside the container.
///
/// chap-core's proxy is read-only by design, so a delete cannot go through
/// it; the service's own API is one `exec` away, and every chapkit image
/// carries the `curl` its healthcheck uses.
fn delete(ctx: &Ctx, project: &Project, service_id: &str, path: &str) -> bool {
    let url = format!("{}{path}", service_url(project, service_id));
    let exec: Vec<String> = [
        "exec", "-T", service_id, "curl", "-fsS", "-X", "DELETE", &url,
    ]
    .iter()
    .map(|part| part.to_string())
    .collect();
    match docker::compose_captured(project, &exec, false, SERVICE_TIMEOUT) {
        Ok(outcome) if outcome.code == 0 => {
            ctx.out.verbose(&format!("{service_id}: deleted {path}"));
            true
        }
        Ok(outcome) => {
            ctx.out.verbose(&format!(
                "{service_id}: DELETE {path} exited {}: {}",
                outcome.code,
                first_line(outcome.stderr.trim())
            ));
            false
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests;
