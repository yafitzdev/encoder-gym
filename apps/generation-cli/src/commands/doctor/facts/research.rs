use std::collections::{BTreeMap, BTreeSet};

use research_core::{
    lifecycle::{ResearchRunState, ToolCallState},
    ports::ResearchStore,
    profile::{AuthenticityProfile, ProfileBinding, ProfileReview},
};
use uuid::Uuid;

use super::super::*;

pub(in crate::commands::doctor) async fn research_facts_check(store: &SqliteStore) -> DoctorCheck {
    let result: anyhow::Result<(usize, usize, usize)> = async {
        let run_ids = sqlx::query_scalar::<_, Uuid>("SELECT id FROM research_runs ORDER BY id")
            .fetch_all(store.pool())
            .await?;
        let mut evidence_count = 0_usize;
        let mut profile_count = 0_usize;
        for run_id in &run_ids {
            let run = store
                .get_run(*run_id)
                .await?
                .with_context(|| format!("research run {run_id} is missing"))?;
            let brief = store
                .get_brief(run.brief_id)
                .await?
                .with_context(|| format!("research brief {} is missing", run.brief_id))?;
            anyhow::ensure!(
                brief.reproduce_fingerprint()? == brief.fingerprint
                    && run.reproduce_specification_fingerprint()? == run.specification_fingerprint
                    && run.brief_fingerprint == brief.fingerprint,
                "research run {run_id} has an invalid brief or specification identity"
            );
            let calls = store.list_tool_calls(*run_id).await?;
            for (index, call) in calls.iter().enumerate() {
                anyhow::ensure!(
                    call.run_id == *run_id && call.sequence == index as u32 + 1,
                    "research run {run_id} has invalid tool-call order"
                );
                anyhow::ensure!(
                    call.state != ToolCallState::Started || run.state == ResearchRunState::Running,
                    "terminal research run {run_id} has an open tool call"
                );
            }
            let evidence = store.list_evidence(*run_id).await?;
            evidence_count += evidence.len();
            let evidence_ids = evidence.iter().map(|item| item.id).collect::<BTreeSet<_>>();
            let call_ids = calls.iter().map(|call| call.id).collect::<BTreeSet<_>>();
            anyhow::ensure!(
                evidence
                    .iter()
                    .all(|item| call_ids.contains(&item.tool_call_id)),
                "research run {run_id} has evidence without its persisted tool call"
            );
            let claims = store.list_claims(*run_id).await?;
            for claim in &claims {
                anyhow::ensure!(
                    claim.run_id == *run_id
                        && claim.reproduce_fingerprint()? == claim.fingerprint
                        && claim
                            .supporting_evidence_ids
                            .iter()
                            .chain(claim.conflicting_evidence_ids.iter())
                            .all(|id| evidence_ids.contains(id)),
                    "research claim {} has invalid evidence or identity",
                    claim.id
                );
            }
            if let Some(profile) = store.latest_profile_for_run(*run_id).await? {
                profile_count += 1;
                anyhow::ensure!(
                    profile.run_id == *run_id
                        && profile.reproduce_fingerprint()? == profile.fingerprint
                        && profile.claims == claims
                        && profile.evidence_ids.len() == profile.evidence_fingerprints.len(),
                    "authenticity profile {} failed integrity",
                    profile.id
                );
            }
        }

        verify_review_chains(store).await?;
        verify_binding_chains(store).await?;
        Ok((run_ids.len(), evidence_count, profile_count))
    }
    .await;
    match result {
        Ok((runs, evidence, profiles)) => pass(
            "research_facts",
            format!(
                "verified {runs} research run(s), {evidence} evidence item(s), and {profiles} latest profile(s)"
            ),
        ),
        Err(error) => fail("research_facts", error.to_string()),
    }
}

async fn verify_review_chains(store: &SqliteStore) -> anyhow::Result<()> {
    let values = sqlx::query_scalar::<_, String>(
        "SELECT review_json FROM authenticity_profile_reviews ORDER BY profile_id, rowid",
    )
    .fetch_all(store.pool())
    .await?;
    let mut previous = BTreeMap::<Uuid, Uuid>::new();
    for value in values {
        let review: ProfileReview = serde_json::from_str(&value)?;
        anyhow::ensure!(
            review.reproduce_fingerprint()? == review.fingerprint
                && review.predecessor_id == previous.get(&review.profile_id).copied(),
            "profile review {} has an invalid fingerprint or predecessor",
            review.id
        );
        previous.insert(review.profile_id, review.id);
    }
    Ok(())
}

async fn verify_binding_chains(store: &SqliteStore) -> anyhow::Result<()> {
    let values = sqlx::query_scalar::<_, String>(
        "SELECT binding_json FROM authenticity_profile_bindings ORDER BY dataset_id, rowid",
    )
    .fetch_all(store.pool())
    .await?;
    let mut previous = BTreeMap::<Uuid, Uuid>::new();
    for value in values {
        let binding: ProfileBinding = serde_json::from_str(&value)?;
        let profile: AuthenticityProfile = store
            .get_profile(binding.profile_id)
            .await?
            .with_context(|| format!("bound profile {} is missing", binding.profile_id))?;
        let review = store
            .latest_review(binding.profile_id)
            .await?
            .with_context(|| format!("bound profile {} has no review", binding.profile_id))?;
        anyhow::ensure!(
            binding.reproduce_fingerprint()? == binding.fingerprint
                && binding.predecessor_id == previous.get(&binding.dataset_id).copied()
                && binding.profile_fingerprint == profile.fingerprint
                && binding.approval_id == review.id,
            "authenticity binding {} has invalid provenance",
            binding.id
        );
        previous.insert(binding.dataset_id, binding.id);
    }
    Ok(())
}
