use std::collections::BTreeMap;

use dataset_architect_core::{
    lifecycle::{ArchitectRunState, ArchitectToolCallState},
    ports::ArchitectStore,
    proposal::{ArchitectProposalReview, DatasetArchitectureApplication},
};
use generation_core::ports::PlanStore;
use uuid::Uuid;

use super::super::*;

pub(in crate::commands::doctor) async fn architect_facts_check(store: &SqliteStore) -> DoctorCheck {
    let result: anyhow::Result<(usize, usize, usize)> = async {
        let run_ids = sqlx::query_scalar::<_, Uuid>("SELECT id FROM dataset_architect_runs ORDER BY id")
            .fetch_all(store.pool()).await?;
        let mut proposals = 0_usize;
        for run_id in &run_ids {
            let run = ArchitectStore::get_run(store, *run_id).await?.with_context(|| format!("architect run {run_id} is missing"))?;
            let brief = ArchitectStore::get_brief(store, run.brief_id).await?.with_context(|| format!("architect brief {} is missing", run.brief_id))?;
            anyhow::ensure!(brief.reproduce_fingerprint()? == brief.fingerprint
                && run.reproduce_specification_fingerprint()? == run.specification_fingerprint
                && run.brief_fingerprint == brief.fingerprint,
                "architect run {run_id} has invalid pinned inputs");
            let calls = ArchitectStore::list_tool_calls(store, *run_id).await?;
            for (index, call) in calls.iter().enumerate() {
                anyhow::ensure!(call.run_id == *run_id && call.sequence == index as u32 + 1,
                    "architect run {run_id} has invalid tool-call order");
                anyhow::ensure!(call.state != ArchitectToolCallState::Started || run.state == ArchitectRunState::Running,
                    "terminal architect run {run_id} has an open tool call");
            }
            if let Some(proposal) = ArchitectStore::latest_proposal_for_run(store, *run_id).await? {
                proposals += 1;
                anyhow::ensure!(proposal.reproduce_fingerprint()? == proposal.fingerprint
                    && proposal.run_id == run.id
                    && proposal.brief_fingerprint == brief.fingerprint,
                    "architecture proposal {} has invalid provenance", proposal.id);
            }
        }
        verify_reviews(store).await?;
        let application_values = sqlx::query_scalar::<_, String>("SELECT application_json FROM dataset_architecture_applications ORDER BY created_at, id")
            .fetch_all(store.pool()).await?;
        for value in &application_values {
            let application: DatasetArchitectureApplication = serde_json::from_str(value)?;
            let persisted = ArchitectStore::get_application(store, application.proposal_id).await?.context("architecture application is missing")?;
            let plan = store.get_plan(application.plan_id).await?.context("applied architecture plan is missing")?;
            let context = store.generation_strategy_context_for_plan(plan.id).await?.context("applied generation strategy context is missing")?;
            anyhow::ensure!(persisted == application
                && application.reproduce_fingerprint()? == application.fingerprint
                && application.plan_fingerprint == artifact_core::fingerprint(&plan)?
                && application.strategy_context_fingerprint == context.fingerprint
                && context.reproduce_fingerprint()? == context.fingerprint,
                "architecture application {} has invalid handoff provenance", application.id);
        }
        Ok((run_ids.len(), proposals, application_values.len()))
    }.await;
    match result {
        Ok((runs, proposals, applications)) => pass(
            "architect_facts",
            format!(
                "verified {runs} architect run(s), {proposals} proposal(s), and {applications} application(s)"
            ),
        ),
        Err(error) => fail("architect_facts", error.to_string()),
    }
}

async fn verify_reviews(store: &SqliteStore) -> anyhow::Result<()> {
    let values = sqlx::query_scalar::<_, String>(
        "SELECT review_json FROM dataset_architecture_reviews ORDER BY proposal_id, rowid",
    )
    .fetch_all(store.pool())
    .await?;
    let mut previous = BTreeMap::<Uuid, Uuid>::new();
    for value in values {
        let review: ArchitectProposalReview = serde_json::from_str(&value)?;
        anyhow::ensure!(
            review.reproduce_fingerprint()? == review.fingerprint
                && review.predecessor_id == previous.get(&review.proposal_id).copied(),
            "architecture review {} has invalid fingerprint or predecessor",
            review.id
        );
        previous.insert(review.proposal_id, review.id);
    }
    Ok(())
}
