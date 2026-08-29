use chrono::{DateTime, Utc};
use generation_core::domain::{GenerationPlan, PlannedCell};
use optimization_core::{
    allocation::AllocationFeasibility,
    campaigns::{
        CampaignArtifactKind, CampaignArtifactLink, CampaignOutcomeAssessment,
        OptimizationCampaign, OutcomeClassification,
    },
    domain::{OptimizationProposal, ProposalApplication},
    ports::{
        BoxFuture, CampaignQuery, OptimizationProposalQuery, OptimizationStore,
        OptimizationStoreError, ProposalReviewQuery,
    },
    protocol::{RecommendationKind, ScoringPolicy},
    reviews::{ProposalReviewRecord, ProposalReviewState},
    scenarios::{OptimizationScenarioGroup, ScenarioKind},
};
use sqlx::{FromRow, QueryBuilder, Sqlite};
use uuid::Uuid;

use super::SqliteStore;

impl OptimizationStore for SqliteStore {
    fn create_optimization_proposal(
        &self,
        proposal: &OptimizationProposal,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>> {
        let proposal = proposal.clone();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            sqlx::query(
                "INSERT INTO optimization_proposals \
                 (id, analysis_report_id, dataset_id, additional_example_budget, \
                  minimum_support, proposal_json, fingerprint, created_at, protocol_fingerprint, \
                  evidence_fingerprint, source_snapshot_id, scoring_policy, unallocated_budget) \
                  VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(proposal.id)
            .bind(proposal.analysis_report_id)
            .bind(proposal.dataset_id)
            .bind(i64::from(proposal.additional_example_budget))
            .bind(to_i64(proposal.minimum_support)?)
            .bind(to_json(&proposal)?)
            .bind(proposal.fingerprint)
            .bind(proposal.created_at)
            .bind(non_empty(&proposal.protocol_fingerprint))
            .bind(non_empty(&proposal.evidence_fingerprint))
            .bind(
                proposal
                    .source_identity
                    .as_ref()
                    .map(|source| source.snapshot_id),
            )
            .bind(
                proposal
                    .protocol
                    .as_ref()
                    .map(|protocol| scoring_policy_text(protocol.scoring_policy)),
            )
            .bind(
                proposal
                    .allocation
                    .as_ref()
                    .map(|allocation| i64::from(allocation.unallocated_budget)),
            )
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            if let Some(protocol) = &proposal.protocol {
                sqlx::query(
                    "INSERT INTO optimization_protocols \
                     (proposal_id, fingerprint, protocol_json) VALUES (?, ?, ?)",
                )
                .bind(proposal.id)
                .bind(&proposal.protocol_fingerprint)
                .bind(to_json(protocol)?)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
            }
            if let Some(source) = &proposal.source_identity {
                sqlx::query(
                    "INSERT INTO optimization_proposal_sources \
                     (proposal_id, analysis_fingerprint, analysis_protocol_fingerprint, \
                      diagnostic_contract_fingerprint, evaluation_run_id, \
                      evaluation_input_fingerprint, evaluation_protocol_fingerprint, \
                      cohort_fingerprint, dataset_fingerprint, snapshot_id, snapshot_fingerprint, \
                      coverage_fingerprint, comparison_id, comparison_fingerprint, \
                      training_configuration_space_fingerprint, source_json) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(proposal.id)
                .bind(&source.analysis_fingerprint)
                .bind(&source.analysis_protocol_fingerprint)
                .bind(&source.diagnostic_contract_fingerprint)
                .bind(source.evaluation_run_id)
                .bind(&source.evaluation_input_fingerprint)
                .bind(&source.evaluation_protocol_fingerprint)
                .bind(&source.cohort_fingerprint)
                .bind(&source.dataset_fingerprint)
                .bind(source.snapshot_id)
                .bind(&source.snapshot_fingerprint)
                .bind(&source.coverage_fingerprint)
                .bind(source.comparison_id)
                .bind(&source.comparison_fingerprint)
                .bind(&source.training_configuration_space_fingerprint)
                .bind(to_json(source)?)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
            }
            if let Some(evidence) = &proposal.decision_evidence {
                sqlx::query(
                    "INSERT INTO optimization_decision_evidence \
                     (proposal_id, fingerprint, diagnostic_contract_fingerprint, \
                      coverage_fingerprint, evidence_json) VALUES (?, ?, ?, ?, ?)",
                )
                .bind(proposal.id)
                .bind(&evidence.fingerprint)
                .bind(&evidence.diagnostics.fingerprint)
                .bind(&evidence.source_identity.coverage_fingerprint)
                .bind(to_json(evidence)?)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
                for (index, coverage) in evidence.current_coverage.iter().enumerate() {
                    sqlx::query(
                        "INSERT INTO optimization_evidence_coverage \
                         (proposal_id, cell_key, label, accepted, coverage_index, coverage_json) \
                         VALUES (?, ?, ?, ?, ?, ?)",
                    )
                    .bind(proposal.id)
                    .bind(coverage.cell.key())
                    .bind(&coverage.cell.label)
                    .bind(i64::from(coverage.accepted))
                    .bind(i64::try_from(index).map_err(store_error)?)
                    .bind(to_json(coverage)?)
                    .execute(&mut *transaction)
                    .await
                    .map_err(store_error)?;
                }
            }
            if let Some(allocation) = &proposal.allocation {
                sqlx::query(
                    "INSERT INTO optimization_allocations \
                     (proposal_id, requested_budget, allocated_budget, unallocated_budget, \
                      feasibility, allocation_json) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(proposal.id)
                .bind(i64::from(allocation.requested_budget))
                .bind(i64::from(allocation.allocated_budget))
                .bind(i64::from(allocation.unallocated_budget))
                .bind(feasibility_text(allocation.feasibility))
                .bind(to_json(allocation)?)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
                for (index, issue) in allocation.issues.iter().enumerate() {
                    sqlx::query(
                        "INSERT INTO optimization_constraint_issues \
                         (proposal_id, issue_index, issue_json) VALUES (?, ?, ?)",
                    )
                    .bind(proposal.id)
                    .bind(i64::try_from(index).map_err(store_error)?)
                    .bind(to_json(issue)?)
                    .execute(&mut *transaction)
                    .await
                    .map_err(store_error)?;
                }
            }
            for decision in &proposal.decision_cells {
                sqlx::query(
                    "INSERT INTO optimization_decision_cells \
                     (proposal_id, finding_key, cell_key, label, eligible, final_score, \
                      evidence_fingerprint, decision_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(proposal.id)
                .bind(&decision.finding_key)
                .bind(decision.cell.key())
                .bind(&decision.cell.label)
                .bind(decision.eligibility.eligible)
                .bind(decision.score.final_score)
                .bind(&decision.evidence_fingerprint)
                .bind(to_json(decision)?)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
            }
            for recommendation in &proposal.normalized_recommendations {
                sqlx::query(
                    "INSERT INTO optimization_recommendations \
                     (id, proposal_id, kind, cell_key, label, eligible, final_score, \
                      current_accepted, additional_count, proposed_target, finding_key, \
                      evidence_fingerprint, recommendation_fingerprint, constraints_json, \
                      recommendation_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&recommendation.id)
                .bind(proposal.id)
                .bind(recommendation_kind_text(recommendation.kind))
                .bind(recommendation.cell.key())
                .bind(&recommendation.cell.label)
                .bind(recommendation.eligibility.eligible)
                .bind(recommendation.score.final_score)
                .bind(i64::from(recommendation.current_accepted))
                .bind(i64::from(recommendation.additional_count))
                .bind(i64::from(recommendation.proposed_target))
                .bind(&recommendation.finding_key)
                .bind(&recommendation.evidence_fingerprint)
                .bind(&recommendation.fingerprint)
                .bind(to_json(&recommendation.constraints)?)
                .bind(to_json(recommendation)?)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
            }
            if let Some(set) = &proposal.training_candidate_set {
                let baseline = &set.configuration_space.baseline;
                sqlx::query(
                    "INSERT INTO optimization_training_configuration_spaces \
                     (proposal_id, fingerprint, baseline_training_run_id, \
                      baseline_training_run_fingerprint, baseline_checkpoint_id, \
                      baseline_checkpoint_fingerprint, snapshot_id, snapshot_fingerprint, \
                      backend_name, model_format, choice_count, space_json) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(proposal.id)
                .bind(&set.configuration_space.fingerprint)
                .bind(baseline.training_run_id)
                .bind(&baseline.training_run_fingerprint)
                .bind(baseline.checkpoint_id)
                .bind(&baseline.checkpoint_fingerprint)
                .bind(baseline.snapshot_id)
                .bind(&baseline.snapshot_fingerprint)
                .bind(&baseline.backend_name)
                .bind(&baseline.model_format)
                .bind(i64::try_from(set.configuration_space.choices.len()).map_err(store_error)?)
                .bind(to_json(&set.configuration_space)?)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
                for (index, candidate) in set.candidates.iter().enumerate() {
                    let fields = candidate
                        .differences
                        .iter()
                        .map(|difference| difference.field.as_str())
                        .collect::<Vec<_>>();
                    sqlx::query(
                        "INSERT INTO optimization_training_candidates \
                         (id, proposal_id, configuration_space_fingerprint, candidate_index, \
                          changed_field_count, changed_fields_json, candidate_fingerprint, \
                          candidate_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    )
                    .bind(&candidate.id)
                    .bind(proposal.id)
                    .bind(&candidate.configuration_space_fingerprint)
                    .bind(i64::try_from(index).map_err(store_error)?)
                    .bind(i64::try_from(candidate.differences.len()).map_err(store_error)?)
                    .bind(to_json(&fields)?)
                    .bind(&candidate.fingerprint)
                    .bind(to_json(candidate)?)
                    .execute(&mut *transaction)
                    .await
                    .map_err(store_error)?;
                }
            }
            for recommendation in &proposal.review_only_recommendations {
                sqlx::query(
                    "INSERT INTO optimization_review_only_recommendations \
                     (id, proposal_id, cell_key, label, finding_key, finding_fingerprint, \
                      evidence_fingerprint, final_score, recommendation_fingerprint, \
                      recommendation_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&recommendation.id)
                .bind(proposal.id)
                .bind(recommendation.cell.key())
                .bind(&recommendation.cell.label)
                .bind(&recommendation.finding_key)
                .bind(&recommendation.finding_fingerprint)
                .bind(&recommendation.evidence_fingerprint)
                .bind(recommendation.score.final_score)
                .bind(&recommendation.fingerprint)
                .bind(to_json(recommendation)?)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
            }
            if let Some(lineage) = &proposal.rebase_lineage {
                sqlx::query(
                    "INSERT INTO optimization_proposal_rebases \
                     (proposal_id, previous_proposal_id, previous_proposal_fingerprint, \
                      previous_coverage_fingerprint, refreshed_coverage_fingerprint, \
                      lineage_json) VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(proposal.id)
                .bind(lineage.previous_proposal_id)
                .bind(&lineage.previous_proposal_fingerprint)
                .bind(&lineage.previous_coverage_fingerprint)
                .bind(&lineage.refreshed_coverage_fingerprint)
                .bind(to_json(lineage)?)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_optimization_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<OptimizationProposal>, OptimizationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ProposalRecord>(
                "SELECT id, analysis_report_id, dataset_id, additional_example_budget, \
                 minimum_support, proposal_json, fingerprint, created_at \
                 FROM optimization_proposals WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(ProposalRecord::into_domain)
            .transpose()
        })
    }

    fn list_optimization_proposals(
        &self,
    ) -> BoxFuture<'_, Result<Vec<OptimizationProposal>, OptimizationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ProposalRecord>(
                "SELECT id, analysis_report_id, dataset_id, additional_example_budget, \
                 minimum_support, proposal_json, fingerprint, created_at FROM optimization_proposals \
                 ORDER BY created_at, id",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?
            .into_iter()
            .map(ProposalRecord::into_domain)
            .collect()
        })
    }

    fn query_optimization_proposals(
        &self,
        query: OptimizationProposalQuery,
    ) -> BoxFuture<'_, Result<Vec<OptimizationProposal>, OptimizationStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, analysis_report_id, dataset_id, additional_example_budget, \
                 minimum_support, proposal_json, fingerprint, created_at \
                 FROM optimization_proposals",
            );
            let mut has_condition = false;
            if let Some(analysis_report_id) = query.analysis_report_id {
                builder
                    .push(" WHERE analysis_report_id = ")
                    .push_bind(analysis_report_id);
                has_condition = true;
            }
            if let Some(dataset_id) = query.dataset_id {
                builder.push(if has_condition { " AND " } else { " WHERE " });
                builder.push("dataset_id = ").push_bind(dataset_id);
            }
            builder.push(" ORDER BY created_at, id LIMIT ");
            builder.push_bind(i64::from(query.limit));
            builder.push(" OFFSET ");
            builder.push_bind(i64::from(query.offset));
            builder
                .build_query_as::<ProposalRecord>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(ProposalRecord::into_domain)
                .collect()
        })
    }

    fn record_proposal_application(
        &self,
        application: &ProposalApplication,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>> {
        let application = application.clone();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO optimization_proposal_applications \
                 (proposal_id, generation_plan_id, approval_review_id, approval_fingerprint, \
                  selected_recommendation_ids_json, verified_coverage_fingerprint, applied_at) \
                  VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(application.proposal_id)
            .bind(application.generation_plan_id)
            .bind(application.approval_review_id)
            .bind(&application.approval_fingerprint)
            .bind(to_json(&application.selected_recommendation_ids)?)
            .bind(&application.verified_coverage_fingerprint)
            .bind(application.applied_at)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_proposal_application(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ProposalApplication>, OptimizationStoreError>> {
        Box::pin(async move {
            let record = sqlx::query_as::<_, ApplicationRecord>(
                "SELECT proposal_id, generation_plan_id, approval_review_id, approval_fingerprint, \
                 selected_recommendation_ids_json, verified_coverage_fingerprint, applied_at \
                 FROM optimization_proposal_applications WHERE proposal_id = ?",
            )
            .bind(proposal_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
            record.map(ApplicationRecord::into_domain).transpose()
        })
    }

    fn apply_proposal_plan(
        &self,
        plan: &GenerationPlan,
        application: &ProposalApplication,
    ) -> BoxFuture<'_, Result<ProposalApplication, OptimizationStoreError>> {
        let plan = plan.clone();
        let application = application.clone();
        Box::pin(async move {
            if plan.id != application.generation_plan_id {
                return Err(OptimizationStoreError(
                    "proposal application references a different generation plan".into(),
                ));
            }
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            sqlx::query(
                "INSERT OR IGNORE INTO generation_plans (id, dataset_id, cells_json, created_at) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(plan.id)
            .bind(plan.dataset_id)
            .bind(to_json(&plan.cells)?)
            .bind(plan.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            let persisted_plan = sqlx::query_as::<_, (Uuid, String, DateTime<Utc>)>(
                "SELECT dataset_id, cells_json, created_at FROM generation_plans WHERE id = ?",
            )
            .bind(plan.id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(store_error)?;
            let persisted_cells: Vec<PlannedCell> = from_json(&persisted_plan.1)?;
            if persisted_plan.0 != plan.dataset_id
                || persisted_cells != plan.cells
                || persisted_plan.2 != plan.created_at
            {
                return Err(OptimizationStoreError(
                    "generation plan identity already exists with different contents".into(),
                ));
            }
            sqlx::query(
                "INSERT OR IGNORE INTO optimization_proposal_applications \
                 (proposal_id, generation_plan_id, approval_review_id, approval_fingerprint, \
                  selected_recommendation_ids_json, verified_coverage_fingerprint, applied_at) \
                  VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(application.proposal_id)
            .bind(application.generation_plan_id)
            .bind(application.approval_review_id)
            .bind(&application.approval_fingerprint)
            .bind(to_json(&application.selected_recommendation_ids)?)
            .bind(&application.verified_coverage_fingerprint)
            .bind(application.applied_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            let existing = sqlx::query_as::<_, ApplicationRecord>(
                "SELECT proposal_id, generation_plan_id, approval_review_id, approval_fingerprint, \
                 selected_recommendation_ids_json, verified_coverage_fingerprint, applied_at \
                 FROM optimization_proposal_applications WHERE proposal_id = ?",
            )
            .bind(application.proposal_id)
            .fetch_one(&mut *transaction)
            .await
            .map_err(store_error)?
            .into_domain()?;
            if existing.generation_plan_id != application.generation_plan_id
                || existing.approval_review_id != application.approval_review_id
                || existing.approval_fingerprint != application.approval_fingerprint
                || existing.selected_recommendation_ids != application.selected_recommendation_ids
                || existing.verified_coverage_fingerprint
                    != application.verified_coverage_fingerprint
            {
                return Err(OptimizationStoreError(
                    "proposal was already applied with different approved inputs".into(),
                ));
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(existing)
        })
    }

    fn create_scenario_group(
        &self,
        group: &OptimizationScenarioGroup,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>> {
        let group = group.clone();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            sqlx::query(
                "INSERT INTO optimization_scenario_groups \
                 (id, source_evidence_fingerprint, fingerprint, group_json, created_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(group.id)
            .bind(&group.source_evidence_fingerprint)
            .bind(&group.fingerprint)
            .bind(to_json(&group)?)
            .bind(group.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            for (index, scenario) in group.scenarios.iter().enumerate() {
                let allocation = scenario.proposal.allocation.as_ref().ok_or_else(|| {
                    OptimizationStoreError("scenario proposal has no allocation".into())
                })?;
                sqlx::query(
                    "INSERT INTO optimization_scenarios \
                     (id, group_id, scenario_index, kind, protocol_fingerprint, proposal_id, \
                      proposal_fingerprint, allocated_budget, unallocated_budget, \
                      concentration_json, scenario_json, fingerprint) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&scenario.id)
                .bind(group.id)
                .bind(i64::try_from(index).map_err(store_error)?)
                .bind(scenario_kind_text(scenario.kind))
                .bind(&scenario.protocol_fingerprint)
                .bind(scenario.proposal.id)
                .bind(&scenario.proposal.fingerprint)
                .bind(i64::from(allocation.allocated_budget))
                .bind(i64::from(allocation.unallocated_budget))
                .bind(to_json(&scenario.concentration)?)
                .bind(to_json(scenario)?)
                .bind(&scenario.fingerprint)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
            }
            for (index, comparison) in group.comparisons.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO optimization_scenario_comparisons \
                     (group_id, comparison_index, left_scenario_id, right_scenario_id, \
                      shared_allocated_cells, union_allocated_cells, allocation_overlap, \
                      comparison_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(group.id)
                .bind(i64::try_from(index).map_err(store_error)?)
                .bind(&comparison.left_scenario_id)
                .bind(&comparison.right_scenario_id)
                .bind(i64::from(comparison.shared_allocated_cells))
                .bind(i64::from(comparison.union_allocated_cells))
                .bind(comparison.allocation_overlap)
                .bind(to_json(comparison)?)
                .execute(&mut *transaction)
                .await
                .map_err(store_error)?;
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn get_scenario_group(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<OptimizationScenarioGroup>, OptimizationStoreError>> {
        Box::pin(async move {
            let record = sqlx::query_as::<_, ScenarioGroupRecord>(
                "SELECT id, source_evidence_fingerprint, fingerprint, group_json, created_at \
                 FROM optimization_scenario_groups WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?;
            record.map(ScenarioGroupRecord::into_domain).transpose()
        })
    }

    fn list_scenario_groups(
        &self,
    ) -> BoxFuture<'_, Result<Vec<OptimizationScenarioGroup>, OptimizationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ScenarioGroupRecord>(
                "SELECT id, source_evidence_fingerprint, fingerprint, group_json, created_at \
                 FROM optimization_scenario_groups ORDER BY created_at, id",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?
            .into_iter()
            .map(ScenarioGroupRecord::into_domain)
            .collect()
        })
    }

    fn append_proposal_review(
        &self,
        review: &ProposalReviewRecord,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>> {
        let review = review.clone();
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(store_error)?;
            sqlx::query(
                "INSERT INTO optimization_proposal_reviews \
                 (id, proposal_id, proposal_fingerprint, state, note, superseding_proposal_id, \
                  campaign_id, fingerprint, review_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(review.id)
            .bind(review.proposal_id)
            .bind(&review.proposal_fingerprint)
            .bind(review_state_text(review.state))
            .bind(&review.note)
            .bind(review.superseding_proposal_id)
            .bind(review.campaign_id)
            .bind(&review.fingerprint)
            .bind(to_json(&review)?)
            .bind(review.created_at)
            .execute(&mut *transaction)
            .await
            .map_err(store_error)?;
            for (index, recommendation_id) in review.selected_recommendation_ids.iter().enumerate()
            {
                let statement =
                    if review.state == ProposalReviewState::AcceptedTrainingExperimentCandidate {
                        "INSERT INTO optimization_proposal_review_training_selections \
                     (review_id, proposal_id, candidate_id, selection_index) VALUES (?, ?, ?, ?)"
                    } else if review.state == ProposalReviewState::AcceptedReviewOnlyCandidates {
                        "INSERT INTO optimization_proposal_review_advisory_selections \
                     (review_id, proposal_id, recommendation_id, selection_index) \
                     VALUES (?, ?, ?, ?)"
                    } else {
                        "INSERT INTO optimization_proposal_review_selections \
                     (review_id, proposal_id, recommendation_id, selection_index) \
                     VALUES (?, ?, ?, ?)"
                    };
                sqlx::query(statement)
                    .bind(review.id)
                    .bind(review.proposal_id)
                    .bind(recommendation_id)
                    .bind(i64::try_from(index).map_err(store_error)?)
                    .execute(&mut *transaction)
                    .await
                    .map_err(store_error)?;
            }
            transaction.commit().await.map_err(store_error)?;
            Ok(())
        })
    }

    fn query_proposal_reviews(
        &self,
        query: ProposalReviewQuery,
    ) -> BoxFuture<'_, Result<Vec<ProposalReviewRecord>, OptimizationStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, proposal_id, proposal_fingerprint, state, note, \
                 superseding_proposal_id, campaign_id, fingerprint, review_json, created_at \
                 FROM optimization_proposal_reviews",
            );
            let mut condition = false;
            if let Some(proposal_id) = query.proposal_id {
                builder.push(" WHERE proposal_id = ").push_bind(proposal_id);
                condition = true;
            }
            if let Some(state) = query.state {
                builder.push(if condition { " AND " } else { " WHERE " });
                builder.push("state = ").push_bind(review_state_text(state));
            }
            builder.push(" ORDER BY created_at, id LIMIT ");
            builder.push_bind(i64::from(query.limit));
            builder.push(" OFFSET ");
            builder.push_bind(i64::from(query.offset));
            builder
                .build_query_as::<ProposalReviewRecordRow>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(ProposalReviewRecordRow::into_domain)
                .collect()
        })
    }

    fn create_campaign(
        &self,
        campaign: &OptimizationCampaign,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>> {
        let campaign = campaign.clone();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO optimization_campaigns \
                 (id, proposal_id, proposal_fingerprint, approval_review_id, \
                  approval_fingerprint, baseline_analysis_report_id, \
                  baseline_analysis_fingerprint, baseline_evaluation_run_id, \
                  baseline_comparison_id, dataset_id, source_snapshot_id, \
                  source_snapshot_fingerprint, fingerprint, campaign_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(campaign.id)
            .bind(campaign.proposal_id)
            .bind(&campaign.proposal_fingerprint)
            .bind(campaign.approval_review_id)
            .bind(&campaign.approval_fingerprint)
            .bind(campaign.baseline_analysis_report_id)
            .bind(&campaign.baseline_analysis_fingerprint)
            .bind(campaign.baseline_evaluation_run_id)
            .bind(campaign.baseline_comparison_id)
            .bind(campaign.dataset_id)
            .bind(campaign.source_snapshot_id)
            .bind(&campaign.source_snapshot_fingerprint)
            .bind(&campaign.fingerprint)
            .bind(to_json(&campaign)?)
            .bind(campaign.created_at)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_campaign(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<OptimizationCampaign>, OptimizationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, CampaignRecord>(
                "SELECT id, proposal_id, approval_review_id, fingerprint, campaign_json, \
                 created_at FROM optimization_campaigns WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(CampaignRecord::into_domain)
            .transpose()
        })
    }

    fn query_campaigns(
        &self,
        query: CampaignQuery,
    ) -> BoxFuture<'_, Result<Vec<OptimizationCampaign>, OptimizationStoreError>> {
        Box::pin(async move {
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, proposal_id, approval_review_id, fingerprint, campaign_json, \
                 created_at FROM optimization_campaigns",
            );
            if let Some(proposal_id) = query.proposal_id {
                builder.push(" WHERE proposal_id = ").push_bind(proposal_id);
            }
            builder.push(" ORDER BY created_at, id LIMIT ");
            builder.push_bind(i64::from(query.limit));
            builder.push(" OFFSET ");
            builder.push_bind(i64::from(query.offset));
            builder
                .build_query_as::<CampaignRecord>()
                .fetch_all(&self.pool)
                .await
                .map_err(store_error)?
                .into_iter()
                .map(CampaignRecord::into_domain)
                .collect()
        })
    }

    fn append_campaign_link(
        &self,
        link: &CampaignArtifactLink,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>> {
        let link = link.clone();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO optimization_campaign_links \
                 (id, campaign_id, artifact_kind, artifact_id, artifact_fingerprint, \
                  details_json, fingerprint, link_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(link.id)
            .bind(link.campaign_id)
            .bind(campaign_artifact_kind_text(link.artifact_kind))
            .bind(link.artifact_id)
            .bind(&link.artifact_fingerprint)
            .bind(to_json(&link.details)?)
            .bind(&link.fingerprint)
            .bind(to_json(&link)?)
            .bind(link.created_at)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn list_campaign_links(
        &self,
        campaign_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<CampaignArtifactLink>, OptimizationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, CampaignLinkRecord>(
                "SELECT id, campaign_id, artifact_kind, artifact_id, fingerprint, link_json, \
                 created_at FROM optimization_campaign_links WHERE campaign_id = ? \
                 ORDER BY created_at, id",
            )
            .bind(campaign_id)
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?
            .into_iter()
            .map(CampaignLinkRecord::into_domain)
            .collect()
        })
    }

    fn create_campaign_outcome(
        &self,
        outcome: &CampaignOutcomeAssessment,
    ) -> BoxFuture<'_, Result<(), OptimizationStoreError>> {
        let outcome = outcome.clone();
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO optimization_campaign_outcomes \
                 (id, campaign_id, comparison_id, comparison_fingerprint, classification, \
                  policy_fingerprint, constraints_realized, fingerprint, outcome_json, \
                  created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(outcome.id)
            .bind(outcome.campaign_id)
            .bind(outcome.comparison_id)
            .bind(&outcome.comparison_fingerprint)
            .bind(outcome_classification_text(outcome.classification))
            .bind(&outcome.policy_fingerprint)
            .bind(outcome.constraints_realized)
            .bind(&outcome.fingerprint)
            .bind(to_json(&outcome)?)
            .bind(outcome.created_at)
            .execute(&self.pool)
            .await
            .map_err(store_error)?;
            Ok(())
        })
    }

    fn get_campaign_outcome(
        &self,
        campaign_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CampaignOutcomeAssessment>, OptimizationStoreError>> {
        Box::pin(async move {
            sqlx::query_as::<_, CampaignOutcomeRecord>(
                "SELECT id, campaign_id, comparison_id, fingerprint, outcome_json, created_at \
                 FROM optimization_campaign_outcomes WHERE campaign_id = ?",
            )
            .bind(campaign_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(CampaignOutcomeRecord::into_domain)
            .transpose()
        })
    }
}

#[derive(Debug, FromRow)]
struct CampaignRecord {
    id: Uuid,
    proposal_id: Uuid,
    approval_review_id: Uuid,
    fingerprint: String,
    campaign_json: String,
    created_at: DateTime<Utc>,
}

impl CampaignRecord {
    fn into_domain(self) -> Result<OptimizationCampaign, OptimizationStoreError> {
        let campaign: OptimizationCampaign =
            serde_json::from_str(&self.campaign_json).map_err(store_error)?;
        if campaign.id != self.id
            || campaign.proposal_id != self.proposal_id
            || campaign.approval_review_id != self.approval_review_id
            || campaign.fingerprint != self.fingerprint
            || campaign.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "campaign columns do not match its immutable payload".into(),
            ));
        }
        Ok(campaign)
    }
}

#[derive(Debug, FromRow)]
struct CampaignLinkRecord {
    id: Uuid,
    campaign_id: Uuid,
    artifact_kind: String,
    artifact_id: Uuid,
    fingerprint: String,
    link_json: String,
    created_at: DateTime<Utc>,
}

impl CampaignLinkRecord {
    fn into_domain(self) -> Result<CampaignArtifactLink, OptimizationStoreError> {
        let link: CampaignArtifactLink =
            serde_json::from_str(&self.link_json).map_err(store_error)?;
        if link.id != self.id
            || link.campaign_id != self.campaign_id
            || campaign_artifact_kind_text(link.artifact_kind) != self.artifact_kind
            || link.artifact_id != self.artifact_id
            || link.fingerprint != self.fingerprint
            || link.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "campaign link columns do not match its immutable payload".into(),
            ));
        }
        Ok(link)
    }
}

#[derive(Debug, FromRow)]
struct CampaignOutcomeRecord {
    id: Uuid,
    campaign_id: Uuid,
    comparison_id: Uuid,
    fingerprint: String,
    outcome_json: String,
    created_at: DateTime<Utc>,
}

impl CampaignOutcomeRecord {
    fn into_domain(self) -> Result<CampaignOutcomeAssessment, OptimizationStoreError> {
        let outcome: CampaignOutcomeAssessment =
            serde_json::from_str(&self.outcome_json).map_err(store_error)?;
        if outcome.id != self.id
            || outcome.campaign_id != self.campaign_id
            || outcome.comparison_id != self.comparison_id
            || outcome.fingerprint != self.fingerprint
            || outcome.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "campaign outcome columns do not match its immutable payload".into(),
            ));
        }
        Ok(outcome)
    }
}

#[derive(Debug, FromRow)]
struct ProposalRecord {
    id: Uuid,
    analysis_report_id: Uuid,
    dataset_id: Uuid,
    additional_example_budget: i64,
    minimum_support: i64,
    proposal_json: String,
    fingerprint: Option<String>,
    created_at: DateTime<Utc>,
}

impl ProposalRecord {
    fn into_domain(self) -> Result<OptimizationProposal, OptimizationStoreError> {
        let proposal: OptimizationProposal =
            serde_json::from_str(&self.proposal_json).map_err(store_error)?;
        if proposal.id != self.id
            || proposal.analysis_report_id != self.analysis_report_id
            || proposal.dataset_id != self.dataset_id
            || u32::try_from(self.additional_example_budget).map_err(store_error)?
                != proposal.additional_example_budget
            || to_u64(self.minimum_support)? != proposal.minimum_support
            || self
                .fingerprint
                .as_deref()
                .is_some_and(|fingerprint| proposal.fingerprint != fingerprint)
            || proposal.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "optimization proposal columns do not match its immutable payload".into(),
            ));
        }
        Ok(proposal)
    }
}

#[derive(Debug, FromRow)]
struct ApplicationRecord {
    proposal_id: Uuid,
    generation_plan_id: Uuid,
    approval_review_id: Option<Uuid>,
    approval_fingerprint: Option<String>,
    selected_recommendation_ids_json: String,
    verified_coverage_fingerprint: Option<String>,
    applied_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct ScenarioGroupRecord {
    id: Uuid,
    source_evidence_fingerprint: String,
    fingerprint: String,
    group_json: String,
    created_at: DateTime<Utc>,
}

impl ScenarioGroupRecord {
    fn into_domain(self) -> Result<OptimizationScenarioGroup, OptimizationStoreError> {
        let group: OptimizationScenarioGroup =
            serde_json::from_str(&self.group_json).map_err(store_error)?;
        if group.id != self.id
            || group.source_evidence_fingerprint != self.source_evidence_fingerprint
            || group.fingerprint != self.fingerprint
            || group.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "optimization scenario-group columns do not match its immutable payload".into(),
            ));
        }
        Ok(group)
    }
}

#[derive(Debug, FromRow)]
struct ProposalReviewRecordRow {
    id: Uuid,
    proposal_id: Uuid,
    proposal_fingerprint: String,
    state: String,
    note: Option<String>,
    superseding_proposal_id: Option<Uuid>,
    campaign_id: Option<Uuid>,
    fingerprint: String,
    review_json: String,
    created_at: DateTime<Utc>,
}

impl ProposalReviewRecordRow {
    fn into_domain(self) -> Result<ProposalReviewRecord, OptimizationStoreError> {
        let review: ProposalReviewRecord =
            serde_json::from_str(&self.review_json).map_err(store_error)?;
        if review.id != self.id
            || review.proposal_id != self.proposal_id
            || review.proposal_fingerprint != self.proposal_fingerprint
            || review_state_text(review.state) != self.state
            || review.note != self.note
            || review.superseding_proposal_id != self.superseding_proposal_id
            || review.campaign_id != self.campaign_id
            || review.fingerprint != self.fingerprint
            || review.created_at != self.created_at
        {
            return Err(OptimizationStoreError(
                "optimization review columns do not match its immutable payload".into(),
            ));
        }
        Ok(review)
    }
}

impl ApplicationRecord {
    fn into_domain(self) -> Result<ProposalApplication, OptimizationStoreError> {
        Ok(ProposalApplication {
            proposal_id: self.proposal_id,
            generation_plan_id: self.generation_plan_id,
            approval_review_id: self.approval_review_id,
            approval_fingerprint: self.approval_fingerprint,
            selected_recommendation_ids: serde_json::from_str(
                &self.selected_recommendation_ids_json,
            )
            .map_err(store_error)?,
            verified_coverage_fingerprint: self.verified_coverage_fingerprint,
            applied_at: self.applied_at,
        })
    }
}

fn to_i64(value: u64) -> Result<i64, OptimizationStoreError> {
    i64::try_from(value).map_err(store_error)
}

fn to_u64(value: i64) -> Result<u64, OptimizationStoreError> {
    u64::try_from(value).map_err(store_error)
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, OptimizationStoreError> {
    serde_json::to_string(value).map_err(store_error)
}

fn from_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, OptimizationStoreError> {
    serde_json::from_str(value).map_err(store_error)
}

fn non_empty(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn scoring_policy_text(policy: ScoringPolicy) -> &'static str {
    match policy {
        ScoringPolicy::ErrorCount => "error_count",
        ScoringPolicy::ErrorRate => "error_rate",
        ScoringPolicy::ErrorRateLift => "error_rate_lift",
        ScoringPolicy::HighConfidenceErrorSeverity => "high_confidence_error_severity",
        ScoringPolicy::MarginalErrorCoverage => "marginal_error_coverage",
        ScoringPolicy::ComparisonRegression => "comparison_regression",
        ScoringPolicy::ConservativeComposite => "conservative_composite",
    }
}

fn recommendation_kind_text(kind: RecommendationKind) -> &'static str {
    match kind {
        RecommendationKind::DataGeneration => "data_generation",
        RecommendationKind::TrainingConfiguration => "training_configuration",
        RecommendationKind::ReviewOnly => "review_only",
    }
}

fn feasibility_text(feasibility: AllocationFeasibility) -> &'static str {
    match feasibility {
        AllocationFeasibility::Feasible => "feasible",
        AllocationFeasibility::Infeasible => "infeasible",
    }
}

fn scenario_kind_text(kind: ScenarioKind) -> &'static str {
    match kind {
        ScenarioKind::Conservative => "conservative",
        ScenarioKind::ErrorVolume => "error_volume",
        ScenarioKind::HighConfidenceRegression => "high_confidence_regression",
        ScenarioKind::BalancedPerLabel => "balanced_per_label",
    }
}

fn review_state_text(state: ProposalReviewState) -> &'static str {
    match state {
        ProposalReviewState::Open => "open",
        ProposalReviewState::ApprovedForPlanCreation => "approved_for_plan_creation",
        ProposalReviewState::Rejected => "rejected",
        ProposalReviewState::Superseded => "superseded",
        ProposalReviewState::PartiallyAccepted => "partially_accepted",
        ProposalReviewState::AcceptedTrainingExperimentCandidate => {
            "accepted_training_experiment_candidate"
        }
        ProposalReviewState::AcceptedReviewOnlyCandidates => "accepted_review_only_candidates",
        ProposalReviewState::CompletedAwaitingOutcomeAssessment => {
            "completed_awaiting_outcome_assessment"
        }
    }
}

fn campaign_artifact_kind_text(kind: CampaignArtifactKind) -> &'static str {
    match kind {
        CampaignArtifactKind::GenerationPlan => "generation_plan",
        CampaignArtifactKind::GenerationJob => "generation_job",
        CampaignArtifactKind::DatasetSnapshot => "dataset_snapshot",
        CampaignArtifactKind::TrainingRun => "training_run",
        CampaignArtifactKind::TrainingCheckpoint => "training_checkpoint",
        CampaignArtifactKind::CandidateEvaluation => "candidate_evaluation",
        CampaignArtifactKind::EvaluationComparison => "evaluation_comparison",
        CampaignArtifactKind::FollowUpAnalysis => "follow_up_analysis",
    }
}

fn outcome_classification_text(classification: OutcomeClassification) -> &'static str {
    match classification {
        OutcomeClassification::Improved => "improved",
        OutcomeClassification::Regressed => "regressed",
        OutcomeClassification::Mixed => "mixed",
        OutcomeClassification::Inconclusive => "inconclusive",
    }
}

fn store_error(error: impl std::fmt::Display) -> OptimizationStoreError {
    OptimizationStoreError(error.to_string())
}
