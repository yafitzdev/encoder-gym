//! Thin, append-only coordination between renewable benchmark generations and
//! finite production encoder experiments.
//!
//! Benchmark qualification remains owned by `workflow-core`; candidate
//! training, evaluation, selection, and acceptance remain owned by
//! `encoder-experiment-core`. This crate only binds their immutable artifacts
//! into a recoverable long-range campaign.

use std::{collections::BTreeMap, future::Future, pin::Pin};

use chrono::{DateTime, Utc};
use encoder_experiment_core::{
    journal::{ExperimentRunState, ExperimentView, FinalDecision},
    protocol::ExperimentProtocol,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use workflow_core::benchmark_generation::{
    BenchmarkGeneration, BenchmarkGenerationEvent, BenchmarkGenerationEventKind,
    BenchmarkGenerationView,
};

pub const CAMPAIGN_SCHEMA_VERSION: u32 = 1;
pub const CAMPAIGN_EVENT_SCHEMA_VERSION: u32 = 1;
pub const SEALED_ASSESSMENT_EXPOSURE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignBudget {
    pub maximum_iterations: u32,
    pub maximum_candidates: u32,
    pub maximum_training_seconds: u64,
    pub maximum_development_evaluations: u32,
    pub maximum_sealed_evaluations: u32,
    pub maximum_backend_operations: u32,
}

impl CampaignBudget {
    pub fn validate(&self) -> Result<(), CampaignError> {
        if self.maximum_iterations == 0
            || self.maximum_candidates == 0
            || self.maximum_training_seconds == 0
            || self.maximum_development_evaluations == 0
            || self.maximum_sealed_evaluations < self.maximum_iterations
            || self.maximum_backend_operations == 0
        {
            return Err(CampaignError::InvalidBudget);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignUsage {
    pub iterations: u32,
    pub candidates: u32,
    pub training_seconds: u64,
    pub development_evaluations: u32,
    pub sealed_evaluations: u32,
    pub backend_operations: u32,
}

impl CampaignUsage {
    pub fn for_protocol(protocol: &ExperimentProtocol) -> Result<Self, CampaignError> {
        let suite_count = u32::try_from(protocol.development_suite_keys().len())
            .map_err(|_| CampaignError::UsageOverflow)?;
        let candidates =
            u32::try_from(protocol.candidates.len()).map_err(|_| CampaignError::UsageOverflow)?;
        let development_evaluations = candidates
            .checked_mul(suite_count)
            .ok_or(CampaignError::UsageOverflow)?;
        let training_seconds = protocol
            .candidates
            .iter()
            .try_fold(0_u64, |total, candidate| {
                total.checked_add(candidate.maximum_training_seconds)
            })
            .ok_or(CampaignError::UsageOverflow)?;
        // Baseline development + sealed references, every candidate training,
        // every candidate-suite evaluation, and the one selected sealed run.
        let backend_operations = suite_count
            .checked_add(1)
            .and_then(|value| value.checked_add(candidates))
            .and_then(|value| value.checked_add(development_evaluations))
            .and_then(|value| value.checked_add(1))
            .ok_or(CampaignError::UsageOverflow)?;
        Ok(Self {
            iterations: 1,
            candidates,
            training_seconds,
            development_evaluations,
            sealed_evaluations: protocol.budget.maximum_sealed_evaluations,
            backend_operations,
        })
    }

    fn checked_add(self, increment: Self) -> Result<Self, CampaignError> {
        Ok(Self {
            iterations: self
                .iterations
                .checked_add(increment.iterations)
                .ok_or(CampaignError::UsageOverflow)?,
            candidates: self
                .candidates
                .checked_add(increment.candidates)
                .ok_or(CampaignError::UsageOverflow)?,
            training_seconds: self
                .training_seconds
                .checked_add(increment.training_seconds)
                .ok_or(CampaignError::UsageOverflow)?,
            development_evaluations: self
                .development_evaluations
                .checked_add(increment.development_evaluations)
                .ok_or(CampaignError::UsageOverflow)?,
            sealed_evaluations: self
                .sealed_evaluations
                .checked_add(increment.sealed_evaluations)
                .ok_or(CampaignError::UsageOverflow)?,
            backend_operations: self
                .backend_operations
                .checked_add(increment.backend_operations)
                .ok_or(CampaignError::UsageOverflow)?,
        })
    }

    fn validate_within(&self, budget: &CampaignBudget) -> Result<(), CampaignError> {
        if self.iterations > budget.maximum_iterations
            || self.candidates > budget.maximum_candidates
            || self.training_seconds > budget.maximum_training_seconds
            || self.development_evaluations > budget.maximum_development_evaluations
            || self.sealed_evaluations > budget.maximum_sealed_evaluations
            || self.backend_operations > budget.maximum_backend_operations
        {
            return Err(CampaignError::BudgetExhausted);
        }
        Ok(())
    }
}

/// Provider-neutral binding derived from one currently active benchmark
/// generation. It contains no benchmark rows or adapter-specific types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignBenchmarkBinding {
    pub generation_id: Uuid,
    pub generation_fingerprint: String,
    pub predecessor_generation_id: Option<Uuid>,
    pub predecessor_generation_fingerprint: Option<String>,
    pub development_suite_fingerprints: BTreeMap<String, String>,
    pub sealed_suite_key: String,
    pub sealed_suite_id: Uuid,
    pub sealed_suite_fingerprint: String,
    pub fingerprint: String,
}

impl CampaignBenchmarkBinding {
    pub fn from_active_generation(
        generation: &BenchmarkGeneration,
        view: &BenchmarkGenerationView,
        sealed_suite_key: impl Into<String>,
    ) -> Result<Self, CampaignError> {
        generation
            .validate_integrity()
            .map_err(|error| CampaignError::Benchmark(error.to_string()))?;
        if view.generation_id != generation.id || !view.is_adaptive_eligible() {
            return Err(CampaignError::GenerationIneligible);
        }
        let development_suite_fingerprints = generation
            .development_suites
            .iter()
            .map(|authority| {
                (
                    authority.suite_key.clone(),
                    authority.bundle.development_suite_fingerprint.clone(),
                )
            })
            .collect();
        let mut value = Self {
            generation_id: generation.id,
            generation_fingerprint: generation.fingerprint.clone(),
            predecessor_generation_id: generation.predecessor_id,
            predecessor_generation_fingerprint: generation.predecessor_fingerprint.clone(),
            development_suite_fingerprints,
            sealed_suite_key: canonical_text(sealed_suite_key, "sealed suite key")?,
            sealed_suite_id: generation.sealed_suite_id,
            sealed_suite_fingerprint: generation.sealed_suite_fingerprint.clone(),
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), CampaignError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(CampaignError::Integrity(
                "campaign benchmark binding fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, CampaignError> {
        fingerprint(&(
            self.generation_id,
            self.generation_fingerprint.as_str(),
            self.predecessor_generation_id,
            self.predecessor_generation_fingerprint.as_deref(),
            &self.development_suite_fingerprints,
            self.sealed_suite_key.as_str(),
            self.sealed_suite_id,
            self.sealed_suite_fingerprint.as_str(),
        ))
    }

    fn validate_fields(&self) -> Result<(), CampaignError> {
        if self.generation_id.is_nil()
            || !canonical_fingerprint(&self.generation_fingerprint)
            || self.predecessor_generation_id.is_some()
                != self.predecessor_generation_fingerprint.is_some()
            || self
                .predecessor_generation_fingerprint
                .as_deref()
                .is_some_and(|value| !canonical_fingerprint(value))
            || self.development_suite_fingerprints.is_empty()
            || self
                .development_suite_fingerprints
                .iter()
                .any(|(key, value)| {
                    canonical_text(key.clone(), "development suite key").is_err()
                        || !canonical_fingerprint(value)
                })
            || canonical_text(self.sealed_suite_key.clone(), "sealed suite key").is_err()
            || self
                .development_suite_fingerprints
                .contains_key(&self.sealed_suite_key)
            || self.sealed_suite_id.is_nil()
            || !canonical_fingerprint(&self.sealed_suite_fingerprint)
            || !self.fingerprint.is_empty() && !canonical_fingerprint(&self.fingerprint)
        {
            return Err(CampaignError::InvalidBinding);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionCampaign {
    pub schema_version: u32,
    pub id: Uuid,
    pub name: String,
    pub project_snapshot_id: Uuid,
    pub project_snapshot_fingerprint: String,
    pub budget: CampaignBudget,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

/// Row-free, immutable proof that one explicitly authorized candidate consumed
/// one generation's sealed suite. The report and assessment stay owned by the
/// experiment journal; this artifact only binds their exact fingerprints into
/// campaign provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SealedAssessmentExposure {
    pub schema_version: u32,
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub campaign_fingerprint: String,
    pub generation_id: Uuid,
    pub generation_fingerprint: String,
    pub iteration: u32,
    pub experiment_run_id: Uuid,
    pub experiment_protocol_id: Uuid,
    pub experiment_protocol_fingerprint: String,
    pub candidate_id: Uuid,
    pub sealed_suite_id: Uuid,
    pub sealed_suite_fingerprint: String,
    pub report_id: Uuid,
    pub report_fingerprint: String,
    pub assessment_id: Uuid,
    pub assessment_fingerprint: String,
    pub authorized_by: String,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl SealedAssessmentExposure {
    pub fn from_completed_experiment(
        campaign: &ProductionCampaign,
        view: &CampaignView,
        experiment: &ExperimentView,
        created_at: DateTime<Utc>,
    ) -> Result<Self, CampaignError> {
        let binding = view
            .current_generation
            .as_ref()
            .ok_or(CampaignError::GenerationIneligible)?;
        let report = experiment
            .sealed_report
            .as_ref()
            .ok_or(CampaignError::ExperimentMismatch)?;
        let assessment = experiment
            .sealed_assessment
            .as_ref()
            .ok_or(CampaignError::ExperimentMismatch)?;
        let candidate_id = experiment
            .selected_candidate_id
            .ok_or(CampaignError::ExperimentMismatch)?;
        if campaign.id != view.campaign_id
            || view.state != CampaignState::SealedAuthorized
            || Some(experiment.run_id) != view.run_id
            || experiment.state != ExperimentRunState::Completed
            || experiment.final_decision.is_none()
            || report.suite_key != binding.sealed_suite_key
            || report.suite_fingerprint != binding.sealed_suite_fingerprint
            || report.model.fingerprint
                != experiment
                    .candidates
                    .get(&candidate_id)
                    .and_then(|candidate| candidate.train_output.as_ref())
                    .map(|output| output.model.fingerprint.as_str())
                    .ok_or(CampaignError::ExperimentMismatch)?
        {
            return Err(CampaignError::ExperimentMismatch);
        }
        let mut value = Self {
            schema_version: SEALED_ASSESSMENT_EXPOSURE_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            campaign_id: campaign.id,
            campaign_fingerprint: campaign.fingerprint.clone(),
            generation_id: binding.generation_id,
            generation_fingerprint: binding.generation_fingerprint.clone(),
            iteration: view.current_iteration,
            experiment_run_id: experiment.run_id,
            experiment_protocol_id: experiment.protocol_id,
            experiment_protocol_fingerprint: view
                .protocol_fingerprint
                .clone()
                .ok_or(CampaignError::ExperimentMismatch)?,
            candidate_id,
            sealed_suite_id: binding.sealed_suite_id,
            sealed_suite_fingerprint: binding.sealed_suite_fingerprint.clone(),
            report_id: report.id,
            report_fingerprint: report.fingerprint.clone(),
            assessment_id: assessment.id,
            assessment_fingerprint: assessment.fingerprint.clone(),
            authorized_by: canonical_text(
                experiment
                    .sealed_authorized_by
                    .clone()
                    .ok_or(CampaignError::ExperimentMismatch)?,
                "sealed authorizer",
            )?,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), CampaignError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(CampaignError::Integrity(
                "sealed assessment exposure fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, CampaignError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "id": self.id,
            "campaign_id": self.campaign_id,
            "campaign_fingerprint": self.campaign_fingerprint,
            "generation_id": self.generation_id,
            "generation_fingerprint": self.generation_fingerprint,
            "iteration": self.iteration,
            "experiment_run_id": self.experiment_run_id,
            "experiment_protocol_id": self.experiment_protocol_id,
            "experiment_protocol_fingerprint": self.experiment_protocol_fingerprint,
            "candidate_id": self.candidate_id,
            "sealed_suite_id": self.sealed_suite_id,
            "sealed_suite_fingerprint": self.sealed_suite_fingerprint,
            "report_id": self.report_id,
            "report_fingerprint": self.report_fingerprint,
            "assessment_id": self.assessment_id,
            "assessment_fingerprint": self.assessment_fingerprint,
            "authorized_by": self.authorized_by,
            "created_at": self.created_at,
        }))
    }

    fn validate_fields(&self) -> Result<(), CampaignError> {
        if self.schema_version != SEALED_ASSESSMENT_EXPOSURE_SCHEMA_VERSION
            || [
                self.id,
                self.campaign_id,
                self.generation_id,
                self.experiment_run_id,
                self.experiment_protocol_id,
                self.candidate_id,
                self.sealed_suite_id,
                self.report_id,
                self.assessment_id,
            ]
            .contains(&Uuid::nil())
            || [
                self.campaign_fingerprint.as_str(),
                self.generation_fingerprint.as_str(),
                self.experiment_protocol_fingerprint.as_str(),
                self.sealed_suite_fingerprint.as_str(),
                self.report_fingerprint.as_str(),
                self.assessment_fingerprint.as_str(),
            ]
            .iter()
            .any(|value| !canonical_fingerprint(value))
            || self.iteration == 0
            || canonical_text(self.authorized_by.clone(), "sealed authorizer").is_err()
            || !self.fingerprint.is_empty() && !canonical_fingerprint(&self.fingerprint)
        {
            return Err(CampaignError::ExperimentMismatch);
        }
        Ok(())
    }
}

impl ProductionCampaign {
    pub fn create(
        name: impl Into<String>,
        project_snapshot_id: Uuid,
        project_snapshot_fingerprint: impl Into<String>,
        budget: CampaignBudget,
        created_at: DateTime<Utc>,
    ) -> Result<Self, CampaignError> {
        budget.validate()?;
        let mut value = Self {
            schema_version: CAMPAIGN_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            name: canonical_text(name, "campaign name")?,
            project_snapshot_id,
            project_snapshot_fingerprint: project_snapshot_fingerprint.into(),
            budget,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), CampaignError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(CampaignError::Integrity(
                "production campaign fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, CampaignError> {
        fingerprint(&(
            self.schema_version,
            self.id,
            self.name.as_str(),
            self.project_snapshot_id,
            self.project_snapshot_fingerprint.as_str(),
            &self.budget,
            self.created_at,
        ))
    }

    fn validate_fields(&self) -> Result<(), CampaignError> {
        self.budget.validate()?;
        if self.schema_version != CAMPAIGN_SCHEMA_VERSION
            || self.id.is_nil()
            || canonical_text(self.name.clone(), "campaign name").is_err()
            || self.project_snapshot_id.is_nil()
            || !canonical_fingerprint(&self.project_snapshot_fingerprint)
            || !self.fingerprint.is_empty() && !canonical_fingerprint(&self.fingerprint)
        {
            return Err(CampaignError::InvalidCampaign);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignState {
    AwaitingGeneration,
    ReadyToPrepare,
    ReadyToStart,
    RunningDevelopment,
    AwaitingSealedAuthorization,
    SealedAuthorized,
    AwaitingFinalization,
    RenewalRequired,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CampaignEventKind {
    Created,
    GenerationBound {
        binding: CampaignBenchmarkBinding,
    },
    IterationPrepared {
        iteration: u32,
        protocol_id: Uuid,
        protocol_fingerprint: String,
        reserved_usage: CampaignUsage,
    },
    RunStarted {
        iteration: u32,
        run_id: Uuid,
    },
    DevelopmentCompleted {
        iteration: u32,
        run_id: Uuid,
        experiment_head_fingerprint: String,
        selected_candidate_id: Option<Uuid>,
    },
    SealedAuthorized {
        iteration: u32,
        run_id: Uuid,
        candidate_id: Uuid,
        authorized_by: String,
    },
    IterationFinalized {
        iteration: u32,
        run_id: Uuid,
        decision: FinalDecision,
        experiment_head_fingerprint: String,
        sealed_exposure: Option<Box<SealedAssessmentExposure>>,
        generation_terminal_event_id: Uuid,
        generation_terminal_event_fingerprint: String,
    },
    RenewalHandoffCreated {
        handoff_id: Uuid,
        handoff_fingerprint: String,
    },
    Completed {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignEvent {
    pub schema_version: u32,
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub campaign_fingerprint: String,
    pub sequence: u32,
    pub previous_event_fingerprint: Option<String>,
    pub event: CampaignEventKind,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl CampaignEvent {
    fn create(
        campaign: &ProductionCampaign,
        sequence: u32,
        previous_event_fingerprint: Option<String>,
        event: CampaignEventKind,
        created_at: DateTime<Utc>,
    ) -> Result<Self, CampaignError> {
        let mut value = Self {
            schema_version: CAMPAIGN_EVENT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            campaign_id: campaign.id,
            campaign_fingerprint: campaign.fingerprint.clone(),
            sequence,
            previous_event_fingerprint,
            event,
            created_at,
            fingerprint: String::new(),
        };
        value.validate_fields()?;
        value.fingerprint = value.reproduce_fingerprint()?;
        Ok(value)
    }

    pub fn validate_integrity(&self) -> Result<(), CampaignError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(CampaignError::Integrity(
                "campaign event fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, CampaignError> {
        fingerprint(&(
            self.schema_version,
            self.id,
            self.campaign_id,
            self.campaign_fingerprint.as_str(),
            self.sequence,
            self.previous_event_fingerprint.as_deref(),
            &self.event,
            self.created_at,
        ))
    }

    fn validate_fields(&self) -> Result<(), CampaignError> {
        if self.schema_version != CAMPAIGN_EVENT_SCHEMA_VERSION
            || self.id.is_nil()
            || self.campaign_id.is_nil()
            || !canonical_fingerprint(&self.campaign_fingerprint)
            || self.sequence == 0
            || self.sequence == 1 && self.previous_event_fingerprint.is_some()
            || self.sequence > 1
                && !self
                    .previous_event_fingerprint
                    .as_deref()
                    .is_some_and(canonical_fingerprint)
            || !self.fingerprint.is_empty() && !canonical_fingerprint(&self.fingerprint)
        {
            return Err(CampaignError::InvalidEvent);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CampaignView {
    pub campaign_id: Uuid,
    pub state: CampaignState,
    pub current_generation: Option<CampaignBenchmarkBinding>,
    pub current_iteration: u32,
    pub protocol_id: Option<Uuid>,
    pub protocol_fingerprint: Option<String>,
    pub run_id: Option<Uuid>,
    pub selected_candidate_id: Option<Uuid>,
    pub reserved_usage: CampaignUsage,
    pub renewal_handoff_id: Option<Uuid>,
    pub renewal_handoff_fingerprint: Option<String>,
    pub last_sequence: u32,
    pub last_event_fingerprint: String,
    pub updated_at: DateTime<Utc>,
}

impl CampaignView {
    fn next_event(
        &self,
        campaign: &ProductionCampaign,
        event: CampaignEventKind,
        created_at: DateTime<Utc>,
    ) -> Result<CampaignEvent, CampaignError> {
        CampaignEvent::create(
            campaign,
            self.last_sequence
                .checked_add(1)
                .ok_or(CampaignError::InvalidEvent)?,
            Some(self.last_event_fingerprint.clone()),
            event,
            created_at,
        )
    }
}

pub fn first_campaign_event(
    campaign: &ProductionCampaign,
    created_at: DateTime<Utc>,
) -> Result<CampaignEvent, CampaignError> {
    campaign.validate_integrity()?;
    CampaignEvent::create(campaign, 1, None, CampaignEventKind::Created, created_at)
}

pub fn bind_generation_event(
    campaign: &ProductionCampaign,
    view: &CampaignView,
    binding: CampaignBenchmarkBinding,
    created_at: DateTime<Utc>,
) -> Result<CampaignEvent, CampaignError> {
    binding.validate_integrity()?;
    if !matches!(
        view.state,
        CampaignState::AwaitingGeneration | CampaignState::RenewalRequired
    ) || view.current_iteration >= campaign.budget.maximum_iterations
    {
        return Err(CampaignError::IllegalTransition);
    }
    if let Some(previous) = &view.current_generation {
        if binding.predecessor_generation_id != Some(previous.generation_id)
            || binding.predecessor_generation_fingerprint.as_deref()
                != Some(previous.generation_fingerprint.as_str())
        {
            return Err(CampaignError::SuccessorMismatch);
        }
    }
    view.next_event(
        campaign,
        CampaignEventKind::GenerationBound { binding },
        created_at,
    )
}

pub fn prepare_iteration_event(
    campaign: &ProductionCampaign,
    view: &CampaignView,
    protocol: &ExperimentProtocol,
    created_at: DateTime<Utc>,
) -> Result<CampaignEvent, CampaignError> {
    if view.state != CampaignState::ReadyToPrepare {
        return Err(CampaignError::IllegalTransition);
    }
    let binding = view
        .current_generation
        .as_ref()
        .ok_or(CampaignError::GenerationIneligible)?;
    if protocol.project_snapshot_id != campaign.project_snapshot_id
        || protocol.project_snapshot_fingerprint != campaign.project_snapshot_fingerprint
        || protocol
            .baseline_development_reports()
            .into_iter()
            .map(|report| (report.suite_key.clone(), report.suite_fingerprint.clone()))
            .collect::<BTreeMap<_, _>>()
            != binding.development_suite_fingerprints
        || protocol.sealed_suite_key != binding.sealed_suite_key
        || protocol.baseline_sealed_report.suite_fingerprint != binding.sealed_suite_fingerprint
    {
        return Err(CampaignError::ProtocolBindingMismatch);
    }
    let usage = CampaignUsage::for_protocol(protocol)?;
    view.reserved_usage
        .checked_add(usage)?
        .validate_within(&campaign.budget)?;
    let iteration = view
        .current_iteration
        .checked_add(1)
        .ok_or(CampaignError::UsageOverflow)?;
    view.next_event(
        campaign,
        CampaignEventKind::IterationPrepared {
            iteration,
            protocol_id: protocol.id,
            protocol_fingerprint: protocol.fingerprint.clone(),
            reserved_usage: usage,
        },
        created_at,
    )
}

pub fn start_run_event(
    campaign: &ProductionCampaign,
    view: &CampaignView,
    experiment: &ExperimentView,
    created_at: DateTime<Utc>,
) -> Result<CampaignEvent, CampaignError> {
    if view.state != CampaignState::ReadyToStart
        || experiment.state != ExperimentRunState::Ready
        || Some(experiment.protocol_id) != view.protocol_id
    {
        return Err(CampaignError::ExperimentMismatch);
    }
    view.next_event(
        campaign,
        CampaignEventKind::RunStarted {
            iteration: view.current_iteration,
            run_id: experiment.run_id,
        },
        created_at,
    )
}

pub fn record_development_event(
    campaign: &ProductionCampaign,
    view: &CampaignView,
    experiment: &ExperimentView,
    created_at: DateTime<Utc>,
) -> Result<CampaignEvent, CampaignError> {
    if view.state != CampaignState::RunningDevelopment
        || Some(experiment.run_id) != view.run_id
        || !matches!(
            experiment.state,
            ExperimentRunState::AwaitingSealedAuthorization | ExperimentRunState::Completed
        )
        || !canonical_fingerprint(&experiment.last_event_fingerprint)
    {
        return Err(CampaignError::ExperimentMismatch);
    }
    view.next_event(
        campaign,
        CampaignEventKind::DevelopmentCompleted {
            iteration: view.current_iteration,
            run_id: experiment.run_id,
            experiment_head_fingerprint: experiment.last_event_fingerprint.clone(),
            selected_candidate_id: experiment.selected_candidate_id,
        },
        created_at,
    )
}

pub fn record_sealed_authorization_event(
    campaign: &ProductionCampaign,
    view: &CampaignView,
    experiment: &ExperimentView,
    authorized_by: impl Into<String>,
    created_at: DateTime<Utc>,
) -> Result<CampaignEvent, CampaignError> {
    if view.state != CampaignState::AwaitingSealedAuthorization
        || Some(experiment.run_id) != view.run_id
        || experiment.state != ExperimentRunState::SealedAuthorized
        || experiment.selected_candidate_id != view.selected_candidate_id
    {
        return Err(CampaignError::ExperimentMismatch);
    }
    view.next_event(
        campaign,
        CampaignEventKind::SealedAuthorized {
            iteration: view.current_iteration,
            run_id: experiment.run_id,
            candidate_id: experiment
                .selected_candidate_id
                .ok_or(CampaignError::ExperimentMismatch)?,
            authorized_by: canonical_text(authorized_by, "sealed authorizer")?,
        },
        created_at,
    )
}

pub fn finalize_iteration_event(
    campaign: &ProductionCampaign,
    view: &CampaignView,
    experiment: &ExperimentView,
    generation_terminal_event: &BenchmarkGenerationEvent,
    sealed_exposure: Option<SealedAssessmentExposure>,
    created_at: DateTime<Utc>,
) -> Result<CampaignEvent, CampaignError> {
    generation_terminal_event
        .validate_integrity()
        .map_err(|error| CampaignError::Benchmark(error.to_string()))?;
    let binding = view
        .current_generation
        .as_ref()
        .ok_or(CampaignError::GenerationIneligible)?;
    let selected = experiment.selected_candidate_id.is_some();
    if !matches!(
        view.state,
        CampaignState::SealedAuthorized | CampaignState::AwaitingFinalization
    ) || Some(experiment.run_id) != view.run_id
        || experiment.state != ExperimentRunState::Completed
        || experiment.final_decision.is_none()
        || experiment.selected_candidate_id != view.selected_candidate_id
        || selected != sealed_exposure.is_some()
        || generation_terminal_event.generation_id != binding.generation_id
        || generation_terminal_event.generation_fingerprint != binding.generation_fingerprint
    {
        return Err(CampaignError::ExperimentMismatch);
    }
    if let Some(exposure) = &sealed_exposure {
        exposure.validate_integrity()?;
        if exposure.campaign_id != campaign.id
            || exposure.generation_id != binding.generation_id
            || exposure.iteration != view.current_iteration
            || exposure.experiment_run_id != experiment.run_id
            || Some(exposure.candidate_id) != experiment.selected_candidate_id
        {
            return Err(CampaignError::ExperimentMismatch);
        }
    }
    let terminal_matches = match &generation_terminal_event.event {
        BenchmarkGenerationEventKind::IterationConsumed {
            experiment_run_id,
            experiment_protocol_fingerprint,
            sealed_exposure_id,
            sealed_exposure_fingerprint,
            final_decision_fingerprint,
        } => {
            let exposure = sealed_exposure.as_ref();
            selected
                && *experiment_run_id == experiment.run_id
                && Some(experiment_protocol_fingerprint) == view.protocol_fingerprint.as_ref()
                && exposure.map(|value| value.id) == Some(*sealed_exposure_id)
                && exposure.map(|value| value.fingerprint.as_str())
                    == Some(sealed_exposure_fingerprint.as_str())
                && final_decision_fingerprint == &experiment.last_event_fingerprint
        }
        BenchmarkGenerationEventKind::Exhausted { .. } => !selected,
        _ => false,
    };
    if !terminal_matches {
        return Err(CampaignError::ExperimentMismatch);
    }
    view.next_event(
        campaign,
        CampaignEventKind::IterationFinalized {
            iteration: view.current_iteration,
            run_id: experiment.run_id,
            decision: experiment
                .final_decision
                .expect("completed experiment owns a decision"),
            experiment_head_fingerprint: experiment.last_event_fingerprint.clone(),
            sealed_exposure: sealed_exposure.map(Box::new),
            generation_terminal_event_id: generation_terminal_event.id,
            generation_terminal_event_fingerprint: generation_terminal_event.fingerprint.clone(),
        },
        created_at,
    )
}

pub fn renewal_handoff_event(
    campaign: &ProductionCampaign,
    view: &CampaignView,
    handoff_id: Uuid,
    handoff_fingerprint: impl Into<String>,
    created_at: DateTime<Utc>,
) -> Result<CampaignEvent, CampaignError> {
    let handoff_fingerprint = handoff_fingerprint.into();
    if view.state != CampaignState::RenewalRequired
        || handoff_id.is_nil()
        || !canonical_fingerprint(&handoff_fingerprint)
    {
        return Err(CampaignError::IllegalTransition);
    }
    view.next_event(
        campaign,
        CampaignEventKind::RenewalHandoffCreated {
            handoff_id,
            handoff_fingerprint,
        },
        created_at,
    )
}

pub fn complete_campaign_event(
    campaign: &ProductionCampaign,
    view: &CampaignView,
    reason: impl Into<String>,
    created_at: DateTime<Utc>,
) -> Result<CampaignEvent, CampaignError> {
    if view.state != CampaignState::RenewalRequired {
        return Err(CampaignError::IllegalTransition);
    }
    view.next_event(
        campaign,
        CampaignEventKind::Completed {
            reason: canonical_text(reason, "completion reason")?,
        },
        created_at,
    )
}

pub fn replay_campaign(
    campaign: &ProductionCampaign,
    events: &[CampaignEvent],
) -> Result<CampaignView, CampaignError> {
    campaign.validate_integrity()?;
    let first = events.first().ok_or(CampaignError::EmptyJournal)?;
    let mut view = CampaignView {
        campaign_id: campaign.id,
        state: CampaignState::AwaitingGeneration,
        current_generation: None,
        current_iteration: 0,
        protocol_id: None,
        protocol_fingerprint: None,
        run_id: None,
        selected_candidate_id: None,
        reserved_usage: CampaignUsage::default(),
        renewal_handoff_id: None,
        renewal_handoff_fingerprint: None,
        last_sequence: 0,
        last_event_fingerprint: String::new(),
        updated_at: first.created_at,
    };
    for event in events {
        event.validate_integrity()?;
        if event.campaign_id != campaign.id
            || event.campaign_fingerprint != campaign.fingerprint
            || event.sequence != view.last_sequence + 1
            || event.previous_event_fingerprint.as_deref()
                != if view.last_sequence == 0 {
                    None
                } else {
                    Some(view.last_event_fingerprint.as_str())
                }
            || event.created_at < view.updated_at
        {
            return Err(CampaignError::EventChain);
        }
        apply_event(campaign, &mut view, event)?;
        view.last_sequence = event.sequence;
        view.last_event_fingerprint = event.fingerprint.clone();
        view.updated_at = event.created_at;
    }
    Ok(view)
}

fn apply_event(
    campaign: &ProductionCampaign,
    view: &mut CampaignView,
    event: &CampaignEvent,
) -> Result<(), CampaignError> {
    match &event.event {
        CampaignEventKind::Created => {
            if event.sequence != 1 || view.last_sequence != 0 {
                return Err(CampaignError::IllegalTransition);
            }
        }
        CampaignEventKind::GenerationBound { binding } => {
            binding.validate_integrity()?;
            if !matches!(
                view.state,
                CampaignState::AwaitingGeneration | CampaignState::RenewalRequired
            ) || view.current_iteration >= campaign.budget.maximum_iterations
            {
                return Err(CampaignError::IllegalTransition);
            }
            if let Some(previous) = &view.current_generation {
                if binding.predecessor_generation_id != Some(previous.generation_id)
                    || binding.predecessor_generation_fingerprint.as_deref()
                        != Some(previous.generation_fingerprint.as_str())
                {
                    return Err(CampaignError::SuccessorMismatch);
                }
            }
            view.current_generation = Some(binding.clone());
            view.protocol_id = None;
            view.protocol_fingerprint = None;
            view.run_id = None;
            view.selected_candidate_id = None;
            view.renewal_handoff_id = None;
            view.renewal_handoff_fingerprint = None;
            view.state = CampaignState::ReadyToPrepare;
        }
        CampaignEventKind::IterationPrepared {
            iteration,
            protocol_id,
            protocol_fingerprint,
            reserved_usage,
        } => {
            let expected_iteration = view
                .current_iteration
                .checked_add(1)
                .ok_or(CampaignError::UsageOverflow)?;
            let cumulative = view.reserved_usage.checked_add(*reserved_usage)?;
            cumulative.validate_within(&campaign.budget)?;
            if view.state != CampaignState::ReadyToPrepare
                || *iteration != expected_iteration
                || protocol_id.is_nil()
                || !canonical_fingerprint(protocol_fingerprint)
                || reserved_usage.iterations != 1
                || reserved_usage.sealed_evaluations != 1
            {
                return Err(CampaignError::IllegalTransition);
            }
            view.current_iteration = *iteration;
            view.protocol_id = Some(*protocol_id);
            view.protocol_fingerprint = Some(protocol_fingerprint.clone());
            view.reserved_usage = cumulative;
            view.state = CampaignState::ReadyToStart;
        }
        CampaignEventKind::RunStarted { iteration, run_id } => {
            if view.state != CampaignState::ReadyToStart
                || *iteration != view.current_iteration
                || run_id.is_nil()
            {
                return Err(CampaignError::IllegalTransition);
            }
            view.run_id = Some(*run_id);
            view.state = CampaignState::RunningDevelopment;
        }
        CampaignEventKind::DevelopmentCompleted {
            iteration,
            run_id,
            experiment_head_fingerprint,
            selected_candidate_id,
        } => {
            if view.state != CampaignState::RunningDevelopment
                || *iteration != view.current_iteration
                || Some(*run_id) != view.run_id
                || !canonical_fingerprint(experiment_head_fingerprint)
            {
                return Err(CampaignError::IllegalTransition);
            }
            view.selected_candidate_id = *selected_candidate_id;
            view.state = if selected_candidate_id.is_some() {
                CampaignState::AwaitingSealedAuthorization
            } else {
                CampaignState::AwaitingFinalization
            };
        }
        CampaignEventKind::SealedAuthorized {
            iteration,
            run_id,
            candidate_id,
            authorized_by,
        } => {
            if view.state != CampaignState::AwaitingSealedAuthorization
                || *iteration != view.current_iteration
                || Some(*run_id) != view.run_id
                || Some(*candidate_id) != view.selected_candidate_id
                || canonical_text(authorized_by.clone(), "sealed authorizer").is_err()
            {
                return Err(CampaignError::IllegalTransition);
            }
            view.state = CampaignState::SealedAuthorized;
        }
        CampaignEventKind::IterationFinalized {
            iteration,
            run_id,
            experiment_head_fingerprint,
            sealed_exposure,
            generation_terminal_event_id,
            generation_terminal_event_fingerprint,
            ..
        } => {
            let selected = view.selected_candidate_id.is_some();
            if !matches!(
                view.state,
                CampaignState::SealedAuthorized | CampaignState::AwaitingFinalization
            ) || *iteration != view.current_iteration
                || Some(*run_id) != view.run_id
                || !canonical_fingerprint(experiment_head_fingerprint)
                || selected != sealed_exposure.is_some()
                || sealed_exposure
                    .as_ref()
                    .is_some_and(|value| value.validate_integrity().is_err())
                || generation_terminal_event_id.is_nil()
                || !canonical_fingerprint(generation_terminal_event_fingerprint)
            {
                return Err(CampaignError::IllegalTransition);
            }
            view.state = CampaignState::RenewalRequired;
        }
        CampaignEventKind::RenewalHandoffCreated {
            handoff_id,
            handoff_fingerprint,
        } => {
            if view.state != CampaignState::RenewalRequired
                || view.renewal_handoff_id.is_some()
                || handoff_id.is_nil()
                || !canonical_fingerprint(handoff_fingerprint)
            {
                return Err(CampaignError::IllegalTransition);
            }
            view.renewal_handoff_id = Some(*handoff_id);
            view.renewal_handoff_fingerprint = Some(handoff_fingerprint.clone());
        }
        CampaignEventKind::Completed { reason } => {
            if view.state != CampaignState::RenewalRequired
                || canonical_text(reason.clone(), "completion reason").is_err()
            {
                return Err(CampaignError::IllegalTransition);
            }
            view.state = CampaignState::Completed;
        }
    }
    Ok(())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CampaignError {
    #[error("campaign budget is invalid")]
    InvalidBudget,
    #[error("campaign budget is exhausted")]
    BudgetExhausted,
    #[error("campaign usage arithmetic overflowed")]
    UsageOverflow,
    #[error("campaign benchmark binding is invalid")]
    InvalidBinding,
    #[error("benchmark generation is not active and unused")]
    GenerationIneligible,
    #[error("successor generation does not descend from the current generation")]
    SuccessorMismatch,
    #[error("experiment protocol does not match the bound benchmark generation")]
    ProtocolBindingMismatch,
    #[error("experiment run does not match the current campaign iteration")]
    ExperimentMismatch,
    #[error("production campaign is invalid")]
    InvalidCampaign,
    #[error("campaign event is invalid")]
    InvalidEvent,
    #[error("campaign journal is empty")]
    EmptyJournal,
    #[error("campaign event chain changed or forked")]
    EventChain,
    #[error("campaign lifecycle transition is not allowed")]
    IllegalTransition,
    #[error("benchmark governance rejected the campaign link: {0}")]
    Benchmark(String),
    #[error("campaign artifact integrity failed: {0}")]
    Integrity(String),
    #[error("campaign fingerprint failed: {0}")]
    Fingerprint(String),
    #[error("{0} must be non-empty and canonical")]
    InvalidText(&'static str),
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("production campaign persistence failed: {0}")]
pub struct CampaignStoreError(pub String);

pub trait CampaignStore: Send + Sync {
    fn create_campaign(
        &self,
        campaign: &ProductionCampaign,
        first_event: &CampaignEvent,
    ) -> BoxFuture<'_, Result<(), CampaignStoreError>>;

    fn get_campaign(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ProductionCampaign>, CampaignStoreError>>;

    fn append_campaign_event(
        &self,
        event: &CampaignEvent,
    ) -> BoxFuture<'_, Result<(), CampaignStoreError>>;

    fn list_campaign_events(
        &self,
        campaign_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<CampaignEvent>, CampaignStoreError>>;

    /// Atomically consumes or exhausts the active benchmark generation and
    /// appends the matching campaign finalization. This closes the recovery
    /// gap where sealed evidence could be consumed without a durable campaign
    /// decision link.
    fn append_iteration_finalization(
        &self,
        generation_terminal_event: &BenchmarkGenerationEvent,
        campaign_finalized_event: &CampaignEvent,
    ) -> BoxFuture<'_, Result<(), CampaignStoreError>>;
}

fn canonical_text(value: impl Into<String>, field: &'static str) -> Result<String, CampaignError> {
    let value = value.into();
    if value.is_empty() || value.trim() != value {
        return Err(CampaignError::InvalidText(field));
    }
    Ok(value)
}

fn canonical_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn fingerprint(value: &impl Serialize) -> Result<String, CampaignError> {
    artifact_core::fingerprint(value).map_err(|error| CampaignError::Fingerprint(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::TimeZone;
    use encoder_experiment_core::{
        domain::{
            BackendIdentity, EncoderTaskKind, EvidenceRole, ExternalArtifactIdentity,
            ExternalProjectSnapshot, ModelArtifactIdentity, OptimizationBudget, ParameterValue,
            TrainingCandidate,
        },
        journal::ExperimentRunState,
        metrics::{
            EvaluationReport, MetricContract, MetricDefinition, MetricDirection, MetricGate,
            MetricGateCondition,
        },
        protocol::DevelopmentSelectionRule,
    };
    use serde_json::json;
    use workflow_core::benchmark_generation::BENCHMARK_GENERATION_EVENT_SCHEMA_VERSION;

    use super::*;

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn time(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, second).unwrap()
    }

    fn project() -> ExternalProjectSnapshot {
        ExternalProjectSnapshot::create(
            "nomos successor",
            EncoderTaskKind::RetrievalRanking,
            "successor-revision",
            digest('a'),
            BackendIdentity::new("fake", "v1", digest('b')).unwrap(),
            vec![
                ExternalArtifactIdentity::new("train", EvidenceRole::Training, 1, digest('1'))
                    .unwrap(),
                ExternalArtifactIdentity::new("generic", EvidenceRole::Development, 1, digest('2'))
                    .unwrap(),
                ExternalArtifactIdentity::new("retired", EvidenceRole::Development, 1, digest('3'))
                    .unwrap(),
                ExternalArtifactIdentity::new(
                    "successor",
                    EvidenceRole::SealedAcceptance,
                    1,
                    digest('4'),
                )
                .unwrap(),
            ],
            ModelArtifactIdentity::new("baseline", "fake", 1, digest('5')).unwrap(),
            json!({"task":"ranking"}),
            time(0),
        )
        .unwrap()
    }

    fn contract() -> MetricContract {
        MetricContract::create(
            vec![MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap()],
            "mrr",
            vec![
                MetricGate::new(
                    "mrr",
                    EvidenceRole::Development,
                    MetricGateCondition::MinimumImprovement { value: 0.0 },
                )
                .unwrap(),
                MetricGate::new(
                    "mrr",
                    EvidenceRole::SealedAcceptance,
                    MetricGateCondition::MaximumRegression { value: 0.0 },
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn protocol(project: &ExternalProjectSnapshot) -> ExperimentProtocol {
        let contract = contract();
        let baseline_report = |suite_key: &str, suite_fingerprint: String, role| {
            EvaluationReport::create(
                project,
                project.baseline_model.clone(),
                role,
                suite_key,
                suite_fingerprint,
                &contract,
                BTreeMap::from([("mrr".into(), 0.8)]),
                100,
                time(1),
            )
            .unwrap()
        };
        let candidate = TrainingCandidate::create(
            project,
            1,
            60,
            BTreeMap::from([("weight".into(), ParameterValue::Number(0.05))]),
        )
        .unwrap();
        let development_reports = vec![
            baseline_report("generic", digest('6'), EvidenceRole::Development),
            baseline_report("retired", digest('7'), EvidenceRole::Development),
        ];
        let sealed_report = baseline_report(
            "successor_sealed",
            digest('8'),
            EvidenceRole::SealedAcceptance,
        );
        ExperimentProtocol::create_multi(
            project,
            contract,
            development_reports,
            sealed_report,
            OptimizationBudget {
                maximum_candidates: 1,
                maximum_training_seconds: 60,
                maximum_development_evaluations: 2,
                maximum_sealed_evaluations: 1,
            },
            60,
            "successor_sealed",
            vec![candidate],
            DevelopmentSelectionRule::MaximizeWorstSuiteThenMean,
            time(2),
        )
        .unwrap()
    }

    fn binding() -> CampaignBenchmarkBinding {
        let mut value = CampaignBenchmarkBinding {
            generation_id: Uuid::new_v4(),
            generation_fingerprint: digest('9'),
            predecessor_generation_id: None,
            predecessor_generation_fingerprint: None,
            development_suite_fingerprints: BTreeMap::from([
                ("generic".into(), digest('6')),
                ("retired".into(), digest('7')),
            ]),
            sealed_suite_key: "successor_sealed".into(),
            sealed_suite_id: Uuid::new_v4(),
            sealed_suite_fingerprint: digest('8'),
            fingerprint: String::new(),
        };
        value.fingerprint = value.reproduce_fingerprint().unwrap();
        value
    }

    fn campaign(project: &ExternalProjectSnapshot) -> ProductionCampaign {
        ProductionCampaign::create(
            "successor campaign",
            project.id,
            project.fingerprint.clone(),
            CampaignBudget {
                maximum_iterations: 2,
                maximum_candidates: 2,
                maximum_training_seconds: 120,
                maximum_development_evaluations: 4,
                maximum_sealed_evaluations: 2,
                maximum_backend_operations: 20,
            },
            time(2),
        )
        .unwrap()
    }

    fn experiment_view(
        protocol: &ExperimentProtocol,
        run_id: Uuid,
        state: ExperimentRunState,
        selected_candidate_id: Option<Uuid>,
        final_decision: Option<FinalDecision>,
        sequence: u32,
    ) -> ExperimentView {
        ExperimentView {
            run_id,
            protocol_id: protocol.id,
            state,
            candidates: BTreeMap::new(),
            selected_candidate_id,
            sealed_authorized_by: None,
            sealed_report: None,
            sealed_assessment: None,
            final_decision,
            failure_reason: None,
            last_sequence: sequence,
            last_event_fingerprint: digest('a'),
            updated_at: time(sequence),
        }
    }

    fn terminal_generation_event(
        binding: &CampaignBenchmarkBinding,
        protocol: &ExperimentProtocol,
        run_id: Uuid,
        exposure: &SealedAssessmentExposure,
    ) -> BenchmarkGenerationEvent {
        let mut event = BenchmarkGenerationEvent {
            schema_version: BENCHMARK_GENERATION_EVENT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            generation_id: binding.generation_id,
            generation_fingerprint: binding.generation_fingerprint.clone(),
            sequence: 4,
            previous_event_fingerprint: Some(digest('b')),
            event: BenchmarkGenerationEventKind::IterationConsumed {
                experiment_run_id: run_id,
                experiment_protocol_fingerprint: protocol.fingerprint.clone(),
                sealed_exposure_id: exposure.id,
                sealed_exposure_fingerprint: exposure.fingerprint.clone(),
                final_decision_fingerprint: digest('a'),
            },
            created_at: time(9),
            fingerprint: String::new(),
        };
        event.fingerprint = event.reproduce_fingerprint().unwrap();
        event
    }

    fn sealed_exposure(
        campaign: &ProductionCampaign,
        binding: &CampaignBenchmarkBinding,
        protocol: &ExperimentProtocol,
        run_id: Uuid,
        candidate_id: Uuid,
    ) -> SealedAssessmentExposure {
        let mut exposure = SealedAssessmentExposure {
            schema_version: SEALED_ASSESSMENT_EXPOSURE_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            campaign_id: campaign.id,
            campaign_fingerprint: campaign.fingerprint.clone(),
            generation_id: binding.generation_id,
            generation_fingerprint: binding.generation_fingerprint.clone(),
            iteration: 1,
            experiment_run_id: run_id,
            experiment_protocol_id: protocol.id,
            experiment_protocol_fingerprint: protocol.fingerprint.clone(),
            candidate_id,
            sealed_suite_id: binding.sealed_suite_id,
            sealed_suite_fingerprint: binding.sealed_suite_fingerprint.clone(),
            report_id: Uuid::new_v4(),
            report_fingerprint: digest('1'),
            assessment_id: Uuid::new_v4(),
            assessment_fingerprint: digest('2'),
            authorized_by: "operator".into(),
            created_at: time(9),
            fingerprint: String::new(),
        };
        exposure.fingerprint = exposure.reproduce_fingerprint().unwrap();
        exposure
    }

    #[test]
    fn campaign_stops_at_sealed_authorization_and_requires_generation_exhaustion() {
        let project = project();
        let protocol = protocol(&project);
        let campaign = campaign(&project);
        let binding = binding();
        let first = first_campaign_event(&campaign, time(2)).unwrap();
        let mut events = vec![first];
        let mut view = replay_campaign(&campaign, &events).unwrap();
        events.push(bind_generation_event(&campaign, &view, binding.clone(), time(3)).unwrap());
        view = replay_campaign(&campaign, &events).unwrap();
        events.push(prepare_iteration_event(&campaign, &view, &protocol, time(4)).unwrap());
        view = replay_campaign(&campaign, &events).unwrap();
        let run_id = Uuid::new_v4();
        let ready = experiment_view(&protocol, run_id, ExperimentRunState::Ready, None, None, 1);
        events.push(start_run_event(&campaign, &view, &ready, time(5)).unwrap());
        view = replay_campaign(&campaign, &events).unwrap();
        let candidate_id = protocol.candidates[0].id;
        let development = experiment_view(
            &protocol,
            run_id,
            ExperimentRunState::AwaitingSealedAuthorization,
            Some(candidate_id),
            None,
            5,
        );
        events.push(record_development_event(&campaign, &view, &development, time(6)).unwrap());
        view = replay_campaign(&campaign, &events).unwrap();
        assert_eq!(view.state, CampaignState::AwaitingSealedAuthorization);

        let authorized = experiment_view(
            &protocol,
            run_id,
            ExperimentRunState::SealedAuthorized,
            Some(candidate_id),
            None,
            6,
        );
        events.push(
            record_sealed_authorization_event(&campaign, &view, &authorized, "operator", time(7))
                .unwrap(),
        );
        view = replay_campaign(&campaign, &events).unwrap();
        let completed = experiment_view(
            &protocol,
            run_id,
            ExperimentRunState::Completed,
            Some(candidate_id),
            Some(FinalDecision::RetainBaseline),
            9,
        );
        let exposure = sealed_exposure(&campaign, &binding, &protocol, run_id, candidate_id);
        let terminal = terminal_generation_event(&binding, &protocol, run_id, &exposure);
        events.push(
            finalize_iteration_event(
                &campaign,
                &view,
                &completed,
                &terminal,
                Some(exposure),
                time(9),
            )
            .unwrap(),
        );
        view = replay_campaign(&campaign, &events).unwrap();
        assert_eq!(view.state, CampaignState::RenewalRequired);
        assert_eq!(view.current_iteration, 1);
        assert_eq!(view.reserved_usage.development_evaluations, 2);
    }

    #[test]
    fn aggregate_budget_accounts_for_every_candidate_suite_pair() {
        let project = project();
        let protocol = protocol(&project);
        let mut campaign = campaign(&project);
        campaign.budget.maximum_development_evaluations = 1;
        campaign.fingerprint = campaign.reproduce_fingerprint().unwrap();
        let first = first_campaign_event(&campaign, time(2)).unwrap();
        let mut events = vec![first];
        let view = replay_campaign(&campaign, &events).unwrap();
        events.push(bind_generation_event(&campaign, &view, binding(), time(3)).unwrap());
        let view = replay_campaign(&campaign, &events).unwrap();
        assert_eq!(
            prepare_iteration_event(&campaign, &view, &protocol, time(4)),
            Err(CampaignError::BudgetExhausted)
        );
    }
}
