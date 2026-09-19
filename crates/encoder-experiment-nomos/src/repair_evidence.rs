//! Bounded display projection of already-saved development inspections.
use std::collections::{BTreeMap, BTreeSet};

use encoder_optimization_core::agent::InspectionItem;
use serde::Serialize;
use serde_json::Value;

use crate::{EncoderTaskAdapterError, adapter_error};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NomosRepairEvidence {
    pub id: String,
    pub fingerprint: String,
    pub label: String,
    pub training_rows: Option<u64>,
    pub total_training_rows: Option<u64>,
    pub overlapping: bool,
    pub development: Vec<ClusterObservation>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterObservation {
    pub suite: String,
    pub support: u64,
    pub recall_at_1: f64,
    pub original_baseline_recall_at_1: Option<f64>,
}

/// Allowed suite/report bindings come from this exact iteration, not today's
/// benchmark defaults. Never forward opaque content, row text or extra keys.
pub fn project_repair_evidence(
    item: &InspectionItem,
    reports: &BTreeMap<String, String>,
) -> Result<NomosRepairEvidence, EncoderTaskAdapterError> {
    if item.fingerprint
        != encoder_optimization_core::fingerprint(&item.content).map_err(adapter_error)?
    {
        return Err(adapter_error("Saved repair evidence fingerprint changed"));
    }
    let value = &item.content;
    let mut result = NomosRepairEvidence {
        id: item.id.clone(),
        fingerprint: item.fingerprint.clone(),
        label: String::new(),
        training_rows: None,
        total_training_rows: None,
        overlapping: false,
        development: vec![],
    };
    match value["kind"].as_str() {
        Some("dataset_cluster") if value["schemaVersion"] == 1 => {
            let dimension = text(&value["cluster"]["dimension"])?;
            if ![
                "expected_capability",
                "task_kind",
                "scenario_family",
                "candidate_pool_size",
            ]
            .contains(&dimension)
            {
                return Err(adapter_error("Unknown repair evidence dimension"));
            }
            result.label = format!(
                "{}: {}",
                dimension.replace('_', " "),
                display_text(&value["cluster"]["value"])?
            );
            let rows = count(&value["training"]["rows"])?;
            let total = count(&value["training"]["totalRows"])?;
            if total == 0 || rows > total {
                return Err(adapter_error("Invalid saved training coverage"));
            }
            result.training_rows = Some(rows);
            result.total_training_rows = Some(total);
            result.overlapping = dimension == "expected_capability";
            let observations = value["development"]
                .as_array()
                .filter(|values| values.len() <= 20)
                .ok_or_else(|| adapter_error("Invalid saved cluster observations"))?;
            let mut suites = BTreeSet::new();
            for observation in observations {
                let suite = text(&observation["suite"])?;
                if !reports.contains_key(suite) || !suites.insert(suite) {
                    return Err(adapter_error(
                        "Repair evidence references another development suite",
                    ));
                }
                result.development.push(ClusterObservation {
                    suite: suite.into(),
                    support: count(&observation["support"])?,
                    recall_at_1: rate(&observation["current"]["recall_at_1"])?,
                    original_baseline_recall_at_1: if observation["originalBaseline"].is_null() {
                        None
                    } else {
                        Some(rate(&observation["originalBaseline"]["recall_at_1"])?)
                    },
                });
            }
        }
        Some("development_summary") => result.label = "Saved development summary".into(),
        _ if value["evidenceScope"] == "retrieval_failure_sample" => {
            let suite = text(&value["suite"])?;
            if reports.get(suite).map(String::as_str) != value["reportId"].as_str() {
                return Err(adapter_error(
                    "Repair failure belongs to another development report",
                ));
            }
            result.label = format!(
                "{suite}: sampled failure {}",
                display_text(&value["sourceRowId"])?
            );
        }
        _ => return Err(adapter_error("Unsupported saved repair evidence")),
    }
    Ok(result)
}

fn text(value: &Value) -> Result<&str, EncoderTaskAdapterError> {
    value
        .as_str()
        .filter(|value| !value.is_empty() && value.chars().count() <= 200)
        .ok_or_else(|| adapter_error("Invalid saved repair evidence label"))
}
fn display_text(value: &Value) -> Result<String, EncoderTaskAdapterError> {
    let value = value
        .as_str()
        .filter(|value| !value.is_empty() && value.len() <= 262144)
        .ok_or_else(|| adapter_error("Invalid saved repair evidence label"))?;
    let mut label: String = value.chars().take(200).collect();
    if value.chars().count() > 200 {
        label.push('…');
    }
    Ok(label)
}
fn count(value: &Value) -> Result<u64, EncoderTaskAdapterError> {
    value
        .as_u64()
        .filter(|value| *value <= 9_007_199_254_740_991)
        .ok_or_else(|| adapter_error("Invalid saved repair evidence count"))
}
fn rate(value: &Value) -> Result<f64, EncoderTaskAdapterError> {
    value
        .as_f64()
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .ok_or_else(|| adapter_error("Invalid saved repair evidence metric"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn projection_is_closed_bounded_and_uses_only_the_bound_development_suites() {
        let content = json!({"kind":"dataset_cluster","schemaVersion":1,"cluster":{"dimension":"expected_capability","value":"search"},
            "training":{"rows":2,"totalRows":10}, "development":[{"suite":"dev","support":4,"current":{"recall_at_1":0.5},"originalBaseline":{"recall_at_1":0.75},"sealedScore":12345}], "secret":"never-forward"});
        let item = InspectionItem {
            id: "cluster".into(),
            fingerprint: encoder_optimization_core::fingerprint(&content).unwrap(),
            content,
        };
        let value =
            project_repair_evidence(&item, &BTreeMap::from([("dev".into(), "report".into())]))
                .unwrap();
        assert_eq!(value.training_rows, Some(2));
        assert_eq!(value.development[0].recall_at_1, 0.5);
        let wire = serde_json::to_string(&value).unwrap();
        assert!(!wire.contains("secret") && !wire.contains("12345"));
        assert!(project_repair_evidence(&item, &BTreeMap::new()).is_err());
        let mut long = item;
        long.content["cluster"]["value"] = "long display label ".repeat(30).into();
        long.fingerprint = encoder_optimization_core::fingerprint(&long.content).unwrap();
        let projected =
            project_repair_evidence(&long, &BTreeMap::from([("dev".into(), "report".into())]))
                .unwrap();
        assert!(projected.label.ends_with('…') && projected.label.chars().count() < 250);
        assert_eq!(projected.fingerprint, long.fingerprint);
    }
}
