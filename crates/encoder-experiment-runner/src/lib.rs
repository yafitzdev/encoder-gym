//! Durable orchestration for finite production encoder experiments.

use chrono::Utc;
use encoder_experiment_core::{
    EncoderExperimentError,
    benchmark::BenchmarkDefinition,
    domain::{ExternalProjectSnapshot, OptimizationBudget, TrainingCandidate},
    journal::{
        CandidateExecutionState, CandidatePhase, ExperimentEvent, ExperimentEventKind,
        ExperimentRunState, ExperimentView, FinalDecision, first_event, replay_experiment,
    },
    metrics::{
        CandidateVerdict, DevelopmentCandidateEvidence, MetricContract, assess_candidate,
        select_multi_development_candidate,
    },
    ports::{EncoderTaskAdapterError, EncoderTaskBackend, ExperimentStore, ExperimentStoreError},
    protocol::{DevelopmentSelectionRule, ExperimentProtocol},
};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ExperimentRunnerError {
    #[error(transparent)]
    Domain(#[from] EncoderExperimentError),
    #[error(transparent)]
    Adapter(#[from] EncoderTaskAdapterError),
    #[error(transparent)]
    Store(#[from] ExperimentStoreError),
    #[error("encoder experiment resource does not exist: {0}")]
    NotFound(String),
}

pub struct ExperimentRunner<'a, S, B> {
    store: &'a S,
    backend: &'a B,
}

impl<'a, S, B> ExperimentRunner<'a, S, B>
where
    S: ExperimentStore,
    B: EncoderTaskBackend,
{
    pub const fn new(store: &'a S, backend: &'a B) -> Self {
        Self { store, backend }
    }

    pub async fn register_project(
        &self,
        proposed: ExternalProjectSnapshot,
    ) -> Result<ExternalProjectSnapshot, ExperimentRunnerError> {
        proposed.validate_integrity()?;
        if proposed.backend != self.backend.identity() {
            return Err(EncoderExperimentError::Validation(
                "project snapshot pins a different task adapter".into(),
            )
            .into());
        }
        self.backend.inspect(proposed.clone()).await?;
        if let Some(existing) = self
            .store
            .find_project_by_source_fingerprint(proposed.source_fingerprint.clone())
            .await?
        {
            self.backend.inspect(existing.clone()).await?;
            return Ok(existing);
        }
        self.store.create_project(proposed.clone()).await?;
        Ok(proposed)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_protocol(
        &self,
        project_id: Uuid,
        metric_contract: MetricContract,
        budget: OptimizationBudget,
        maximum_evaluation_seconds: u64,
        development_suite_key: impl Into<String>,
        sealed_suite_key: impl Into<String>,
        candidates: Vec<TrainingCandidate>,
    ) -> Result<ExperimentProtocol, ExperimentRunnerError> {
        let project = self.project(project_id).await?;
        self.backend.inspect(project.clone()).await?;
        let development_suite_key = development_suite_key.into();
        let sealed_suite_key = sealed_suite_key.into();
        let baseline_development_report = self
            .backend
            .evaluate(
                project.clone(),
                project.baseline_model.clone(),
                metric_contract.clone(),
                development_suite_key.clone(),
                maximum_evaluation_seconds,
            )
            .await?;
        // This is a frozen baseline reference, created before any adaptive candidate work. It is
        // never passed to development selection. The finite sealed budget remains reserved for the
        // one selected candidate report.
        let baseline_sealed_report = self
            .backend
            .evaluate(
                project.clone(),
                project.baseline_model.clone(),
                metric_contract.clone(),
                sealed_suite_key.clone(),
                maximum_evaluation_seconds,
            )
            .await?;
        let protocol = ExperimentProtocol::create(
            &project,
            metric_contract,
            baseline_development_report,
            baseline_sealed_report,
            budget,
            maximum_evaluation_seconds,
            development_suite_key,
            sealed_suite_key,
            candidates,
            Utc::now(),
        )?;
        self.store.create_protocol(protocol.clone()).await?;
        Ok(protocol)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_multi_protocol(
        &self,
        project_id: Uuid,
        metric_contract: MetricContract,
        budget: OptimizationBudget,
        maximum_evaluation_seconds: u64,
        development_suite_keys: Vec<String>,
        sealed_suite_key: impl Into<String>,
        candidates: Vec<TrainingCandidate>,
    ) -> Result<ExperimentProtocol, ExperimentRunnerError> {
        self.prepare_multi_protocol_identified(
            Uuid::new_v4(),
            project_id,
            metric_contract,
            budget,
            maximum_evaluation_seconds,
            development_suite_keys,
            sealed_suite_key,
            candidates,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_multi_protocol_identified(
        &self,
        protocol_id: Uuid,
        project_id: Uuid,
        metric_contract: MetricContract,
        budget: OptimizationBudget,
        maximum_evaluation_seconds: u64,
        mut development_suite_keys: Vec<String>,
        sealed_suite_key: impl Into<String>,
        candidates: Vec<TrainingCandidate>,
    ) -> Result<ExperimentProtocol, ExperimentRunnerError> {
        let project = self.project(project_id).await?;
        development_suite_keys.sort();
        if development_suite_keys.is_empty()
            || development_suite_keys
                .windows(2)
                .any(|pair| pair[0] == pair[1])
        {
            return Err(EncoderExperimentError::Validation(
                "multi-suite protocol requires unique development suite keys".into(),
            )
            .into());
        }
        let sealed_suite_key = sealed_suite_key.into();
        if let Some(existing) = self.store.get_protocol(protocol_id).await? {
            existing.validate_integrity(&project)?;
            if existing.project_snapshot_id != project_id
                || existing.metric_contract != metric_contract
                || existing.budget != budget
                || existing.maximum_evaluation_seconds != maximum_evaluation_seconds
                || existing.development_suite_keys() != development_suite_keys
                || existing.sealed_suite_key != sealed_suite_key
                || existing.candidates != candidates
                || existing.development_selection_rule
                    != Some(DevelopmentSelectionRule::MaximizeWorstSuiteThenMean)
            {
                return Err(EncoderExperimentError::Integrity(
                    "reserved experiment protocol belongs to a different request".into(),
                )
                .into());
            }
            return Ok(existing);
        }
        self.backend.inspect(project.clone()).await?;
        let mut baseline_development_reports = Vec::with_capacity(development_suite_keys.len());
        for suite_key in development_suite_keys {
            baseline_development_reports.push(
                self.backend
                    .evaluate(
                        project.clone(),
                        project.baseline_model.clone(),
                        metric_contract.clone(),
                        suite_key,
                        maximum_evaluation_seconds,
                    )
                    .await?,
            );
        }
        // This baseline reference is frozen before adaptive candidate work and
        // cannot participate in development selection.
        let baseline_sealed_report = self
            .backend
            .evaluate(
                project.clone(),
                project.baseline_model.clone(),
                metric_contract.clone(),
                sealed_suite_key.clone(),
                maximum_evaluation_seconds,
            )
            .await?;
        let protocol = ExperimentProtocol::create_multi_identified(
            protocol_id,
            &project,
            metric_contract,
            baseline_development_reports,
            baseline_sealed_report,
            budget,
            maximum_evaluation_seconds,
            sealed_suite_key,
            candidates,
            DevelopmentSelectionRule::MaximizeWorstSuiteThenMean,
            Utc::now(),
        )?;
        self.store.create_protocol(protocol.clone()).await?;
        Ok(protocol)
    }

    /// Prepare a new candidate protocol from an already-recorded shared benchmark.
    /// Baseline reports retain exact source provenance and are not evaluated again.
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_multi_protocol_from_benchmark_identified(
        &self,
        protocol_id: Uuid,
        project_id: Uuid,
        source_protocol_id: Uuid,
        benchmark: BenchmarkDefinition,
        budget: OptimizationBudget,
        maximum_evaluation_seconds: u64,
        candidates: Vec<TrainingCandidate>,
    ) -> Result<ExperimentProtocol, ExperimentRunnerError> {
        let project = self.project(project_id).await?;
        self.backend.inspect(project.clone()).await?;
        let source_protocol = self.protocol(source_protocol_id).await?;
        let source_project = self.project(source_protocol.project_snapshot_id).await?;
        source_protocol.validate_integrity(&source_project)?;
        benchmark.validate_integrity()?;
        if BenchmarkDefinition::from_protocol(
            &source_project,
            &source_protocol,
            benchmark.evaluation_configuration_fingerprint.clone(),
        )? != benchmark
        {
            return Err(EncoderExperimentError::Integrity(
                "shared benchmark differs from its source protocol".into(),
            )
            .into());
        }

        if let Some(existing) = self.store.get_protocol(protocol_id).await? {
            existing.validate_integrity(&project)?;
            let source_development = source_protocol.baseline_development_reports();
            let referenced_development = existing.baseline_development_reports();
            if existing.project_snapshot_id != project_id
                || existing.metric_contract != benchmark.metric_contract
                || existing.budget != budget
                || existing.maximum_evaluation_seconds != maximum_evaluation_seconds
                || existing.development_suite_keys() != source_protocol.development_suite_keys()
                || existing.sealed_suite_key != source_protocol.sealed_suite_key
                || existing.candidates != candidates
                || existing.development_selection_rule
                    != Some(DevelopmentSelectionRule::MaximizeWorstSuiteThenMean)
                || source_development.len() != referenced_development.len()
            {
                return Err(EncoderExperimentError::Integrity(
                    "reserved shared-benchmark protocol belongs to a different request".into(),
                )
                .into());
            }
            for (source, referenced) in source_development.into_iter().zip(referenced_development) {
                benchmark.validate_baseline_reference(
                    &source_project,
                    &source_protocol,
                    &project,
                    source,
                    referenced,
                )?;
            }
            benchmark.validate_baseline_reference(
                &source_project,
                &source_protocol,
                &project,
                &source_protocol.baseline_sealed_report,
                &existing.baseline_sealed_report,
            )?;
            return Ok(existing);
        }

        let now = Utc::now();
        let mut baseline_development_reports = Vec::new();
        for source in source_protocol.baseline_development_reports() {
            baseline_development_reports.push(benchmark.reference_baseline_report(
                Uuid::new_v4(),
                &source_project,
                &source_protocol,
                &project,
                source,
                now,
            )?);
        }
        let baseline_sealed_report = benchmark.reference_baseline_report(
            Uuid::new_v4(),
            &source_project,
            &source_protocol,
            &project,
            &source_protocol.baseline_sealed_report,
            now,
        )?;
        let protocol = ExperimentProtocol::create_multi_identified(
            protocol_id,
            &project,
            benchmark.metric_contract,
            baseline_development_reports,
            baseline_sealed_report,
            budget,
            maximum_evaluation_seconds,
            source_protocol.sealed_suite_key,
            candidates,
            DevelopmentSelectionRule::MaximizeWorstSuiteThenMean,
            now,
        )?;
        self.store.create_protocol(protocol.clone()).await?;
        Ok(protocol)
    }

    pub async fn create_run(
        &self,
        protocol_id: Uuid,
    ) -> Result<ExperimentView, ExperimentRunnerError> {
        self.create_run_identified(protocol_id, Uuid::new_v4())
            .await
    }

    pub async fn create_run_identified(
        &self,
        protocol_id: Uuid,
        run_id: Uuid,
    ) -> Result<ExperimentView, ExperimentRunnerError> {
        let protocol = self.protocol(protocol_id).await?;
        let project = self.project(protocol.project_snapshot_id).await?;
        protocol.validate_integrity(&project)?;
        let existing = self.store.load_events(run_id).await?;
        if !existing.is_empty() {
            if existing[0].protocol_id != protocol_id {
                return Err(EncoderExperimentError::Integrity(
                    "reserved experiment run belongs to another protocol".into(),
                )
                .into());
            }
            return Ok(replay_experiment(&project, &protocol, &existing)?);
        }
        let first = first_event(&protocol, run_id, Utc::now())?;
        self.store.create_run(first).await?;
        self.status(run_id).await
    }

    pub async fn status(&self, run_id: Uuid) -> Result<ExperimentView, ExperimentRunnerError> {
        let (project, protocol, events) = self.context(run_id).await?;
        Ok(replay_experiment(&project, &protocol, &events)?)
    }

    pub async fn run_development(
        &self,
        run_id: Uuid,
    ) -> Result<ExperimentView, ExperimentRunnerError> {
        let (_, protocol, _) = self.context(run_id).await?;
        let maximum_steps = protocol
            .candidates
            .len()
            .checked_mul(
                protocol
                    .development_suite_keys()
                    .len()
                    .checked_add(2)
                    .ok_or_else(|| {
                        EncoderExperimentError::Validation(
                            "candidate step budget overflowed".into(),
                        )
                    })?,
            )
            .and_then(|value| value.checked_add(3))
            .ok_or_else(|| {
                EncoderExperimentError::Validation("candidate step budget overflowed".into())
            })?;
        for _ in 0..maximum_steps {
            let (project, protocol, events) = self.context(run_id).await?;
            let view = replay_experiment(&project, &protocol, &events)?;
            if !matches!(
                view.state,
                ExperimentRunState::Ready | ExperimentRunState::Running
            ) {
                return Ok(view);
            }

            if let Some((candidate_id, _)) = view
                .candidates
                .iter()
                .find(|(_, execution)| execution.state == CandidateExecutionState::Training)
            {
                let candidate = protocol
                    .candidate(*candidate_id)
                    .expect("journal candidate belongs to protocol")
                    .clone();
                match self.backend.train(project.clone(), candidate).await {
                    Ok(output) => {
                        self.append(
                            &protocol,
                            &view,
                            ExperimentEventKind::CandidateTrainingCompleted {
                                candidate_id: *candidate_id,
                                output,
                            },
                        )
                        .await?;
                    }
                    Err(error) => {
                        self.append(
                            &protocol,
                            &view,
                            ExperimentEventKind::CandidateFailed {
                                candidate_id: *candidate_id,
                                phase: CandidatePhase::Training,
                                reason: bounded_failure(&error),
                            },
                        )
                        .await?;
                    }
                }
                continue;
            }

            if let Some((candidate_id, execution)) = view
                .candidates
                .iter()
                .find(|(_, execution)| execution.state == CandidateExecutionState::Trained)
            {
                let suite_key = protocol
                    .development_suite_keys()
                    .into_iter()
                    .find(|suite_key| !execution.development_reports.contains_key(suite_key))
                    .expect("trained candidate has an unfinished development suite");
                let model = execution
                    .train_output
                    .as_ref()
                    .expect("trained candidate has output")
                    .model
                    .clone();
                match self
                    .backend
                    .evaluate(
                        project.clone(),
                        model,
                        protocol.metric_contract.clone(),
                        suite_key.clone(),
                        protocol.maximum_evaluation_seconds,
                    )
                    .await
                {
                    Ok(report) => {
                        let assessment = assess_candidate(
                            &project,
                            &protocol.metric_contract,
                            protocol
                                .baseline_development_report_for(&suite_key)
                                .expect("protocol owns the development suite"),
                            &report,
                            Utc::now(),
                        )?;
                        self.append(
                            &protocol,
                            &view,
                            ExperimentEventKind::CandidateDevelopmentSuiteCompleted {
                                candidate_id: *candidate_id,
                                suite_key,
                                report,
                                assessment,
                            },
                        )
                        .await?;
                    }
                    Err(error) => {
                        self.append(
                            &protocol,
                            &view,
                            ExperimentEventKind::CandidateFailed {
                                candidate_id: *candidate_id,
                                phase: CandidatePhase::DevelopmentEvaluation,
                                reason: bounded_failure(&error),
                            },
                        )
                        .await?;
                    }
                }
                continue;
            }

            if let Some(candidate) = protocol.candidates.iter().find(|candidate| {
                view.candidates[&candidate.id].state == CandidateExecutionState::Pending
            }) {
                self.append(
                    &protocol,
                    &view,
                    ExperimentEventKind::CandidateTrainingStarted {
                        candidate_id: candidate.id,
                    },
                )
                .await?;
                continue;
            }

            let evidence = view
                .candidates
                .iter()
                .filter(|(_, execution)| {
                    execution.state == CandidateExecutionState::DevelopmentCompleted
                })
                .map(|(candidate_id, execution)| DevelopmentCandidateEvidence {
                    candidate_id: *candidate_id,
                    suite_assessments: execution.development_assessments.clone(),
                })
                .collect::<Vec<_>>();
            let selected_candidate_id =
                select_multi_development_candidate(&protocol.development_suite_keys(), &evidence)?
                    .map(|selection| selection.candidate_id);
            self.append(
                &protocol,
                &view,
                ExperimentEventKind::DevelopmentSelected {
                    candidate_id: selected_candidate_id,
                },
            )
            .await?;
            if selected_candidate_id.is_none() {
                let view = self.status(run_id).await?;
                self.append(
                    &protocol,
                    &view,
                    ExperimentEventKind::Finalized {
                        decision: FinalDecision::RetainBaseline,
                    },
                )
                .await?;
            }
            return self.status(run_id).await;
        }
        Err(EncoderExperimentError::Integrity(
            "development runner exceeded its deterministic step bound".into(),
        )
        .into())
    }

    pub async fn authorize_sealed(
        &self,
        run_id: Uuid,
        authorized_by: impl Into<String>,
    ) -> Result<ExperimentView, ExperimentRunnerError> {
        let authorized_by = authorized_by.into();
        let (_, protocol, events) = self.context(run_id).await?;
        let project = self.project(protocol.project_snapshot_id).await?;
        let view = replay_experiment(&project, &protocol, &events)?;
        if let Some(original) = &view.sealed_authorized_by {
            if original != authorized_by.trim() {
                return Err(EncoderExperimentError::Validation(
                    "sealed authorization is already frozen to a different authorizer".into(),
                )
                .into());
            }
            return Ok(view);
        }
        let candidate_id = view.selected_candidate_id.ok_or_else(|| {
            EncoderExperimentError::Validation(
                "experiment has no development-selected candidate".into(),
            )
        })?;
        self.append(
            &protocol,
            &view,
            ExperimentEventKind::SealedAuthorized {
                candidate_id,
                authorized_by,
            },
        )
        .await?;
        self.status(run_id).await
    }

    pub async fn run_sealed(&self, run_id: Uuid) -> Result<ExperimentView, ExperimentRunnerError> {
        let (project, protocol, events) = self.context(run_id).await?;
        let mut view = replay_experiment(&project, &protocol, &events)?;
        let candidate_id = view.selected_candidate_id.ok_or_else(|| {
            EncoderExperimentError::Validation(
                "experiment has no development-selected candidate".into(),
            )
        })?;
        if view.state == ExperimentRunState::Completed {
            return Ok(view);
        }
        if view.state == ExperimentRunState::SealedEvaluated {
            return self.finalize_sealed(&protocol, &view).await;
        }
        if view.state == ExperimentRunState::SealedAuthorized {
            self.append(
                &protocol,
                &view,
                ExperimentEventKind::SealedStarted { candidate_id },
            )
            .await?;
            view = self.status(run_id).await?;
        }
        if view.state != ExperimentRunState::SealedEvaluating {
            return Err(EncoderExperimentError::Validation(
                "sealed evaluation is not authorized or is already terminal".into(),
            )
            .into());
        }
        let model = view
            .selected_model()
            .expect("development-selected candidate has a model")
            .clone();
        let report = match self
            .backend
            .evaluate(
                project.clone(),
                model,
                protocol.metric_contract.clone(),
                protocol.sealed_suite_key.clone(),
                protocol.maximum_evaluation_seconds,
            )
            .await
        {
            Ok(report) => report,
            Err(error) => {
                self.append(
                    &protocol,
                    &view,
                    ExperimentEventKind::RunFailed {
                        reason: bounded_failure(&error),
                    },
                )
                .await?;
                return self.status(run_id).await;
            }
        };
        let assessment = assess_candidate(
            &project,
            &protocol.metric_contract,
            &protocol.baseline_sealed_report,
            &report,
            Utc::now(),
        )?;
        self.append(
            &protocol,
            &view,
            ExperimentEventKind::SealedCompleted {
                candidate_id,
                report,
                assessment,
            },
        )
        .await?;
        view = self.status(run_id).await?;
        self.finalize_sealed(&protocol, &view).await
    }

    /// Finish only from deeply replayed sealed evidence. This step never calls the backend.
    async fn finalize_sealed(
        &self,
        protocol: &ExperimentProtocol,
        view: &ExperimentView,
    ) -> Result<ExperimentView, ExperimentRunnerError> {
        let assessment = view.sealed_assessment.as_ref().ok_or_else(|| {
            EncoderExperimentError::Validation("sealed assessment is not persisted".into())
        })?;
        self.append(
            protocol,
            view,
            ExperimentEventKind::Finalized {
                decision: if assessment.verdict == CandidateVerdict::Passed {
                    FinalDecision::PromoteCandidate
                } else {
                    FinalDecision::RetainBaseline
                },
            },
        )
        .await?;
        self.status(view.run_id).await
    }

    async fn append(
        &self,
        protocol: &ExperimentProtocol,
        view: &ExperimentView,
        kind: ExperimentEventKind,
    ) -> Result<(), ExperimentRunnerError> {
        let now = Utc::now().max(view.updated_at);
        let event = view.next_event(protocol, kind, now)?;
        self.store.append_event(event).await?;
        Ok(())
    }

    async fn context(
        &self,
        run_id: Uuid,
    ) -> Result<
        (
            ExternalProjectSnapshot,
            ExperimentProtocol,
            Vec<ExperimentEvent>,
        ),
        ExperimentRunnerError,
    > {
        let events = self.store.load_events(run_id).await?;
        let first = events
            .first()
            .ok_or_else(|| ExperimentRunnerError::NotFound(format!("experiment run {run_id}")))?;
        let protocol = self.protocol(first.protocol_id).await?;
        let project = self.project(protocol.project_snapshot_id).await?;
        Ok((project, protocol, events))
    }

    async fn project(&self, id: Uuid) -> Result<ExternalProjectSnapshot, ExperimentRunnerError> {
        self.store
            .get_project(id)
            .await?
            .ok_or_else(|| ExperimentRunnerError::NotFound(format!("project snapshot {id}")))
    }

    async fn protocol(&self, id: Uuid) -> Result<ExperimentProtocol, ExperimentRunnerError> {
        self.store
            .get_protocol(id)
            .await?
            .ok_or_else(|| ExperimentRunnerError::NotFound(format!("experiment protocol {id}")))
    }
}

fn bounded_failure(error: &EncoderTaskAdapterError) -> String {
    error
        .to_string()
        .chars()
        .take(1_000)
        .collect::<String>()
        .trim()
        .to_owned()
}
