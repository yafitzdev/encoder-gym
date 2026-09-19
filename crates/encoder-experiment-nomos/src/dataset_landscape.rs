//! Deterministic, development-only dataset intelligence for Agent protocol V2.
//! The complete training population is scanned locally; only aggregate cluster
//! summaries and explicitly sampled task-visible rows leave the adapter.

use std::collections::{BTreeMap, BTreeSet};

use encoder_optimization_core::{
    agent::InspectionItem,
    repair_strategy::{RepairPlanningAnchor, RepairPlanningCluster, RepairPlanningContext},
};
use serde_json::{Value, json};

use crate::{
    EncoderTaskAdapterError, NomosBackend, NomosDevelopmentCluster, NomosDevelopmentEvidence,
    NomosNativeInventory, adapter_error,
};

const LANDSCAPE_SCHEMA_VERSION: u32 = 1;
const INVESTIGATION_SCHEMA_VERSION: u32 = 1;
const INVESTIGATION_SELECTION_METHOD: &str = "native-context-strata-then-content-v1";
const MAX_INVESTIGATION_CLUSTERS: usize = 4;
const MAX_ANCHORS_PER_CLUSTER: usize = 8;
const MAX_INVESTIGATED_ROWS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ClusterKey {
    dimension: String,
    value: String,
}

#[derive(Debug, Clone)]
pub struct NomosDatasetLandscape {
    summaries: Vec<InspectionItem>,
    members: BTreeMap<String, Vec<String>>,
}

/// Protocol-V3 inventory joined to development evidence. The native inventory
/// supplies only fingerprints; task-visible row content remains behind bounded
/// inspection and only returned rows can authorize later edits.
#[derive(Debug, Clone)]
pub struct NomosDatasetInvestigation {
    summaries: Vec<InspectionItem>,
    clusters: BTreeMap<String, InvestigationCluster>,
    identities: BTreeMap<String, InvestigationIdentity>,
    memberships: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone)]
struct InvestigationCluster {
    members: Vec<String>,
    selection: Vec<String>,
    roles: BTreeMap<String, InvestigationRole>,
}

#[derive(Debug, Clone)]
struct InvestigationIdentity {
    context: String,
    model_input: String,
    label: String,
    content: String,
    exact_duplicate_group_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum InvestigationRole {
    SuspectContradictory { group_id: String },
    SuspectExactDuplicate { group_id: String },
    Contrast,
    Representative,
}

impl InvestigationRole {
    fn name(&self) -> &'static str {
        match self {
            Self::SuspectContradictory { .. } => "suspect_contradictory_label",
            Self::SuspectExactDuplicate { .. } => "suspect_exact_duplicate",
            Self::Contrast => "contrast",
            Self::Representative => "representative",
        }
    }

    fn group_id(&self) -> Option<&str> {
        match self {
            Self::SuspectContradictory { group_id } | Self::SuspectExactDuplicate { group_id } => {
                Some(group_id)
            }
            Self::Contrast | Self::Representative => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NomosDatasetInvestigationPage {
    pub items: Vec<InspectionItem>,
    pub selection_method: &'static str,
    pub total_eligible: u64,
    pub total_inspectable: u64,
}

impl NomosDatasetLandscape {
    pub fn build(
        rows: &BTreeMap<String, Value>,
        current: &[NomosDevelopmentEvidence],
        baseline: &[NomosDevelopmentEvidence],
    ) -> Result<Self, EncoderTaskAdapterError> {
        if rows.is_empty() || current.is_empty() || baseline.is_empty() {
            return Err(adapter_error(
                "Dataset landscape requires training rows and development reports",
            ));
        }
        let mut training_members: BTreeMap<ClusterKey, Vec<String>> = BTreeMap::new();
        for (id, row) in rows {
            for key in row_clusters(row)? {
                training_members.entry(key).or_default().push(id.clone());
            }
        }
        let current_clusters = development_clusters(current)?;
        let baseline_clusters = development_clusters(baseline)?;
        let current_support: BTreeMap<_, _> = current
            .iter()
            .map(|evidence| (evidence.suite_key.clone(), evidence.retrieval_support))
            .collect();
        let mut keys: BTreeSet<_> = training_members.keys().cloned().collect();
        keys.extend(current_clusters.keys().map(|(_, key)| key.clone()));
        keys.extend(baseline_clusters.keys().map(|(_, key)| key.clone()));

        let total_rows = rows.len() as u64;
        let mut ranked = Vec::new();
        for key in keys {
            let training_rows = training_members.get(&key).map_or(0, Vec::len) as u64;
            let training_share_ppm = share_ppm(training_rows, total_rows);
            let mut development = Vec::new();
            let mut error_count = 0_u64;
            let mut regression_ppm = 0_u64;
            let mut underrepresentation_ppm = 0_u64;
            for evidence in current {
                let lookup = (evidence.suite_key.clone(), key.clone());
                let Some(observed) = current_clusters.get(&lookup) else {
                    continue;
                };
                let reference = baseline_clusters.get(&lookup);
                let recall = observed.metrics["recall_at_1"];
                let errors = ((observed.support as f64) * (1.0 - recall)).round() as u64;
                error_count = error_count.saturating_add(errors);
                let regression = reference
                    .map(|value| value.metrics["recall_at_1"] - recall)
                    .unwrap_or(0.0)
                    .max(0.0);
                regression_ppm = regression_ppm.max((regression * 1_000_000.0).round() as u64);
                let baseline_metrics = reference.map(|value| &value.metrics);
                let deltas =
                    baseline_metrics.map(|metrics| metric_deltas(&observed.metrics, metrics));
                let support_share_ppm =
                    share_ppm(observed.support, current_support[&evidence.suite_key]);
                let coverage_gap_ppm = support_share_ppm as i64 - training_share_ppm as i64;
                underrepresentation_ppm =
                    underrepresentation_ppm.max(coverage_gap_ppm.max(0) as u64);
                development.push(json!({
                    "suite": evidence.suite_key,
                    "support": observed.support,
                    "population": {
                        "retrievalStates": evidence.retrieval_support,
                        "scientificReportSupport": evidence.report_support,
                        "agent": evidence.agent_population,
                    },
                    "supportSharePpm": support_share_ppm,
                    "trainingSharePpm": training_share_ppm,
                    "coverageGapPpm": coverage_gap_ppm,
                    "estimatedTop1Errors": errors,
                    "current": observed.metrics,
                    "originalBaseline": baseline_metrics,
                    "deltaFromOriginalBaseline": deltas,
                    "sampledFailureDiagnostics": sampled_failure_diagnostics(evidence, &key),
                }));
            }
            let content = json!({
                "kind": "dataset_cluster",
                "schemaVersion": LANDSCAPE_SCHEMA_VERSION,
                "cluster": {
                    "dimension": key.dimension,
                    "value": key.value,
                    "overlapsOtherClusters": key.dimension == "expected_capability",
                },
                "training": {
                    "rows": training_rows,
                    "totalRows": total_rows,
                    "sharePpm": training_share_ppm,
                },
                "development": development,
                "priority": {
                    "estimatedTop1Errors": error_count,
                    "maximumRecallAt1RegressionPpm": regression_ppm,
                    "maximumUnderrepresentationPpm": underrepresentation_ppm,
                },
                "interpretation": "Training coverage is descriptive, not proof that more rows will improve the metric.",
            });
            let fingerprint = artifact_core::fingerprint(&content).map_err(adapter_error)?;
            let id = format!("dataset-cluster-{fingerprint}");
            ranked.push((
                std::cmp::Reverse(error_count),
                std::cmp::Reverse(regression_ppm),
                std::cmp::Reverse(underrepresentation_ppm),
                key.dimension.clone(),
                key.value.clone(),
                id,
                fingerprint,
                content,
                training_members.remove(&key).unwrap_or_default(),
            ));
        }
        ranked.sort_by(|left, right| {
            (&left.0, &left.1, &left.2, &left.3, &left.4)
                .cmp(&(&right.0, &right.1, &right.2, &right.3, &right.4))
        });
        let mut summaries = Vec::with_capacity(ranked.len());
        let mut members = BTreeMap::new();
        for (_, _, _, _, _, id, fingerprint, content, cluster_members) in ranked {
            members.insert(id.clone(), cluster_members);
            summaries.push(InspectionItem {
                id,
                fingerprint,
                content,
            });
        }
        Ok(Self { summaries, members })
    }

    pub fn summaries(&self) -> &[InspectionItem] {
        &self.summaries
    }

    pub fn sample_rows(
        &self,
        rows: &BTreeMap<String, Value>,
        cluster_ids: &[String],
        examples_per_cluster: u32,
    ) -> Result<Vec<InspectionItem>, EncoderTaskAdapterError> {
        if cluster_ids.is_empty()
            || cluster_ids.len() > 4
            || !(1..=4).contains(&examples_per_cluster)
            || cluster_ids.iter().collect::<BTreeSet<_>>().len() != cluster_ids.len()
        {
            return Err(adapter_error(
                "Dataset cluster inspection requires 1–4 unique clusters and 1–4 examples each",
            ));
        }
        let mut selected: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for cluster_id in cluster_ids {
            let members = self.members.get(cluster_id).ok_or_else(|| {
                adapter_error("Dataset cluster identity was not returned by the landscape")
            })?;
            let mut ranked = members
                .iter()
                .map(|row_id| {
                    let rank = artifact_core::fingerprint(&json!({
                        "sampling": "cluster-content-hash-v1",
                        "cluster": cluster_id,
                        "row": row_id,
                    }))
                    .map_err(adapter_error)?;
                    Ok((rank, row_id))
                })
                .collect::<Result<Vec<_>, EncoderTaskAdapterError>>()?;
            ranked.sort();
            for (_, row_id) in ranked.into_iter().take(examples_per_cluster as usize) {
                selected
                    .entry(row_id.clone())
                    .or_default()
                    .insert(cluster_id.clone());
            }
        }
        let mut output = Vec::with_capacity(selected.len());
        for (row_id, clusters) in selected {
            let row = rows.get(&row_id).ok_or_else(|| {
                adapter_error("Dataset cluster member is missing from the pinned version")
            })?;
            let mut item = NomosBackend::training_row_for_agent(&row_id, row)?;
            let content = item
                .content
                .as_object_mut()
                .ok_or_else(|| adapter_error("Task-visible training row is not an object"))?;
            content.insert(
                "selectedForClusters".into(),
                serde_json::to_value(clusters).map_err(adapter_error)?,
            );
            content.insert(
                "sampling".into(),
                Value::String("cluster-content-hash-v1".into()),
            );
            item.fingerprint = artifact_core::fingerprint(&item.content).map_err(adapter_error)?;
            output.push(item);
        }
        Ok(output)
    }
}

impl NomosDatasetInvestigation {
    pub fn build(
        rows: &BTreeMap<String, Value>,
        current: &[NomosDevelopmentEvidence],
        baseline: &[NomosDevelopmentEvidence],
        inventory: &NomosNativeInventory,
    ) -> Result<Self, EncoderTaskAdapterError> {
        inventory.validate_population(&rows.keys().cloned().collect())?;
        let native = inventory.by_member();
        if native.len() != rows.len() || rows.keys().any(|id| !native.contains_key(id.as_str())) {
            return Err(adapter_error(
                "Native dataset inventory membership is incomplete",
            ));
        }
        let legacy = NomosDatasetLandscape::build(rows, current, baseline)?;
        let mut identities = rows
            .iter()
            .map(|(id, row)| {
                let projected = native[id.as_str()];
                Ok((
                    id.clone(),
                    InvestigationIdentity {
                        context: projected.native_context_fingerprint.clone(),
                        model_input: projected.native_model_input_fingerprint.clone(),
                        label: projected.label_fingerprint.clone(),
                        content: artifact_core::fingerprint(row).map_err(adapter_error)?,
                        exact_duplicate_group_id: None,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, EncoderTaskAdapterError>>()?;
        let groups = input_groups(&identities)?;
        for identity in identities.values_mut() {
            let group = &groups[&identity.model_input];
            if group.members.len() > 1 && group.labels.len() == 1 {
                identity.exact_duplicate_group_id = Some(group.id.clone());
            }
        }
        let context_labels = context_labels(&identities);
        let mut summaries = Vec::with_capacity(legacy.summaries.len());
        let mut clusters = BTreeMap::new();
        let mut memberships = BTreeMap::<String, BTreeSet<String>>::new();
        for summary in legacy.summaries {
            let legacy_members = legacy
                .members
                .get(&summary.id)
                .ok_or_else(|| adapter_error("Dataset landscape cluster membership is missing"))?;
            let dimension = summary.content["cluster"]["dimension"]
                .as_str()
                .ok_or_else(|| adapter_error("Dataset landscape cluster dimension is missing"))?;
            let value = summary.content["cluster"]["value"]
                .as_str()
                .ok_or_else(|| adapter_error("Dataset landscape cluster value is missing"))?;
            let stable_key = stable_cluster_key(dimension, value)?;
            for member in legacy_members {
                memberships
                    .entry(member.clone())
                    .or_default()
                    .insert(stable_key.clone());
            }
            let member_set = legacy_members.iter().collect::<BTreeSet<_>>();
            let distinct_contexts = legacy_members
                .iter()
                .map(|id| identities[id].context.as_str())
                .collect::<BTreeSet<_>>()
                .len() as u64;
            let mut exact_groups = BTreeSet::new();
            let mut contradictory_groups = BTreeSet::new();
            let mut roles = BTreeMap::new();
            for id in legacy_members {
                let identity = &identities[id];
                let group = &groups[&identity.model_input];
                let role = if group.labels.len() > 1 {
                    contradictory_groups.insert(group.id.clone());
                    InvestigationRole::SuspectContradictory {
                        group_id: group.id.clone(),
                    }
                } else if group.members.len() > 1 {
                    exact_groups.insert(group.id.clone());
                    InvestigationRole::SuspectExactDuplicate {
                        group_id: group.id.clone(),
                    }
                } else if context_labels[&identity.context].len() > 1 {
                    InvestigationRole::Contrast
                } else {
                    InvestigationRole::Representative
                };
                roles.insert(id.clone(), role);
            }
            let selection = investigation_selection(legacy_members, &identities, &roles);
            let exact_rows = groups
                .values()
                .filter(|group| exact_groups.contains(&group.id))
                .flat_map(|group| &group.members)
                .filter(|id| member_set.contains(id))
                .count() as u64;
            let contradictory_rows = groups
                .values()
                .filter(|group| contradictory_groups.contains(&group.id))
                .flat_map(|group| &group.members)
                .filter(|id| member_set.contains(id))
                .count() as u64;
            let generation_capacity = distinct_contexts
                .min(MAX_ANCHORS_PER_CLUSTER as u64)
                .saturating_mul(8);
            let mut content = summary.content;
            content["schemaVersion"] = INVESTIGATION_SCHEMA_VERSION.into();
            content["cluster"]["stableKey"] = stable_key.clone().into();
            content["training"]["inventory"] = json!({
                "protocol": inventory.protocol,
                "fingerprint": inventory.fingerprint,
                "datasetFingerprint": inventory.dataset_fingerprint,
                "distinctNativeContexts": distinct_contexts,
                "generationCapacityRows": generation_capacity,
                "capacityLimit": {
                    "maximumAnchors": MAX_ANCHORS_PER_CLUSTER,
                    "maximumAdditionsPerAnchor": 8,
                },
                "suspectGroups": {
                    "exactDuplicateGroups": exact_groups.len(),
                    "exactDuplicateRowsInCluster": exact_rows,
                    "contradictoryLabelGroups": contradictory_groups.len(),
                    "contradictoryLabelRowsInCluster": contradictory_rows,
                    "interpretation": "Contradictions are review findings and never automatic relabels or removals.",
                },
            });
            content["inspection"] = json!({
                "selectionMethod": INVESTIGATION_SELECTION_METHOD,
                "totalEligibleRows": legacy_members.len(),
                "maximumInspectableRows": selection.len(),
                "distinctContextStrataFirst": true,
            });
            let fingerprint = artifact_core::fingerprint(&content).map_err(adapter_error)?;
            summaries.push(InspectionItem {
                id: stable_key.clone(),
                fingerprint,
                content,
            });
            if clusters
                .insert(
                    stable_key,
                    InvestigationCluster {
                        members: legacy_members.clone(),
                        selection,
                        roles,
                    },
                )
                .is_some()
            {
                return Err(adapter_error("Stable dataset cluster identity is repeated"));
            }
        }
        Ok(Self {
            summaries,
            clusters,
            identities,
            memberships,
        })
    }

    pub fn summaries(&self) -> &[InspectionItem] {
        &self.summaries
    }

    pub fn sample_rows(
        &self,
        rows: &BTreeMap<String, Value>,
        cluster_ids: &[String],
        offset: u64,
        limit: u32,
    ) -> Result<NomosDatasetInvestigationPage, EncoderTaskAdapterError> {
        if cluster_ids.is_empty()
            || cluster_ids.len() > MAX_INVESTIGATION_CLUSTERS
            || cluster_ids.iter().collect::<BTreeSet<_>>().len() != cluster_ids.len()
            || !(1..=MAX_INVESTIGATED_ROWS as u32).contains(&limit)
        {
            return Err(adapter_error(
                "Dataset investigation requires 1–4 unique clusters and a 1–32 row page",
            ));
        }
        let mut requested = cluster_ids.to_vec();
        requested.sort();
        let mut selectable = Vec::new();
        let mut eligible = BTreeSet::new();
        let mut selected_for = BTreeMap::<String, BTreeSet<String>>::new();
        for cluster_id in &requested {
            let cluster = self.clusters.get(cluster_id).ok_or_else(|| {
                adapter_error("Dataset cluster identity was not returned by the investigation")
            })?;
            eligible.extend(cluster.members.iter().cloned());
            for row_id in &cluster.selection {
                selected_for
                    .entry(row_id.clone())
                    .or_default()
                    .insert(cluster_id.clone());
                if !selectable.contains(row_id) {
                    selectable.push(row_id.clone());
                }
            }
        }
        let start = usize::try_from(offset)
            .map_err(|_| adapter_error("Dataset investigation cursor exceeds this platform"))?;
        if start > selectable.len() {
            return Err(adapter_error(
                "Dataset investigation cursor exceeds the inspectable selection",
            ));
        }
        let mut items = Vec::new();
        for row_id in selectable.iter().skip(start).take(limit as usize) {
            let row = rows.get(row_id).ok_or_else(|| {
                adapter_error("Dataset investigation member is missing from the pinned version")
            })?;
            let mut item = NomosBackend::training_row_for_agent(row_id, row)?;
            let anchor_fingerprint = item.fingerprint.clone();
            let content = item
                .content
                .as_object_mut()
                .ok_or_else(|| adapter_error("Task-visible training row is not an object"))?;
            let cluster_selections = selected_for[row_id]
                .iter()
                .map(|cluster_id| {
                    let role = &self.clusters[cluster_id].roles[row_id];
                    json!({
                        "clusterId": cluster_id,
                        "role": role.name(),
                        "suspectGroupId": role.group_id(),
                    })
                })
                .collect::<Vec<_>>();
            let identity = &self.identities[row_id];
            content.insert(
                "selectedForClusters".into(),
                serde_json::to_value(&selected_for[row_id]).map_err(adapter_error)?,
            );
            content.insert(
                "investigation".into(),
                json!({
                    "selectionMethod": INVESTIGATION_SELECTION_METHOD,
                    "nativeContextFingerprint": identity.context,
                    "nativeModelInputFingerprint": identity.model_input,
                    "labelFingerprint": identity.label,
                    "anchorFingerprint": anchor_fingerprint,
                    "selections": cluster_selections,
                }),
            );
            item.fingerprint = artifact_core::fingerprint(&item.content).map_err(adapter_error)?;
            items.push(item);
        }
        Ok(NomosDatasetInvestigationPage {
            items,
            selection_method: INVESTIGATION_SELECTION_METHOD,
            total_eligible: eligible.len() as u64,
            total_inspectable: selectable.len() as u64,
        })
    }

    pub fn planning_context(
        &self,
        rows: &BTreeMap<String, Value>,
        inspected_cluster_ids: &[String],
        inspected_row_ids: &[String],
        remaining_row_changes: u32,
    ) -> Result<RepairPlanningContext, EncoderTaskAdapterError> {
        let mut clusters = BTreeMap::new();
        let summaries = self
            .summaries
            .iter()
            .map(|summary| (summary.id.as_str(), summary))
            .collect::<BTreeMap<_, _>>();
        for id in inspected_cluster_ids {
            let summary = summaries.get(id.as_str()).ok_or_else(|| {
                adapter_error("Repair planning references an uninspected dataset cluster")
            })?;
            let metrics = summary.content["development"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|suite| suite["current"].as_object())
                .flat_map(|metrics| metrics.keys().cloned())
                .collect::<BTreeSet<_>>();
            clusters.insert(
                id.clone(),
                RepairPlanningCluster {
                    training_rows: summary.content["training"]["rows"].as_u64().ok_or_else(
                        || adapter_error("Dataset cluster training count is missing"),
                    )?,
                    metrics,
                },
            );
        }
        let mut anchors = BTreeMap::new();
        for id in inspected_row_ids {
            let row = rows.get(id).ok_or_else(|| {
                adapter_error("Repair planning references an uninspected training row")
            })?;
            let identity = self.identities.get(id).ok_or_else(|| {
                adapter_error("Repair planning row has no native inventory identity")
            })?;
            let projection = NomosBackend::training_row_for_agent(id, row)?;
            anchors.insert(
                id.clone(),
                RepairPlanningAnchor {
                    fingerprint: projection.fingerprint,
                    cluster_keys: self
                        .memberships
                        .get(id)
                        .into_iter()
                        .flatten()
                        .filter(|key| clusters.contains_key(*key))
                        .cloned()
                        .collect(),
                    native_context_fingerprint: identity.context.clone(),
                    native_model_input_fingerprint: identity.model_input.clone(),
                    label_fingerprint: identity.label.clone(),
                    exact_duplicate_group_id: identity.exact_duplicate_group_id.clone(),
                },
            );
        }
        let context = RepairPlanningContext {
            dataset_rows: rows.len() as u64,
            remaining_row_changes,
            clusters,
            anchors,
            evidence_ids: inspected_cluster_ids.iter().cloned().collect(),
            prior_interventions: BTreeSet::new(),
        };
        context.validate().map_err(adapter_error)?;
        Ok(context)
    }
}

#[derive(Debug)]
struct NativeInputGroup {
    id: String,
    members: Vec<String>,
    labels: BTreeSet<String>,
}

fn input_groups(
    identities: &BTreeMap<String, InvestigationIdentity>,
) -> Result<BTreeMap<String, NativeInputGroup>, EncoderTaskAdapterError> {
    let mut groups = BTreeMap::<String, (Vec<String>, BTreeSet<String>)>::new();
    for (id, identity) in identities {
        let entry = groups.entry(identity.model_input.clone()).or_default();
        entry.0.push(id.clone());
        entry.1.insert(identity.label.clone());
    }
    groups
        .into_iter()
        .map(|(input, (members, labels))| {
            let kind = if labels.len() > 1 {
                "contradictory-label"
            } else {
                "exact-duplicate"
            };
            let id = format!(
                "nomos-suspect-{}",
                artifact_core::fingerprint(&json!({
                    "protocol": "nomos-native-input-group-v1",
                    "kind": kind,
                    "input": input,
                }))
                .map_err(adapter_error)?
            );
            Ok((
                input,
                NativeInputGroup {
                    id,
                    members,
                    labels,
                },
            ))
        })
        .collect()
}

fn context_labels(
    identities: &BTreeMap<String, InvestigationIdentity>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut result = BTreeMap::<String, BTreeSet<String>>::new();
    for identity in identities.values() {
        result
            .entry(identity.context.clone())
            .or_default()
            .insert(identity.label.clone());
    }
    result
}

fn investigation_selection(
    members: &[String],
    identities: &BTreeMap<String, InvestigationIdentity>,
    roles: &BTreeMap<String, InvestigationRole>,
) -> Vec<String> {
    let mut strata = BTreeMap::<String, Vec<String>>::new();
    for id in members {
        strata
            .entry(identities[id].context.clone())
            .or_default()
            .push(id.clone());
    }
    for rows in strata.values_mut() {
        rows.sort_by(|left, right| {
            (&roles[left], &identities[left].content, left).cmp(&(
                &roles[right],
                &identities[right].content,
                right,
            ))
        });
    }
    let mut contexts = strata.keys().cloned().collect::<Vec<_>>();
    contexts.sort_by(|left, right| {
        let left_id = &strata[left][0];
        let right_id = &strata[right][0];
        (&roles[left_id], &identities[left_id].content, left).cmp(&(
            &roles[right_id],
            &identities[right_id].content,
            right,
        ))
    });
    let mut output = Vec::new();
    for layer in 0..members.len() {
        for context in &contexts {
            if let Some(id) = strata[context].get(layer) {
                output.push(id.clone());
                if output.len() == MAX_ANCHORS_PER_CLUSTER {
                    return output;
                }
            }
        }
    }
    output
}

pub(crate) fn stable_cluster_key(
    dimension: &str,
    value: &str,
) -> Result<String, EncoderTaskAdapterError> {
    Ok(format!(
        "nomos-cluster-{}",
        artifact_core::fingerprint(&json!({
            "protocol": "nomos-semantic-cluster-key-v1",
            "dimension": dimension,
            "value": value,
        }))
        .map_err(adapter_error)?
    ))
}

fn development_clusters(
    evidence: &[NomosDevelopmentEvidence],
) -> Result<BTreeMap<(String, ClusterKey), NomosDevelopmentCluster>, EncoderTaskAdapterError> {
    let mut result = BTreeMap::new();
    for report in evidence {
        if report.clusters.is_empty() {
            return Err(adapter_error(
                "Development report predates dataset-intelligence cluster evidence",
            ));
        }
        for cluster in &report.clusters {
            let key = (
                report.suite_key.clone(),
                ClusterKey {
                    dimension: cluster.dimension.clone(),
                    value: cluster.value.clone(),
                },
            );
            if result.insert(key, cluster.clone()).is_some() {
                return Err(adapter_error("Development cluster identity is repeated"));
            }
        }
    }
    Ok(result)
}

fn row_clusters(row: &Value) -> Result<BTreeSet<ClusterKey>, EncoderTaskAdapterError> {
    crate::managed_training::validate_native_training_row(row)?;
    let task_kind = row["task_kind"]
        .as_str()
        .ok_or_else(|| adapter_error("Validated training row has no task kind"))?;
    let scenario = row
        .get("matrix_cell")
        .and_then(|value| value.get("scenario_family"))
        .and_then(Value::as_str)
        .unwrap_or("unspecified");
    let legal: BTreeSet<_> = row["legal_candidate_ids"]
        .as_array()
        .ok_or_else(|| adapter_error("Validated training row has no legal candidate set"))?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let previous: BTreeSet<_> = row["previous_candidate_ids"]
        .as_array()
        .ok_or_else(|| adapter_error("Validated training row has no previous candidate set"))?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let eligible: BTreeSet<_> = if task_kind == "recover" {
        legal.difference(&previous).copied().collect()
    } else {
        legal
    };
    let pool = eligible.len().to_string();
    let acceptable: BTreeSet<_> = row["label"]["acceptable_tools"]
        .as_array()
        .ok_or_else(|| adapter_error("Validated training row has no acceptable tools"))?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut result = BTreeSet::from([
        ClusterKey {
            dimension: "task_kind".into(),
            value: task_kind.into(),
        },
        ClusterKey {
            dimension: "scenario_family".into(),
            value: scenario.into(),
        },
        ClusterKey {
            dimension: "candidate_pool_size".into(),
            value: pool,
        },
    ]);
    for tool in row["tool_registry"]["tools"]
        .as_array()
        .ok_or_else(|| adapter_error("Validated training row has no tool registry"))?
    {
        if !tool["tool_id"]
            .as_str()
            .is_some_and(|id| acceptable.contains(id) && eligible.contains(id))
        {
            continue;
        }
        for capability in tool["capabilities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            result.insert(ClusterKey {
                dimension: "expected_capability".into(),
                value: capability.into(),
            });
        }
    }
    Ok(result)
}

fn metric_deltas(
    current: &BTreeMap<String, f64>,
    baseline: &BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    current
        .iter()
        .filter_map(|(name, value)| {
            baseline
                .get(name)
                .map(|reference| (name.clone(), value - reference))
        })
        .collect()
}

fn sampled_failure_diagnostics(evidence: &NomosDevelopmentEvidence, key: &ClusterKey) -> Value {
    let matching: Vec<_> = evidence
        .failures
        .iter()
        .filter(|failure| failure_matches_cluster(&failure.content, key))
        .collect();
    let mut predicted_capabilities = BTreeMap::<String, u64>::new();
    let mut rank_counts = BTreeMap::<String, u64>::new();
    for failure in &matching {
        for capability in failure.content["predictedCapabilities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            *predicted_capabilities
                .entry(capability.to_owned())
                .or_default() += 1;
        }
        let rank = failure.content["expectedRank"]
            .as_u64()
            .map_or_else(|| "missing".into(), |value| value.to_string());
        *rank_counts.entry(rank).or_default() += 1;
    }
    let examples: Vec<_> = matching
        .iter()
        .take(2)
        .map(|failure| {
            let question = failure.content["question"].as_str().unwrap_or_default();
            let preview: String = question.chars().take(1_000).collect();
            json!({
                "evidenceId": failure.id,
                "questionPreview": preview,
                "questionPreviewTruncated": question.chars().count() > 1_000,
                "expectedCapabilities": failure.content["expectedCapabilities"],
                "predictedCapabilities": failure.content["predictedCapabilities"],
                "expectedRank": failure.content["expectedRank"],
            })
        })
        .collect();
    json!({
        "evidenceScope": "retrieval_failure_sample",
        "reportSampleLimit": evidence.failure_sample_limit,
        "matchingFailures": matching.len(),
        "predictedCapabilityCounts": predicted_capabilities,
        "expectedRankCounts": rank_counts,
        "examples": examples,
        "interpretation": "This is a bounded diagnostic sample, not the complete error population.",
    })
}

fn failure_matches_cluster(content: &Value, key: &ClusterKey) -> bool {
    match key.dimension.as_str() {
        "task_kind" => content["taskKind"].as_str() == Some(&key.value),
        "expected_capability" => content["expectedCapabilities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|capability| capability == key.value),
        // The native failure projection does not expose these fields. Do not
        // guess membership from question text.
        "scenario_family" | "candidate_pool_size" => false,
        _ => false,
    }
}

fn share_ppm(part: u64, total: u64) -> u64 {
    if total == 0 {
        0
    } else {
        ((part as f64 / total as f64) * 1_000_000.0).round() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(recall: f64) -> BTreeMap<String, f64> {
        BTreeMap::from([
            ("recall_at_1".into(), recall),
            ("recall_at_2".into(), recall),
            ("recall_at_3".into(), recall),
            ("mrr".into(), recall),
            ("mean_positive_margin".into(), 0.1),
        ])
    }

    fn evidence(suite: &str, recall: f64) -> NomosDevelopmentEvidence {
        evidence_with_support(suite, recall, 10, 10, 10)
    }

    fn evidence_with_support(
        suite: &str,
        recall: f64,
        report_support: u64,
        retrieval_support: u64,
        cluster_support: u64,
    ) -> NomosDevelopmentEvidence {
        NomosDevelopmentEvidence {
            schema_version: crate::development_evidence::DEVELOPMENT_EVIDENCE_SCHEMA_VERSION,
            report_id: uuid::Uuid::new_v4(),
            report_fingerprint: format!("sha256:{}", "1".repeat(64)),
            suite_key: suite.into(),
            artifact_fingerprint: format!("sha256:{}", "2".repeat(64)),
            report_support,
            retrieval_support,
            agent_population: crate::NomosAgentPopulation::NotRequired,
            clusters: vec![NomosDevelopmentCluster {
                dimension: "expected_capability".into(),
                value: "search".into(),
                support: cluster_support,
                metrics: metric(recall),
            }],
            failures: vec![],
            failure_sample_limit: 50,
        }
    }

    fn row(question: &str) -> Value {
        json!({
            "schema_version":"decision-state.v2", "evaluation_partition":"train",
            "accepted":true, "decision_state_id":format!("state-{question}"),
            "question":question, "task_kind":"route",
            "previous_candidate_ids":[], "legal_candidate_ids":["tool-1","tool-2"],
            "label":{"acceptable_tools":["tool-1"],"hard_negative_tools":["tool-2"]},
            "tool_registry":{"registry_id":"registry","registry_fingerprint":"sha256:registry","tools":[
                {"tool_id":"tool-1","description":"search", "capabilities":["search"], "side_effect_class":"none", "argument_schema":{}, "evidence_roles":["observation"], "input_modalities":["text"], "output_modalities":["evidence"], "tool_family":"search"},
                {"tool_id":"tool-2","description":"write", "capabilities":["write"], "side_effect_class":"write", "argument_schema":{}, "evidence_roles":["action"], "input_modalities":["text"], "output_modalities":["text"], "tool_family":"write"}
            ]}
        })
    }

    fn inventory(
        rows: &BTreeMap<String, Value>,
        identities: &[(&str, &str, &str, &str)],
    ) -> NomosNativeInventory {
        let mut inventory = NomosNativeInventory {
            protocol: "nomos-training-inventory-v1".into(),
            request_fingerprint: artifact_core::fingerprint(&"request").unwrap(),
            dataset_fingerprint: artifact_core::fingerprint(&"dataset").unwrap(),
            rows: rows.len() as u64,
            members: identities
                .iter()
                .map(
                    |(id, context, input, label)| crate::NomosNativeInventoryMember {
                        member_id: (*id).into(),
                        native_context_fingerprint: artifact_core::fingerprint(context).unwrap(),
                        native_model_input_fingerprint: artifact_core::fingerprint(input).unwrap(),
                        label_fingerprint: artifact_core::fingerprint(label).unwrap(),
                    },
                )
                .collect(),
            fingerprint: String::new(),
        };
        inventory
            .members
            .sort_by(|left, right| left.member_id.cmp(&right.member_id));
        inventory.fingerprint = inventory.reproduce().unwrap();
        inventory
    }

    #[test]
    fn complete_training_scan_joins_development_clusters_and_samples_by_cluster() {
        let rows = BTreeMap::from([
            ("row-a".into(), row("find alpha")),
            ("row-b".into(), row("find beta")),
        ]);
        let landscape = NomosDatasetLandscape::build(
            &rows,
            &[evidence("development", 0.5)],
            &[evidence("development", 0.8)],
        )
        .unwrap();
        let search = landscape
            .summaries()
            .iter()
            .find(|item| item.content["cluster"]["value"] == "search")
            .unwrap();
        assert_eq!(search.content["training"]["rows"], 2);
        assert_eq!(search.content["development"][0]["estimatedTop1Errors"], 5);
        assert_eq!(
            search.content["priority"]["maximumRecallAt1RegressionPpm"],
            300_000
        );
        let sampled = landscape
            .sample_rows(&rows, std::slice::from_ref(&search.id), 2)
            .unwrap();
        assert_eq!(sampled.len(), 2);
        assert!(sampled.iter().all(|item| {
            item.content["selectedForClusters"]
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == &search.id))
        }));
    }

    #[test]
    fn retrieval_cluster_share_does_not_use_smaller_agent_report_support() {
        let rows = BTreeMap::from([("row-a".into(), row("find alpha"))]);
        let mut current = evidence_with_support("development", 0.5, 16, 1_000, 100);
        current.agent_population = crate::NomosAgentPopulation::Available {
            artifact_fingerprint: format!("sha256:{}", "3".repeat(64)),
            sessions: 16,
            tool_call_attempts: 32,
            valid_execution_attempts: 30,
            metric_denominators: BTreeMap::from([
                ("agent_success_rate".into(), Some(16)),
                ("agent_wrong_tool_execution_rate".into(), Some(30)),
            ]),
        };
        let landscape = NomosDatasetLandscape::build(
            &rows,
            &[current],
            &[evidence_with_support("development", 0.8, 16, 1_000, 100)],
        )
        .unwrap();
        let search = landscape
            .summaries()
            .iter()
            .find(|item| item.content["cluster"]["value"] == "search")
            .unwrap();
        assert_eq!(search.content["development"][0]["supportSharePpm"], 100_000);
        assert_eq!(
            search.content["development"][0]["population"]["retrievalStates"],
            1_000
        );
        assert_eq!(
            search.content["development"][0]["population"]["scientificReportSupport"],
            16
        );
        assert_eq!(
            search.content["development"][0]["population"]["agent"]["sessions"],
            16
        );
    }

    #[test]
    fn v3_inventory_has_stable_clusters_native_suspects_and_context_diverse_pages() {
        let mut conflicting = row("same input");
        conflicting["label"] = json!({
            "acceptable_tools":["tool-2"],
            "hard_negative_tools":["tool-1"],
        });
        let rows = BTreeMap::from([
            ("row-a".into(), row("same input")),
            ("row-b".into(), row("same input")),
            ("row-c".into(), conflicting),
            ("row-d".into(), row("different input")),
        ]);
        let native = inventory(
            &rows,
            &[
                ("row-a", "context-one", "input-one", "label-one"),
                ("row-b", "context-one", "input-one", "label-one"),
                ("row-c", "context-one", "input-one", "label-two"),
                ("row-d", "context-two", "input-two", "label-one"),
            ],
        );
        let investigation = NomosDatasetInvestigation::build(
            &rows,
            &[evidence("development", 0.5)],
            &[evidence("development", 0.8)],
            &native,
        )
        .unwrap();
        let route = investigation
            .summaries()
            .iter()
            .find(|item| item.content["cluster"]["dimension"] == "task_kind")
            .unwrap();
        assert!(route.id.starts_with("nomos-cluster-sha256:"));
        assert_eq!(route.content["cluster"]["stableKey"], route.id);
        assert_eq!(
            route.content["training"]["inventory"]["distinctNativeContexts"],
            2
        );
        assert_eq!(
            route.content["training"]["inventory"]["generationCapacityRows"],
            16
        );
        assert_eq!(
            route.content["training"]["inventory"]["suspectGroups"]["contradictoryLabelGroups"],
            1
        );
        let page = investigation
            .sample_rows(&rows, std::slice::from_ref(&route.id), 0, 2)
            .unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.total_eligible, 4);
        assert_eq!(page.total_inspectable, 4);
        assert_ne!(
            page.items[0].content["investigation"]["nativeContextFingerprint"],
            page.items[1].content["investigation"]["nativeContextFingerprint"]
        );
        assert_eq!(
            page.items[0].content["investigation"]["selections"][0]["role"],
            "suspect_contradictory_label"
        );
        let inspected_ids = page
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        let context = investigation
            .planning_context(&rows, std::slice::from_ref(&route.id), &inspected_ids, 12)
            .unwrap();
        assert_eq!(context.dataset_rows, 4);
        assert_eq!(context.remaining_row_changes, 12);
        assert_eq!(context.clusters.len(), 1);
        assert_eq!(context.anchors.len(), 2);
        for item in &page.items {
            assert_eq!(
                context.anchors[&item.id].fingerprint,
                item.content["investigation"]["anchorFingerprint"]
            );
            assert_eq!(
                context.anchors[&item.id].cluster_keys,
                BTreeSet::from([route.id.clone()])
            );
            assert_eq!(
                context.anchors[&item.id].exact_duplicate_group_id, None,
                "contradictory labels are findings, not proven exact duplicates"
            );
        }
        assert!(!context.anchors.contains_key("row-c"));
        let refreshed = NomosDatasetInvestigation::build(
            &rows,
            &[evidence("development", 0.2)],
            &[evidence("development", 0.8)],
            &native,
        )
        .unwrap();
        let refreshed_route = refreshed
            .summaries()
            .iter()
            .find(|item| item.content["cluster"]["dimension"] == "task_kind")
            .unwrap();
        assert_eq!(route.id, refreshed_route.id);
        let search = investigation
            .summaries()
            .iter()
            .find(|item| item.content["cluster"]["value"] == "search")
            .unwrap();
        let refreshed_search = refreshed
            .summaries()
            .iter()
            .find(|item| item.content["cluster"]["value"] == "search")
            .unwrap();
        assert_eq!(search.id, refreshed_search.id);
        assert_ne!(search.fingerprint, refreshed_search.fingerprint);
    }
}
