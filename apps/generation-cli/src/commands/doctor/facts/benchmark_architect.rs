use std::collections::BTreeSet;

use benchmark_architect_core::{
    lifecycle::{BenchmarkArchitectRunState, BenchmarkArchitectToolCallState},
    ports::BenchmarkArchitectStore,
};
use uuid::Uuid;

use super::super::*;

pub(in crate::commands::doctor) async fn benchmark_architect_facts_check(
    store: &SqliteStore,
) -> DoctorCheck {
    let result: anyhow::Result<(usize, usize, usize, usize)> = async {
        let run_ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM benchmark_architect_runs ORDER BY created_at, id",
        )
        .fetch_all(store.pool())
        .await?;
        let mut evidence_count = 0_usize;
        let mut proposal_count = 0_usize;
        for run_id in &run_ids {
            let run = BenchmarkArchitectStore::get_run(store, *run_id)
                .await?
                .with_context(|| format!("benchmark architect run {run_id} is missing"))?;
            let brief = BenchmarkArchitectStore::get_brief(store, run.brief_id)
                .await?
                .with_context(|| {
                    format!("benchmark architect brief {} is missing", run.brief_id)
                })?;
            anyhow::ensure!(
                brief.reproduce_fingerprint()? == brief.fingerprint
                    && run.reproduce_specification_fingerprint()? == run.specification_fingerprint
                    && run.brief_fingerprint == brief.fingerprint,
                "benchmark architect run {run_id} has invalid pinned inputs"
            );
            let calls = BenchmarkArchitectStore::list_tool_calls(store, *run_id).await?;
            for (index, call) in calls.iter().enumerate() {
                anyhow::ensure!(
                    call.run_id == *run_id && call.sequence == index as u32 + 1,
                    "benchmark architect run {run_id} has invalid tool-call order"
                );
                anyhow::ensure!(
                    call.state != BenchmarkArchitectToolCallState::Started
                        || run.state == BenchmarkArchitectRunState::Running,
                    "terminal benchmark architect run {run_id} has an open tool call"
                );
            }
            let call_ids = calls.iter().map(|call| call.id).collect::<BTreeSet<_>>();
            let evidence = BenchmarkArchitectStore::list_evidence(store, *run_id).await?;
            evidence_count += evidence.len();
            anyhow::ensure!(
                evidence
                    .iter()
                    .all(|item| call_ids.contains(&item.tool_call_id)),
                "benchmark architect run {run_id} has evidence without its tool call"
            );
            if let Some(proposal) =
                BenchmarkArchitectStore::latest_proposal_for_run(store, *run_id).await?
            {
                proposal_count += 1;
                anyhow::ensure!(
                    proposal.run_id == run.id
                        && proposal.brief_id == brief.id
                        && run.state == BenchmarkArchitectRunState::AwaitingReview,
                    "benchmark architecture proposal {} has invalid run provenance",
                    proposal.id
                );
            }
        }

        let review_ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM benchmark_architecture_reviews ORDER BY proposal_id, rowid",
        )
        .fetch_all(store.pool())
        .await?;
        for id in &review_ids {
            BenchmarkArchitectStore::get_review(store, *id)
                .await?
                .with_context(|| format!("benchmark architecture review {id} is missing"))?;
        }
        let handoff_ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM benchmark_acquisition_handoffs ORDER BY created_at, id",
        )
        .fetch_all(store.pool())
        .await?;
        for id in &handoff_ids {
            BenchmarkArchitectStore::get_handoff_by_id(store, *id)
                .await?
                .with_context(|| format!("benchmark acquisition handoff {id} is missing"))?;
        }
        Ok((
            run_ids.len(),
            evidence_count,
            proposal_count,
            handoff_ids.len(),
        ))
    }
    .await;
    match result {
        Ok((runs, evidence, proposals, handoffs)) => pass(
            "benchmark_architect_facts",
            format!(
                "verified {runs} run(s), {evidence} evidence item(s), {proposals} proposal(s), and {handoffs} acquisition handoff(s)"
            ),
        ),
        Err(error) => fail("benchmark_architect_facts", error.to_string()),
    }
}
