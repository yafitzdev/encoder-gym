//! Immutable source-population capture and deterministic audit selection.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use dataset_core::domain::{SourceProvenance, SourceRow};
use generation_core::domain::{DatasetDefinition, GenerationCell};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    QualityError, fingerprint,
    policy::{AuditMode, QualityPolicy},
    required,
};

pub const AUDIT_PLAN_SCHEMA_VERSION: u32 = 1;

/// The exact task vocabulary evaluators and assessment validators must use.
/// Values are stored in canonical lexical order even when the source dataset
/// used a different presentation order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedDatasetSchema {
    pub dataset_definition_id: Uuid,
    pub dataset_definition_fingerprint: String,
    pub task_description: String,
    pub labels: Vec<String>,
    pub dimensions: Vec<NormalizedDimensionSchema>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizedDimensionSchema {
    pub name: String,
    pub values: Vec<String>,
}

impl NormalizedDatasetSchema {
    pub fn from_definition(definition: &DatasetDefinition) -> Result<Self, QualityError> {
        if definition.id.is_nil() {
            return Err(QualityError::Validation(
                "dataset definition ID must not be nil".into(),
            ));
        }
        let task_description = exact_required(
            definition.task_description.clone(),
            "dataset.task_description",
        )?;
        let mut labels = definition
            .labels
            .iter()
            .cloned()
            .map(|value| exact_required(value, "dataset.labels"))
            .collect::<Result<Vec<_>, _>>()?;
        labels.sort();
        if labels.is_empty() || has_duplicates(&labels) {
            return Err(QualityError::Validation(
                "dataset labels must be non-empty and unique".into(),
            ));
        }
        let mut dimensions = definition
            .dimensions
            .iter()
            .map(|dimension| {
                let name = exact_required(dimension.name.clone(), "dataset.dimension.name")?;
                let mut values = dimension
                    .values
                    .iter()
                    .cloned()
                    .map(|value| exact_required(value, "dataset.dimension.values"))
                    .collect::<Result<Vec<_>, _>>()?;
                values.sort();
                if values.is_empty() || has_duplicates(&values) {
                    return Err(QualityError::Validation(format!(
                        "dimension {name:?} values must be non-empty and unique"
                    )));
                }
                Ok(NormalizedDimensionSchema { name, values })
            })
            .collect::<Result<Vec<_>, QualityError>>()?;
        dimensions.sort_by(|left, right| left.name.cmp(&right.name));
        if dimensions
            .windows(2)
            .any(|pair| pair[0].name == pair[1].name)
        {
            return Err(QualityError::Validation(
                "dataset dimension names must be unique".into(),
            ));
        }
        Ok(Self {
            dataset_definition_id: definition.id,
            dataset_definition_fingerprint: fingerprint(definition)?,
            task_description,
            labels,
            dimensions,
        })
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        if self.dataset_definition_id.is_nil() {
            return Err(QualityError::Validation(
                "dataset definition ID must not be nil".into(),
            ));
        }
        exact_required(
            self.dataset_definition_fingerprint.clone(),
            "dataset.dataset_definition_fingerprint",
        )?;
        exact_required(self.task_description.clone(), "dataset.task_description")?;
        if self.labels.is_empty()
            || !is_strictly_sorted(&self.labels)
            || self
                .labels
                .iter()
                .any(|value| exact_required(value.clone(), "dataset.labels").is_err())
        {
            return Err(QualityError::Validation(
                "normalized labels must be non-empty, unique, and canonically sorted".into(),
            ));
        }
        if self
            .dimensions
            .windows(2)
            .any(|pair| pair[0].name >= pair[1].name)
        {
            return Err(QualityError::Validation(
                "normalized dimensions must have unique canonically sorted names".into(),
            ));
        }
        for dimension in &self.dimensions {
            exact_required(dimension.name.clone(), "dataset.dimension.name")?;
            if dimension.values.is_empty()
                || !is_strictly_sorted(&dimension.values)
                || dimension
                    .values
                    .iter()
                    .any(|value| exact_required(value.clone(), "dataset.dimension.values").is_err())
            {
                return Err(QualityError::Validation(format!(
                    "dimension {:?} values must be non-empty, unique, and canonically sorted",
                    dimension.name
                )));
            }
        }
        Ok(())
    }

    pub fn allowed_dimensions(&self) -> BTreeMap<&str, BTreeSet<&str>> {
        self.dimensions
            .iter()
            .map(|dimension| {
                (
                    dimension.name.as_str(),
                    dimension.values.iter().map(String::as_str).collect(),
                )
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellIdentity {
    pub label: String,
    pub dimensions: BTreeMap<String, String>,
}

impl CellIdentity {
    pub fn key(&self) -> String {
        GenerationCell {
            label: self.label.clone(),
            dimensions: self.dimensions.clone(),
        }
        .key()
    }

    pub fn validate(&self, schema: &NormalizedDatasetSchema) -> Result<(), QualityError> {
        if schema.labels.binary_search(&self.label).is_err() {
            return Err(QualityError::Validation(format!(
                "source row has unknown label {:?}",
                self.label
            )));
        }
        let expected = schema.allowed_dimensions();
        if self.dimensions.len() != expected.len() {
            return Err(QualityError::Validation(format!(
                "source row cell has {} dimensions; expected {}",
                self.dimensions.len(),
                expected.len()
            )));
        }
        for (name, allowed) in expected {
            let Some(value) = self.dimensions.get(name) else {
                return Err(QualityError::Validation(format!(
                    "source row is missing dimension {name:?}"
                )));
            };
            if !allowed.contains(value.as_str()) {
                return Err(QualityError::Validation(format!(
                    "source row has unknown value {value:?} for dimension {name:?}"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProvenanceStratum {
    Generated {
        generation_job_id: Uuid,
        backend: String,
        model: String,
    },
    Imported {
        import_id: Uuid,
    },
}

impl ProvenanceStratum {
    fn from_source(value: &SourceProvenance) -> Result<Self, QualityError> {
        match value {
            SourceProvenance::Generated {
                generation_job_id,
                backend,
                model,
                ..
            } => {
                if generation_job_id.is_nil() {
                    return Err(QualityError::Validation(
                        "generation provenance job ID must not be nil".into(),
                    ));
                }
                Ok(Self::Generated {
                    generation_job_id: *generation_job_id,
                    backend: required(backend.clone(), "source provenance backend")?,
                    model: required(model.clone(), "source provenance model")?,
                })
            }
            SourceProvenance::Imported { import_id, .. } => {
                if import_id.is_nil() {
                    return Err(QualityError::Validation(
                        "import provenance ID must not be nil".into(),
                    ));
                }
                Ok(Self::Imported {
                    import_id: *import_id,
                })
            }
        }
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        match self {
            Self::Generated {
                generation_job_id,
                backend,
                model,
            } => {
                if generation_job_id.is_nil() {
                    return Err(QualityError::Validation(
                        "generation provenance job ID must not be nil".into(),
                    ));
                }
                exact_required(backend.clone(), "source provenance backend")?;
                exact_required(model.clone(), "source provenance model")?;
            }
            Self::Imported { import_id } if import_id.is_nil() => {
                return Err(QualityError::Validation(
                    "import provenance ID must not be nil".into(),
                ));
            }
            Self::Imported { .. } => {}
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditSelection {
    Selected,
    UnselectedReportOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditPlanItem {
    pub source_row_id: Uuid,
    pub source_row_fingerprint: String,
    pub cell: CellIdentity,
    pub provenance_stratum: ProvenanceStratum,
    pub selection: AuditSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidanceReference {
    pub id: Uuid,
    pub fingerprint: String,
}

impl GuidanceReference {
    pub fn new(id: Uuid, fingerprint: impl Into<String>) -> Result<Self, QualityError> {
        let value = Self {
            id,
            fingerprint: required(fingerprint, "guidance fingerprint")?,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), QualityError> {
        if self.id.is_nil() {
            return Err(QualityError::Validation(
                "guidance reference ID must not be nil".into(),
            ));
        }
        exact_required(self.fingerprint.clone(), "guidance fingerprint")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuidanceReferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_context: Option<GuidanceReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authenticity_context: Option<GuidanceReference>,
}

impl GuidanceReferences {
    pub fn validate(&self) -> Result<(), QualityError> {
        if let Some(value) = &self.semantic_context {
            value.validate()?;
        }
        if let Some(value) = &self.authenticity_context {
            value.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditPlan {
    pub id: Uuid,
    pub schema_version: u32,
    pub dataset_schema: NormalizedDatasetSchema,
    pub policy: QualityPolicy,
    pub guidance: GuidanceReferences,
    /// Fingerprint of the exact normalized guidance payload later sent to the
    /// evaluator. Context references alone are not sufficient to prove prompt
    /// reproduction.
    pub resolved_guidance_fingerprint: String,
    pub evaluator_protocol_version: String,
    /// Complete pinned source population in canonical order. Report-only
    /// sampling changes `selection`; it never removes an item.
    pub items: Vec<AuditPlanItem>,
    pub source_set_fingerprint: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

/// A plan whose complete immutable population has been verified once and
/// indexed for bounded evidence checks.
pub struct CheckedAuditPlan<'a> {
    plan: &'a AuditPlan,
    items_by_source_row: BTreeMap<Uuid, &'a AuditPlanItem>,
}

impl<'a> CheckedAuditPlan<'a> {
    pub fn new(plan: &'a AuditPlan) -> Result<Self, QualityError> {
        plan.verify_integrity()?;
        Ok(Self {
            plan,
            items_by_source_row: plan
                .items
                .iter()
                .map(|item| (item.source_row_id, item))
                .collect(),
        })
    }

    pub const fn plan(&self) -> &'a AuditPlan {
        self.plan
    }

    pub fn item(&self, source_row_id: Uuid) -> Option<&'a AuditPlanItem> {
        self.items_by_source_row.get(&source_row_id).copied()
    }
}

impl AuditPlan {
    pub fn create(
        dataset: &DatasetDefinition,
        policy: QualityPolicy,
        guidance: GuidanceReferences,
        resolved_guidance_fingerprint: impl Into<String>,
        evaluator_protocol_version: impl Into<String>,
        source_rows: Vec<SourceRow>,
    ) -> Result<Self, QualityError> {
        Self::with_identity(
            Uuid::new_v4(),
            dataset,
            policy,
            guidance,
            resolved_guidance_fingerprint,
            evaluator_protocol_version,
            source_rows,
            Utc::now(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_identity(
        id: Uuid,
        dataset: &DatasetDefinition,
        policy: QualityPolicy,
        guidance: GuidanceReferences,
        resolved_guidance_fingerprint: impl Into<String>,
        evaluator_protocol_version: impl Into<String>,
        source_rows: Vec<SourceRow>,
        created_at: DateTime<Utc>,
    ) -> Result<Self, QualityError> {
        if id.is_nil() {
            return Err(QualityError::Validation(
                "audit plan ID must not be nil".into(),
            ));
        }
        policy.verify_integrity()?;
        guidance.validate()?;
        if policy.thresholds.minimum_authenticity_score.is_some()
            && guidance.authenticity_context.is_none()
        {
            return Err(QualityError::Validation(
                "an authenticity threshold requires a pinned authenticity guidance reference"
                    .into(),
            ));
        }
        let dataset_schema = NormalizedDatasetSchema::from_definition(dataset)?;
        let resolved_guidance_fingerprint = required(
            resolved_guidance_fingerprint,
            "resolved evaluator guidance fingerprint",
        )?;
        let evaluator_protocol_version =
            required(evaluator_protocol_version, "evaluator protocol version")?;
        let mut seen = BTreeSet::new();
        let mut items = Vec::with_capacity(source_rows.len());
        for row in source_rows {
            if row.dataset_id != dataset.id {
                return Err(QualityError::Validation(format!(
                    "source row {} belongs to dataset {}, expected {}",
                    row.id, row.dataset_id, dataset.id
                )));
            }
            if row.id.is_nil() {
                return Err(QualityError::Validation(
                    "source row ID must not be nil".into(),
                ));
            }
            if !seen.insert(row.id) {
                return Err(QualityError::Validation(format!(
                    "source row appears more than once: {}",
                    row.id
                )));
            }
            if row.text.trim().is_empty() {
                return Err(QualityError::Validation(format!(
                    "source row {} has empty text",
                    row.id
                )));
            }
            let cell = CellIdentity {
                label: row.label.clone(),
                dimensions: row.dimensions.clone(),
            };
            cell.validate(&dataset_schema)?;
            items.push(AuditPlanItem {
                source_row_id: row.id,
                source_row_fingerprint: fingerprint(&row)?,
                cell,
                provenance_stratum: ProvenanceStratum::from_source(&row.provenance)?,
                selection: AuditSelection::Selected,
            });
        }
        if items.is_empty() {
            return Err(QualityError::Validation(
                "an audit plan requires at least one accepted source row".into(),
            ));
        }
        sort_items(&mut items);
        apply_selection(
            &mut items,
            &policy.audit_mode,
            &dataset_schema.dataset_definition_fingerprint,
        )?;
        let selected_count = count_selected(&items);
        let assessment_rounds = 1 + u64::from(
            policy
                .borderline_review_policy
                .required_additional_assessments(),
        );
        let required_requests = policy
            .budgets
            .required_requests_for_rows(selected_count, assessment_rounds)?;
        if required_requests > u64::from(policy.budgets.maximum_evaluator_requests) {
            return Err(QualityError::Validation(format!(
                "audit policy needs up to {required_requests} evaluator requests to complete all configured review rounds, exceeding its maximum of {}",
                policy.budgets.maximum_evaluator_requests
            )));
        }
        let source_set_fingerprint = source_set_fingerprint(&dataset_schema, &items)?;
        let mut value = Self {
            id,
            schema_version: AUDIT_PLAN_SCHEMA_VERSION,
            dataset_schema,
            policy,
            guidance,
            resolved_guidance_fingerprint,
            evaluator_protocol_version,
            items,
            source_set_fingerprint,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn population_count(&self) -> u64 {
        self.items.len() as u64
    }

    pub fn selected_count(&self) -> u64 {
        count_selected(&self.items)
    }

    pub fn selected_items(&self) -> impl Iterator<Item = &AuditPlanItem> {
        self.items
            .iter()
            .filter(|item| item.selection == AuditSelection::Selected)
    }

    pub fn reproduce_source_set_fingerprint(&self) -> Result<String, QualityError> {
        source_set_fingerprint(&self.dataset_schema, &self.items)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, QualityError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        fingerprint(&value)
    }

    pub fn verify_integrity(&self) -> Result<(), QualityError> {
        note_audit_plan_verification();
        if self.id.is_nil() || self.schema_version != AUDIT_PLAN_SCHEMA_VERSION {
            return Err(QualityError::Integrity(
                "audit plan identity or schema version is invalid".into(),
            ));
        }
        self.dataset_schema.validate()?;
        self.policy.verify_integrity()?;
        self.guidance.validate()?;
        if self.policy.thresholds.minimum_authenticity_score.is_some()
            && self.guidance.authenticity_context.is_none()
        {
            return Err(QualityError::Integrity(
                "audit plan authenticity policy has no pinned guidance".into(),
            ));
        }
        exact_required(
            self.resolved_guidance_fingerprint.clone(),
            "resolved evaluator guidance fingerprint",
        )?;
        exact_required(
            self.evaluator_protocol_version.clone(),
            "evaluator protocol version",
        )?;
        if self.items.is_empty() {
            return Err(QualityError::Integrity(
                "audit plan source population is empty".into(),
            ));
        }
        let mut seen = BTreeSet::new();
        for item in &self.items {
            if item.source_row_id.is_nil() || !seen.insert(item.source_row_id) {
                return Err(QualityError::Integrity(
                    "audit plan contains a nil or duplicate source row ID".into(),
                ));
            }
            exact_required(
                item.source_row_fingerprint.clone(),
                "source row fingerprint",
            )?;
            item.cell.validate(&self.dataset_schema)?;
            item.provenance_stratum.validate()?;
        }
        let mut canonical = self.items.clone();
        sort_items(&mut canonical);
        if canonical != self.items {
            return Err(QualityError::Integrity(
                "audit plan items are not in canonical order".into(),
            ));
        }
        apply_selection(
            &mut canonical,
            &self.policy.audit_mode,
            &self.dataset_schema.dataset_definition_fingerprint,
        )?;
        if canonical != self.items {
            return Err(QualityError::Integrity(
                "audit plan selection does not reproduce".into(),
            ));
        }
        let assessment_rounds = 1 + u64::from(
            self.policy
                .borderline_review_policy
                .required_additional_assessments(),
        );
        let required_requests = self
            .policy
            .budgets
            .required_requests_for_rows(self.selected_count(), assessment_rounds)?;
        if required_requests > u64::from(self.policy.budgets.maximum_evaluator_requests) {
            return Err(QualityError::Integrity(
                "audit plan selection and required review rounds exceed request capacity".into(),
            ));
        }
        if self.source_set_fingerprint.is_empty()
            || self.reproduce_source_set_fingerprint()? != self.source_set_fingerprint
        {
            return Err(QualityError::Integrity(
                "audit source-set fingerprint does not reproduce".into(),
            ));
        }
        if self.fingerprint.is_empty() || self.reproduce_fingerprint()? != self.fingerprint {
            return Err(QualityError::Integrity(
                "audit plan fingerprint does not reproduce".into(),
            ));
        }
        Ok(())
    }
}

fn apply_selection(
    items: &mut [AuditPlanItem],
    mode: &AuditMode,
    dataset_definition_fingerprint: &str,
) -> Result<(), QualityError> {
    match mode {
        AuditMode::FullPopulation => {
            for item in items {
                item.selection = AuditSelection::Selected;
            }
        }
        AuditMode::DeterministicSampleReportOnly {
            sample_size,
            seed,
            minimum_rows_per_cell,
        } => {
            if *sample_size == 0 || *minimum_rows_per_cell == 0 {
                return Err(QualityError::Validation(
                    "deterministic report-only sample size and per-cell minimum must be positive"
                        .into(),
                ));
            }
            let mut ranked = items
                .iter()
                .map(|item| {
                    let rank = fingerprint(&(
                        "dataset-quality-sample-v1",
                        dataset_definition_fingerprint,
                        seed,
                        item.source_row_id,
                        &item.source_row_fingerprint,
                    ))?;
                    Ok((rank, item.source_row_id))
                })
                .collect::<Result<Vec<_>, QualityError>>()?;
            ranked.sort();
            let ranks_by_id = ranked
                .iter()
                .cloned()
                .map(|(rank, id)| (id, rank))
                .collect::<BTreeMap<_, _>>();
            let mut by_cell = BTreeMap::<CellIdentity, Vec<(String, Uuid)>>::new();
            for item in items.iter() {
                by_cell.entry(item.cell.clone()).or_default().push((
                    ranks_by_id
                        .get(&item.source_row_id)
                        .expect("every item was ranked")
                        .clone(),
                    item.source_row_id,
                ));
            }
            let minimum_rows_per_cell = u64::from(*minimum_rows_per_cell);
            let required_minimum = by_cell.values().try_fold(0_u64, |total, rows| {
                total.checked_add(minimum_rows_per_cell.min(rows.len() as u64))
            });
            let Some(required_minimum) = required_minimum else {
                return Err(QualityError::Validation(
                    "deterministic sample minimum overflowed".into(),
                ));
            };
            if *sample_size < required_minimum {
                return Err(QualityError::Validation(format!(
                    "sample size {sample_size} cannot satisfy the per-cell minimum; at least {required_minimum} rows are required"
                )));
            }
            let mut selected_ids = BTreeSet::new();
            for rows in by_cell.values_mut() {
                rows.sort();
                selected_ids.extend(
                    rows.iter()
                        .take(minimum_rows_per_cell.min(rows.len() as u64) as usize)
                        .map(|(_, id)| *id),
                );
            }
            let target = (*sample_size).min(items.len() as u64) as usize;
            for (_, id) in ranked {
                if selected_ids.len() >= target {
                    break;
                }
                selected_ids.insert(id);
            }
            for item in items {
                item.selection = if selected_ids.contains(&item.source_row_id) {
                    AuditSelection::Selected
                } else {
                    AuditSelection::UnselectedReportOnly
                };
            }
        }
    }
    Ok(())
}

fn sort_items(items: &mut [AuditPlanItem]) {
    items.sort_by(|left, right| {
        left.cell
            .cmp(&right.cell)
            .then_with(|| left.provenance_stratum.cmp(&right.provenance_stratum))
            .then_with(|| left.source_row_id.cmp(&right.source_row_id))
            .then_with(|| {
                left.source_row_fingerprint
                    .cmp(&right.source_row_fingerprint)
            })
    });
}

fn count_selected(items: &[AuditPlanItem]) -> u64 {
    items
        .iter()
        .filter(|item| item.selection == AuditSelection::Selected)
        .count() as u64
}

fn source_set_fingerprint(
    schema: &NormalizedDatasetSchema,
    items: &[AuditPlanItem],
) -> Result<String, QualityError> {
    #[derive(Serialize)]
    struct SourceSet<'a> {
        schema: &'a NormalizedDatasetSchema,
        items: &'a [AuditPlanItem],
    }
    fingerprint(&SourceSet { schema, items })
}

fn exact_required(value: String, field: &str) -> Result<String, QualityError> {
    let original = value.clone();
    let normalized = required(value, field)?;
    if normalized != original {
        return Err(QualityError::Validation(format!(
            "{field} must already be normalized"
        )));
    }
    Ok(normalized)
}

fn has_duplicates<T: PartialEq>(values: &[T]) -> bool {
    values.windows(2).any(|pair| pair[0] == pair[1])
}

#[cfg(test)]
thread_local! {
    static AUDIT_PLAN_VERIFICATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn note_audit_plan_verification() {
    AUDIT_PLAN_VERIFICATIONS.with(|count| count.set(count.get() + 1));
}

#[cfg(not(test))]
fn note_audit_plan_verification() {}

fn is_strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{TimeZone, Utc};
    use dataset_core::domain::{SourceProvenance, SourceRow};
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};
    use uuid::Uuid;

    use crate::assessment::EvaluatorGuidance;
    use crate::policy::{
        AuditMode, EvaluatorEgressPolicy, QualityPolicyPresetControls, QualityPreset,
    };

    use super::{
        AUDIT_PLAN_VERIFICATIONS, AuditPlan, AuditSelection, CheckedAuditPlan, GuidanceReferences,
    };

    fn dataset() -> DatasetDefinition {
        DatasetDefinition::with_identity(
            Uuid::parse_str("10000000-0000-4000-8000-000000000001").expect("dataset ID"),
            "support",
            "Classify support requests",
            vec!["fraud".into(), "billing".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["hard".into(), "easy".into()])
                    .expect("dimension"),
            ],
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("dataset")
    }

    fn rows(dataset_id: Uuid) -> Vec<SourceRow> {
        (0..8_u128)
            .map(|index| SourceRow {
                id: Uuid::from_u128(0x20000000000040008000000000000000 + index),
                dataset_id,
                text: format!("candidate text {index}"),
                label: if index % 2 == 0 {
                    "billing".into()
                } else {
                    "fraud".into()
                },
                dimensions: BTreeMap::from([(
                    "difficulty".into(),
                    if (index / 2) % 2 == 0 { "easy" } else { "hard" }.into(),
                )]),
                fields: BTreeMap::from([(
                    "source_index".into(),
                    u64::try_from(index).expect("fixture index").into(),
                )]),
                provenance: if index < 6 {
                    SourceProvenance::Generated {
                        generation_job_id: Uuid::from_u128(
                            0x30000000000040008000000000000000 + index,
                        ),
                        backend: "fake".into(),
                        model: "deterministic-v1".into(),
                        construction_plan_fingerprint: Some("sha256:construction".into()),
                    }
                } else {
                    SourceProvenance::Imported {
                        import_id: Uuid::parse_str("40000000-0000-4000-8000-000000000001")
                            .expect("import ID"),
                        source_path: "fixture.jsonl".into(),
                        source_row_number: u64::try_from(index).expect("fixture index") + 1,
                    }
                },
                created_at: Utc
                    .with_ymd_and_hms(2026, 1, 2, 0, 0, index as u32)
                    .single()
                    .expect("time"),
            })
            .collect()
    }

    fn policy(mode: AuditMode) -> crate::policy::QualityPolicy {
        QualityPreset::Balanced
            .compile(QualityPolicyPresetControls {
                audit_mode: mode,
                egress_policy: EvaluatorEgressPolicy::LocalOnly,
                evaluate_authenticity: false,
                maximum_cost_microusd: None,
            })
            .expect("policy")
    }

    fn fixed_plan(
        dataset: &DatasetDefinition,
        mode: AuditMode,
        source_rows: Vec<SourceRow>,
    ) -> Result<AuditPlan, crate::QualityError> {
        AuditPlan::with_identity(
            Uuid::parse_str("50000000-0000-4000-8000-000000000001").expect("plan ID"),
            dataset,
            policy(mode),
            GuidanceReferences::default(),
            EvaluatorGuidance::default()
                .reproduce_fingerprint()
                .expect("empty guidance fingerprint"),
            "quality-evaluator-v1",
            source_rows,
            Utc.with_ymd_and_hms(2026, 1, 3, 0, 0, 0)
                .single()
                .expect("time"),
        )
    }

    #[test]
    fn plan_and_source_set_are_stable_regardless_of_input_order() {
        let dataset = dataset();
        let original = rows(dataset.id);
        let mut reversed = original.clone();
        reversed.reverse();
        let mode = AuditMode::DeterministicSampleReportOnly {
            sample_size: 4,
            seed: 99,
            minimum_rows_per_cell: 1,
        };

        let left = fixed_plan(&dataset, mode.clone(), original).expect("left plan");
        let right = fixed_plan(&dataset, mode, reversed).expect("right plan");

        assert_eq!(left.source_set_fingerprint, right.source_set_fingerprint);
        assert_eq!(left.fingerprint, right.fingerprint);
        assert_eq!(left.items, right.items);
        assert!(left.verify_integrity().is_ok());
    }

    #[test]
    fn report_only_sample_keeps_unsampled_population_visible() {
        let dataset = dataset();
        let plan = fixed_plan(
            &dataset,
            AuditMode::DeterministicSampleReportOnly {
                sample_size: 4,
                seed: 7,
                minimum_rows_per_cell: 1,
            },
            rows(dataset.id),
        )
        .expect("plan");

        assert_eq!(plan.population_count(), 8);
        assert_eq!(plan.selected_count(), 4);
        assert_eq!(
            plan.items
                .iter()
                .filter(|item| item.selection == AuditSelection::UnselectedReportOnly)
                .count(),
            4
        );
        let selected_cells = plan
            .selected_items()
            .map(|item| item.cell.key())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            selected_cells.len(),
            4,
            "every populated cell is represented"
        );
    }

    #[test]
    fn report_only_sample_rejects_a_budget_that_cannot_cover_every_cell() {
        let dataset = dataset();
        let result = fixed_plan(
            &dataset,
            AuditMode::DeterministicSampleReportOnly {
                sample_size: 3,
                seed: 7,
                minimum_rows_per_cell: 1,
            },
            rows(dataset.id),
        );

        assert!(result.is_err());
    }

    #[test]
    fn full_population_selects_every_pinned_row() {
        let dataset = dataset();
        let plan = fixed_plan(&dataset, AuditMode::FullPopulation, rows(dataset.id)).expect("plan");

        assert_eq!(plan.population_count(), 8);
        assert_eq!(plan.selected_count(), 8);
        assert!(
            plan.items
                .iter()
                .all(|item| item.selection == AuditSelection::Selected)
        );
    }

    #[test]
    fn checked_plan_verifies_once_for_twenty_thousand_indexed_bindings() {
        let dataset = dataset();
        let plan = fixed_plan(&dataset, AuditMode::FullPopulation, rows(dataset.id)).expect("plan");
        AUDIT_PLAN_VERIFICATIONS.with(|count| count.set(0));

        let checked = CheckedAuditPlan::new(&plan).expect("checked plan");
        for index in 0..20_000 {
            let item = &plan.items[index % plan.items.len()];
            assert_eq!(checked.item(item.source_row_id), Some(item));
        }

        assert_eq!(AUDIT_PLAN_VERIFICATIONS.with(std::cell::Cell::get), 1);
    }

    #[test]
    fn duplicate_and_mismatched_source_rows_are_rejected() {
        let dataset = dataset();
        let mut duplicates = rows(dataset.id);
        duplicates.push(duplicates[0].clone());
        assert!(fixed_plan(&dataset, AuditMode::FullPopulation, duplicates).is_err());

        let mut mismatched = rows(dataset.id);
        mismatched[0].dataset_id = Uuid::new_v4();
        assert!(fixed_plan(&dataset, AuditMode::FullPopulation, mismatched).is_err());
    }

    #[test]
    fn full_source_row_fingerprint_and_cell_shape_are_pinned() {
        let dataset = dataset();
        let baseline =
            fixed_plan(&dataset, AuditMode::FullPopulation, rows(dataset.id)).expect("baseline");
        let mut changed_rows = rows(dataset.id);
        changed_rows[0]
            .fields
            .insert("new_fact".into(), true.into());
        let changed =
            fixed_plan(&dataset, AuditMode::FullPopulation, changed_rows).expect("changed");
        assert_ne!(
            baseline.source_set_fingerprint,
            changed.source_set_fingerprint
        );

        let mut invalid_rows = rows(dataset.id);
        invalid_rows[0]
            .dimensions
            .insert("unknown".into(), "value".into());
        assert!(fixed_plan(&dataset, AuditMode::FullPopulation, invalid_rows).is_err());
    }

    #[test]
    fn resolved_guidance_payload_changes_plan_identity() {
        let dataset = dataset();
        let source_rows = rows(dataset.id);
        let baseline = fixed_plan(&dataset, AuditMode::FullPopulation, source_rows.clone())
            .expect("baseline plan");
        let changed = AuditPlan::with_identity(
            baseline.id,
            &dataset,
            policy(AuditMode::FullPopulation),
            GuidanceReferences::default(),
            "sha256:different-resolved-guidance",
            "quality-evaluator-v1",
            source_rows,
            baseline.created_at,
        )
        .expect("changed guidance plan");

        assert_ne!(baseline.fingerprint, changed.fingerprint);
        assert_ne!(
            baseline.resolved_guidance_fingerprint,
            changed.resolved_guidance_fingerprint
        );
    }
}
