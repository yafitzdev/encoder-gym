use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    EncoderExperimentError, canonical_sha256,
    domain::{EvidenceRole, ExternalProjectSnapshot, ModelArtifactIdentity},
    fingerprint,
    metrics::{
        CandidateAssessment, CandidateVerdict, EvaluationReport, select_development_candidate,
    },
    ports::TrainOutput,
    protocol::ExperimentProtocol,
    required,
};

pub const EXPERIMENT_EVENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidatePhase {
    Training,
    DevelopmentEvaluation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalDecision {
    PromoteCandidate,
    RetainBaseline,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExperimentEventKind {
    RunCreated,
    CandidateTrainingStarted {
        candidate_id: Uuid,
    },
    CandidateTrainingCompleted {
        candidate_id: Uuid,
        output: TrainOutput,
    },
    CandidateDevelopmentCompleted {
        candidate_id: Uuid,
        report: EvaluationReport,
        assessment: CandidateAssessment,
    },
    CandidateFailed {
        candidate_id: Uuid,
        phase: CandidatePhase,
        reason: String,
    },
    DevelopmentSelected {
        candidate_id: Option<Uuid>,
    },
    SealedAuthorized {
        candidate_id: Uuid,
        authorized_by: String,
    },
    SealedStarted {
        candidate_id: Uuid,
    },
    SealedCompleted {
        candidate_id: Uuid,
        report: EvaluationReport,
        assessment: CandidateAssessment,
    },
    Finalized {
        decision: FinalDecision,
    },
    RunFailed {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentEvent {
    pub schema_version: u32,
    pub id: Uuid,
    pub run_id: Uuid,
    pub protocol_id: Uuid,
    pub protocol_fingerprint: String,
    pub sequence: u32,
    pub previous_event_fingerprint: Option<String>,
    pub event: ExperimentEventKind,
    pub created_at: DateTime<Utc>,
    pub fingerprint: String,
}

impl ExperimentEvent {
    pub fn create(
        run_id: Uuid,
        protocol: &ExperimentProtocol,
        sequence: u32,
        previous_event_fingerprint: Option<String>,
        event: ExperimentEventKind,
        created_at: DateTime<Utc>,
    ) -> Result<Self, EncoderExperimentError> {
        if sequence == 0
            || sequence == 1 && previous_event_fingerprint.is_some()
            || sequence > 1
                && !previous_event_fingerprint
                    .as_deref()
                    .is_some_and(canonical_sha256)
        {
            return Err(EncoderExperimentError::Validation(
                "experiment event sequence or predecessor is invalid".into(),
            ));
        }
        let mut value = Self {
            schema_version: EXPERIMENT_EVENT_SCHEMA_VERSION,
            id: Uuid::new_v4(),
            run_id,
            protocol_id: protocol.id,
            protocol_fingerprint: protocol.fingerprint.clone(),
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

    pub fn validate_integrity(&self) -> Result<(), EncoderExperimentError> {
        self.validate_fields()?;
        if self.reproduce_fingerprint()? != self.fingerprint {
            return Err(EncoderExperimentError::Integrity(
                "experiment event fingerprint changed".into(),
            ));
        }
        Ok(())
    }

    pub fn reproduce_fingerprint(&self) -> Result<String, EncoderExperimentError> {
        fingerprint(&serde_json::json!({
            "schema_version": self.schema_version,
            "id": self.id,
            "run_id": self.run_id,
            "protocol_id": self.protocol_id,
            "protocol_fingerprint": self.protocol_fingerprint,
            "sequence": self.sequence,
            "previous_event_fingerprint": self.previous_event_fingerprint,
            "event": self.event,
            "created_at": self.created_at,
        }))
    }

    fn validate_fields(&self) -> Result<(), EncoderExperimentError> {
        if self.schema_version != EXPERIMENT_EVENT_SCHEMA_VERSION
            || self.sequence == 0
            || !canonical_sha256(&self.protocol_fingerprint)
            || self.sequence == 1 && self.previous_event_fingerprint.is_some()
            || self.sequence > 1
                && !self
                    .previous_event_fingerprint
                    .as_deref()
                    .is_some_and(canonical_sha256)
            || !self.fingerprint.is_empty() && !canonical_sha256(&self.fingerprint)
        {
            return Err(EncoderExperimentError::Validation(
                "experiment event fields are invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateExecutionState {
    Pending,
    Training,
    Trained,
    DevelopmentCompleted,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateExecution {
    pub state: CandidateExecutionState,
    pub train_output: Option<TrainOutput>,
    pub development_report: Option<EvaluationReport>,
    pub development_assessment: Option<CandidateAssessment>,
    pub failure_phase: Option<CandidatePhase>,
    pub failure_reason: Option<String>,
}

impl CandidateExecution {
    fn pending() -> Self {
        Self {
            state: CandidateExecutionState::Pending,
            train_output: None,
            development_report: None,
            development_assessment: None,
            failure_phase: None,
            failure_reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentRunState {
    Ready,
    Running,
    AwaitingSealedAuthorization,
    SealedAuthorized,
    SealedEvaluating,
    SealedEvaluated,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentView {
    pub run_id: Uuid,
    pub protocol_id: Uuid,
    pub state: ExperimentRunState,
    pub candidates: BTreeMap<Uuid, CandidateExecution>,
    pub selected_candidate_id: Option<Uuid>,
    pub sealed_authorized_by: Option<String>,
    pub sealed_report: Option<EvaluationReport>,
    pub sealed_assessment: Option<CandidateAssessment>,
    pub final_decision: Option<FinalDecision>,
    pub failure_reason: Option<String>,
    pub last_sequence: u32,
    pub last_event_fingerprint: String,
    pub updated_at: DateTime<Utc>,
}

impl ExperimentView {
    pub fn next_event(
        &self,
        protocol: &ExperimentProtocol,
        event: ExperimentEventKind,
        created_at: DateTime<Utc>,
    ) -> Result<ExperimentEvent, EncoderExperimentError> {
        ExperimentEvent::create(
            self.run_id,
            protocol,
            self.last_sequence.checked_add(1).ok_or_else(|| {
                EncoderExperimentError::Validation("experiment event sequence overflowed".into())
            })?,
            Some(self.last_event_fingerprint.clone()),
            event,
            created_at,
        )
    }

    pub fn selected_model(&self) -> Option<&ModelArtifactIdentity> {
        let id = self.selected_candidate_id?;
        self.candidates
            .get(&id)?
            .train_output
            .as_ref()
            .map(|output| &output.model)
    }
}

pub fn first_event(
    protocol: &ExperimentProtocol,
    run_id: Uuid,
    created_at: DateTime<Utc>,
) -> Result<ExperimentEvent, EncoderExperimentError> {
    ExperimentEvent::create(
        run_id,
        protocol,
        1,
        None,
        ExperimentEventKind::RunCreated,
        created_at,
    )
}

pub fn replay_experiment(
    project: &ExternalProjectSnapshot,
    protocol: &ExperimentProtocol,
    events: &[ExperimentEvent],
) -> Result<ExperimentView, EncoderExperimentError> {
    protocol.validate_integrity(project)?;
    let first = events
        .first()
        .ok_or_else(|| EncoderExperimentError::Validation("experiment journal is empty".into()))?;
    if first.sequence != 1 || !matches!(first.event, ExperimentEventKind::RunCreated) {
        return Err(EncoderExperimentError::Integrity(
            "experiment journal must start with run_created".into(),
        ));
    }
    let mut view = ExperimentView {
        run_id: first.run_id,
        protocol_id: protocol.id,
        state: ExperimentRunState::Ready,
        candidates: protocol
            .candidates
            .iter()
            .map(|candidate| (candidate.id, CandidateExecution::pending()))
            .collect(),
        selected_candidate_id: None,
        sealed_authorized_by: None,
        sealed_report: None,
        sealed_assessment: None,
        final_decision: None,
        failure_reason: None,
        last_sequence: 0,
        last_event_fingerprint: String::new(),
        updated_at: first.created_at,
    };

    for event in events {
        validate_chain_event(protocol, &view, event)?;
        apply_event(project, protocol, &mut view, event)?;
        view.last_sequence = event.sequence;
        view.last_event_fingerprint = event.fingerprint.clone();
        view.updated_at = event.created_at;
    }
    Ok(view)
}

fn validate_chain_event(
    protocol: &ExperimentProtocol,
    view: &ExperimentView,
    event: &ExperimentEvent,
) -> Result<(), EncoderExperimentError> {
    event.validate_integrity()?;
    if event.run_id != view.run_id
        || event.protocol_id != protocol.id
        || event.protocol_fingerprint != protocol.fingerprint
        || event.sequence != view.last_sequence + 1
        || event.previous_event_fingerprint.as_deref()
            != if view.last_sequence == 0 {
                None
            } else {
                Some(view.last_event_fingerprint.as_str())
            }
        || event.created_at < view.updated_at
    {
        return Err(EncoderExperimentError::Integrity(
            "experiment event chain changed or forked".into(),
        ));
    }
    Ok(())
}

fn apply_event(
    project: &ExternalProjectSnapshot,
    protocol: &ExperimentProtocol,
    view: &mut ExperimentView,
    event: &ExperimentEvent,
) -> Result<(), EncoderExperimentError> {
    if matches!(
        view.state,
        ExperimentRunState::Completed | ExperimentRunState::Failed
    ) {
        return Err(EncoderExperimentError::Validation(
            "terminal experiment journals cannot be extended".into(),
        ));
    }
    match &event.event {
        ExperimentEventKind::RunCreated => {
            if event.sequence != 1 {
                return Err(EncoderExperimentError::Integrity(
                    "run_created may appear only once".into(),
                ));
            }
        }
        ExperimentEventKind::CandidateTrainingStarted { candidate_id } => {
            if !matches!(
                view.state,
                ExperimentRunState::Ready | ExperimentRunState::Running
            ) || view.candidates.values().any(|candidate| {
                matches!(
                    candidate.state,
                    CandidateExecutionState::Training | CandidateExecutionState::Trained
                )
            }) {
                return illegal_event("candidate training cannot start in the current state");
            }
            let execution = candidate_execution_mut(view, *candidate_id)?;
            if execution.state != CandidateExecutionState::Pending {
                return illegal_event("candidate training cannot start twice");
            }
            execution.state = CandidateExecutionState::Training;
            view.state = ExperimentRunState::Running;
        }
        ExperimentEventKind::CandidateTrainingCompleted {
            candidate_id,
            output,
        } => {
            output.model.validate()?;
            let candidate = protocol.candidate(*candidate_id).ok_or_else(|| {
                EncoderExperimentError::Validation("unknown experiment candidate".into())
            })?;
            if output.duration_seconds > candidate.maximum_training_seconds {
                return Err(EncoderExperimentError::BudgetExhausted(
                    "candidate exceeded its training-time authority".into(),
                ));
            }
            let execution = candidate_execution_mut(view, *candidate_id)?;
            if execution.state != CandidateExecutionState::Training {
                return illegal_event("candidate training completion has no reservation");
            }
            execution.state = CandidateExecutionState::Trained;
            execution.train_output = Some(output.clone());
        }
        ExperimentEventKind::CandidateDevelopmentCompleted {
            candidate_id,
            report,
            assessment,
        } => {
            let execution = candidate_execution_mut(view, *candidate_id)?;
            if execution.state != CandidateExecutionState::Trained {
                return illegal_event("candidate development report has no trained model");
            }
            let output = execution
                .train_output
                .as_ref()
                .expect("trained output exists");
            report.validate_integrity(project, &protocol.metric_contract)?;
            assessment.validate_integrity(
                project,
                &protocol.metric_contract,
                &protocol.baseline_development_report,
                report,
            )?;
            if report.evidence_role != EvidenceRole::Development
                || report.suite_key != protocol.development_suite_key
                || report.model != output.model
            {
                return illegal_event("candidate development evidence does not match the run");
            }
            execution.state = CandidateExecutionState::DevelopmentCompleted;
            execution.development_report = Some(report.clone());
            execution.development_assessment = Some(assessment.clone());
        }
        ExperimentEventKind::CandidateFailed {
            candidate_id,
            phase,
            reason,
        } => {
            let reason = bounded_reason(reason)?;
            let execution = candidate_execution_mut(view, *candidate_id)?;
            let valid_phase = matches!(
                (execution.state, phase),
                (CandidateExecutionState::Training, CandidatePhase::Training)
                    | (
                        CandidateExecutionState::Trained,
                        CandidatePhase::DevelopmentEvaluation
                    )
            );
            if !valid_phase {
                return illegal_event("candidate failure does not match its active phase");
            }
            execution.state = CandidateExecutionState::Failed;
            execution.failure_phase = Some(*phase);
            execution.failure_reason = Some(reason);
        }
        ExperimentEventKind::DevelopmentSelected { candidate_id } => {
            if view.state != ExperimentRunState::Running
                || view.candidates.values().any(|candidate| {
                    !matches!(
                        candidate.state,
                        CandidateExecutionState::DevelopmentCompleted
                            | CandidateExecutionState::Failed
                    )
                })
            {
                return illegal_event("development selection requires every candidate to finish");
            }
            let assessments = view
                .candidates
                .values()
                .filter_map(|candidate| candidate.development_assessment.clone())
                .collect::<Vec<_>>();
            let expected_assessment = select_development_candidate(&assessments)?;
            let expected_candidate = expected_assessment.and_then(|assessment| {
                view.candidates.iter().find_map(|(id, execution)| {
                    (execution
                        .development_assessment
                        .as_ref()
                        .is_some_and(|value| value.id == assessment.id))
                    .then_some(*id)
                })
            });
            if *candidate_id != expected_candidate {
                return illegal_event(
                    "development selection does not match deterministic passing evidence",
                );
            }
            view.selected_candidate_id = *candidate_id;
            view.state = if candidate_id.is_some() {
                ExperimentRunState::AwaitingSealedAuthorization
            } else {
                ExperimentRunState::Running
            };
        }
        ExperimentEventKind::SealedAuthorized {
            candidate_id,
            authorized_by,
        } => {
            if view.state != ExperimentRunState::AwaitingSealedAuthorization
                || view.selected_candidate_id != Some(*candidate_id)
            {
                return illegal_event("sealed authorization does not match the selected candidate");
            }
            view.sealed_authorized_by = Some(required(authorized_by, "sealed authorizer")?);
            view.state = ExperimentRunState::SealedAuthorized;
        }
        ExperimentEventKind::SealedStarted { candidate_id } => {
            if view.state != ExperimentRunState::SealedAuthorized
                || view.selected_candidate_id != Some(*candidate_id)
            {
                return illegal_event("sealed evaluation start was not explicitly authorized");
            }
            view.state = ExperimentRunState::SealedEvaluating;
        }
        ExperimentEventKind::SealedCompleted {
            candidate_id,
            report,
            assessment,
        } => {
            if view.state != ExperimentRunState::SealedEvaluating
                || view.selected_candidate_id != Some(*candidate_id)
            {
                return illegal_event("sealed evidence was not explicitly authorized");
            }
            let execution = view
                .candidates
                .get(candidate_id)
                .expect("selected candidate exists");
            let model = &execution
                .train_output
                .as_ref()
                .expect("selected candidate was trained")
                .model;
            report.validate_integrity(project, &protocol.metric_contract)?;
            assessment.validate_integrity(
                project,
                &protocol.metric_contract,
                &protocol.baseline_sealed_report,
                report,
            )?;
            if report.evidence_role != EvidenceRole::SealedAcceptance
                || report.suite_key != protocol.sealed_suite_key
                || &report.model != model
            {
                return illegal_event("sealed evidence does not match the selected candidate");
            }
            view.sealed_report = Some(report.clone());
            view.sealed_assessment = Some(assessment.clone());
            view.state = ExperimentRunState::SealedEvaluated;
        }
        ExperimentEventKind::Finalized { decision } => {
            let expected = if view.selected_candidate_id.is_none() {
                if view.candidates.values().any(|candidate| {
                    matches!(
                        candidate.state,
                        CandidateExecutionState::Pending
                            | CandidateExecutionState::Training
                            | CandidateExecutionState::Trained
                    )
                }) {
                    return illegal_event("baseline cannot be retained before candidates finish");
                }
                FinalDecision::RetainBaseline
            } else if view.state == ExperimentRunState::SealedEvaluated {
                if view
                    .sealed_assessment
                    .as_ref()
                    .is_some_and(|value| value.verdict == CandidateVerdict::Passed)
                {
                    FinalDecision::PromoteCandidate
                } else {
                    FinalDecision::RetainBaseline
                }
            } else {
                return illegal_event(
                    "selected candidate requires sealed evidence before finalization",
                );
            };
            if *decision != expected {
                return illegal_event("final decision contradicts deterministic acceptance");
            }
            view.final_decision = Some(*decision);
            view.state = ExperimentRunState::Completed;
        }
        ExperimentEventKind::RunFailed { reason } => {
            view.failure_reason = Some(bounded_reason(reason)?);
            view.state = ExperimentRunState::Failed;
        }
    }
    Ok(())
}

fn candidate_execution_mut(
    view: &mut ExperimentView,
    id: Uuid,
) -> Result<&mut CandidateExecution, EncoderExperimentError> {
    view.candidates.get_mut(&id).ok_or_else(|| {
        EncoderExperimentError::Validation("event references an unknown candidate".into())
    })
}

fn bounded_reason(reason: &str) -> Result<String, EncoderExperimentError> {
    if reason.trim() != reason || reason.is_empty() || reason.chars().count() > 1_000 {
        return Err(EncoderExperimentError::Validation(
            "experiment failure reason must contain 1 to 1000 canonical characters".into(),
        ));
    }
    Ok(reason.to_owned())
}

fn illegal_event<T>(message: &str) -> Result<T, EncoderExperimentError> {
    Err(EncoderExperimentError::Validation(message.into()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::{
        domain::{
            BackendIdentity, EncoderTaskKind, ExternalArtifactIdentity, OptimizationBudget,
            ParameterValue, TrainingCandidate,
        },
        metrics::{
            MetricContract, MetricDefinition, MetricDirection, MetricGate, MetricGateCondition,
            assess_candidate,
        },
        protocol::ExperimentProtocol,
    };

    fn digest(character: char) -> String {
        format!("sha256:{}", character.to_string().repeat(64))
    }

    fn time(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, second).unwrap()
    }

    fn project() -> ExternalProjectSnapshot {
        ExternalProjectSnapshot::create(
            "nomos",
            EncoderTaskKind::RetrievalRanking,
            "experiment-revision",
            digest('a'),
            BackendIdentity::new("nomos", "nomos-ranking-v1", digest('b')).unwrap(),
            vec![
                ExternalArtifactIdentity::new("train", EvidenceRole::Training, 1, digest('c'))
                    .unwrap(),
                ExternalArtifactIdentity::new(
                    "development",
                    EvidenceRole::Development,
                    1,
                    digest('d'),
                )
                .unwrap(),
                ExternalArtifactIdentity::new(
                    "sealed",
                    EvidenceRole::SealedAcceptance,
                    1,
                    digest('e'),
                )
                .unwrap(),
            ],
            ModelArtifactIdentity::new("baseline", "sentence-transformers", 1, digest('f'))
                .unwrap(),
            json!({"adapter_protocol":"nomos-ranking-v1"}),
            time(0),
        )
        .unwrap()
    }

    fn contract() -> MetricContract {
        MetricContract::create(
            vec![
                MetricDefinition::new("mrr", MetricDirection::HigherIsBetter).unwrap(),
                MetricDefinition::new("recall_at_3", MetricDirection::HigherIsBetter).unwrap(),
            ],
            "mrr",
            vec![
                MetricGate::new(
                    "mrr",
                    EvidenceRole::Development,
                    MetricGateCondition::MinimumImprovement { value: 0.001 },
                )
                .unwrap(),
                MetricGate::new(
                    "recall_at_3",
                    EvidenceRole::Development,
                    MetricGateCondition::MaximumRegression { value: 0.0 },
                )
                .unwrap(),
                MetricGate::new(
                    "mrr",
                    EvidenceRole::SealedAcceptance,
                    MetricGateCondition::MinimumImprovement { value: 0.001 },
                )
                .unwrap(),
                MetricGate::new(
                    "recall_at_3",
                    EvidenceRole::SealedAcceptance,
                    MetricGateCondition::MaximumRegression { value: 0.0 },
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    fn report(
        project: &ExternalProjectSnapshot,
        contract: &MetricContract,
        model: ModelArtifactIdentity,
        role: EvidenceRole,
        mrr: f64,
        recall: f64,
        second: u32,
    ) -> EvaluationReport {
        EvaluationReport::create(
            project,
            model,
            role,
            if role == EvidenceRole::Development {
                "development"
            } else {
                "sealed"
            },
            if role == EvidenceRole::Development {
                digest('1')
            } else {
                digest('2')
            },
            contract,
            BTreeMap::from([("mrr".into(), mrr), ("recall_at_3".into(), recall)]),
            100,
            time(second),
        )
        .unwrap()
    }

    fn protocol(project: &ExternalProjectSnapshot) -> ExperimentProtocol {
        let contract = contract();
        let candidates = (1..=2)
            .map(|sequence| {
                TrainingCandidate::create(
                    project,
                    sequence,
                    60,
                    BTreeMap::from([
                        ("loss".into(), ParameterValue::Text("triplet".into())),
                        (
                            "learning_rate".into(),
                            ParameterValue::Number(0.000_003 * f64::from(sequence)),
                        ),
                    ]),
                )
                .unwrap()
            })
            .collect();
        ExperimentProtocol::create(
            project,
            contract.clone(),
            report(
                project,
                &contract,
                project.baseline_model.clone(),
                EvidenceRole::Development,
                0.8,
                0.9,
                1,
            ),
            report(
                project,
                &contract,
                project.baseline_model.clone(),
                EvidenceRole::SealedAcceptance,
                0.79,
                0.89,
                2,
            ),
            OptimizationBudget {
                maximum_candidates: 2,
                maximum_training_seconds: 120,
                maximum_development_evaluations: 2,
                maximum_sealed_evaluations: 1,
            },
            60,
            "development",
            "sealed",
            candidates,
            time(3),
        )
        .unwrap()
    }

    fn append(
        project: &ExternalProjectSnapshot,
        protocol: &ExperimentProtocol,
        events: &mut Vec<ExperimentEvent>,
        event: ExperimentEventKind,
        second: u32,
    ) {
        let view = replay_experiment(project, protocol, events).unwrap();
        events.push(view.next_event(protocol, event, time(second)).unwrap());
        replay_experiment(project, protocol, events).unwrap();
    }

    #[test]
    fn journal_requires_development_selection_and_explicit_sealed_authorization() {
        let project = project();
        let protocol = protocol(&project);
        let mut events = vec![first_event(&protocol, Uuid::new_v4(), time(4)).unwrap()];
        let mut candidate_outputs = BTreeMap::new();

        for (offset, candidate) in protocol.candidates.iter().enumerate() {
            let start = 5 + u32::try_from(offset).unwrap() * 4;
            append(
                &project,
                &protocol,
                &mut events,
                ExperimentEventKind::CandidateTrainingStarted {
                    candidate_id: candidate.id,
                },
                start,
            );
            let model = ModelArtifactIdentity::new(
                format!("candidate-{}", candidate.sequence),
                "sentence-transformers",
                10,
                if candidate.sequence == 1 {
                    digest('7')
                } else {
                    digest('8')
                },
            )
            .unwrap();
            let output = TrainOutput {
                model: model.clone(),
                duration_seconds: 10,
                metadata: json!({"candidate":candidate.sequence}),
            };
            candidate_outputs.insert(candidate.id, output.clone());
            append(
                &project,
                &protocol,
                &mut events,
                ExperimentEventKind::CandidateTrainingCompleted {
                    candidate_id: candidate.id,
                    output,
                },
                start + 1,
            );
            let candidate_report = report(
                &project,
                &protocol.metric_contract,
                model,
                EvidenceRole::Development,
                if candidate.sequence == 1 { 0.82 } else { 0.81 },
                0.9,
                start + 2,
            );
            let assessment = assess_candidate(
                &project,
                &protocol.metric_contract,
                &protocol.baseline_development_report,
                &candidate_report,
                time(start + 3),
            )
            .unwrap();
            append(
                &project,
                &protocol,
                &mut events,
                ExperimentEventKind::CandidateDevelopmentCompleted {
                    candidate_id: candidate.id,
                    report: candidate_report,
                    assessment,
                },
                start + 3,
            );
        }

        let selected = protocol.candidates[0].id;
        append(
            &project,
            &protocol,
            &mut events,
            ExperimentEventKind::DevelopmentSelected {
                candidate_id: Some(selected),
            },
            13,
        );

        let selected_model = candidate_outputs[&selected].model.clone();
        let sealed_report = report(
            &project,
            &protocol.metric_contract,
            selected_model,
            EvidenceRole::SealedAcceptance,
            0.8,
            0.89,
            14,
        );
        let sealed_assessment = assess_candidate(
            &project,
            &protocol.metric_contract,
            &protocol.baseline_sealed_report,
            &sealed_report,
            time(15),
        )
        .unwrap();
        let view = replay_experiment(&project, &protocol, &events).unwrap();
        let unauthorized = view
            .next_event(
                &protocol,
                ExperimentEventKind::SealedCompleted {
                    candidate_id: selected,
                    report: sealed_report.clone(),
                    assessment: sealed_assessment.clone(),
                },
                time(16),
            )
            .unwrap();
        let mut invalid = events.clone();
        invalid.push(unauthorized);
        assert!(replay_experiment(&project, &protocol, &invalid).is_err());

        append(
            &project,
            &protocol,
            &mut events,
            ExperimentEventKind::SealedAuthorized {
                candidate_id: selected,
                authorized_by: "local-operator".into(),
            },
            16,
        );
        append(
            &project,
            &protocol,
            &mut events,
            ExperimentEventKind::SealedStarted {
                candidate_id: selected,
            },
            17,
        );
        append(
            &project,
            &protocol,
            &mut events,
            ExperimentEventKind::SealedCompleted {
                candidate_id: selected,
                report: sealed_report,
                assessment: sealed_assessment,
            },
            18,
        );
        append(
            &project,
            &protocol,
            &mut events,
            ExperimentEventKind::Finalized {
                decision: FinalDecision::PromoteCandidate,
            },
            19,
        );

        let final_view = replay_experiment(&project, &protocol, &events).unwrap();
        assert_eq!(final_view.state, ExperimentRunState::Completed);
        assert_eq!(
            final_view.final_decision,
            Some(FinalDecision::PromoteCandidate)
        );
        assert_eq!(final_view.selected_candidate_id, Some(selected));
    }

    #[test]
    fn event_chain_rejects_a_rehashed_fork() {
        let project = project();
        let protocol = protocol(&project);
        let mut events = vec![first_event(&protocol, Uuid::new_v4(), time(4)).unwrap()];
        let view = replay_experiment(&project, &protocol, &events).unwrap();
        let mut event = view
            .next_event(
                &protocol,
                ExperimentEventKind::CandidateTrainingStarted {
                    candidate_id: protocol.candidates[0].id,
                },
                time(5),
            )
            .unwrap();
        event.previous_event_fingerprint = Some(digest('9'));
        event.fingerprint = event.reproduce_fingerprint().unwrap();
        events.push(event);

        assert!(replay_experiment(&project, &protocol, &events).is_err());
    }
}
