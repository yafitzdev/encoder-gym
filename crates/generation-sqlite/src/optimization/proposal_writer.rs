use optimization_core::{domain::OptimizationProposal, ports::OptimizationStoreError};
use sqlx::SqliteConnection;

use super::{
    feasibility_text, non_empty, recommendation_kind_text, scoring_policy_text, store_error,
    to_i64, to_json,
};

pub(super) async fn insert_proposal(
    connection: &mut SqliteConnection,
    proposal: &OptimizationProposal,
) -> Result<(), OptimizationStoreError> {
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
    .bind(&proposal.fingerprint)
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
    .execute(&mut *connection)
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
        .execute(&mut *connection)
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
        .execute(&mut *connection)
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
        .execute(&mut *connection)
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
            .execute(&mut *connection)
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
        .execute(&mut *connection)
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
            .execute(&mut *connection)
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
        .execute(&mut *connection)
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
        .execute(&mut *connection)
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
        .execute(&mut *connection)
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
            .execute(&mut *connection)
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
        .execute(&mut *connection)
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
        .execute(&mut *connection)
        .await
        .map_err(store_error)?;
    }
    Ok(())
}
