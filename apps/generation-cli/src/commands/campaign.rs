use std::collections::BTreeMap;

use analysis_core::ports::AnalysisStore;
use anyhow::Context;
use dataset_core::ports::SnapshotStore;
use evaluation_core::ports::EvaluationStore;
use generation_core::ports::{JobStore, PlanStore, RowStore};
use optimization_core::{
    campaigns::{
        CampaignArtifactLink, OutcomeAssessmentPolicy, assess_campaign_outcome, create_campaign,
        link_candidate_evaluation, link_checkpoint, link_comparison, link_follow_up_analysis,
        link_generation_job, link_generation_plan, link_snapshot, link_training_run,
    },
    ports::{CampaignQuery, OptimizationStore, ProposalReviewQuery},
};
use synthetic_data_sqlite::SqliteStore;
use training_core::ports::TrainingStore;

use crate::cli::{CampaignCommand, CampaignLinkArgs};

pub async fn execute(command: CampaignCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        CampaignCommand::Create {
            proposal_id,
            approval_id,
        } => {
            let proposal = store
                .get_optimization_proposal(proposal_id)
                .await?
                .with_context(|| format!("optimization proposal not found: {proposal_id}"))?;
            let approval = store
                .query_proposal_reviews(ProposalReviewQuery {
                    proposal_id: Some(proposal_id),
                    state: None,
                    limit: 10_000,
                    offset: 0,
                })
                .await?
                .into_iter()
                .find(|review| review.id == approval_id)
                .with_context(|| format!("proposal review not found: {approval_id}"))?;
            let campaign = create_campaign(&proposal, &approval)?;
            store.create_campaign(&campaign).await?;
            crate::presentation::print(&campaign)?;
        }
        CampaignCommand::List { proposal_id, page } => {
            let campaigns = store
                .query_campaigns(CampaignQuery {
                    proposal_id,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&campaigns, campaigns.len(), page)?;
        }
        CampaignCommand::Show { id } => {
            let campaign = require_campaign(store, id).await?;
            crate::presentation::print(&serde_json::json!({
                "campaign": campaign,
                "links": store.list_campaign_links(id).await?,
                "outcome": store.get_campaign_outcome(id).await?,
            }))?;
        }
        CampaignCommand::Link(args) => link(args, store).await?,
        CampaignCommand::Assess {
            id,
            comparison_id,
            minimum_accuracy_delta,
            minimum_macro_f1_delta,
            allow_non_significant,
        } => {
            if let Some(existing) = store.get_campaign_outcome(id).await? {
                crate::presentation::print(&serde_json::json!({
                    "outcome": existing,
                    "already_assessed": true,
                }))?;
                return Ok(());
            }
            let campaign = require_campaign(store, id).await?;
            let proposal = store
                .get_optimization_proposal(campaign.proposal_id)
                .await?
                .with_context(|| {
                    format!("optimization proposal not found: {}", campaign.proposal_id)
                })?;
            let comparison = store
                .get_comparison(comparison_id)
                .await?
                .with_context(|| format!("evaluation comparison not found: {comparison_id}"))?;
            let links = store.list_campaign_links(id).await?;
            let coverage = accepted_coverage(store, campaign.dataset_id).await?;
            let outcome = assess_campaign_outcome(
                &campaign,
                &proposal,
                &links,
                &comparison,
                &coverage,
                OutcomeAssessmentPolicy {
                    minimum_accuracy_delta,
                    minimum_macro_f1_delta,
                    require_significance: !allow_non_significant,
                },
            )?;
            store.create_campaign_outcome(&outcome).await?;
            crate::presentation::print(&serde_json::json!({
                "outcome": outcome,
                "already_assessed": false,
            }))?;
        }
    }
    Ok(())
}

async fn link(args: CampaignLinkArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let supplied = [
        args.generation_plan_id,
        args.generation_job_id,
        args.snapshot_id,
        args.training_run_id,
        args.checkpoint_id,
        args.evaluation_run_id,
        args.comparison_id,
        args.analysis_report_id,
    ]
    .into_iter()
    .flatten()
    .count();
    if supplied != 1 {
        anyhow::bail!("exactly one artifact ID option is required");
    }
    let campaign = require_campaign(store, args.id).await?;
    let links = store.list_campaign_links(args.id).await?;
    let link = if let Some(id) = args.generation_plan_id {
        let plan = store
            .get_plan(id)
            .await?
            .with_context(|| format!("generation plan not found: {id}"))?;
        let application = store
            .get_proposal_application(campaign.proposal_id)
            .await?
            .context("campaign proposal has not been applied")?;
        link_generation_plan(&campaign, &plan, &application)?
    } else if let Some(id) = args.generation_job_id {
        let job = store
            .get_job(id)
            .await?
            .with_context(|| format!("generation job not found: {id}"))?;
        link_generation_job(&campaign, &links, &job)?
    } else if let Some(id) = args.snapshot_id {
        let snapshot = store
            .get_snapshot(id)
            .await?
            .with_context(|| format!("dataset snapshot not found: {id}"))?;
        let members = store.list_snapshot_members(id).await?;
        link_snapshot(&campaign, &links, &snapshot, &members)?
    } else if let Some(id) = args.training_run_id {
        let run = store
            .get_training_run(id)
            .await?
            .with_context(|| format!("training run not found: {id}"))?;
        link_training_run(&campaign, &links, &run)?
    } else if let Some(id) = args.checkpoint_id {
        let checkpoint = store
            .get_checkpoint(id)
            .await?
            .with_context(|| format!("training checkpoint not found: {id}"))?;
        link_checkpoint(&campaign, &links, &checkpoint)?
    } else if let Some(id) = args.evaluation_run_id {
        let evaluation = store
            .get_evaluation_run(id)
            .await?
            .with_context(|| format!("evaluation run not found: {id}"))?;
        link_candidate_evaluation(&campaign, &links, &evaluation)?
    } else if let Some(id) = args.comparison_id {
        let comparison = store
            .get_comparison(id)
            .await?
            .with_context(|| format!("evaluation comparison not found: {id}"))?;
        link_comparison(&campaign, &links, &comparison)?
    } else if let Some(id) = args.analysis_report_id {
        let report = store
            .get_analysis_report(id)
            .await?
            .with_context(|| format!("analysis report not found: {id}"))?;
        link_follow_up_analysis(&campaign, &links, &report)?
    } else {
        anyhow::bail!("exactly one artifact ID is required");
    };
    persist_link(store, &links, link).await
}

async fn persist_link(
    store: &SqliteStore,
    existing: &[CampaignArtifactLink],
    link: CampaignArtifactLink,
) -> anyhow::Result<()> {
    if let Some(existing) = existing.iter().find(|existing| {
        existing.artifact_kind == link.artifact_kind && existing.artifact_id == link.artifact_id
    }) {
        crate::presentation::print(&serde_json::json!({
            "link": existing,
            "already_linked": true,
        }))?;
        return Ok(());
    }
    store.append_campaign_link(&link).await?;
    crate::presentation::print(&serde_json::json!({
        "link": link,
        "already_linked": false,
    }))?;
    Ok(())
}

async fn require_campaign(
    store: &SqliteStore,
    id: uuid::Uuid,
) -> anyhow::Result<optimization_core::campaigns::OptimizationCampaign> {
    store
        .get_campaign(id)
        .await?
        .with_context(|| format!("optimization campaign not found: {id}"))
}

async fn accepted_coverage(
    store: &SqliteStore,
    dataset_id: uuid::Uuid,
) -> anyhow::Result<BTreeMap<String, u32>> {
    Ok(store
        .dataset_cell_counts(dataset_id)
        .await?
        .into_iter()
        .map(|(key, counts)| (key, counts.accepted))
        .collect())
}
