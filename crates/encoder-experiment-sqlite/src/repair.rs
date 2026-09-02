use encoder_campaign_core::{CampaignEventKind, CampaignStore, replay_campaign};
use encoder_experiment_core::{journal::ExperimentEventKind, ports::ExperimentStore};
use encoder_repair_core::{
    diagnosis::{CandidateSuiteOutcome, ComparativeDiagnosis},
    observation::DevelopmentObservationSet,
    ports::{BoxFuture, RepairEvidenceStore, RepairEvidenceStoreError},
    proposal::{
        RepairBenchmarkBinding, RepairProposal, RepairProposalApplication, RepairProposalReview,
    },
};
use sqlx::{Sqlite, Transaction};
use uuid::Uuid;
use workflow_core::{
    benchmark_generation::{BenchmarkGenerationState, replay_benchmark_generation},
    ports::BenchmarkGenerationStore,
};

use crate::SqliteExperimentStore;

impl RepairEvidenceStore for SqliteExperimentStore {
    fn create_observation_set(
        &self,
        observation_set: DevelopmentObservationSet,
    ) -> BoxFuture<'_, Result<DevelopmentObservationSet, RepairEvidenceStoreError>> {
        Box::pin(async move {
            observation_set.validate_integrity().map_err(store_error)?;
            self.validate_observation_source(&observation_set).await?;
            if let Some(existing) = self
                .find_observation_set_by_evidence(observation_set.evidence_fingerprint.clone())
                .await?
            {
                return Ok(existing);
            }
            let artifact_json = serde_json::to_string(&observation_set).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_repair_observation_sets \
                 (id, evidence_fingerprint, fingerprint, project_snapshot_id, \
                  source_campaign_id, source_experiment_run_id, candidate_id, \
                  evaluation_report_id, suite_key, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(observation_set.id)
            .bind(&observation_set.evidence_fingerprint)
            .bind(&observation_set.fingerprint)
            .bind(observation_set.project_snapshot_id)
            .bind(observation_set.source_campaign_id)
            .bind(observation_set.source_experiment_run_id)
            .bind(observation_set.candidate_id)
            .bind(observation_set.evaluation_report_id)
            .bind(&observation_set.suite_key)
            .bind(artifact_json)
            .bind(observation_set.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(observation_set)
        })
    }

    fn get_observation_set(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DevelopmentObservationSet>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let artifact: Option<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_repair_observation_sets WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            let Some(artifact) = artifact else {
                return Ok(None);
            };
            let value = decode_observation_set(artifact)?;
            self.validate_observation_source(&value).await?;
            Ok(Some(value))
        })
    }

    fn find_observation_set_by_evidence(
        &self,
        evidence_fingerprint: String,
    ) -> BoxFuture<'_, Result<Option<DevelopmentObservationSet>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM encoder_repair_observation_sets \
                 WHERE evidence_fingerprint = ?",
            )
            .bind(evidence_fingerprint)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            match id {
                Some(id) => self.get_observation_set(id).await,
                None => Ok(None),
            }
        })
    }

    fn list_observation_sets_for_campaign(
        &self,
        campaign_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<DevelopmentObservationSet>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM encoder_repair_observation_sets \
                 WHERE source_campaign_id = ? ORDER BY created_at, id",
            )
            .bind(campaign_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;
            let mut values = Vec::with_capacity(ids.len());
            for id in ids {
                values.push(self.get_observation_set(id).await?.ok_or_else(|| {
                    RepairEvidenceStoreError("repair observation set disappeared".into())
                })?);
            }
            Ok(values)
        })
    }

    fn create_diagnosis(
        &self,
        diagnosis: ComparativeDiagnosis,
    ) -> BoxFuture<'_, Result<ComparativeDiagnosis, RepairEvidenceStoreError>> {
        Box::pin(async move {
            self.validate_diagnosis_derivation(&diagnosis, false)
                .await?;
            if let Some(existing) = self
                .find_diagnosis_by_derivation(diagnosis.derivation_fingerprint.clone())
                .await?
            {
                return Ok(existing);
            }
            let artifact_json = serde_json::to_string(&diagnosis).map_err(store_error)?;
            let mut transaction = self.pool().begin().await.map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_repair_diagnoses \
                 (id, derivation_fingerprint, fingerprint, project_snapshot_id, \
                  source_campaign_id, source_experiment_run_id, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(diagnosis.id)
            .bind(&diagnosis.derivation_fingerprint)
            .bind(&diagnosis.fingerprint)
            .bind(diagnosis.project_snapshot_id)
            .bind(diagnosis.source_campaign_id)
            .bind(diagnosis.source_experiment_run_id)
            .bind(artifact_json)
            .bind(diagnosis.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            for (ordinal, binding) in diagnosis.observation_sets.iter().enumerate() {
                insert_diagnosis_binding(
                    &mut transaction,
                    diagnosis.id,
                    ordinal,
                    binding.id,
                    &binding.fingerprint,
                )
                .await?;
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(diagnosis)
        })
    }

    fn get_diagnosis(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ComparativeDiagnosis>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let artifact: Option<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_repair_diagnoses WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            let Some(artifact) = artifact else {
                return Ok(None);
            };
            let diagnosis = decode_diagnosis(artifact)?;
            self.validate_diagnosis_derivation(&diagnosis, true).await?;
            Ok(Some(diagnosis))
        })
    }

    fn find_diagnosis_by_derivation(
        &self,
        derivation_fingerprint: String,
    ) -> BoxFuture<'_, Result<Option<ComparativeDiagnosis>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM encoder_repair_diagnoses WHERE derivation_fingerprint = ?",
            )
            .bind(derivation_fingerprint)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            match id {
                Some(id) => self.get_diagnosis(id).await,
                None => Ok(None),
            }
        })
    }

    fn list_diagnoses_for_campaign(
        &self,
        campaign_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ComparativeDiagnosis>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM encoder_repair_diagnoses \
                 WHERE source_campaign_id = ? ORDER BY created_at, id",
            )
            .bind(campaign_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;
            let mut diagnoses = Vec::with_capacity(ids.len());
            for id in ids {
                diagnoses.push(self.get_diagnosis(id).await?.ok_or_else(|| {
                    RepairEvidenceStoreError("repair diagnosis disappeared".into())
                })?);
            }
            Ok(diagnoses)
        })
    }

    fn create_proposal(
        &self,
        proposal: RepairProposal,
    ) -> BoxFuture<'_, Result<RepairProposal, RepairEvidenceStoreError>> {
        Box::pin(async move {
            self.validate_proposal_dependencies(&proposal, proposal.created_at, true)
                .await?;
            if let Some(existing) = self
                .find_proposal_by_specification(proposal.specification_fingerprint.clone())
                .await?
            {
                return Ok(existing);
            }
            let artifact_json = serde_json::to_string(&proposal).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_repair_proposals \
                 (id, specification_fingerprint, fingerprint, diagnosis_id, \
                  source_project_snapshot_id, execution_project_snapshot_id, \
                  source_campaign_id, source_experiment_run_id, benchmark_generation_id, \
                  artifact_json, expires_at, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(proposal.id)
            .bind(&proposal.specification_fingerprint)
            .bind(&proposal.fingerprint)
            .bind(proposal.context.diagnosis.id)
            .bind(proposal.context.source_project.id)
            .bind(proposal.context.execution_project.id)
            .bind(proposal.context.source_campaign_id)
            .bind(proposal.context.source_experiment_run_id)
            .bind(proposal.context.benchmark.generation_id)
            .bind(artifact_json)
            .bind(proposal.expires_at)
            .bind(proposal.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(proposal)
        })
    }

    fn get_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<RepairProposal>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let artifact: Option<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_repair_proposals WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            let Some(artifact) = artifact else {
                return Ok(None);
            };
            let proposal: RepairProposal = serde_json::from_str(&artifact).map_err(store_error)?;
            self.validate_proposal_dependencies(&proposal, proposal.created_at, false)
                .await?;
            Ok(Some(proposal))
        })
    }

    fn find_proposal_by_specification(
        &self,
        specification_fingerprint: String,
    ) -> BoxFuture<'_, Result<Option<RepairProposal>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM encoder_repair_proposals WHERE specification_fingerprint = ?",
            )
            .bind(specification_fingerprint)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            match id {
                Some(id) => self.get_proposal(id).await,
                None => Ok(None),
            }
        })
    }

    fn append_proposal_review(
        &self,
        review: RepairProposalReview,
    ) -> BoxFuture<'_, Result<RepairProposalReview, RepairEvidenceStoreError>> {
        Box::pin(async move {
            if self
                .get_proposal_application(review.proposal_id)
                .await?
                .is_some()
            {
                return Err(RepairEvidenceStoreError(
                    "repair proposal review chain is frozen by its application reservation".into(),
                ));
            }
            let proposal = self
                .get_proposal(review.proposal_id)
                .await?
                .ok_or_else(|| RepairEvidenceStoreError("repair proposal does not exist".into()))?;
            let diagnosis = self
                .get_diagnosis(proposal.context.diagnosis.id)
                .await?
                .ok_or_else(|| {
                    RepairEvidenceStoreError("repair diagnosis does not exist".into())
                })?;
            self.validate_proposal_dependencies(&proposal, review.created_at, true)
                .await?;
            let reviews = self.list_proposal_reviews(proposal.id).await?;
            let predecessor = reviews.last();
            review
                .validate_against(&proposal, predecessor)
                .map_err(store_error)?;
            proposal
                .validate_integrity(&diagnosis)
                .map_err(store_error)?;
            let artifact_json = serde_json::to_string(&review).map_err(store_error)?;
            let decision = serde_json::to_string(&review.decision).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_repair_proposal_reviews \
                 (id, proposal_id, predecessor_id, decision, fingerprint, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(review.id)
            .bind(review.proposal_id)
            .bind(review.predecessor.as_ref().map(|value| value.id))
            .bind(decision)
            .bind(&review.fingerprint)
            .bind(artifact_json)
            .bind(review.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(review)
        })
    }

    fn list_proposal_reviews(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<RepairProposalReview>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let proposal = self
                .get_proposal(proposal_id)
                .await?
                .ok_or_else(|| RepairEvidenceStoreError("repair proposal does not exist".into()))?;
            let artifacts: Vec<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_repair_proposal_reviews \
                 WHERE proposal_id = ? ORDER BY created_at, id",
            )
            .bind(proposal_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;
            let mut values = Vec::with_capacity(artifacts.len());
            for artifact in artifacts {
                let value: RepairProposalReview =
                    serde_json::from_str(&artifact).map_err(store_error)?;
                value
                    .validate_against(&proposal, values.last())
                    .map_err(store_error)?;
                values.push(value);
            }
            Ok(values)
        })
    }

    fn reserve_proposal_application(
        &self,
        application: RepairProposalApplication,
    ) -> BoxFuture<'_, Result<RepairProposalApplication, RepairEvidenceStoreError>> {
        Box::pin(async move {
            if let Some(existing) = self
                .get_proposal_application(application.proposal_id)
                .await?
            {
                if existing.reservation_key != application.reservation_key {
                    return Err(RepairEvidenceStoreError(
                        "repair proposal already has a different application reservation".into(),
                    ));
                }
                return Ok(existing);
            }
            let proposal = self
                .get_proposal(application.proposal_id)
                .await?
                .ok_or_else(|| RepairEvidenceStoreError("repair proposal does not exist".into()))?;
            self.validate_proposal_dependencies(&proposal, application.created_at, true)
                .await?;
            let reviews = self.list_proposal_reviews(proposal.id).await?;
            let approval = reviews
                .iter()
                .find(|value| value.id == application.approval_id)
                .ok_or_else(|| RepairEvidenceStoreError("repair approval does not exist".into()))?;
            application
                .validate_against(&proposal, approval)
                .map_err(store_error)?;
            let artifact_json = serde_json::to_string(&application).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_repair_proposal_applications \
                 (id, proposal_id, approval_id, reservation_key, fingerprint, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(application.id)
            .bind(application.proposal_id)
            .bind(application.approval_id)
            .bind(&application.reservation_key)
            .bind(&application.fingerprint)
            .bind(artifact_json)
            .bind(application.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(application)
        })
    }

    fn get_proposal_application(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<RepairProposalApplication>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let artifact: Option<String> = sqlx::query_scalar(
                "SELECT artifact_json FROM encoder_repair_proposal_applications \
                 WHERE proposal_id = ?",
            )
            .bind(proposal_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            let Some(artifact) = artifact else {
                return Ok(None);
            };
            let application: RepairProposalApplication =
                serde_json::from_str(&artifact).map_err(store_error)?;
            let proposal = self
                .get_proposal(proposal_id)
                .await?
                .ok_or_else(|| RepairEvidenceStoreError("repair proposal disappeared".into()))?;
            let approval = self
                .list_proposal_reviews(proposal_id)
                .await?
                .into_iter()
                .find(|value| value.id == application.approval_id)
                .ok_or_else(|| RepairEvidenceStoreError("repair approval disappeared".into()))?;
            application
                .validate_against(&proposal, &approval)
                .map_err(store_error)?;
            Ok(Some(application))
        })
    }
}

impl SqliteExperimentStore {
    async fn validate_proposal_dependencies(
        &self,
        proposal: &RepairProposal,
        at: chrono::DateTime<chrono::Utc>,
        require_current_authority: bool,
    ) -> Result<(), RepairEvidenceStoreError> {
        let diagnosis = self
            .get_diagnosis(proposal.context.diagnosis.id)
            .await?
            .ok_or_else(|| RepairEvidenceStoreError("repair diagnosis does not exist".into()))?;
        let source_project = self
            .get_project(proposal.context.source_project.id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| {
                RepairEvidenceStoreError("repair source project does not exist".into())
            })?;
        let execution_project = self
            .get_project(proposal.context.execution_project.id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| {
                RepairEvidenceStoreError("repair execution project does not exist".into())
            })?;
        let benchmark = self
            .repair_benchmark_binding(
                proposal.context.benchmark.generation_id,
                proposal.context.benchmark.journal_sequence,
                &proposal.context.benchmark.journal_head_fingerprint,
                require_current_authority,
            )
            .await?;
        proposal
            .context
            .verify_against(&diagnosis, &source_project, &execution_project, &benchmark)
            .map_err(store_error)?;
        proposal
            .validate_integrity(&diagnosis)
            .map_err(store_error)?;
        if require_current_authority {
            proposal
                .validate_against(&diagnosis, &proposal.context, at)
                .map_err(store_error)?;
        }
        Ok(())
    }

    async fn repair_benchmark_binding(
        &self,
        generation_id: Uuid,
        journal_sequence: u32,
        journal_head_fingerprint: &str,
        require_current: bool,
    ) -> Result<RepairBenchmarkBinding, RepairEvidenceStoreError> {
        let generation = self
            .get_benchmark_generation(generation_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| {
                RepairEvidenceStoreError("repair benchmark generation does not exist".into())
            })?;
        let all_events = self
            .list_benchmark_generation_events(generation_id)
            .await
            .map_err(store_error)?;
        let prefix_len = usize::try_from(journal_sequence).map_err(store_error)?;
        if prefix_len == 0
            || prefix_len > all_events.len()
            || all_events[prefix_len - 1].fingerprint != journal_head_fingerprint
            || require_current && prefix_len != all_events.len()
        {
            return Err(RepairEvidenceStoreError(
                "repair benchmark journal head is stale or missing".into(),
            ));
        }
        let view = replay_benchmark_generation(&generation, &all_events[..prefix_len])
            .map_err(store_error)?;
        if view.state != BenchmarkGenerationState::Active {
            return Err(RepairEvidenceStoreError(
                "repair benchmark generation is not active".into(),
            ));
        }
        let freshness = generation.freshness.as_ref().ok_or_else(|| {
            RepairEvidenceStoreError("repair benchmark freshness is missing".into())
        })?;
        let suites = generation
            .development_suites
            .iter()
            .map(|value| {
                (
                    value.suite_key.clone(),
                    value.bundle.development_suite_fingerprint.clone(),
                )
            })
            .collect();
        RepairBenchmarkBinding::create(
            generation.id,
            generation.fingerprint,
            view.last_sequence,
            view.last_event_fingerprint,
            suites,
            generation.sealed_suite_id,
            generation.sealed_suite_fingerprint,
            freshness.valid_until,
        )
        .map_err(store_error)
    }

    async fn validate_repair_scope(
        &self,
        project_id: Uuid,
        project_fingerprint: &str,
        campaign_id: Uuid,
        run_id: Uuid,
    ) -> Result<(), RepairEvidenceStoreError> {
        let project = self
            .get_project(project_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| RepairEvidenceStoreError("repair project does not exist".into()))?;
        if project.fingerprint != project_fingerprint {
            return Err(RepairEvidenceStoreError(
                "repair evidence project fingerprint does not match persistence".into(),
            ));
        }
        let campaign = self
            .get_campaign(campaign_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| RepairEvidenceStoreError("source campaign does not exist".into()))?;
        let campaign_events = self
            .list_campaign_events(campaign_id)
            .await
            .map_err(store_error)?;
        replay_campaign(&campaign, &campaign_events).map_err(store_error)?;
        let run_is_linked = campaign_events.iter().any(|event| {
            matches!(
                &event.event,
                CampaignEventKind::RunStarted {
                    run_id: linked_run_id,
                    ..
                } if *linked_run_id == run_id
            )
        });
        if campaign.project_snapshot_id != project_id
            || campaign.project_snapshot_fingerprint != project_fingerprint
            || !run_is_linked
        {
            return Err(RepairEvidenceStoreError(
                "repair evidence does not match the campaign's exact project and run".into(),
            ));
        }
        let run_project_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT protocols.project_snapshot_id FROM encoder_experiment_runs AS runs \
             JOIN encoder_experiment_protocols AS protocols ON protocols.id = runs.protocol_id \
             WHERE runs.id = ?",
        )
        .bind(run_id)
        .fetch_optional(self.pool())
        .await
        .map_err(store_error)?;
        if run_project_id != Some(project_id) {
            return Err(RepairEvidenceStoreError(
                "repair experiment run does not belong to the exact project".into(),
            ));
        }
        Ok(())
    }

    async fn validate_observation_source(
        &self,
        observation_set: &DevelopmentObservationSet,
    ) -> Result<(), RepairEvidenceStoreError> {
        self.validate_repair_scope(
            observation_set.project_snapshot_id,
            &observation_set.project_snapshot_fingerprint,
            observation_set.source_campaign_id,
            observation_set.source_experiment_run_id,
        )
        .await?;
        let protocol_id: Uuid =
            sqlx::query_scalar("SELECT protocol_id FROM encoder_experiment_runs WHERE id = ?")
                .bind(observation_set.source_experiment_run_id)
                .fetch_one(self.pool())
                .await
                .map_err(store_error)?;
        let protocol = self
            .get_protocol(protocol_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| RepairEvidenceStoreError("repair protocol does not exist".into()))?;
        let source_matches = match observation_set.candidate_id {
            None => protocol
                .baseline_development_reports()
                .into_iter()
                .any(|report| {
                    report.id == observation_set.evaluation_report_id
                        && report.fingerprint == observation_set.evaluation_report_fingerprint
                        && report.model.fingerprint == observation_set.model_fingerprint
                        && report.suite_key == observation_set.suite_key
                        && report.suite_fingerprint == observation_set.suite_fingerprint
                }),
            Some(candidate_id) => self
                .load_events(observation_set.source_experiment_run_id)
                .await
                .map_err(store_error)?
                .into_iter()
                .any(|event| match event.event {
                    ExperimentEventKind::CandidateDevelopmentCompleted {
                        candidate_id: event_candidate_id,
                        report,
                        ..
                    }
                    | ExperimentEventKind::CandidateDevelopmentSuiteCompleted {
                        candidate_id: event_candidate_id,
                        report,
                        ..
                    } => {
                        event_candidate_id == candidate_id
                            && report.id == observation_set.evaluation_report_id
                            && report.fingerprint == observation_set.evaluation_report_fingerprint
                            && report.model.fingerprint == observation_set.model_fingerprint
                            && report.suite_key == observation_set.suite_key
                            && report.suite_fingerprint == observation_set.suite_fingerprint
                    }
                    _ => false,
                }),
        };
        if !source_matches {
            return Err(RepairEvidenceStoreError(
                "repair observation report is not exact persisted development evidence".into(),
            ));
        }
        Ok(())
    }

    async fn validate_diagnosis_derivation(
        &self,
        diagnosis: &ComparativeDiagnosis,
        check_persisted_bindings: bool,
    ) -> Result<(), RepairEvidenceStoreError> {
        diagnosis.validate_integrity().map_err(store_error)?;
        self.validate_repair_scope(
            diagnosis.project_snapshot_id,
            &diagnosis.project_snapshot_fingerprint,
            diagnosis.source_campaign_id,
            diagnosis.source_experiment_run_id,
        )
        .await?;
        let project = self
            .get_project(diagnosis.project_snapshot_id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| RepairEvidenceStoreError("repair project does not exist".into()))?;
        if check_persisted_bindings {
            let persisted_bindings: Vec<(Uuid, String)> = sqlx::query_as(
                "SELECT observation_set_id, observation_set_fingerprint \
                 FROM encoder_repair_diagnosis_observation_sets \
                 WHERE diagnosis_id = ? ORDER BY ordinal",
            )
            .bind(diagnosis.id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;
            let expected_bindings = diagnosis
                .observation_sets
                .iter()
                .map(|binding| (binding.id, binding.fingerprint.clone()))
                .collect::<Vec<_>>();
            if persisted_bindings != expected_bindings {
                return Err(RepairEvidenceStoreError(
                    "diagnosis persistence bindings changed or became incomplete".into(),
                ));
            }
        }
        let mut baseline_sets = Vec::new();
        let mut candidate_sets = Vec::new();
        for binding in &diagnosis.observation_sets {
            let set = self.get_observation_set(binding.id).await?.ok_or_else(|| {
                RepairEvidenceStoreError("diagnosis observation set is missing".into())
            })?;
            if set.fingerprint != binding.fingerprint {
                return Err(RepairEvidenceStoreError(
                    "diagnosis observation binding fingerprint changed".into(),
                ));
            }
            self.validate_repair_scope(
                set.project_snapshot_id,
                &set.project_snapshot_fingerprint,
                set.source_campaign_id,
                set.source_experiment_run_id,
            )
            .await?;
            if set.candidate_id.is_some() {
                candidate_sets.push(set);
            } else {
                baseline_sets.push(set);
            }
        }
        let outcomes = diagnosis
            .candidate_tradeoffs
            .iter()
            .flat_map(|tradeoff| tradeoff.outcomes.iter().cloned())
            .collect::<Vec<CandidateSuiteOutcome>>();
        diagnosis
            .verify_derivation(&project, &baseline_sets, &candidate_sets, &outcomes)
            .map_err(store_error)
    }
}

async fn insert_diagnosis_binding(
    transaction: &mut Transaction<'_, Sqlite>,
    diagnosis_id: Uuid,
    ordinal: usize,
    observation_set_id: Uuid,
    observation_set_fingerprint: &str,
) -> Result<(), RepairEvidenceStoreError> {
    let ordinal = i64::try_from(ordinal)
        .map_err(|_| RepairEvidenceStoreError("diagnosis binding ordinal overflowed".into()))?;
    sqlx::query(
        "INSERT INTO encoder_repair_diagnosis_observation_sets \
         (diagnosis_id, observation_set_id, observation_set_fingerprint, ordinal) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(diagnosis_id)
    .bind(observation_set_id)
    .bind(observation_set_fingerprint)
    .bind(ordinal)
    .execute(&mut **transaction)
    .await
    .map_err(store_error)?;
    Ok(())
}

fn decode_observation_set(
    artifact: String,
) -> Result<DevelopmentObservationSet, RepairEvidenceStoreError> {
    let value: DevelopmentObservationSet = serde_json::from_str(&artifact).map_err(store_error)?;
    value.validate_integrity().map_err(store_error)?;
    Ok(value)
}

fn decode_diagnosis(artifact: String) -> Result<ComparativeDiagnosis, RepairEvidenceStoreError> {
    let value: ComparativeDiagnosis = serde_json::from_str(&artifact).map_err(store_error)?;
    value.validate_integrity().map_err(store_error)?;
    Ok(value)
}

fn store_error(error: impl std::fmt::Display) -> RepairEvidenceStoreError {
    RepairEvidenceStoreError(error.to_string())
}
