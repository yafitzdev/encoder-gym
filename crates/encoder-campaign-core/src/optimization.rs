//! Durable, row-free launch contract for one finite production optimization cycle.
//!
//! This module owns only resolved authority, lifecycle links, and recovery state. Repair
//! qualification, native training, evaluation, selection, benchmark consumption, and final
//! decisions remain with their existing owners.

mod continuation;
pub use continuation::automatic_stage_rank;

use std::{collections::BTreeSet, future::Future, pin::Pin};

use chrono::{DateTime, Utc};
use encoder_experiment_core::{
    domain::{ExternalProjectSnapshot, TrainingCandidate},
    journal::FinalDecision,
    metrics::MetricContract,
    protocol::{DevelopmentSelectionRule, ExperimentProtocol},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{CampaignBenchmarkBinding, CampaignBudget};

pub const OPTIMIZATION_DEFINITION_SCHEMA_VERSION: u32 = 1;
pub const OPTIMIZATION_RUN_SCHEMA_VERSION: u32 = 1;
pub const OPTIMIZATION_EVENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationArtifactBinding {
    pub id: Uuid,
    pub fingerprint: String,
}

impl OptimizationArtifactBinding {
    pub fn new(id: Uuid, fingerprint: impl Into<String>) -> Result<Self, OptimizationError> {
        let value = Self {
            id,
            fingerprint: fingerprint.into(),
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), OptimizationError> {
        if self.id.is_nil() || !canonical_fingerprint(&self.fingerprint) {
            return Err(OptimizationError::InvalidDefinition(
                "optimization artifact binding is incomplete".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationApprovalMode {
    ExplicitSealedUse,
}

/// Fully resolved immutable authority for one post-review repair experiment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionOptimizationDefinition {
    pub schema_version: u32,
    pub id: Uuid,
    pub name: String,
    pub manifest_fingerprint: String,
    pub specification_fingerprint: String,
    pub project: OptimizationArtifactBinding,
    pub proposal: OptimizationArtifactBinding,
    pub delta_selection: OptimizationArtifactBinding,
    pub training_snapshot: OptimizationArtifactBinding,
    pub training_snapshot_specification_fingerprint: String,
    pub benchmark: CampaignBenchmarkBinding,
    pub metric_source_protocol: OptimizationArtifactBinding,
    pub metric_contract: MetricContract,
    pub development_suite_keys: Vec<String>,
    pub sealed_suite_key: String,
    pub candidates: Vec<TrainingCandidate>,
    pub campaign_budget: CampaignBudget,
    pub maximum_evaluation_seconds: u64,
    pub approval_mode: OptimizationApprovalMode,
    pub selection_rule: DevelopmentSelectionRule,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProductionOptimizationDefinition {
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        name: impl Into<String>,
        manifest_fingerprint: impl Into<String>,
        project: &ExternalProjectSnapshot,
        proposal: OptimizationArtifactBinding,
        delta_selection: OptimizationArtifactBinding,
        training_snapshot: OptimizationArtifactBinding,
        training_snapshot_specification_fingerprint: impl Into<String>,
        benchmark: CampaignBenchmarkBinding,
        metric_source_protocol: &ExperimentProtocol,
        candidates: Vec<TrainingCandidate>,
        campaign_budget: CampaignBudget,
        maximum_evaluation_seconds: u64,
        created_at: DateTime<Utc>,
    ) -> Result<Self, OptimizationError> {
        project
            .validate_integrity()
            .map_err(|error| OptimizationError::InvalidDefinition(error.to_string()))?;
        let mut value = Self {
            schema_version: OPTIMIZATION_DEFINITION_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            name: name.into(),
            manifest_fingerprint: manifest_fingerprint.into(),
            specification_fingerprint: String::new(),
            project: OptimizationArtifactBinding::new(project.id, project.fingerprint.clone())?,
            proposal,
            delta_selection,
            training_snapshot,
            training_snapshot_specification_fingerprint:
                training_snapshot_specification_fingerprint.into(),
            benchmark,
            metric_source_protocol: OptimizationArtifactBinding::new(
                metric_source_protocol.id,
                metric_source_protocol.fingerprint.clone(),
            )?,
            metric_contract: metric_source_protocol.metric_contract.clone(),
            development_suite_keys: metric_source_protocol.development_suite_keys(),
            sealed_suite_key: metric_source_protocol.sealed_suite_key.clone(),
            candidates,
            campaign_budget,
            maximum_evaluation_seconds,
            approval_mode: OptimizationApprovalMode::ExplicitSealedUse,
            selection_rule: DevelopmentSelectionRule::MaximizeWorstSuiteThenMean,
            created_at,
            fingerprint: String::new(),
        };
        value.development_suite_keys.sort();
        value.validate_fields(project, metric_source_protocol)?;
        value.specification_fingerprint = value.reproduce_specification_fingerprint()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(
        &self,
        project: &ExternalProjectSnapshot,
        metric_source_protocol: &ExperimentProtocol,
    ) -> Result<(), OptimizationError> {
        self.validate_fields(project, metric_source_protocol)?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(OptimizationError::Integrity(
                "production optimization definition fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, OptimizationError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        artifact_core::fingerprint(&value)
            .map_err(|error| OptimizationError::Integrity(error.to_string()))
    }

    pub fn reproduce_specification_fingerprint(&self) -> Result<String, OptimizationError> {
        artifact_core::fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "name": self.name,
            "manifest_fingerprint": self.manifest_fingerprint,
            "project": self.project,
            "proposal": self.proposal,
            "delta_selection": self.delta_selection,
            "training_snapshot": self.training_snapshot,
            "training_snapshot_specification_fingerprint": self.training_snapshot_specification_fingerprint,
            "benchmark": self.benchmark,
            "metric_source_protocol": self.metric_source_protocol,
            "metric_contract": self.metric_contract,
            "development_suite_keys": self.development_suite_keys,
            "sealed_suite_key": self.sealed_suite_key,
            "candidates": self.candidates,
            "campaign_budget": self.campaign_budget,
            "maximum_evaluation_seconds": self.maximum_evaluation_seconds,
            "approval_mode": self.approval_mode,
            "selection_rule": self.selection_rule,
        }))
        .map_err(|error| OptimizationError::Integrity(error.to_string()))
    }

    fn validate_fields(
        &self,
        project: &ExternalProjectSnapshot,
        metric_source_protocol: &ExperimentProtocol,
    ) -> Result<(), OptimizationError> {
        self.project.validate()?;
        self.proposal.validate()?;
        self.delta_selection.validate()?;
        self.training_snapshot.validate()?;
        self.metric_source_protocol.validate()?;
        self.metric_contract
            .validate_integrity()
            .map_err(|error| OptimizationError::InvalidDefinition(error.to_string()))?;
        self.benchmark
            .validate_integrity()
            .map_err(|error| OptimizationError::InvalidDefinition(error.to_string()))?;
        self.campaign_budget
            .validate()
            .map_err(|error| OptimizationError::InvalidDefinition(error.to_string()))?;
        let expected_development_keys = metric_source_protocol.development_suite_keys();
        if self.schema_version != OPTIMIZATION_DEFINITION_SCHEMA_VERSION
            || self.id.is_nil()
            || self.name.trim() != self.name
            || self.name.is_empty()
            || !canonical_fingerprint(&self.manifest_fingerprint)
            || !self.specification_fingerprint.is_empty()
                && !canonical_fingerprint(&self.specification_fingerprint)
            || self.project.id != project.id
            || self.project.fingerprint != project.fingerprint
            || self.metric_source_protocol.id != metric_source_protocol.id
            || self.metric_source_protocol.fingerprint != metric_source_protocol.fingerprint
            || self.metric_contract != metric_source_protocol.metric_contract
            || !canonical_fingerprint(&self.training_snapshot_specification_fingerprint)
            || self.development_suite_keys != expected_development_keys
            || self.sealed_suite_key != metric_source_protocol.sealed_suite_key
            || self.maximum_evaluation_seconds == 0
            || self.candidates.is_empty()
            || self.campaign_budget.maximum_iterations != 1
            || self.campaign_budget.maximum_sealed_evaluations != 1
            || self.campaign_budget.maximum_candidates != self.candidates.len() as u32
            || self.approval_mode != OptimizationApprovalMode::ExplicitSealedUse
            || self.selection_rule != DevelopmentSelectionRule::MaximizeWorstSuiteThenMean
            || !self.fingerprint.is_empty() && !canonical_fingerprint(&self.fingerprint)
        {
            return Err(OptimizationError::InvalidDefinition(
                "production optimization definition is incomplete or contradictory".into(),
            ));
        }
        let candidate_ids = self
            .candidates
            .iter()
            .map(|value| value.id)
            .collect::<BTreeSet<_>>();
        if candidate_ids.len() != self.candidates.len()
            || self
                .candidates
                .iter()
                .any(|candidate| candidate.validate_integrity(project).is_err())
            || self
                .development_suite_keys
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(OptimizationError::InvalidDefinition(
                "production optimization candidates or suites are not canonical".into(),
            ));
        }
        let expected_development = metric_source_protocol
            .baseline_development_reports()
            .into_iter()
            .map(|report| (report.suite_key.clone(), report.suite_fingerprint.clone()))
            .collect();
        if self.benchmark.development_suite_fingerprints != expected_development
            || self.benchmark.sealed_suite_key != metric_source_protocol.sealed_suite_key
            || self.benchmark.sealed_suite_fingerprint
                != metric_source_protocol
                    .baseline_sealed_report
                    .suite_fingerprint
        {
            return Err(OptimizationError::InvalidDefinition(
                "production optimization benchmark authority differs from its metric source".into(),
            ));
        }
        if !self.specification_fingerprint.is_empty()
            && self.reproduce_specification_fingerprint()? != self.specification_fingerprint
        {
            return Err(OptimizationError::Integrity(
                "production optimization specification fingerprint changed".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionOptimizationRun {
    pub schema_version: u32,
    pub id: Uuid,
    pub definition_id: Uuid,
    pub definition_fingerprint: String,
    pub reserved_campaign_id: Uuid,
    pub reserved_protocol_id: Uuid,
    pub reserved_experiment_run_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ProductionOptimizationRun {
    pub fn create(
        definition: &ProductionOptimizationDefinition,
        created_at: DateTime<Utc>,
    ) -> Result<Self, OptimizationError> {
        Self::create_with_reservations(
            definition,
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            created_at,
        )
    }

    /// Reserve every downstream identity before a side effect can start. Existing exact
    /// protocol/run identities may be supplied when adopting a previously proven experiment.
    pub fn create_with_reservations(
        definition: &ProductionOptimizationDefinition,
        reserved_campaign_id: Uuid,
        reserved_protocol_id: Uuid,
        reserved_experiment_run_id: Uuid,
        created_at: DateTime<Utc>,
    ) -> Result<Self, OptimizationError> {
        let mut value = Self {
            schema_version: OPTIMIZATION_RUN_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            definition_id: definition.id,
            definition_fingerprint: definition.fingerprint.clone(),
            reserved_campaign_id,
            reserved_protocol_id,
            reserved_experiment_run_id,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate_integrity(definition)?;
        Ok(value)
    }

    pub fn validate_integrity(
        &self,
        definition: &ProductionOptimizationDefinition,
    ) -> Result<(), OptimizationError> {
        if self.schema_version != OPTIMIZATION_RUN_SCHEMA_VERSION
            || self.id.is_nil()
            || self.reserved_campaign_id.is_nil()
            || self.reserved_protocol_id.is_nil()
            || self.reserved_experiment_run_id.is_nil()
            || self.definition_id != definition.id
            || self.definition_fingerprint != definition.fingerprint
            || !canonical_fingerprint(&self.fingerprint)
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(OptimizationError::Integrity(
                "production optimization run changed or is foreign".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, OptimizationError> {
        artifact_core::fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "id": self.id,
            "definition_id": self.definition_id,
            "definition_fingerprint": self.definition_fingerprint,
            "reserved_campaign_id": self.reserved_campaign_id,
            "reserved_protocol_id": self.reserved_protocol_id,
            "reserved_experiment_run_id": self.reserved_experiment_run_id,
            "created_at": self.created_at,
        }))
        .map_err(|error| OptimizationError::Integrity(error.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationRunState {
    Planned,
    CampaignActive,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OptimizationEventKind {
    Created,
    AutomaticExecutionAuthorized {
        authorized_by: String,
    },
    CampaignAttached {
        campaign_fingerprint: String,
    },
    Completed {
        decision: FinalDecision,
        campaign_head_fingerprint: String,
    },
    Cancelled {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

impl OptimizationEventKind {
    fn schema_version(&self) -> u32 {
        match self {
            Self::AutomaticExecutionAuthorized { .. } => 2,
            _ => OPTIMIZATION_EVENT_SCHEMA_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationEvent {
    pub schema_version: u32,
    pub id: Uuid,
    pub run_id: Uuid,
    pub run_fingerprint: String,
    pub sequence: u32,
    pub previous_event_fingerprint: Option<String>,
    pub event: OptimizationEventKind,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl OptimizationEvent {
    fn create(
        run: &ProductionOptimizationRun,
        sequence: u32,
        previous_event_fingerprint: Option<String>,
        event: OptimizationEventKind,
        created_at: DateTime<Utc>,
    ) -> Result<Self, OptimizationError> {
        let mut value = Self {
            schema_version: event.schema_version(),
            id: Uuid::new_v4(),
            run_id: run.id,
            run_fingerprint: run.fingerprint.clone(),
            sequence,
            previous_event_fingerprint,
            event,
            created_at,
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint()?;
        value.validate_integrity(run)?;
        Ok(value)
    }

    pub fn validate_integrity(
        &self,
        run: &ProductionOptimizationRun,
    ) -> Result<(), OptimizationError> {
        if let OptimizationEventKind::AutomaticExecutionAuthorized { authorized_by } = &self.event {
            validate_authorizer(authorized_by)?;
        }
        if self.schema_version != self.event.schema_version()
            || self.id.is_nil()
            || self.run_id != run.id
            || self.run_fingerprint != run.fingerprint
            || self.sequence == 0
            || self.sequence == 1 && self.previous_event_fingerprint.is_some()
            || self.sequence > 1
                && !self
                    .previous_event_fingerprint
                    .as_deref()
                    .is_some_and(canonical_fingerprint)
            || !canonical_fingerprint(&self.fingerprint)
            || self.reproduce_fingerprint()? != self.fingerprint
        {
            return Err(OptimizationError::Integrity(
                "production optimization event changed or is foreign".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, OptimizationError> {
        let mut value = self.clone();
        value.fingerprint.clear();
        artifact_core::fingerprint(&value)
            .map_err(|error| OptimizationError::Integrity(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptimizationView {
    pub run_id: Uuid,
    pub state: OptimizationRunState,
    pub campaign_fingerprint: Option<String>,
    pub decision: Option<FinalDecision>,
    pub failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_execution_authorized_by: Option<String>,
    pub last_sequence: u32,
    pub last_event_fingerprint: String,
    pub updated_at: DateTime<Utc>,
}

impl OptimizationView {
    /// Authorizes routine continuation of this exact immutable run, not new
    /// candidates, budgets, provider calls, or protected evaluation. A retry by
    /// the same authorizer reuses its event instead of changing the journal.
    pub fn authorize_automatic_execution(
        &self,
        run: &ProductionOptimizationRun,
        authorized_by: &str,
        created_at: DateTime<Utc>,
    ) -> Result<Option<OptimizationEvent>, OptimizationError> {
        if self.run_id != run.id {
            return Err(OptimizationError::Integrity(
                "optimization view is foreign".into(),
            ));
        }
        validate_authorizer(authorized_by)?;
        if let Some(previous) = &self.automatic_execution_authorized_by {
            if previous != authorized_by {
                return Err(OptimizationError::InvalidTransition(
                    "automatic execution already belongs to another authorizer".into(),
                ));
            }
            return Ok(None);
        }
        self.next_event(
            run,
            OptimizationEventKind::AutomaticExecutionAuthorized {
                authorized_by: authorized_by.to_owned(),
            },
            created_at,
        )
        .map(Some)
    }

    pub fn next_event(
        &self,
        run: &ProductionOptimizationRun,
        event: OptimizationEventKind,
        created_at: DateTime<Utc>,
    ) -> Result<OptimizationEvent, OptimizationError> {
        if self.run_id != run.id {
            return Err(OptimizationError::Integrity(
                "optimization view is foreign".into(),
            ));
        }
        self.validate_new_authorization(&event)?;
        validate_transition(self.state, &event)?;
        OptimizationEvent::create(
            run,
            self.last_sequence.checked_add(1).ok_or_else(|| {
                OptimizationError::InvalidTransition("event sequence overflowed".into())
            })?,
            Some(self.last_event_fingerprint.clone()),
            event,
            created_at.max(self.updated_at),
        )
    }

    fn validate_new_authorization(
        &self,
        event: &OptimizationEventKind,
    ) -> Result<(), OptimizationError> {
        if matches!(
            event,
            OptimizationEventKind::AutomaticExecutionAuthorized { .. }
        ) && self.automatic_execution_authorized_by.is_some()
        {
            return Err(OptimizationError::InvalidTransition(
                "automatic execution is already authorized".into(),
            ));
        }
        Ok(())
    }
}

pub fn first_optimization_event(
    run: &ProductionOptimizationRun,
    created_at: DateTime<Utc>,
) -> Result<OptimizationEvent, OptimizationError> {
    OptimizationEvent::create(run, 1, None, OptimizationEventKind::Created, created_at)
}

pub fn replay_optimization(
    run: &ProductionOptimizationRun,
    events: &[OptimizationEvent],
) -> Result<OptimizationView, OptimizationError> {
    let first = events
        .first()
        .ok_or_else(|| OptimizationError::Integrity("optimization journal is empty".into()))?;
    if first.sequence != 1 || first.event != OptimizationEventKind::Created {
        return Err(OptimizationError::Integrity(
            "optimization journal does not begin with creation".into(),
        ));
    }
    let mut view = OptimizationView {
        run_id: run.id,
        state: OptimizationRunState::Planned,
        campaign_fingerprint: None,
        decision: None,
        failure_reason: None,
        automatic_execution_authorized_by: None,
        last_sequence: 0,
        last_event_fingerprint: String::new(),
        updated_at: run.created_at,
    };
    for event in events {
        event.validate_integrity(run)?;
        let expected_previous = (!view.last_event_fingerprint.is_empty())
            .then_some(view.last_event_fingerprint.as_str());
        if event.sequence != view.last_sequence + 1
            || event.previous_event_fingerprint.as_deref() != expected_previous
        {
            return Err(OptimizationError::Integrity(
                "optimization journal is missing, reordered, or forked".into(),
            ));
        }
        if event.sequence > 1 {
            view.validate_new_authorization(&event.event)?;
            validate_transition(view.state, &event.event)?;
        }
        match &event.event {
            OptimizationEventKind::Created => {}
            OptimizationEventKind::AutomaticExecutionAuthorized { authorized_by } => {
                view.automatic_execution_authorized_by = Some(authorized_by.clone());
            }
            OptimizationEventKind::CampaignAttached {
                campaign_fingerprint,
            } => {
                if !canonical_fingerprint(campaign_fingerprint) {
                    return Err(OptimizationError::Integrity(
                        "campaign fingerprint is malformed".into(),
                    ));
                }
                view.state = OptimizationRunState::CampaignActive;
                view.campaign_fingerprint = Some(campaign_fingerprint.clone());
            }
            OptimizationEventKind::Completed {
                decision,
                campaign_head_fingerprint,
            } => {
                if !canonical_fingerprint(campaign_head_fingerprint) {
                    return Err(OptimizationError::Integrity(
                        "campaign head fingerprint is malformed".into(),
                    ));
                }
                view.state = OptimizationRunState::Completed;
                view.decision = Some(*decision);
            }
            OptimizationEventKind::Cancelled { reason } => {
                view.state = OptimizationRunState::Cancelled;
                view.failure_reason = Some(canonical_reason(reason)?);
            }
            OptimizationEventKind::Failed { reason } => {
                view.state = OptimizationRunState::Failed;
                view.failure_reason = Some(canonical_reason(reason)?);
            }
        }
        view.last_sequence = event.sequence;
        view.last_event_fingerprint = event.fingerprint.clone();
        view.updated_at = event.created_at;
    }
    Ok(view)
}

fn validate_transition(
    state: OptimizationRunState,
    event: &OptimizationEventKind,
) -> Result<(), OptimizationError> {
    let allowed = matches!(
        (state, event),
        (
            OptimizationRunState::Planned | OptimizationRunState::CampaignActive,
            OptimizationEventKind::AutomaticExecutionAuthorized { .. }
        ) | (
            OptimizationRunState::Planned,
            OptimizationEventKind::CampaignAttached { .. }
        ) | (
            OptimizationRunState::Planned,
            OptimizationEventKind::Cancelled { .. }
        ) | (
            OptimizationRunState::Planned,
            OptimizationEventKind::Failed { .. }
        ) | (
            OptimizationRunState::CampaignActive,
            OptimizationEventKind::Completed { .. }
        ) | (
            OptimizationRunState::CampaignActive,
            OptimizationEventKind::Cancelled { .. }
        ) | (
            OptimizationRunState::CampaignActive,
            OptimizationEventKind::Failed { .. }
        )
    );
    if allowed {
        Ok(())
    } else {
        Err(OptimizationError::InvalidTransition(
            "production optimization lifecycle transition is not allowed".into(),
        ))
    }
}

fn validate_authorizer(value: &str) -> Result<(), OptimizationError> {
    if value.trim() != value
        || value.is_empty()
        || value.chars().count() > 120
        || value.chars().any(char::is_control)
    {
        return Err(OptimizationError::InvalidTransition(
            "automatic execution authorizer must be canonical and bounded".into(),
        ));
    }
    Ok(())
}

fn canonical_reason(value: &str) -> Result<String, OptimizationError> {
    if value.trim() != value || value.is_empty() || value.chars().count() > 1_000 {
        return Err(OptimizationError::InvalidTransition(
            "optimization reason must be canonical and bounded".into(),
        ));
    }
    Ok(value.to_owned())
}

fn canonical_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .chars()
                .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
    })
}

#[derive(Debug, Error)]
pub enum OptimizationError {
    #[error("invalid production optimization definition: {0}")]
    InvalidDefinition(String),
    #[error("invalid production optimization lifecycle transition: {0}")]
    InvalidTransition(String),
    #[error("production optimization integrity failure: {0}")]
    Integrity(String),
}

#[derive(Debug, Error)]
#[error("production optimization store failure: {0}")]
pub struct OptimizationStoreError(pub String);

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait OptimizationLaunchStore: Send + Sync {
    fn create_optimization(
        &self,
        definition: ProductionOptimizationDefinition,
        run: ProductionOptimizationRun,
        first_event: OptimizationEvent,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>>;

    fn find_optimization_by_manifest(
        &self,
        manifest_fingerprint: String,
    ) -> BoxFuture<
        '_,
        Result<
            Option<(ProductionOptimizationDefinition, ProductionOptimizationRun)>,
            OptimizationStoreError,
        >,
    >;

    fn get_optimization_run(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<
        '_,
        Result<
            Option<(ProductionOptimizationDefinition, ProductionOptimizationRun)>,
            OptimizationStoreError,
        >,
    >;

    fn append_optimization_event(
        &self,
        event: OptimizationEvent,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>>;

    fn list_optimization_events(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<OptimizationEvent>, OptimizationStoreError>>;
}
