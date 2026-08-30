use super::super::*;

pub(in crate::commands::doctor) async fn optimization_facts_check(
    store: &SqliteStore,
) -> DoctorCheck {
    let proposals = match store.list_optimization_proposals().await {
        Ok(proposals) => proposals,
        Err(error) => return fail("optimization_facts", error.to_string()),
    };
    let mut failures = Vec::new();
    for proposal in &proposals {
        if let Some(source) = &proposal.source_identity {
            if let Err(error) = verify_constrained_proposal(proposal) {
                failures.push(format!("{}: {error}", proposal.id));
                continue;
            }
            match store.get_analysis_report(source.analysis_report_id).await {
                Ok(Some(report))
                    if report.fingerprint == source.analysis_fingerprint
                        && report.protocol_fingerprint == source.analysis_protocol_fingerprint => {}
                Ok(Some(_)) => {
                    failures.push(format!("{}: analysis identity mismatch", proposal.id))
                }
                Ok(None) => failures.push(format!("{}: source analysis missing", proposal.id)),
                Err(error) => failures.push(format!("{}: {error}", proposal.id)),
            }
            match store.get_dataset(source.dataset_id).await {
                Ok(Some(dataset))
                    if artifact_core::fingerprint(&dataset).ok().as_deref()
                        == Some(source.dataset_fingerprint.as_str()) => {}
                Ok(Some(_)) => {
                    failures.push(format!("{}: dataset fingerprint mismatch", proposal.id))
                }
                Ok(None) => failures.push(format!("{}: source dataset missing", proposal.id)),
                Err(error) => failures.push(format!("{}: {error}", proposal.id)),
            }
            match store.get_snapshot(source.snapshot_id).await {
                Ok(Some(snapshot))
                    if snapshot.fingerprint == source.snapshot_fingerprint
                        && snapshot.source_dataset_id == source.dataset_id => {}
                Ok(Some(_)) => {
                    failures.push(format!("{}: snapshot identity mismatch", proposal.id))
                }
                Ok(None) => failures.push(format!("{}: source snapshot missing", proposal.id)),
                Err(error) => failures.push(format!("{}: {error}", proposal.id)),
            }
            let normalized_counts = [
                (
                    "decisions",
                    "optimization_decision_cells",
                    proposal.decision_cells.len(),
                ),
                (
                    "data recommendations",
                    "optimization_recommendations",
                    proposal.normalized_recommendations.len(),
                ),
                (
                    "review recommendations",
                    "optimization_review_only_recommendations",
                    proposal.review_only_recommendations.len(),
                ),
                (
                    "coverage cells",
                    "optimization_evidence_coverage",
                    proposal
                        .decision_evidence
                        .as_ref()
                        .map_or(0, |evidence| evidence.current_coverage.len()),
                ),
            ];
            for (name, table, expected) in normalized_counts {
                let sql = format!("SELECT COUNT(*) FROM {table} WHERE proposal_id = ?");
                let actual = sqlx::query_scalar::<_, i64>(&sql)
                    .bind(proposal.id)
                    .fetch_one(store.pool())
                    .await
                    .unwrap_or(-1);
                if usize::try_from(actual).ok() != Some(expected) {
                    failures.push(format!("{}: normalized {name} differ", proposal.id));
                }
            }
            if let Some(set) = &proposal.training_candidate_set {
                let count = sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM optimization_training_candidates WHERE proposal_id = ?",
                )
                .bind(proposal.id)
                .fetch_one(store.pool())
                .await
                .unwrap_or(-1);
                if usize::try_from(count).ok() != Some(set.candidates.len()) {
                    failures.push(format!(
                        "{}: normalized training candidates differ",
                        proposal.id
                    ));
                }
                if store
                    .get_training_run(set.configuration_space.baseline.training_run_id)
                    .await
                    .ok()
                    .flatten()
                    .is_none()
                    || store
                        .get_checkpoint(set.configuration_space.baseline.checkpoint_id)
                        .await
                        .ok()
                        .flatten()
                        .is_none()
                {
                    failures.push(format!(
                        "{}: training baseline artifact missing",
                        proposal.id
                    ));
                }
            }
            if let Some(lineage) = &proposal.rebase_lineage {
                match store
                    .get_optimization_proposal(lineage.previous_proposal_id)
                    .await
                {
                    Ok(Some(previous))
                        if previous.fingerprint == lineage.previous_proposal_fingerprint => {}
                    _ => failures.push(format!("{}: rebase predecessor mismatch", proposal.id)),
                }
            }
        }
        let reviews = match store
            .query_proposal_reviews(ProposalReviewQuery {
                proposal_id: Some(proposal.id),
                state: None,
                limit: u32::MAX,
                offset: 0,
            })
            .await
        {
            Ok(reviews) => reviews,
            Err(error) => {
                failures.push(format!("{}: {error}", proposal.id));
                Vec::new()
            }
        };
        for review in &reviews {
            if let Err(error) = review.validate(proposal) {
                failures.push(format!("{} review {}: {error}", proposal.id, review.id));
            }
        }
        if let Ok(Some(application)) = store.get_proposal_application(proposal.id).await {
            if let Some(review_id) = application.approval_review_id {
                let approval = reviews.iter().find(|review| review.id == review_id);
                if approval.is_none_or(|review| {
                    application.approval_fingerprint.as_deref() != Some(review.fingerprint.as_str())
                        || review
                            .selected_data_recommendation_ids(proposal)
                            .ok()
                            .as_ref()
                            != Some(&application.selected_recommendation_ids)
                }) {
                    failures.push(format!("{}: approved application mismatch", proposal.id));
                }
            } else if proposal.source_identity.is_some() {
                failures.push(format!(
                    "{}: decision-grade application lacks approval",
                    proposal.id
                ));
            }
        }
    }

    let scenario_groups = match store.list_scenario_groups().await {
        Ok(groups) => groups,
        Err(error) => {
            failures.push(error.to_string());
            Vec::new()
        }
    };
    for group in &scenario_groups {
        if let Err(error) = group.validate() {
            failures.push(format!("scenario group {}: {error}", group.id));
        }
        let scenarios = sqlx::query_scalar::<_, String>(
            "SELECT scenario_json FROM optimization_scenarios \
             WHERE group_id = ? ORDER BY scenario_index",
        )
        .bind(group.id)
        .fetch_all(store.pool())
        .await
        .ok()
        .and_then(|rows| {
            rows.into_iter()
                .map(|json| serde_json::from_str::<OptimizationScenario>(&json).ok())
                .collect::<Option<Vec<_>>>()
        });
        if scenarios.as_ref() != Some(&group.scenarios) {
            failures.push(format!(
                "scenario group {}: normalized scenarios differ",
                group.id
            ));
        }
        let comparisons = sqlx::query_scalar::<_, String>(
            "SELECT comparison_json FROM optimization_scenario_comparisons \
             WHERE group_id = ? ORDER BY comparison_index",
        )
        .bind(group.id)
        .fetch_all(store.pool())
        .await
        .ok()
        .and_then(|rows| {
            rows.into_iter()
                .map(|json| serde_json::from_str::<ScenarioPairComparison>(&json).ok())
                .collect::<Option<Vec<_>>>()
        });
        if comparisons.as_ref() != Some(&group.comparisons) {
            failures.push(format!(
                "scenario group {}: normalized comparisons differ",
                group.id
            ));
        }
    }

    let campaigns = match store
        .query_campaigns(CampaignQuery {
            proposal_id: None,
            limit: u32::MAX,
            offset: 0,
        })
        .await
    {
        Ok(campaigns) => campaigns,
        Err(error) => {
            failures.push(error.to_string());
            Vec::new()
        }
    };
    for campaign in &campaigns {
        if let Err(error) = campaign.validate() {
            failures.push(format!("campaign {}: {error}", campaign.id));
        }
        let proposal = store
            .get_optimization_proposal(campaign.proposal_id)
            .await
            .ok()
            .flatten();
        let approval = store
            .query_proposal_reviews(ProposalReviewQuery {
                proposal_id: Some(campaign.proposal_id),
                state: None,
                limit: u32::MAX,
                offset: 0,
            })
            .await
            .ok()
            .and_then(|reviews| {
                reviews
                    .into_iter()
                    .find(|review| review.id == campaign.approval_review_id)
            });
        match (proposal.as_ref(), approval.as_ref()) {
            (Some(proposal), Some(approval)) => {
                if let Err(error) = campaign.validate_decision(proposal, approval) {
                    failures.push(format!("campaign {} decision: {error}", campaign.id));
                }
            }
            _ => failures.push(format!(
                "campaign {}: proposal or approval missing",
                campaign.id
            )),
        }
        let links = match store.list_campaign_links(campaign.id).await {
            Ok(links) => {
                for link in &links {
                    if let Err(error) = link.validate(campaign) {
                        failures.push(format!("campaign link {}: {error}", link.id));
                    }
                    if let Err(error) = verify_campaign_link(store, campaign, &links, link).await {
                        failures.push(format!("campaign link {}: {error}", link.id));
                    }
                }
                links
            }
            Err(error) => {
                failures.push(format!("campaign {}: {error}", campaign.id));
                Vec::new()
            }
        };
        if let Ok(Some(outcome)) = store.get_campaign_outcome(campaign.id).await {
            match store.get_comparison(outcome.comparison_id).await {
                Ok(Some(comparison)) => {
                    if let Some(proposal) = proposal.as_ref() {
                        let coverage = store
                            .dataset_cell_counts(campaign.dataset_id)
                            .await
                            .unwrap_or_default()
                            .into_iter()
                            .map(|(key, counts)| (key, counts.accepted))
                            .collect();
                        if let Err(error) = outcome.validate_reproduction(
                            campaign,
                            proposal,
                            &links,
                            &comparison,
                            &coverage,
                        ) {
                            failures.push(format!("outcome {}: {error}", outcome.id));
                        }
                    } else {
                        failures.push(format!("outcome {}: proposal missing", outcome.id));
                    }
                }
                _ => failures.push(format!("outcome {}: comparison missing", outcome.id)),
            }
        }
    }
    if failures.is_empty() {
        pass(
            "optimization_facts",
            format!(
                "{} proposal(s), {} scenario group(s), and {} campaign(s) verified",
                proposals.len(),
                scenario_groups.len(),
                campaigns.len()
            ),
        )
    } else {
        fail("optimization_facts", failures.join("; "))
    }
}

async fn verify_campaign_link(
    store: &SqliteStore,
    campaign: &OptimizationCampaign,
    links: &[CampaignArtifactLink],
    actual: &CampaignArtifactLink,
) -> Result<(), String> {
    let expected = match actual.artifact_kind {
        CampaignArtifactKind::GenerationPlan => {
            let plan = required_artifact(store.get_plan(actual.artifact_id).await, "plan")?;
            let application = required_artifact(
                store.get_proposal_application(campaign.proposal_id).await,
                "proposal application",
            )?;
            link_generation_plan(campaign, &plan, &application)
        }
        CampaignArtifactKind::GenerationJob => {
            let job = required_artifact(store.get_job(actual.artifact_id).await, "job")?;
            link_generation_job(campaign, links, &job)
        }
        CampaignArtifactKind::DatasetSnapshot => {
            let snapshot =
                required_artifact(store.get_snapshot(actual.artifact_id).await, "snapshot")?;
            let members = store
                .list_snapshot_members(snapshot.id)
                .await
                .map_err(|error| error.to_string())?;
            link_snapshot(campaign, links, &snapshot, &members)
        }
        CampaignArtifactKind::TrainingRun => {
            let run = required_artifact(
                store.get_training_run(actual.artifact_id).await,
                "training run",
            )?;
            link_training_run(campaign, links, &run)
        }
        CampaignArtifactKind::TrainingCheckpoint => {
            let checkpoint =
                required_artifact(store.get_checkpoint(actual.artifact_id).await, "checkpoint")?;
            link_checkpoint(campaign, links, &checkpoint)
        }
        CampaignArtifactKind::CandidateEvaluation => {
            let evaluation = required_artifact(
                store.get_evaluation_run(actual.artifact_id).await,
                "evaluation run",
            )?;
            link_candidate_evaluation(campaign, links, &evaluation)
        }
        CampaignArtifactKind::EvaluationComparison => {
            let comparison =
                required_artifact(store.get_comparison(actual.artifact_id).await, "comparison")?;
            link_comparison(campaign, links, &comparison)
        }
        CampaignArtifactKind::FollowUpAnalysis => {
            let report = required_artifact(
                store.get_analysis_report(actual.artifact_id).await,
                "analysis report",
            )?;
            link_follow_up_analysis(campaign, links, &report)
        }
    }
    .map_err(|error| error.to_string())?;
    if expected.artifact_kind != actual.artifact_kind
        || expected.artifact_id != actual.artifact_id
        || expected.artifact_fingerprint != actual.artifact_fingerprint
        || expected.details != actual.details
    {
        return Err("persisted link does not reproduce from its artifact".into());
    }
    Ok(())
}

fn required_artifact<T, E: std::fmt::Display>(
    result: Result<Option<T>, E>,
    name: &str,
) -> Result<T, String> {
    result
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("{name} is missing"))
}
