//! Application handoff from completed supervision to ordinary curation.

use std::{collections::BTreeMap, sync::Arc};

use artifact_core::fingerprint;
use chrono::Utc;
use dataset_core::domain::SourceRow;
use dataset_quality_core::{
    assessment::{
        EvaluatorExecutionLocation, EvaluatorGuidance, EvaluatorIdentity, EvaluatorIndependence,
        RowAssessmentDraft, RowQualityAssessment,
    },
    curation::{CurationProposal, DatasetQualityReport},
    lifecycle::{ProviderUsage, QualityAuditRun, QualityAuditRunState},
    policy::{
        AuditBudgets, AuditMode, BorderlineReviewPolicy, EvaluatorEgressPolicy, QualityPolicy,
    },
    population::{AuditPlan, GuidanceReferences},
    ports::{
        BoxFuture, DatasetQualityStore, EvaluatorBatchOutput, QualityCandidateSource,
        QualityEvaluationError, QualityEvaluationErrorKind, QualityEvaluator,
    },
};
use dataset_quality_runner::DatasetQualityRunner;
use generation_core::{
    domain::{DatasetDefinition, GenerationPlan},
    ports::GenerationStore,
};
use generation_supervisor_core::{
    SupervisorError,
    contract::{ArtifactBinding, GenerationQualityContract},
    lifecycle::SupervisorRun,
    ports::GenerationSupervisorStore,
    qualification::{
        QualificationDisposition, SupervisorQualificationApplication,
        SupervisorQualificationHandoff, SupervisorQualificationSelection,
    },
    revision::{PromptGuidanceVersion, PromptRevisionActivation},
};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::orchestration::SupervisorLoopError;

const REPLAY_EVALUATOR_BACKEND: &str = "supervisor-evidence-replay";
const REPLAY_EVALUATOR_PROTOCOL: &str = "supervisor-qualification-replay-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QualificationFinalizationOutcome {
    pub handoff: SupervisorQualificationHandoff,
    pub application: SupervisorQualificationApplication,
    pub report: DatasetQualityReport,
    pub proposal: CurationProposal,
}

/// Rebinds existing supervisor assessments to one ordinary quality audit.
/// The local replay evaluator never calls a provider or changes a score.
pub struct GenerationQualificationFinalizer {
    generation_store: Arc<dyn GenerationStore>,
    supervisor_store: Arc<dyn GenerationSupervisorStore>,
    quality_store: Arc<dyn DatasetQualityStore>,
    candidates: Arc<dyn QualityCandidateSource>,
}

impl GenerationQualificationFinalizer {
    pub fn new(
        generation_store: Arc<dyn GenerationStore>,
        supervisor_store: Arc<dyn GenerationSupervisorStore>,
        quality_store: Arc<dyn DatasetQualityStore>,
        candidates: Arc<dyn QualityCandidateSource>,
    ) -> Self {
        Self {
            generation_store,
            supervisor_store,
            quality_store,
            candidates,
        }
    }

    pub async fn finalize(
        &self,
        run_id: Uuid,
    ) -> Result<QualificationFinalizationOutcome, SupervisorLoopError> {
        let run = self.load_run(run_id).await?;
        let contract = self.load_contract(run.contract_id).await?;
        let dataset = self.load_dataset(contract.dataset.id).await?;
        let plan = self.load_plan(contract.plan.id).await?;
        let evidence = self
            .load_supervisor_evidence(&run, &contract, &plan)
            .await?;

        let (handoff, replay_plan, replay_run, replay_guidance, drafts) = match self
            .supervisor_store
            .qualification_handoff_for_run(run.id)
            .await?
        {
            Some(handoff) => {
                let replay_plan = self
                    .quality_store
                    .get_audit_plan(handoff.replay_audit_plan.id)
                    .await?
                    .ok_or_else(|| {
                        SupervisorLoopError::NotFound(format!(
                            "qualification replay plan {}",
                            handoff.replay_audit_plan.id
                        ))
                    })?;
                let replay_run = self
                    .quality_store
                    .get_audit_run(handoff.replay_audit_run.id)
                    .await?
                    .ok_or_else(|| {
                        SupervisorLoopError::NotFound(format!(
                            "qualification replay run {}",
                            handoff.replay_audit_run.id
                        ))
                    })?;
                let replay_guidance = self
                    .quality_store
                    .get_audit_guidance(replay_plan.id)
                    .await?
                    .ok_or_else(|| {
                        SupervisorLoopError::NotFound(format!(
                            "qualification replay guidance {}",
                            replay_plan.id
                        ))
                    })?;
                let drafts = self
                    .load_selected_drafts(&handoff, &contract, Some(&replay_guidance))
                    .await?;
                (handoff, replay_plan, replay_run, replay_guidance, drafts)
            }
            None => {
                let selection = SupervisorQualificationSelection::compile(
                    &contract,
                    &run,
                    &plan,
                    &evidence.events,
                    &evidence.observations,
                    evidence.assignments.as_ref(),
                    &evidence.prompts,
                    &evidence.activations,
                )?;
                let selected_rows = self
                    .candidates
                    .get_source_rows(dataset.id, selection.selected_source_row_ids())
                    .await?;
                verify_selected_sources(&selection, &selected_rows)?;
                let (drafts, original_guidance) = self
                    .load_selection_drafts(&selection, &contract, None)
                    .await?;
                let replay_plan_id = Uuid::new_v4();
                let replay_guidance = replay_guidance(original_guidance, replay_plan_id);
                let replay_plan = AuditPlan::with_identity(
                    replay_plan_id,
                    &dataset,
                    replay_policy(&contract, selected_rows.len())?,
                    guidance_references(&replay_guidance),
                    replay_guidance.reproduce_fingerprint()?,
                    REPLAY_EVALUATOR_PROTOCOL,
                    selected_rows,
                    Utc::now(),
                )?;
                replay_guidance.verify_against(&replay_plan)?;
                let replay_run = QualityAuditRun::queue(
                    &replay_plan,
                    replay_identity(&selection, &contract)?,
                    vec![],
                )?;
                let completion = evidence.events.last().ok_or_else(|| {
                    SupervisorLoopError::Validation(
                        "completed supervisor run has no terminal event".into(),
                    )
                })?;
                let handoff = SupervisorQualificationHandoff::create(
                    Uuid::new_v4(),
                    &contract,
                    &run,
                    completion,
                    selection,
                    ArtifactBinding::new(replay_plan.id, replay_plan.fingerprint.clone())?,
                    ArtifactBinding::new(
                        replay_run.id,
                        replay_run.specification_fingerprint.clone(),
                    )?,
                    Utc::now(),
                )?;
                self.supervisor_store
                    .create_qualification_handoff(
                        &handoff,
                        &replay_plan,
                        &replay_run,
                        &replay_guidance,
                    )
                    .await?;
                (handoff, replay_plan, replay_run, replay_guidance, drafts)
            }
        };

        replay_guidance.verify_against(&replay_plan)?;
        if replay_run.plan_id != replay_plan.id
            || replay_run.primary_evaluator.backend != REPLAY_EVALUATOR_BACKEND
            || replay_run.primary_evaluator.protocol_version != REPLAY_EVALUATOR_PROTOCOL
        {
            return Err(SupervisorLoopError::Validation(
                "qualification replay audit identity is invalid".into(),
            ));
        }
        let evaluator: Arc<dyn QualityEvaluator> = Arc::new(SupervisorEvidenceReplayEvaluator {
            identity: replay_run.primary_evaluator.clone(),
            handoff_id: handoff.id,
            drafts,
        });
        let runner = DatasetQualityRunner::new(
            Arc::clone(&self.quality_store),
            Arc::clone(&self.candidates),
            vec![evaluator],
        )?;
        let outcome = runner.execute(replay_run.id).await?;
        if outcome.run.state != QualityAuditRunState::Completed {
            return Err(SupervisorLoopError::Validation(format!(
                "qualification evidence replay ended in {:?}",
                outcome.run.state
            )));
        }
        let report = outcome.report.ok_or_else(|| {
            SupervisorLoopError::Validation(
                "completed qualification evidence replay has no report".into(),
            )
        })?;
        let selected_count = handoff.selected_source_row_ids().len() as u64;
        if report.totals.population_rows != selected_count
            || report.totals.qualified_rows != selected_count
            || report.totals.borderline_rows != 0
            || report.totals.quarantined_rows != 0
            || report.totals.invalid_rows != 0
            || report.totals.unaudited_rows != 0
        {
            return Err(SupervisorLoopError::Validation(
                "qualification replay did not reproduce an all-qualified population".into(),
            ));
        }
        let proposal = match self
            .quality_store
            .latest_curation_proposal(report.id)
            .await?
        {
            Some(proposal) => proposal,
            None => {
                let proposal = CurationProposal::create(&report, None, &[])?;
                self.quality_store.save_curation_proposal(&proposal).await?;
                proposal
            }
        };
        if proposal.counts.included_rows != selected_count
            || proposal.counts.excluded_rows != 0
            || proposal.counts.needs_review_rows != 0
        {
            return Err(SupervisorLoopError::Validation(
                "qualification proposal is not the exact qualified selection".into(),
            ));
        }
        let application = match self
            .supervisor_store
            .qualification_application_for_handoff(handoff.id)
            .await?
        {
            Some(application) => application,
            None => {
                let application = SupervisorQualificationApplication::create(
                    Uuid::new_v4(),
                    &handoff,
                    ArtifactBinding::new(report.id, report.fingerprint.clone())?,
                    ArtifactBinding::new(proposal.id, proposal.fingerprint.clone())?,
                    Utc::now(),
                )?;
                self.supervisor_store
                    .save_qualification_application(&application)
                    .await?;
                application
            }
        };
        Ok(QualificationFinalizationOutcome {
            handoff,
            application,
            report,
            proposal,
        })
    }

    async fn load_supervisor_evidence(
        &self,
        run: &SupervisorRun,
        contract: &GenerationQualityContract,
        plan: &GenerationPlan,
    ) -> Result<LoadedSupervisorEvidence, SupervisorLoopError> {
        if run.contract_id != contract.id
            || run.contract_fingerprint != contract.fingerprint
            || plan.id != contract.plan.id
            || fingerprint(plan)? != contract.plan.fingerprint
        {
            return Err(SupervisorLoopError::Validation(
                "supervisor finalization authority does not reproduce".into(),
            ));
        }
        let events = self.supervisor_store.list_run_events(run.id).await?;
        let observations = self.supervisor_store.list_row_observations(run.id).await?;
        let assignments = self
            .supervisor_store
            .get_strategy_assignments(run.id)
            .await?;
        let prompts = self.supervisor_store.list_prompt_versions(run.id).await?;
        let mut activations = Vec::new();
        for prompt in &prompts {
            if let Some(activation) = self
                .supervisor_store
                .get_revision_activation(prompt.id)
                .await?
            {
                activations.push(activation);
            }
        }
        Ok(LoadedSupervisorEvidence {
            events,
            observations,
            assignments,
            prompts,
            activations,
        })
    }

    async fn load_selected_drafts(
        &self,
        handoff: &SupervisorQualificationHandoff,
        contract: &GenerationQualityContract,
        expected_guidance: Option<&EvaluatorGuidance>,
    ) -> Result<BTreeMap<Uuid, RowAssessmentDraft>, SupervisorLoopError> {
        let selection = SupervisorQualificationSelection {
            entries: handoff.entries.clone(),
            coverage_by_cell: handoff.coverage_by_cell.clone(),
            complete_evidence_fingerprint: handoff.complete_evidence_fingerprint.clone(),
            selected_member_fingerprint: handoff.selected_member_fingerprint.clone(),
        };
        self.load_selection_drafts(&selection, contract, expected_guidance)
            .await
            .map(|(drafts, _)| drafts)
    }

    async fn load_selection_drafts(
        &self,
        selection: &SupervisorQualificationSelection,
        contract: &GenerationQualityContract,
        expected_guidance: Option<&EvaluatorGuidance>,
    ) -> Result<(BTreeMap<Uuid, RowAssessmentDraft>, EvaluatorGuidance), SupervisorLoopError> {
        let mut drafts = BTreeMap::new();
        let mut common_guidance: Option<EvaluatorGuidance> = None;
        for entry in selection
            .entries
            .iter()
            .filter(|entry| entry.disposition == QualificationDisposition::Selected)
        {
            let assessment_id = entry.assessment_id.ok_or_else(|| {
                SupervisorLoopError::Validation(
                    "selected qualification row has no assessment".into(),
                )
            })?;
            let assessment = self
                .quality_store
                .get_assessment(assessment_id)
                .await?
                .ok_or_else(|| {
                    SupervisorLoopError::NotFound(format!("row assessment {assessment_id}"))
                })?;
            verify_assessment_binding(entry, &assessment)?;
            let source_plan = self
                .quality_store
                .get_audit_plan(assessment.audit_plan_id)
                .await?
                .ok_or_else(|| {
                    SupervisorLoopError::NotFound(format!(
                        "source quality plan {}",
                        assessment.audit_plan_id
                    ))
                })?;
            assessment.verify_integrity(&source_plan)?;
            if source_plan.dataset_schema.dataset_definition_id != contract.dataset.id
                || source_plan.policy != contract.quality_policy
            {
                return Err(SupervisorLoopError::Validation(
                    "selected assessment used a foreign dataset or quality policy".into(),
                ));
            }
            let guidance = self
                .quality_store
                .get_audit_guidance(source_plan.id)
                .await?
                .ok_or_else(|| {
                    SupervisorLoopError::NotFound(format!(
                        "source quality guidance {}",
                        source_plan.id
                    ))
                })?;
            guidance.verify_against(&source_plan)?;
            if let Some(common) = &common_guidance {
                if normalize_guidance(common) != normalize_guidance(&guidance) {
                    return Err(SupervisorLoopError::Validation(
                        "selected assessments used different evaluator guidance".into(),
                    ));
                }
            } else {
                common_guidance = Some(guidance.clone());
            }
            if let Some(expected) = expected_guidance
                && normalize_guidance(expected) != normalize_guidance(&guidance)
            {
                return Err(SupervisorLoopError::Validation(
                    "persisted replay guidance differs from supervisor evidence".into(),
                ));
            }
            if drafts
                .insert(assessment.source_row_id, draft_from_assessment(&assessment))
                .is_some()
            {
                return Err(SupervisorLoopError::Validation(
                    "qualification evidence repeats a source row".into(),
                ));
            }
        }
        Ok((
            drafts,
            common_guidance.ok_or_else(|| {
                SupervisorLoopError::Validation(
                    "qualification selected no assessment evidence".into(),
                )
            })?,
        ))
    }

    async fn load_run(&self, id: Uuid) -> Result<SupervisorRun, SupervisorLoopError> {
        self.supervisor_store
            .get_supervisor_run(id)
            .await?
            .ok_or_else(|| SupervisorLoopError::NotFound(format!("supervisor run {id}")))
    }

    async fn load_contract(
        &self,
        id: Uuid,
    ) -> Result<GenerationQualityContract, SupervisorLoopError> {
        self.supervisor_store
            .get_contract(id)
            .await?
            .ok_or_else(|| SupervisorLoopError::NotFound(format!("quality contract {id}")))
    }

    async fn load_dataset(&self, id: Uuid) -> Result<DatasetDefinition, SupervisorLoopError> {
        self.generation_store
            .get_dataset(id)
            .await?
            .ok_or_else(|| SupervisorLoopError::NotFound(format!("dataset {id}")))
    }

    async fn load_plan(&self, id: Uuid) -> Result<GenerationPlan, SupervisorLoopError> {
        self.generation_store
            .get_plan(id)
            .await?
            .ok_or_else(|| SupervisorLoopError::NotFound(format!("generation plan {id}")))
    }
}

struct LoadedSupervisorEvidence {
    events: Vec<generation_supervisor_core::lifecycle::SupervisorRunEvent>,
    observations: Vec<generation_supervisor_core::observation::RowQualityObservation>,
    assignments: Option<generation_supervisor_core::strategy::StrategyAssignmentSet>,
    prompts: Vec<PromptGuidanceVersion>,
    activations: Vec<PromptRevisionActivation>,
}

struct SupervisorEvidenceReplayEvaluator {
    identity: EvaluatorIdentity,
    handoff_id: Uuid,
    drafts: BTreeMap<Uuid, RowAssessmentDraft>,
}

impl QualityEvaluator for SupervisorEvidenceReplayEvaluator {
    fn identity(&self) -> EvaluatorIdentity {
        self.identity.clone()
    }

    fn evaluate(
        &self,
        request: dataset_quality_core::assessment::BlindEvaluatorRequest,
    ) -> BoxFuture<'_, Result<EvaluatorBatchOutput, QualityEvaluationError>> {
        Box::pin(async move {
            let mut assessments = Vec::with_capacity(request.rows.len());
            for row in &request.rows {
                let draft = self.drafts.get(&row.source_row_id).ok_or_else(|| {
                    QualityEvaluationError::new(
                        QualityEvaluationErrorKind::InvalidResponse,
                        "qualification replay row is outside the handoff",
                    )
                })?;
                if draft.source_row_fingerprint != row.source_row_fingerprint {
                    return Err(QualityEvaluationError::new(
                        QualityEvaluationErrorKind::InvalidResponse,
                        "qualification replay source fingerprint changed",
                    ));
                }
                assessments.push(draft.clone());
            }
            Ok(EvaluatorBatchOutput {
                assessments,
                usage: ProviderUsage::default(),
                metadata: json!({
                    "source": "generation_supervisor_qualification_handoff",
                    "handoff_id": self.handoff_id,
                    "external_io": false,
                }),
            })
        })
    }
}

fn replay_policy(
    contract: &GenerationQualityContract,
    selected_rows: usize,
) -> Result<QualityPolicy, SupervisorError> {
    let source = &contract.quality_policy;
    let batch = source.budgets.maximum_rows_per_batch;
    let rows = u64::try_from(selected_rows)
        .map_err(|_| SupervisorError::Validation("qualification row count overflowed".into()))?;
    let requests = rows.div_ceil(u64::from(batch)).max(1);
    QualityPolicy::new(
        None,
        source.thresholds.clone(),
        source.invalid_output_policy,
        BorderlineReviewPolicy::None,
        AuditBudgets {
            maximum_rows_per_batch: batch,
            maximum_evaluator_requests: u32::try_from(requests).map_err(|_| {
                SupervisorError::Validation("qualification replay needs too many batches".into())
            })?,
            maximum_attempts_per_request: 1,
            maximum_input_tokens: source.budgets.maximum_input_tokens,
            maximum_output_tokens: source.budgets.maximum_output_tokens,
            maximum_total_tokens: source.budgets.maximum_total_tokens,
            maximum_cost_microusd: None,
        },
        EvaluatorEgressPolicy::LocalOnly,
        AuditMode::FullPopulation,
    )
    .map_err(|error| SupervisorError::Validation(error.to_string()))
}

fn replay_identity(
    selection: &SupervisorQualificationSelection,
    contract: &GenerationQualityContract,
) -> Result<EvaluatorIdentity, SupervisorLoopError> {
    Ok(EvaluatorIdentity::new(
        REPLAY_EVALUATOR_BACKEND,
        "persisted-supervisor-assessments",
        REPLAY_EVALUATOR_PROTOCOL,
        fingerprint(&(
            &selection.selected_member_fingerprint,
            &selection.complete_evidence_fingerprint,
            &contract.evaluator.fingerprint,
        ))?,
        EvaluatorIndependence::Primary,
        EvaluatorExecutionLocation::LocalProcess,
    )?)
}

fn replay_guidance(mut guidance: EvaluatorGuidance, replay_plan_id: Uuid) -> EvaluatorGuidance {
    if let Some(semantic) = &mut guidance.semantic {
        semantic.reference.id = replay_plan_id;
    }
    guidance
}

fn guidance_references(guidance: &EvaluatorGuidance) -> GuidanceReferences {
    GuidanceReferences {
        semantic_context: guidance
            .semantic
            .as_ref()
            .map(|value| value.reference.clone()),
        authenticity_context: guidance
            .authenticity
            .as_ref()
            .map(|value| value.reference.clone()),
    }
}

fn normalize_guidance(guidance: &EvaluatorGuidance) -> EvaluatorGuidance {
    let mut value = guidance.clone();
    if let Some(semantic) = &mut value.semantic {
        semantic.reference.id = Uuid::nil();
    }
    value
}

fn draft_from_assessment(assessment: &RowQualityAssessment) -> RowAssessmentDraft {
    RowAssessmentDraft {
        source_row_id: assessment.source_row_id,
        source_row_fingerprint: assessment.source_row_fingerprint.clone(),
        label_scores: assessment.label_scores.clone(),
        dimension_scores: assessment.dimension_scores.clone(),
        authenticity_score: assessment.authenticity_score,
        label_leakage_risk: assessment.label_leakage_risk,
        shortcut_risk: assessment.shortcut_risk,
        confidence: assessment.confidence,
        issue_codes: assessment.issue_codes.clone(),
        rationale: assessment.rationale.clone(),
    }
}

fn verify_assessment_binding(
    entry: &generation_supervisor_core::qualification::SupervisorQualificationEntry,
    assessment: &RowQualityAssessment,
) -> Result<(), SupervisorLoopError> {
    if entry.assessment_id != Some(assessment.id)
        || entry.assessment_fingerprint.as_deref() != Some(assessment.fingerprint.as_str())
        || entry.source_row_id != Some(assessment.source_row_id)
        || entry.source_row_fingerprint.as_deref()
            != Some(assessment.source_row_fingerprint.as_str())
        || assessment.reproduce_fingerprint()? != assessment.fingerprint
    {
        return Err(SupervisorLoopError::Validation(
            "selected entry does not bind its exact assessment".into(),
        ));
    }
    Ok(())
}

fn verify_selected_sources(
    selection: &SupervisorQualificationSelection,
    rows: &[SourceRow],
) -> Result<(), SupervisorLoopError> {
    let expected = selection
        .entries
        .iter()
        .filter(|entry| entry.disposition == QualificationDisposition::Selected)
        .map(|entry| {
            Ok((
                entry.source_row_id.ok_or_else(|| {
                    SupervisorLoopError::Validation("selected entry has no source row".into())
                })?,
                entry.source_row_fingerprint.clone().ok_or_else(|| {
                    SupervisorLoopError::Validation(
                        "selected entry has no source fingerprint".into(),
                    )
                })?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, SupervisorLoopError>>()?;
    let actual = rows
        .iter()
        .map(|row| Ok((row.id, fingerprint(row)?)))
        .collect::<Result<BTreeMap<_, _>, SupervisorLoopError>>()?;
    if expected != actual {
        return Err(SupervisorLoopError::Validation(
            "selected source rows changed before finalization".into(),
        ));
    }
    Ok(())
}
