//! Compare only saved development disagreements. Missing sampled predictions
//! never imply a fix/regression, and ranks never replace aggregate acceptance.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use uuid::Uuid;

use crate::{
    EncoderTaskAdapterError, EvaluationReport, EvidenceRole, ExternalProjectSnapshot,
    MetricContract, NomosBackend, NomosDevelopmentEvidence, Value, adapter_error,
    development_evidence::NomosDevelopmentEvidenceRead,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedCasePrediction {
    pub fingerprint: String,
    pub question: Option<String>,
    pub task_kind: Option<String>,
    pub expected_capabilities: Option<Vec<String>>,
    pub predicted_capabilities: Option<Vec<String>>,
    pub expected_rank: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedCaseChange {
    RankImproved,
    RankRegressed,
    RankUnchanged,
    NotComparable,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedCasePair {
    pub source_row_id: String,
    pub baseline: Option<SavedCasePrediction>,
    pub candidate: Option<SavedCasePrediction>,
    pub change: SavedCaseChange,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedCaseSource {
    pub report_id: Uuid,
    pub report_fingerprint: String,
    pub diagnostics_fingerprint: Option<String>,
    /// The conservative support on the combined scientific report. This may
    /// be the smaller Agent-session population and is never a retrieval label.
    pub report_support: u64,
    pub retrieval_support: Option<u64>,
    pub availability: SavedDiagnosticAvailability,
    /// None means diagnostics were unavailable, not an empty sample.
    pub sample_size: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedDiagnosticAvailability {
    AvailableSample,
    AvailableEmpty,
    Missing,
    Corrupt,
    Incompatible,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NomosDevelopmentComparison {
    pub suite: String,
    pub suite_fingerprint: String,
    pub sample_limit: u32,
    pub baseline: SavedCaseSource,
    pub candidate: SavedCaseSource,
    pub cases: Vec<SavedCasePair>,
}

impl NomosBackend {
    pub fn read_development_comparison(
        &self,
        project: &ExternalProjectSnapshot,
        contract: &MetricContract,
        baseline: &EvaluationReport,
        candidate: &EvaluationReport,
    ) -> Result<NomosDevelopmentComparison, EncoderTaskAdapterError> {
        // Role and exact scientific identities are checked before filesystem
        // access; an unavailable file cannot bypass report custody validation.
        if baseline.evidence_role != EvidenceRole::Development
            || candidate.evidence_role != EvidenceRole::Development
            || baseline.suite_key != candidate.suite_key
            || baseline.suite_fingerprint != candidate.suite_fingerprint
            || baseline.model != project.baseline_model
        {
            return Err(adapter_error(
                "Case comparison requires the original baseline and same development suite",
            ));
        }
        baseline
            .validate_integrity(project, contract)
            .map_err(adapter_error)?;
        candidate
            .validate_integrity(project, contract)
            .map_err(adapter_error)?;
        let read = |report: &EvaluationReport| -> Result<SavedCases, EncoderTaskAdapterError> {
            Ok(
                match self.read_development_evidence_status(project, contract, report)? {
                    NomosDevelopmentEvidenceRead::Available(evidence) => {
                        match project_cases(&evidence) {
                            Ok(cases) => SavedCases::Available { evidence, cases },
                            Err(_) => SavedCases::Incompatible,
                        }
                    }
                    NomosDevelopmentEvidenceRead::Missing => SavedCases::Missing,
                    NomosDevelopmentEvidenceRead::Corrupt => SavedCases::Corrupt,
                    NomosDevelopmentEvidenceRead::Incompatible => SavedCases::Incompatible,
                },
            )
        };
        // Missing, corrupt or incompatible native diagnostics remain explicitly
        // unavailable. Do not rerun inference or discard the aggregate report.
        let left = read(baseline)?;
        let right = read(candidate)?;
        let source = |report: &EvaluationReport, saved: &SavedCases| {
            let (diagnostics_fingerprint, retrieval_support, availability, sample_size) =
                match saved {
                    SavedCases::Available { evidence, cases } => (
                        Some(evidence.artifact_fingerprint.clone()),
                        Some(evidence.retrieval_support),
                        if cases.is_empty() {
                            SavedDiagnosticAvailability::AvailableEmpty
                        } else {
                            SavedDiagnosticAvailability::AvailableSample
                        },
                        Some(cases.len() as u32),
                    ),
                    SavedCases::Missing => (None, None, SavedDiagnosticAvailability::Missing, None),
                    SavedCases::Corrupt => (None, None, SavedDiagnosticAvailability::Corrupt, None),
                    SavedCases::Incompatible => {
                        (None, None, SavedDiagnosticAvailability::Incompatible, None)
                    }
                };
            SavedCaseSource {
                report_id: report.id,
                report_fingerprint: report.fingerprint.clone(),
                diagnostics_fingerprint,
                report_support: report.support,
                retrieval_support,
                availability,
                sample_size,
            }
        };
        Ok(NomosDevelopmentComparison {
            suite: baseline.suite_key.clone(),
            suite_fingerprint: baseline.suite_fingerprint.clone(),
            sample_limit: 50,
            baseline: source(baseline, &left),
            candidate: source(candidate, &right),
            cases: pair_cases(left.into_cases(), right.into_cases()),
        })
    }
}

enum SavedCases {
    Available {
        evidence: Box<NomosDevelopmentEvidence>,
        cases: BTreeMap<String, SavedCasePrediction>,
    },
    Missing,
    Corrupt,
    Incompatible,
}

impl SavedCases {
    fn into_cases(self) -> BTreeMap<String, SavedCasePrediction> {
        match self {
            Self::Available { cases, .. } => cases,
            Self::Missing | Self::Corrupt | Self::Incompatible => BTreeMap::new(),
        }
    }
}

fn project_cases(
    evidence: &NomosDevelopmentEvidence,
) -> Result<BTreeMap<String, SavedCasePrediction>, EncoderTaskAdapterError> {
    if evidence.failure_sample_limit != 50
        || evidence.failures.len() > 50
        || evidence.failures.len() as u64 > evidence.retrieval_support
    {
        return Err(adapter_error("Unsupported saved diagnostic sample"));
    }
    let mut result = BTreeMap::new();
    for item in &evidence.failures {
        let value = &item.content;
        if artifact_core::fingerprint(value).map_err(adapter_error)? != item.fingerprint
            || value["reportId"] != evidence.report_id.to_string()
            || value["suite"] != evidence.suite_key
            || value["sampleLimit"] != 50
            || value["evidenceScope"] != "retrieval_failure_sample"
        {
            return Err(adapter_error("Saved case belongs to another report"));
        }
        let id = optional_text(&value["sourceRowId"], 200)?
            .ok_or_else(|| adapter_error("Saved case identity missing"))?;
        let expected_rank = if value["expectedRank"].is_null() {
            None
        } else {
            Some(
                value["expectedRank"]
                    .as_u64()
                    .filter(|rank| (1..=9_007_199_254_740_991).contains(rank))
                    .ok_or_else(|| adapter_error("Saved case rank is invalid"))?,
            )
        };
        let prediction = SavedCasePrediction {
            fingerprint: item.fingerprint.clone(),
            question: optional_text(&value["question"], 8192)?,
            task_kind: optional_text(&value["taskKind"], 200)?,
            expected_capabilities: capabilities(&value["expectedCapabilities"])?,
            predicted_capabilities: capabilities(&value["predictedCapabilities"])?,
            expected_rank,
        };
        if result.insert(id, prediction).is_some() {
            return Err(adapter_error("Saved sample repeats a case"));
        }
    }
    Ok(result)
}

fn optional_text(value: &Value, maximum: usize) -> Result<Option<String>, EncoderTaskAdapterError> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .filter(|text| !text.trim().is_empty() && text.chars().count() <= maximum)
        .map(|text| Some(text.to_owned()))
        .ok_or_else(|| adapter_error("Saved case text is invalid"))
}

fn capabilities(value: &Value) -> Result<Option<Vec<String>>, EncoderTaskAdapterError> {
    if value.is_null() {
        return Ok(None);
    }
    let values = value
        .as_array()
        .filter(|values| values.len() <= 128)
        .ok_or_else(|| adapter_error("Saved case capabilities are invalid"))?;
    let unique = values
        .iter()
        .map(|value| {
            optional_text(value, 200)?.ok_or_else(|| adapter_error("Missing saved capability"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(Some(unique.into_iter().collect()))
}

fn pair_cases(
    mut baseline: BTreeMap<String, SavedCasePrediction>,
    mut candidate: BTreeMap<String, SavedCasePrediction>,
) -> Vec<SavedCasePair> {
    let ids: BTreeSet<_> = baseline.keys().chain(candidate.keys()).cloned().collect();
    ids.into_iter()
        .map(|source_row_id| {
            let baseline = baseline.remove(&source_row_id);
            let candidate = candidate.remove(&source_row_id);
            let change = match (&baseline, &candidate) {
                (Some(left), Some(right))
                    if left.question.is_some()
                        && left.question == right.question
                        && left.task_kind.is_some()
                        && left.task_kind == right.task_kind
                        && left.expected_capabilities.is_some()
                        && left.expected_capabilities == right.expected_capabilities =>
                {
                    match (left.expected_rank, right.expected_rank) {
                        (Some(a), Some(b)) if b < a => SavedCaseChange::RankImproved,
                        (Some(a), Some(b)) if b > a => SavedCaseChange::RankRegressed,
                        (Some(_), Some(_)) => SavedCaseChange::RankUnchanged,
                        _ => SavedCaseChange::NotComparable,
                    }
                }
                _ => SavedCaseChange::NotComparable,
            };
            SavedCasePair {
                source_row_id,
                baseline,
                candidate,
                change,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoder_optimization_core::agent::InspectionItem;
    use serde_json::json;

    fn evidence() -> NomosDevelopmentEvidence {
        let report_id = Uuid::new_v4();
        let content = json!({"reportId":report_id,"suite":"development","sourceRowId":"row-1","evidenceScope":"retrieval_failure_sample","sampleLimit":50,
            "question":"Find primary evidence","taskKind":"route","expectedCapabilities":["search"],"predictedCapabilities":["write"],"expectedRank":3,
            "privateNativeField":"NEVER_DISPLAY"});
        NomosDevelopmentEvidence {
            schema_version: crate::development_evidence::DEVELOPMENT_EVIDENCE_SCHEMA_VERSION,
            report_id,
            report_fingerprint: artifact_core::fingerprint(&1).unwrap(),
            suite_key: "development".into(),
            artifact_fingerprint: artifact_core::fingerprint(&2).unwrap(),
            report_support: 16,
            retrieval_support: 100,
            agent_population: crate::NomosAgentPopulation::NotRequired,
            clusters: vec![],
            failures: vec![InspectionItem {
                id: "sample".into(),
                fingerprint: artifact_core::fingerprint(&content).unwrap(),
                content,
            }],
            failure_sample_limit: 50,
        }
    }

    #[test]
    fn pairs_exact_cases_and_distinguishes_both_rank_directions_without_claiming_acceptance() {
        let left = project_cases(&evidence()).unwrap();
        for (rank, expected) in [
            (2, SavedCaseChange::RankImproved),
            (4, SavedCaseChange::RankRegressed),
            (3, SavedCaseChange::RankUnchanged),
        ] {
            let mut right = left.clone();
            right.get_mut("row-1").unwrap().expected_rank = Some(rank);
            let pairs = pair_cases(left.clone(), right);
            assert_eq!(pairs[0].change, expected);
            assert!(
                !serde_json::to_string(&pairs)
                    .unwrap()
                    .contains("NEVER_DISPLAY")
            );
        }
    }

    #[test]
    fn unmatched_changed_or_incomplete_case_is_not_a_fix_or_regression() {
        let left = project_cases(&evidence()).unwrap();
        let mut right = left.clone();
        let row = right.remove("row-1").unwrap();
        right.insert("row-2".into(), row);
        let pairs = pair_cases(left.clone(), right);
        assert_eq!(pairs.len(), 2);
        assert!(
            pairs
                .iter()
                .all(|pair| pair.change == SavedCaseChange::NotComparable)
        );
        for field in 0..4 {
            let mut right = left.clone();
            let value = right.get_mut("row-1").unwrap();
            match field {
                0 => value.question = Some("Different input".into()),
                1 => value.expected_rank = None,
                2 => value.expected_capabilities = None,
                _ => value.task_kind = None,
            }
            assert_eq!(
                pair_cases(left.clone(), right)[0].change,
                SavedCaseChange::NotComparable
            );
        }
    }

    #[test]
    fn rejects_foreign_tampered_duplicate_and_malformed_samples() {
        let good = evidence();
        for field in ["reportId", "suite", "expectedRank", "question"] {
            let mut bad = evidence();
            bad.failures[0].content[field] = json!({"secret":"do not show"});
            bad.failures[0].fingerprint =
                artifact_core::fingerprint(&bad.failures[0].content).unwrap();
            assert!(project_cases(&bad).is_err());
        }
        let mut duplicate = good;
        duplicate.failures.push(duplicate.failures[0].clone());
        assert!(project_cases(&duplicate).is_err());
        let mut tampered = evidence();
        tampered.failures[0].content["expectedRank"] = 1.into();
        assert!(project_cases(&tampered).is_err());
    }

    #[test]
    fn retrieval_sample_is_bounded_by_retrieval_states_not_agent_report_support() {
        let mut evidence = evidence();
        evidence.report_support = 16;
        evidence.retrieval_support = 1_000;
        evidence.failures = (0..50)
            .map(|index| {
                let content = json!({
                    "reportId": evidence.report_id,
                    "suite": "development",
                    "sourceRowId": format!("row-{index}"),
                    "evidenceScope": "retrieval_failure_sample",
                    "sampleLimit": 50,
                    "question": format!("Find evidence {index}"),
                    "taskKind": "route",
                    "expectedCapabilities": ["search"],
                    "predictedCapabilities": ["write"],
                    "expectedRank": 3,
                });
                InspectionItem {
                    id: format!("sample-{index}"),
                    fingerprint: artifact_core::fingerprint(&content).unwrap(),
                    content,
                }
            })
            .collect();
        assert_eq!(project_cases(&evidence).unwrap().len(), 50);
    }
}
