//! Immutable, provider-neutral strategy guidance attached to an approved plan.

use std::collections::{BTreeMap, BTreeSet};

use artifact_core::{FingerprintError, fingerprint};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::domain::{DatasetDefinition, GenerationCell, GenerationPlan};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationStrategyDirective {
    pub source_directive_id: Uuid,
    pub kind: String,
    pub share_basis_points: u16,
    pub instructions: Vec<String>,
    pub related_labels: Vec<String>,
    pub rationale: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGenerationStrategyContext {
    pub id: Uuid,
    pub dataset_id: Uuid,
    pub dataset_fingerprint: String,
    pub plan_id: Uuid,
    pub plan_fingerprint: String,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub approval_id: Uuid,
    pub approval_fingerprint: String,
    pub per_cell: BTreeMap<String, Vec<GenerationStrategyDirective>>,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ResolvedGenerationStrategyContext {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        dataset: &DatasetDefinition,
        plan: &GenerationPlan,
        proposal_id: Uuid,
        proposal_fingerprint: String,
        approval_id: Uuid,
        approval_fingerprint: String,
        per_cell: BTreeMap<String, Vec<GenerationStrategyDirective>>,
    ) -> Result<Self, StrategyError> {
        if plan.dataset_id != dataset.id {
            return Err(StrategyError::Identity(
                "plan belongs to another dataset".into(),
            ));
        }
        let known = plan
            .cells
            .iter()
            .map(|value| value.cell.key())
            .collect::<BTreeSet<_>>();
        if per_cell.keys().any(|key| !known.contains(key)) {
            return Err(StrategyError::Identity(
                "strategy guidance references a cell outside the plan".into(),
            ));
        }
        for directives in per_cell.values() {
            let mut ids = BTreeSet::new();
            for directive in directives {
                if directive.source_directive_id == Uuid::nil()
                    || !ids.insert(directive.source_directive_id)
                    || directive.share_basis_points == 0
                    || directive.share_basis_points > 10_000
                    || directive.kind.trim().is_empty()
                    || directive.rationale.trim().is_empty()
                    || directive.confidence.trim().is_empty()
                    || directive.instructions.is_empty()
                    || directive
                        .instructions
                        .iter()
                        .any(|value| value.trim().is_empty())
                {
                    return Err(StrategyError::InvalidDirective);
                }
            }
        }
        let mut value = Self {
            id: Uuid::new_v4(),
            dataset_id: dataset.id,
            dataset_fingerprint: fingerprint(dataset)?,
            plan_id: plan.id,
            plan_fingerprint: fingerprint(plan)?,
            proposal_id,
            proposal_fingerprint: required(proposal_fingerprint, "proposal fingerprint")?,
            approval_id,
            approval_fingerprint: required(approval_fingerprint, "approval fingerprint")?,
            per_cell,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, StrategyError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        Ok(fingerprint(&value)?)
    }

    pub fn for_cell(&self, cell: &GenerationCell) -> &[GenerationStrategyDirective] {
        self.per_cell.get(&cell.key()).map_or(&[], Vec::as_slice)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationStrategyAssignment {
    pub job_id: Uuid,
    pub context: ResolvedGenerationStrategyContext,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl GenerationStrategyAssignment {
    pub fn create(
        job_id: Uuid,
        context: ResolvedGenerationStrategyContext,
    ) -> Result<Self, StrategyError> {
        if context.reproduce_fingerprint()? != context.fingerprint {
            return Err(StrategyError::Identity(
                "strategy context fingerprint mismatch".into(),
            ));
        }
        let mut value = Self {
            job_id,
            context,
            created_at: Utc::now(),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, StrategyError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        Ok(fingerprint(&value)?)
    }
}

#[derive(Debug, Error)]
pub enum StrategyError {
    #[error("generation strategy identity failed: {0}")]
    Identity(String),
    #[error("generation strategy directive is invalid")]
    InvalidDirective,
    #[error(transparent)]
    Fingerprint(#[from] FingerprintError),
}

fn required(value: String, field: &str) -> Result<String, StrategyError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(StrategyError::Identity(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(value)
    }
}
