//! Which of chap-core's configured models a backtest is of.

/// One row of `GET /v1/crud/configured-models`, cut to what the choice needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredModel {
    /// chap-core's primary key, which is what `create-backtest` is given.
    pub id: i64,
    /// chap-core's name for it: the service id for a service that has just
    /// registered, and `<service id>:<config name>` for one whose configs
    /// chap-core has synced.
    pub name: String,
    /// Whether chap-core has retired it. An archived row still has a name and
    /// is still listed; it is simply not a model anything can be run with.
    pub archived: bool,
    /// Its `additionalContinuousCovariates`: the columns a backtest of it
    /// hands the model, which the dataset therefore has to carry.
    pub covariates: Vec<String>,
    /// The version of the model template it belongs to, which is the version
    /// the service registered with. `None` when chap-core sent none.
    pub version: Option<String>,
    /// Its `healthStatus`: `live`, or `revision_mismatch` when the service
    /// behind its template reports another git revision, or none. `None` when
    /// chap-core sent none, which every chap-core before 2.4 does.
    pub health: Option<String>,
}

/// The configured models of one listing, ignoring anything that is not a row.
pub fn configured_models(listed: &serde_json::Value) -> Vec<ConfiguredModel> {
    let Some(rows) = listed.as_array() else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|row| {
            Some(ConfiguredModel {
                id: row.get("id")?.as_i64()?,
                name: row.get("name")?.as_str()?.to_string(),
                archived: row
                    .get("archived")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
                covariates: row
                    .get("additionalContinuousCovariates")
                    .and_then(|value| value.as_array())
                    .map(|names| {
                        names
                            .iter()
                            .filter_map(|name| name.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                version: row
                    .get("version")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                health: row
                    .get("healthStatus")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            })
        })
        .collect()
}

/// The configured model a backtest of `service_id` has to name.
///
/// `create-backtest` resolves a string `modelId` against these names, and a
/// service is only named plainly for as long as chap-core has nothing else
/// from it: a service that re-registers with a new version has its own stored
/// configs synced as `<service id>:<config name>` instead, and the bare name
/// stops existing. Sending the service id is then a `ValueError` from inside
/// chap-core rather than a backtest, so the row is picked here and its
/// integer id is what goes out.
///
/// The order is the order of how much the row is the service itself: its bare
/// name, then a config of it, and a `test_config_` last of all, because that
/// is what a previous `varde models test` or `chapkit test` left behind and
/// not a configuration anybody made. Ties go to the lowest id, so two runs of
/// the same command backtest the same model.
pub fn configured_model_for<'a>(
    models: &'a [ConfiguredModel],
    service_id: &str,
) -> Option<&'a ConfiguredModel> {
    let prefix = format!("{service_id}:");
    let left_behind = format!("{prefix}test_config_");
    models
        .iter()
        .filter(|model| !model.archived)
        .filter_map(|model| {
            let rank = if model.name == service_id {
                0
            } else if !model.name.starts_with(&prefix) {
                return None;
            } else if model.name.starts_with(&left_behind) {
                2
            } else {
                1
            };
            Some((rank, model.id, model))
        })
        .min_by_key(|(rank, id, _)| (*rank, *id))
        .map(|(_, _, model)| model)
}
