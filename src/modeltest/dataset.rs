//! Turning chapkit's sample frame into a chap-core dataset, and reading the
//! scores the backtest of it comes back with.

use super::{LOCATION_COLUMN, METRICS, TIME_COLUMN};
use crate::error::Result;
use serde::Serialize;

/// One value of one feature, for one org unit, in one period.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Observation {
    pub period: String,
    #[serde(rename = "orgUnit")]
    pub org_unit: String,
    pub value: Option<f64>,
    #[serde(rename = "featureName")]
    pub feature_name: String,
}

/// The observations and the org units of one `$generate-sample-data` frame.
///
/// chapkit hands back a column-oriented frame - a list of names and a list of
/// rows - and chap-core takes one object per value, so every column but
/// `time_period` and `location` becomes a feature name. A cell that is not a
/// number goes over as `null`, which chap-core accepts as a known-missing
/// observation.
pub fn observations(frame: &serde_json::Value) -> Result<(Vec<Observation>, Vec<String>)> {
    let columns: Vec<String> = frame
        .get("columns")
        .and_then(|c| c.as_array())
        .ok_or_else(|| anyhow::anyhow!("the sample data has no `columns` list"))?
        .iter()
        .map(|name| name.as_str().unwrap_or_default().to_string())
        .collect();
    let rows = frame
        .get("data")
        .and_then(|d| d.as_array())
        .ok_or_else(|| anyhow::anyhow!("the sample data has no `data` rows"))?;

    let time_at = index_of(&columns, TIME_COLUMN)?;
    let location_at = index_of(&columns, LOCATION_COLUMN)?;

    let mut observations = Vec::with_capacity(rows.len() * columns.len());
    let mut locations: Vec<String> = Vec::new();
    for row in rows {
        let cells = row
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("a sample data row is not a list"))?;
        let period = period_code(
            cells
                .get(time_at)
                .and_then(|c| c.as_str())
                .unwrap_or_default(),
        );
        let org_unit = cells
            .get(location_at)
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string();
        if !locations.iter().any(|seen| seen == &org_unit) {
            locations.push(org_unit.clone());
        }
        for (at, name) in columns.iter().enumerate() {
            if at == time_at || at == location_at {
                continue;
            }
            observations.push(Observation {
                period: period.clone(),
                org_unit: org_unit.clone(),
                value: cells.get(at).and_then(|c| c.as_f64()),
                feature_name: name.clone(),
            });
        }
    }
    Ok((observations, locations))
}

/// Give the frame every covariate in `wanted`, renaming chapkit's spare
/// `feature_N` columns into the ones it lacks, and return the renames.
///
/// `$generate-sample-data` generates the service's required covariates and a
/// fixed pair of climate ones, but not a configuration's
/// `additional_continuous_covariates`: a model whose configuration defaults to
/// `mean_relative_humidity` gets a frame without it, and the backtest then
/// fails inside the model with a `KeyError` that says nothing about the model.
/// The `feature_N` columns are the same kind of synthetic seasonal series, so
/// one of them stands in. A covariate with no spare column left stays missing,
/// and the model says so the way it would with real data.
pub fn fill_covariates(frame: &mut serde_json::Value, wanted: &[String]) -> Vec<(String, String)> {
    let Some(columns) = frame.get_mut("columns").and_then(|c| c.as_array_mut()) else {
        return Vec::new();
    };
    let has = |columns: &[serde_json::Value], name: &str| {
        columns.iter().any(|column| column.as_str() == Some(name))
    };
    let mut renamed = Vec::new();
    for name in wanted {
        if has(columns, name) {
            continue;
        }
        let spare = columns.iter().position(|column| {
            column.as_str().is_some_and(|text| {
                text.strip_prefix("feature_")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                    && !wanted.iter().any(|want| want == text)
            })
        });
        let Some(at) = spare else { continue };
        let from = columns[at].as_str().unwrap_or_default().to_string();
        columns[at] = serde_json::json!(name);
        renamed.push((from, name.clone()));
    }
    renamed
}

/// The column named `want`, or the error that says the frame is not one.
fn index_of(columns: &[String], want: &str) -> Result<usize> {
    columns
        .iter()
        .position(|name| name == want)
        .ok_or_else(|| anyhow::anyhow!("the sample data has no `{want}` column"))
}

/// chapkit's period as chap-core spells it: `2020-01` -> `202001`,
/// `2020-W01` -> `2020W01`.
pub fn period_code(text: &str) -> String {
    text.trim().replace('-', "")
}

/// The org-unit geometry for the dataset.
///
/// chapkit's generator puts the location id in `properties.id` and nothing at
/// the top level, and chap-core matches org units on the feature's top-level
/// `id` and drops every one it cannot match - silently, as an import that
/// found nothing - so setting it is what makes the dataset non-empty. A model
/// that needs no geometry gets one feature per location with no geometry at
/// all, which is enough for chap-core to know the org units exist.
pub fn feature_collection(
    geo: Option<&serde_json::Value>,
    locations: &[String],
) -> serde_json::Value {
    let Some(geo) = geo.filter(|value| value.get("features").is_some()) else {
        let features: Vec<serde_json::Value> = locations
            .iter()
            .map(|id| {
                serde_json::json!({
                    "type": "Feature",
                    "id": id,
                    "geometry": serde_json::Value::Null,
                    "properties": {"id": id},
                })
            })
            .collect();
        return serde_json::json!({"type": "FeatureCollection", "features": features});
    };

    let mut collection = geo.clone();
    let features = collection
        .get_mut("features")
        .and_then(|f| f.as_array_mut())
        .expect("checked above");
    for (at, feature) in features.iter_mut().enumerate() {
        let existing = feature
            .get("id")
            .and_then(|id| id.as_str())
            .map(str::to_string);
        let from_properties = feature
            .get("properties")
            .and_then(|p| p.get("id"))
            .and_then(|id| id.as_str())
            .map(str::to_string);
        let id = existing
            .or(from_properties)
            .or_else(|| locations.get(at).cloned());
        if let (Some(id), Some(object)) = (id, feature.as_object_mut()) {
            object.insert("id".to_string(), serde_json::json!(id));
        }
    }
    collection
}

/// `crps 20.0  mae 29.0  rmse 34.9` from chap-core's `aggregateMetrics`.
///
/// Only [`METRICS`], because a row is read at a glance and chap-core reports
/// fourteen scores; `--json` hands back all of them.
pub fn metrics_cell(metrics: &serde_json::Value) -> String {
    let parts: Vec<String> = METRICS
        .iter()
        .filter_map(|name| {
            let value = metrics.get(*name)?.as_f64()?;
            Some(format!("{name} {value:.1}"))
        })
        .collect();
    if parts.is_empty() {
        return "no scores reported".to_string();
    }
    parts.join("  ")
}
