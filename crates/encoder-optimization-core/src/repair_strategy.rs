//! Pure protocol-V3 repair planning and exact allocation.
//!
//! This module grants no execution authority. It compiles a model-authored
//! hypothesis against host-owned inspected clusters/anchors and returns either
//! an immutable preview or typed constraints. Generation must reproduce the
//! preview fingerprint before dispatch.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{OptimizationError, fingerprint, require};

pub const REPAIR_PLAN_SCHEMA_VERSION: u32 = 3;
pub const MAX_REPAIR_TARGETS: usize = 4;
pub const MAX_ANCHORS_PER_TARGET: usize = 8;
pub const MAX_INSPECTED_ROWS_PER_ITERATION: usize = 32;
pub const MAX_ADDITIONS_PER_ANCHOR: u32 = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairPlan {
    pub schema_version: u32,
    pub summary: String,
    pub stop: bool,
    pub targets: Vec<RepairTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairTarget {
    pub target_id: String,
    pub cluster_keys: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub hypothesis: String,
    pub evidence_limitations: String,
    pub intended_failure_pattern: String,
    pub alternative_explanation: String,
    pub operation: RepairOperation,
    pub target_metric: TargetMetric,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RepairOperation {
    LabelPreservingVariants {
        count: AdditionCount,
        #[serde(rename = "allocationRationale")]
        allocation_rationale: String,
        anchors: Vec<AnchorAllocation>,
    },
    ExistingAnchorContrast {
        count: AdditionCount,
        #[serde(rename = "allocationRationale")]
        allocation_rationale: String,
        pairs: Vec<ContrastPairAllocation>,
    },
    ProvenRedundantRowRemoval {
        removals: Vec<ProvenRemoval>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "basis", rename_all = "snake_case", deny_unknown_fields)]
pub enum AdditionCount {
    AbsoluteRows {
        #[serde(rename = "desiredRows")]
        desired_rows: u32,
    },
    RelativeClusterGrowth {
        #[serde(rename = "sourceClusterKey")]
        source_cluster_key: String,
        #[serde(rename = "pinnedSourceRows")]
        pinned_source_rows: u64,
        #[serde(rename = "basisPoints")]
        basis_points: u32,
        #[serde(rename = "desiredRows")]
        desired_rows: u32,
    },
}

impl AdditionCount {
    fn desired(
        &self,
        clusters: &BTreeMap<String, RepairPlanningCluster>,
    ) -> Result<u32, RepairPlanConstraintCode> {
        match self {
            Self::AbsoluteRows { desired_rows } if *desired_rows > 0 => Ok(*desired_rows),
            Self::AbsoluteRows { .. } => Err(RepairPlanConstraintCode::InvalidCountBasis),
            Self::RelativeClusterGrowth {
                source_cluster_key,
                pinned_source_rows,
                basis_points,
                desired_rows,
            } => {
                let Some(cluster) = clusters.get(source_cluster_key) else {
                    return Err(RepairPlanConstraintCode::UnknownCluster);
                };
                if *pinned_source_rows == 0
                    || *pinned_source_rows != cluster.training_rows
                    || *basis_points == 0
                    || *basis_points > 10_000
                {
                    return Err(RepairPlanConstraintCode::InvalidCountBasis);
                }
                let numerator = pinned_source_rows
                    .checked_mul(u64::from(*basis_points))
                    .ok_or(RepairPlanConstraintCode::CountOverflow)?;
                let calculated = numerator
                    .checked_add(9_999)
                    .ok_or(RepairPlanConstraintCode::CountOverflow)?
                    / 10_000;
                let calculated = u32::try_from(calculated)
                    .map_err(|_| RepairPlanConstraintCode::CountOverflow)?;
                if calculated == 0 || calculated != *desired_rows {
                    return Err(RepairPlanConstraintCode::InvalidCountBasis);
                }
                Ok(calculated)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnchorAllocation {
    pub row_id: String,
    pub row_fingerprint: String,
    pub additions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContrastPairAllocation {
    pub pair_id: String,
    pub left: AnchorReference,
    pub right: AnchorReference,
    pub additions_per_side: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AnchorReference {
    pub row_id: String,
    pub row_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProvenRemoval {
    pub row_id: String,
    pub row_fingerprint: String,
    pub retained_row_id: String,
    pub retained_row_fingerprint: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetMetric {
    pub name: String,
    pub direction: MetricDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricDirection {
    Increase,
    Decrease,
}

/// Host-owned facts derived only from inspection results actually returned in
/// this iteration. A caller may not populate it from uninspected dataset rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairPlanningContext {
    pub dataset_rows: u64,
    pub remaining_row_changes: u32,
    pub clusters: BTreeMap<String, RepairPlanningCluster>,
    pub anchors: BTreeMap<String, RepairPlanningAnchor>,
    pub evidence_ids: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub prior_interventions: BTreeSet<String>,
}

impl RepairPlanningContext {
    pub fn validate(&self) -> Result<(), OptimizationError> {
        require(self.dataset_rows > 0, "Repair planning dataset is empty")?;
        require(
            self.remaining_row_changes <= 5_000,
            "Repair planning row-change allowance exceeds 5000",
        )?;
        require(
            self.anchors.len() <= MAX_INSPECTED_ROWS_PER_ITERATION,
            "Repair planning contains more than 32 inspected rows",
        )?;
        for (key, cluster) in &self.clusters {
            require(
                valid_id(key) && cluster.training_rows <= self.dataset_rows,
                "Repair planning cluster is invalid",
            )?;
            require(
                cluster.metrics.iter().all(|name| valid_text(name, 128)),
                "Repair planning cluster metrics are invalid",
            )?;
        }
        for (id, anchor) in &self.anchors {
            require(
                valid_id(id)
                    && canonical_fingerprint(&anchor.fingerprint)
                    && canonical_fingerprint(&anchor.native_context_fingerprint)
                    && canonical_fingerprint(&anchor.native_model_input_fingerprint)
                    && canonical_fingerprint(&anchor.label_fingerprint)
                    && anchor
                        .exact_duplicate_group_id
                        .as_deref()
                        .is_none_or(valid_id)
                    && anchor
                        .cluster_keys
                        .iter()
                        .all(|key| self.clusters.contains_key(key)),
                "Repair planning anchor is invalid",
            )?;
        }
        require(
            self.evidence_ids.iter().all(|id| valid_id(id))
                && self
                    .prior_interventions
                    .iter()
                    .all(|value| canonical_fingerprint(value)),
            "Repair planning evidence identity is invalid",
        )
    }

    pub fn fingerprint(&self) -> Result<String, OptimizationError> {
        self.validate()?;
        fingerprint(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairPlanningCluster {
    pub training_rows: u64,
    pub metrics: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairPlanningAnchor {
    pub fingerprint: String,
    pub cluster_keys: BTreeSet<String>,
    pub native_context_fingerprint: String,
    pub native_model_input_fingerprint: String,
    pub label_fingerprint: String,
    pub exact_duplicate_group_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairPlanPreview {
    pub schema_version: u32,
    pub plan_fingerprint: String,
    pub context_fingerprint: String,
    pub input_rows: u64,
    pub projected_rows: u64,
    pub desired_additions: u32,
    pub executable_additions: u32,
    pub removals: u32,
    pub unused_row_change_allowance: u32,
    pub targets: Vec<RepairTargetPreview>,
    pub projected_clusters: BTreeMap<String, ProjectedCluster>,
    pub fingerprint: String,
}

impl RepairPlanPreview {
    fn reproduce(&self) -> Result<String, OptimizationError> {
        let mut value = serde_json::to_value(self)?;
        value
            .as_object_mut()
            .expect("preview object")
            .remove("fingerprint");
        fingerprint(&value)
    }

    pub fn validate(&self) -> Result<(), OptimizationError> {
        require(
            self.schema_version == REPAIR_PLAN_SCHEMA_VERSION
                && self.executable_additions == self.desired_additions
                && self.fingerprint == self.reproduce()?,
            "Repair-plan preview is invalid",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairTargetPreview {
    pub target_id: String,
    pub desired_additions: u32,
    pub executable_additions: u32,
    pub removals: u32,
    pub limiting_constraints: Vec<RepairPlanConstraintCode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectedCluster {
    pub current_rows: u64,
    pub additions: u64,
    pub removals: u64,
    pub projected_rows: u64,
    pub projected_share_ppm: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairPlanCompilation {
    pub preview: Option<RepairPlanPreview>,
    pub constraints: Vec<RepairPlanConstraint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepairPlanConstraint {
    pub code: RepairPlanConstraintCode,
    pub target_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairPlanConstraintCode {
    InvalidPlanShape,
    InvalidTargetText,
    UnknownCluster,
    UnknownEvidence,
    UnknownMetric,
    UnknownAnchor,
    AnchorOutsideTarget,
    AnchorFingerprintMismatch,
    TooManyTargets,
    TooManyAnchors,
    TooManyInspectedRows,
    InvalidCountBasis,
    CountOverflow,
    AllocationMismatch,
    AdditionPerAnchorExceeded,
    ContrastRequiresEvenCount,
    ContrastContextMismatch,
    ContrastLabelMismatch,
    DuplicateRemovalUnproven,
    RepeatedEdit,
    RepeatedUnchangedIntervention,
    RowChangeBudgetExceeded,
}

pub fn compile_repair_plan(
    plan: &RepairPlan,
    context: &RepairPlanningContext,
) -> Result<RepairPlanCompilation, OptimizationError> {
    context.validate()?;
    let mut compiler = Compiler::new(plan, context);
    compiler.compile();
    if !compiler.constraints.is_empty() {
        return Ok(RepairPlanCompilation {
            preview: None,
            constraints: compiler.constraints,
        });
    }
    let mut preview = compiler.preview()?;
    preview.fingerprint = preview.reproduce()?;
    preview.validate()?;
    Ok(RepairPlanCompilation {
        preview: Some(preview),
        constraints: Vec::new(),
    })
}

pub fn verify_repair_plan_submission(
    plan: &RepairPlan,
    context: &RepairPlanningContext,
    preview_fingerprint: &str,
) -> Result<RepairPlanPreview, OptimizationError> {
    let compilation = compile_repair_plan(plan, context)?;
    let preview = compilation
        .preview
        .ok_or_else(|| OptimizationError::Validation("Repair plan is not feasible".into()))?;
    require(
        canonical_fingerprint(preview_fingerprint) && preview.fingerprint == preview_fingerprint,
        "Repair plan does not reproduce the accepted preview",
    )?;
    Ok(preview)
}

struct Compiler<'a> {
    plan: &'a RepairPlan,
    context: &'a RepairPlanningContext,
    constraints: Vec<RepairPlanConstraint>,
    target_previews: Vec<RepairTargetPreview>,
    additions: BTreeMap<String, u64>,
    removals: BTreeMap<String, u64>,
    edited_rows: BTreeSet<String>,
    desired_additions: u32,
    removal_count: u32,
}

impl<'a> Compiler<'a> {
    fn new(plan: &'a RepairPlan, context: &'a RepairPlanningContext) -> Self {
        Self {
            plan,
            context,
            constraints: Vec::new(),
            target_previews: Vec::new(),
            additions: BTreeMap::new(),
            removals: BTreeMap::new(),
            edited_rows: BTreeSet::new(),
            desired_additions: 0,
            removal_count: 0,
        }
    }

    fn constraint(&mut self, code: RepairPlanConstraintCode, target: Option<&str>) {
        let value = RepairPlanConstraint {
            code,
            target_id: target.map(str::to_owned),
        };
        if !self.constraints.contains(&value) {
            self.constraints.push(value);
        }
    }

    fn compile(&mut self) {
        if self.plan.schema_version != REPAIR_PLAN_SCHEMA_VERSION
            || !valid_text(&self.plan.summary, 400)
            || self.plan.stop != self.plan.targets.is_empty()
        {
            self.constraint(RepairPlanConstraintCode::InvalidPlanShape, None);
        }
        if self.plan.targets.len() > MAX_REPAIR_TARGETS {
            self.constraint(RepairPlanConstraintCode::TooManyTargets, None);
        }
        let mut target_ids = BTreeSet::new();
        for target in &self.plan.targets {
            if !target_ids.insert(&target.target_id) {
                self.constraint(RepairPlanConstraintCode::InvalidPlanShape, None);
                continue;
            }
            self.compile_target(target);
        }
        if self.edited_rows.len() > MAX_INSPECTED_ROWS_PER_ITERATION {
            self.constraint(RepairPlanConstraintCode::TooManyInspectedRows, None);
        }
        let changes = self.desired_additions.saturating_add(self.removal_count);
        if changes > self.context.remaining_row_changes {
            self.constraint(RepairPlanConstraintCode::RowChangeBudgetExceeded, None);
        }
    }

    fn compile_target(&mut self, target: &RepairTarget) {
        let id = target.target_id.as_str();
        if crate::repair_outcome::intervention_fingerprint(target)
            .is_ok_and(|signature| self.context.prior_interventions.contains(&signature))
        {
            self.constraint(
                RepairPlanConstraintCode::RepeatedUnchangedIntervention,
                Some(id),
            );
        }
        if !valid_id(id)
            || !valid_text(&target.hypothesis, 1_000)
            || !valid_text(&target.evidence_limitations, 1_000)
            || !valid_text(&target.intended_failure_pattern, 1_000)
            || !valid_text(&target.alternative_explanation, 1_000)
        {
            self.constraint(RepairPlanConstraintCode::InvalidTargetText, Some(id));
        }
        if target.cluster_keys.is_empty()
            || target.cluster_keys.len() > 4
            || !unique(&target.cluster_keys)
            || target
                .cluster_keys
                .iter()
                .any(|key| !self.context.clusters.contains_key(key))
        {
            self.constraint(RepairPlanConstraintCode::UnknownCluster, Some(id));
        }
        if target.evidence_ids.is_empty()
            || target.evidence_ids.len() > 20
            || !unique(&target.evidence_ids)
            || target
                .evidence_ids
                .iter()
                .any(|evidence| !self.context.evidence_ids.contains(evidence))
        {
            self.constraint(RepairPlanConstraintCode::UnknownEvidence, Some(id));
        }
        if !valid_text(&target.target_metric.name, 128)
            || !target.cluster_keys.iter().any(|key| {
                self.context
                    .clusters
                    .get(key)
                    .is_some_and(|cluster| cluster.metrics.contains(&target.target_metric.name))
            })
        {
            self.constraint(RepairPlanConstraintCode::UnknownMetric, Some(id));
        }
        match &target.operation {
            RepairOperation::LabelPreservingVariants {
                count,
                allocation_rationale,
                anchors,
            } => self.compile_variants(
                id,
                &target.cluster_keys,
                count,
                allocation_rationale,
                anchors,
            ),
            RepairOperation::ExistingAnchorContrast {
                count,
                allocation_rationale,
                pairs,
            } => {
                self.compile_contrast(id, &target.cluster_keys, count, allocation_rationale, pairs)
            }
            RepairOperation::ProvenRedundantRowRemoval { removals } => {
                self.compile_removals(id, &target.cluster_keys, removals)
            }
        }
    }

    fn desired(&mut self, id: &str, count: &AdditionCount) -> Option<u32> {
        match count.desired(&self.context.clusters) {
            Ok(value) => Some(value),
            Err(code) => {
                self.constraint(code, Some(id));
                None
            }
        }
    }

    fn validate_count_cluster(
        &mut self,
        id: &str,
        target_clusters: &[String],
        count: &AdditionCount,
    ) {
        if let AdditionCount::RelativeClusterGrowth {
            source_cluster_key, ..
        } = count
            && !target_clusters.contains(source_cluster_key)
        {
            self.constraint(RepairPlanConstraintCode::InvalidCountBasis, Some(id));
        }
    }

    fn checked_anchor(
        &mut self,
        id: &str,
        target_clusters: &[String],
        anchor: &AnchorReference,
    ) -> Option<RepairPlanningAnchor> {
        let Some(found) = self.context.anchors.get(&anchor.row_id) else {
            self.constraint(RepairPlanConstraintCode::UnknownAnchor, Some(id));
            return None;
        };
        if found.fingerprint != anchor.row_fingerprint {
            self.constraint(
                RepairPlanConstraintCode::AnchorFingerprintMismatch,
                Some(id),
            );
            return None;
        }
        if !target_clusters
            .iter()
            .any(|cluster| found.cluster_keys.contains(cluster))
        {
            self.constraint(RepairPlanConstraintCode::AnchorOutsideTarget, Some(id));
            return None;
        }
        self.edited_rows.insert(anchor.row_id.clone());
        Some(found.clone())
    }

    fn compile_variants(
        &mut self,
        id: &str,
        target_clusters: &[String],
        count: &AdditionCount,
        rationale: &str,
        anchors: &[AnchorAllocation],
    ) {
        self.validate_count_cluster(id, target_clusters, count);
        let desired = self.desired(id, count).unwrap_or(0);
        if !valid_text(rationale, 1_000) || anchors.is_empty() {
            self.constraint(RepairPlanConstraintCode::InvalidPlanShape, Some(id));
        }
        if anchors.len() > MAX_ANCHORS_PER_TARGET {
            self.constraint(RepairPlanConstraintCode::TooManyAnchors, Some(id));
        }
        let references = anchors
            .iter()
            .map(|anchor| AnchorReference {
                row_id: anchor.row_id.clone(),
                row_fingerprint: anchor.row_fingerprint.clone(),
            })
            .collect::<Vec<_>>();
        let mut ids = references
            .iter()
            .map(|anchor| anchor.row_id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        if !unique(&ids) {
            self.constraint(RepairPlanConstraintCode::RepeatedEdit, Some(id));
        }
        let expected = even_allocation(desired, &ids);
        for (anchor, reference) in anchors.iter().zip(&references) {
            if anchor.additions == 0 || anchor.additions > MAX_ADDITIONS_PER_ANCHOR {
                self.constraint(
                    RepairPlanConstraintCode::AdditionPerAnchorExceeded,
                    Some(id),
                );
            }
            if expected.get(&anchor.row_id) != Some(&anchor.additions) {
                self.constraint(RepairPlanConstraintCode::AllocationMismatch, Some(id));
            }
            if let Some(found) = self.checked_anchor(id, target_clusters, reference) {
                for cluster in found.cluster_keys {
                    *self.additions.entry(cluster).or_default() += u64::from(anchor.additions);
                }
            }
        }
        self.record_addition_target(id, desired);
    }

    fn compile_contrast(
        &mut self,
        id: &str,
        target_clusters: &[String],
        count: &AdditionCount,
        rationale: &str,
        pairs: &[ContrastPairAllocation],
    ) {
        self.validate_count_cluster(id, target_clusters, count);
        let desired = self.desired(id, count).unwrap_or(0);
        if !valid_text(rationale, 1_000) || pairs.is_empty() {
            self.constraint(RepairPlanConstraintCode::InvalidPlanShape, Some(id));
        }
        if desired % 2 != 0 {
            self.constraint(
                RepairPlanConstraintCode::ContrastRequiresEvenCount,
                Some(id),
            );
        }
        if pairs.len().saturating_mul(2) > MAX_ANCHORS_PER_TARGET {
            self.constraint(RepairPlanConstraintCode::TooManyAnchors, Some(id));
        }
        let mut pair_ids = pairs
            .iter()
            .map(|pair| pair.pair_id.clone())
            .collect::<Vec<_>>();
        pair_ids.sort();
        if !unique(&pair_ids) {
            self.constraint(RepairPlanConstraintCode::RepeatedEdit, Some(id));
        }
        let expected = even_allocation(desired / 2, &pair_ids);
        let mut anchor_ids = BTreeSet::new();
        for pair in pairs {
            let canonical_pair = contrast_pair_id(&pair.left.row_id, &pair.right.row_id);
            if pair.pair_id != canonical_pair
                || expected.get(&pair.pair_id) != Some(&pair.additions_per_side)
            {
                self.constraint(RepairPlanConstraintCode::AllocationMismatch, Some(id));
            }
            if pair.additions_per_side == 0 || pair.additions_per_side > MAX_ADDITIONS_PER_ANCHOR {
                self.constraint(
                    RepairPlanConstraintCode::AdditionPerAnchorExceeded,
                    Some(id),
                );
            }
            if !anchor_ids.insert(&pair.left.row_id) || !anchor_ids.insert(&pair.right.row_id) {
                self.constraint(RepairPlanConstraintCode::RepeatedEdit, Some(id));
            }
            let left = self.checked_anchor(id, target_clusters, &pair.left);
            let right = self.checked_anchor(id, target_clusters, &pair.right);
            if let (Some(left), Some(right)) = (left, right) {
                if left.native_context_fingerprint != right.native_context_fingerprint {
                    self.constraint(RepairPlanConstraintCode::ContrastContextMismatch, Some(id));
                }
                if left.label_fingerprint == right.label_fingerprint {
                    self.constraint(RepairPlanConstraintCode::ContrastLabelMismatch, Some(id));
                }
                for anchor in [left, right] {
                    for cluster in anchor.cluster_keys {
                        *self.additions.entry(cluster).or_default() +=
                            u64::from(pair.additions_per_side);
                    }
                }
            }
        }
        self.record_addition_target(id, desired);
    }

    fn record_addition_target(&mut self, id: &str, desired: u32) {
        if let Some(total) = self.desired_additions.checked_add(desired) {
            self.desired_additions = total;
        } else {
            self.constraint(RepairPlanConstraintCode::CountOverflow, Some(id));
        }
        self.target_previews.push(RepairTargetPreview {
            target_id: id.into(),
            desired_additions: desired,
            executable_additions: desired,
            removals: 0,
            limiting_constraints: Vec::new(),
        });
    }

    fn compile_removals(
        &mut self,
        id: &str,
        target_clusters: &[String],
        removals: &[ProvenRemoval],
    ) {
        if removals.is_empty() || removals.len() > MAX_ANCHORS_PER_TARGET {
            self.constraint(RepairPlanConstraintCode::InvalidPlanShape, Some(id));
        }
        let mut count = 0_u32;
        for removal in removals {
            if !valid_text(&removal.reason, 1_000)
                || removal.row_id == removal.retained_row_id
                || !self.edited_rows.insert(removal.row_id.clone())
            {
                self.constraint(RepairPlanConstraintCode::RepeatedEdit, Some(id));
                continue;
            }
            let removed = self.checked_anchor(
                id,
                target_clusters,
                &AnchorReference {
                    row_id: removal.row_id.clone(),
                    row_fingerprint: removal.row_fingerprint.clone(),
                },
            );
            let retained = self.checked_anchor(
                id,
                target_clusters,
                &AnchorReference {
                    row_id: removal.retained_row_id.clone(),
                    row_fingerprint: removal.retained_row_fingerprint.clone(),
                },
            );
            if let (Some(removed), Some(retained)) = (removed, retained) {
                if removed.exact_duplicate_group_id.is_none()
                    || removed.exact_duplicate_group_id != retained.exact_duplicate_group_id
                    || removed.native_model_input_fingerprint
                        != retained.native_model_input_fingerprint
                    || removed.label_fingerprint != retained.label_fingerprint
                {
                    self.constraint(RepairPlanConstraintCode::DuplicateRemovalUnproven, Some(id));
                }
                for cluster in removed.cluster_keys {
                    *self.removals.entry(cluster).or_default() += 1;
                }
                count += 1;
            }
        }
        self.removal_count = self.removal_count.saturating_add(count);
        self.target_previews.push(RepairTargetPreview {
            target_id: id.into(),
            desired_additions: 0,
            executable_additions: 0,
            removals: count,
            limiting_constraints: Vec::new(),
        });
    }

    fn preview(&self) -> Result<RepairPlanPreview, OptimizationError> {
        let changes = self.desired_additions + self.removal_count;
        let projected_rows = self
            .context
            .dataset_rows
            .checked_add(u64::from(self.desired_additions))
            .and_then(|value| value.checked_sub(u64::from(self.removal_count)))
            .ok_or_else(|| {
                OptimizationError::Validation("Projected dataset size overflow".into())
            })?;
        let mut projected_clusters = BTreeMap::new();
        let keys = self
            .additions
            .keys()
            .chain(self.removals.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        for key in keys {
            let current = self.context.clusters[&key].training_rows;
            let additions = self.additions.get(&key).copied().unwrap_or(0);
            let removals = self.removals.get(&key).copied().unwrap_or(0);
            let projected = current
                .checked_add(additions)
                .and_then(|value| value.checked_sub(removals))
                .ok_or_else(|| {
                    OptimizationError::Validation("Projected cluster size overflow".into())
                })?;
            projected_clusters.insert(
                key,
                ProjectedCluster {
                    current_rows: current,
                    additions,
                    removals,
                    projected_rows: projected,
                    projected_share_ppm: share_ppm(projected, projected_rows),
                },
            );
        }
        Ok(RepairPlanPreview {
            schema_version: REPAIR_PLAN_SCHEMA_VERSION,
            plan_fingerprint: fingerprint(self.plan)?,
            context_fingerprint: self.context.fingerprint()?,
            input_rows: self.context.dataset_rows,
            projected_rows,
            desired_additions: self.desired_additions,
            executable_additions: self.desired_additions,
            removals: self.removal_count,
            unused_row_change_allowance: self.context.remaining_row_changes - changes,
            targets: self.target_previews.clone(),
            projected_clusters,
            fingerprint: String::new(),
        })
    }
}

fn even_allocation(total: u32, ids: &[String]) -> BTreeMap<String, u32> {
    if ids.is_empty() {
        return BTreeMap::new();
    }
    let quotient = total / ids.len() as u32;
    let remainder = total % ids.len() as u32;
    ids.iter()
        .enumerate()
        .map(|(index, id)| (id.clone(), quotient + u32::from(index < remainder as usize)))
        .collect()
}

pub fn contrast_pair_id(left: &str, right: &str) -> String {
    let mut members = [left, right];
    members.sort();
    format!(
        "contrast-{}",
        fingerprint(&serde_json::json!({
            "protocol": "repair-contrast-pair-v1",
            "members": members,
        }))
        .expect("string pair fingerprint cannot fail")
    )
}

fn unique(values: &[String]) -> bool {
    values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn valid_id(value: &str) -> bool {
    valid_text(value, 128) && !value.chars().any(char::is_whitespace)
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty()
        && value.trim() == value
        && value.chars().count() <= maximum
        && !value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}

fn canonical_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn share_ppm(part: u64, total: u64) -> u64 {
    if total == 0 {
        0
    } else {
        ((part as f64 / total as f64) * 1_000_000.0).round() as u64
    }
}
